use std::{
    collections::VecDeque,
    io::{self, Read},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

const PROCESS_LOG_LIMIT: usize = 100_000;

pub(super) struct ProcessOutput {
    pub(super) code: Option<i32>,
    pub(super) log: String,
}

pub(super) fn run_process(command: Command, cancel: &AtomicBool) -> io::Result<ProcessOutput> {
    if cancel.load(Ordering::Relaxed) {
        return Ok(ProcessOutput {
            code: None,
            log: "Compilation cancelled.".into(),
        });
    }
    let (code, mut output) =
        capture_process(command, cancel, None, PROCESS_LOG_LIMIT).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "A local LaTeX installation with pdflatex is required.",
                )
            } else {
                error
            }
        })?;
    let mut log = String::from_utf8_lossy(output.bytes.make_contiguous()).into_owned();
    if cancel.load(Ordering::Relaxed) {
        log.push_str("\nCompilation cancelled.");
    }
    Ok(ProcessOutput { code, log })
}

pub(super) fn query_process(
    command: Command,
    cancel: &AtomicBool,
    timeout: Duration,
    limit: usize,
) -> io::Result<Vec<u8>> {
    if cancel.load(Ordering::Relaxed) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "SyncTeX query cancelled",
        ));
    }
    let (code, mut output) = capture_process(command, cancel, Some(timeout), limit)?;
    if cancel.load(Ordering::Relaxed) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "SyncTeX query cancelled",
        ));
    }
    if output.truncated {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "SyncTeX output is too large",
        ));
    }
    match code {
        Some(0) => Ok(output.bytes.into()),
        _ => Err(io::Error::other(format!(
            "SyncTeX query failed. {}",
            String::from_utf8_lossy(output.bytes.make_contiguous()).trim()
        ))),
    }
}

fn capture_process(
    mut command: Command,
    cancel: &AtomicBool,
    timeout: Option<Duration>,
    limit: usize,
) -> io::Result<(Option<i32>, CapturedOutput)> {
    let (mut reader, writer) = io::pipe()?;
    command
        .stdin(Stdio::null())
        .stdout(writer.try_clone()?)
        .stderr(writer);
    configure_process(&mut command);
    let mut child = command.spawn()?;
    drop(command);
    let tree = match ProcessTree::attach(&child) {
        Ok(tree) => tree,
        Err(error) => {
            if let Ok(Some(status)) = child.try_wait() {
                return Ok((status.code(), read_tail(&mut reader, limit)?));
            }
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    let started = Instant::now();
    thread::scope(|scope| {
        let output = scope.spawn(move || read_tail(&mut reader, limit));
        let status = loop {
            if cancel.load(Ordering::Relaxed) {
                tree.stop(&mut child);
                break child.wait().map(|_| None);
            }
            if timeout.is_some_and(|timeout| started.elapsed() >= timeout) {
                tree.stop(&mut child);
                let _ = child.wait();
                break Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "SyncTeX query timed out",
                ));
            }
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status.code()),
                Ok(None) => thread::sleep(Duration::from_millis(25)),
                Err(error) => {
                    tree.stop(&mut child);
                    let _ = child.wait();
                    break Err(error);
                }
            }
        };
        drop(tree);
        let output = output
            .join()
            .map_err(|_| io::Error::other("Compiler output reader failed"))??;
        Ok((status?, output))
    })
}

struct CapturedOutput {
    bytes: VecDeque<u8>,
    truncated: bool,
}

fn read_tail(reader: &mut impl Read, limit: usize) -> io::Result<CapturedOutput> {
    let mut buffer = [0u8; 8192];
    let mut bytes = VecDeque::new();
    let mut truncated = false;
    loop {
        let count = reader.read(&mut buffer[..limit.min(8192)])?;
        if count == 0 {
            return Ok(CapturedOutput { bytes, truncated });
        }
        let discard = (bytes.len() + count).saturating_sub(limit);
        truncated |= discard != 0;
        bytes.drain(..discard);
        bytes.extend(&buffer[..count]);
    }
}

#[cfg(windows)]
fn configure_process(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(unix)]
fn configure_process(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    command.process_group(0);
}

#[cfg(windows)]
struct ProcessTree(std::os::windows::io::OwnedHandle);

#[cfg(windows)]
impl ProcessTree {
    fn attach(child: &Child) -> io::Result<Self> {
        use std::{
            mem::size_of_val,
            os::windows::io::{AsRawHandle, FromRawHandle},
            ptr::null,
        };
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };

        // The job is unnamed and owns only the compiler and its descendants.
        let raw = unsafe { CreateJobObjectW(null(), null()) };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        // CreateJobObjectW returns a live handle whose ownership transfers to OwnedHandle.
        let job = unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(raw) };
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // The limits structure and the child process handle remain valid for both calls.
        if unsafe {
            SetInformationJobObject(
                raw,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of_val(&limits) as u32,
            ) == 0
                || AssignProcessToJobObject(raw, child.as_raw_handle()) == 0
        } {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(job))
    }

    fn stop(&self, child: &mut Child) {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::TerminateJobObject;

        // The owned job handle stays valid until ProcessTree is dropped.
        if unsafe { TerminateJobObject(self.0.as_raw_handle(), 1) } == 0 {
            let _ = child.kill();
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct ProcessTree;

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl ProcessTree {
    fn attach(_: &Child) -> io::Result<Self> {
        Ok(Self)
    }

    fn stop(&self, child: &mut Child) {
        use rustix::process::{Pid, Signal, kill_process_group};

        if let Some(group) = Pid::from_raw(child.id() as i32) {
            let _ = kill_process_group(group, Signal::KILL);
        }
        let _ = child.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_before_spawn_does_not_launch_a_process() {
        let output = run_process(
            Command::new("missing-test-compiler"),
            &AtomicBool::new(true),
        )
        .unwrap();
        assert_eq!(output.code, None);
        assert_eq!(output.log, "Compilation cancelled.");
    }

    #[test]
    fn keeps_the_tail_when_compiler_output_exceeds_the_limit() {
        let mut bytes = vec![b'x'; PROCESS_LOG_LIMIT + 1];
        bytes.extend_from_slice(b"final error");
        let mut tail = read_tail(&mut bytes.as_slice(), PROCESS_LOG_LIMIT).unwrap();
        assert_eq!(tail.bytes.len(), PROCESS_LOG_LIMIT);
        assert!(tail.bytes.make_contiguous().ends_with(b"final error"));
        assert!(tail.truncated);
    }
}

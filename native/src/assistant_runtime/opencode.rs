use super::{CANCEL_POLL, CANCELLED, cancelled};
use crate::{
    assistant_launch::OpenCodeCli,
    assistant_protocol::{OpenCodeEvent, OpenCodeEventKind, OpenCodeEvents},
    tex::process::{ProcessTree, configure_process},
};
use serde_json::{Map, Value, json};
use std::{
    collections::{HashMap, VecDeque},
    env,
    ffi::OsString,
    fmt,
    io::{self, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Mutex, OnceLock,
        atomic::AtomicBool,
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

pub use super::CodexUpdate as OpenCodeUpdate;

const HELP_TIMEOUT: Duration = Duration::from_secs(10);
const PREPARE_TIMEOUT: Duration = Duration::from_secs(20);
const HELP_LIMIT: usize = 64_000;
const STDERR_LIMIT: usize = 12_000;
const REPLY_LIMIT: usize = 64 * 1024 * 1024;
const CONFIG_ENV: &str = "OPENCODE_CONFIG_CONTENT";

pub struct OpenCodeTurnRequest<'a> {
    pub cwd: &'a Path,
    pub session_id: Option<&'a str>,
    pub prompt: &'a str,
    pub images: &'a [PathBuf],
    pub model: Option<&'a str>,
    pub variant: Option<&'a str>,
    pub tools: Option<OpenCodeToolConnection<'a>>,
}

pub struct OpenCodeToolConnection<'a> {
    pub url: &'a str,
    pub token: &'a str,
}

#[derive(Debug)]
pub struct OpenCodeTurnReply {
    pub session_id: Option<String>,
    pub text: String,
}

#[derive(Debug)]
pub struct OpenCodeRunError {
    pub session_id: Option<String>,
    pub message: String,
}

impl fmt::Display for OpenCodeRunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for OpenCodeRunError {}

/// Run one turn with the installed OpenCode CLI. Call on a worker thread.
pub fn run_opencode(
    cli: &OpenCodeCli,
    request: OpenCodeTurnRequest<'_>,
    cancel: &AtomicBool,
    mut update: impl FnMut(OpenCodeUpdate<'_>),
) -> Result<OpenCodeTurnReply, OpenCodeRunError> {
    let mut turn = TurnOutput {
        session_id: request
            .session_id
            .filter(|id| !id.is_empty())
            .map(str::to_owned),
        ..TurnOutput::default()
    };
    let result = run(cli, &request, cancel, &mut turn, &mut update);
    match result {
        Ok(()) => Ok(OpenCodeTurnReply {
            session_id: turn.session_id,
            text: turn.text,
        }),
        Err(message) => Err(OpenCodeRunError {
            session_id: turn.session_id,
            message: redact_token(message, request.tools.as_ref()),
        }),
    }
}

fn run(
    cli: &OpenCodeCli,
    request: &OpenCodeTurnRequest<'_>,
    cancel: &AtomicBool,
    turn: &mut TurnOutput,
    update: &mut impl FnMut(OpenCodeUpdate<'_>),
) -> Result<(), String> {
    check_cancelled(cancel)?;
    if !request.cwd.is_absolute() {
        return Err("OpenCode requires an absolute project directory".into());
    }
    let standalone = request.tools.is_some() && supports_standalone(cli, cancel)?;
    let mut command = cli
        .run_command(
            request.cwd,
            turn.session_id.as_deref(),
            request.images,
            request.model,
            request.variant,
            standalone,
        )
        .map_err(|error| error.to_string())?;
    if let Some(tools) = &request.tools {
        let inherited = match env::var(CONFIG_ENV) {
            Ok(value) => value,
            Err(env::VarError::NotPresent) => String::new(),
            Err(env::VarError::NotUnicode(_)) => {
                return Err("Invalid OpenCode CLI configuration".into());
            }
        };
        command.env(CONFIG_ENV, tools_config(&inherited, tools, standalone)?);
    }
    if let (true, Some(session_id)) = (standalone, turn.session_id.as_deref()) {
        prepare_session(cli, request.cwd, session_id, cancel)?;
    }
    check_cancelled(cancel)?;
    let result = execute_turn(command, request.prompt, cancel, turn, update);
    check_cancelled(cancel)?;
    let (status, stderr) = result?;
    turn.check_result(status, &stderr)
}

fn supports_standalone(cli: &OpenCodeCli, cancel: &AtomicBool) -> Result<bool, String> {
    static CACHE: OnceLock<Mutex<HashMap<OsString, bool>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut help = cli.run_help_command();
    let program = help.get_program().to_owned();
    if let Some(supported) = cache
        .lock()
        .ok()
        .and_then(|cache| cache.get(&program).copied())
    {
        return Ok(supported);
    }
    help.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let inspected = (|| -> io::Result<bool> {
        let process = ManagedProcess::spawn(help, cancel)?;
        thread::scope(move |scope| {
            let mut process = process;
            let stdout = process
                .child
                .stdout
                .take()
                .ok_or_else(|| io::Error::other("Missing OpenCode output"))?;
            let output = thread::Builder::new()
                .spawn_scoped(scope, move || read_tail(stdout, HELP_LIMIT))?;
            let status = process.wait_bounded(HELP_TIMEOUT, cancel)?;
            let output = output
                .join()
                .map_err(|_| io::Error::other("OpenCode output reader failed"))??;
            if !status.success() {
                return Err(io::Error::other("OpenCode's help command failed"));
            }
            Ok(String::from_utf8_lossy(&output).contains("--standalone"))
        })
    })();
    check_cancelled(cancel)?;
    let supported = inspected.map_err(|_| "Could not inspect OpenCode CLI capabilities")?;
    if let Ok(mut cache) = cache.lock() {
        cache.insert(program, supported);
    }
    Ok(supported)
}

fn prepare_session(
    cli: &OpenCodeCli,
    cwd: &Path,
    session_id: &str,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let prepared = (|| -> io::Result<ExitStatus> {
        let mut command = cli.prepare_session_command(cwd, session_id)?;
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        ManagedProcess::spawn(command, cancel)?.wait_bounded(PREPARE_TIMEOUT, cancel)
    })();
    check_cancelled(cancel)?;
    if prepared.is_ok_and(|status| status.success()) {
        Ok(())
    } else {
        Err(
            "Could not prepare the OpenCode session for resumption. Retry or start a new chat."
                .into(),
        )
    }
}

fn tools_config(
    inherited: &str,
    tools: &OpenCodeToolConnection<'_>,
    standalone: bool,
) -> Result<String, String> {
    let invalid = |_| "Invalid OpenCode CLI configuration".to_owned();
    let mut config: Map<String, Value> = if inherited.is_empty() {
        Map::new()
    } else {
        serde_json::from_str(inherited).map_err(invalid)?
    };
    let mut mcp = take_object(config.remove("mcp"));
    let mut server = json!({
        "type": "remote",
        "url": tools.url,
        "headers": { "Authorization": format!("Bearer {}", tools.token) },
        "oauth": false,
    });
    if standalone {
        server["codemode"] = json!(false);
        let mut servers = take_object(mcp.remove("servers"));
        mcp.retain(|name, value| {
            let is_legacy_server = matches!(
                value.get("type").and_then(Value::as_str),
                Some("local" | "remote")
            );
            if is_legacy_server {
                servers.insert(name.clone(), value.take());
            }
            !is_legacy_server
        });
        servers.insert("refract".into(), server);
        mcp.insert("servers".into(), Value::Object(servers));
    } else {
        mcp.insert("refract".into(), server);
    }
    config.insert("mcp".into(), Value::Object(mcp));
    serde_json::to_string(&config).map_err(invalid)
}

fn take_object(value: Option<Value>) -> Map<String, Value> {
    match value {
        Some(Value::Object(object)) => object,
        _ => Map::new(),
    }
}

fn redact_token(message: String, tools: Option<&OpenCodeToolConnection<'_>>) -> String {
    match tools.filter(|tools| !tools.token.is_empty()) {
        Some(tools) => message.replace(tools.token, "[redacted]"),
        None => message,
    }
}

fn execute_turn(
    mut command: Command,
    prompt: &str,
    cancel: &AtomicBool,
    turn: &mut TurnOutput,
    update: &mut impl FnMut(OpenCodeUpdate<'_>),
) -> Result<(ExitStatus, Vec<u8>), String> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let process = ManagedProcess::spawn(command, cancel).map_err(|_| {
        if cancelled(cancel) {
            CANCELLED
        } else {
            "Could not start OpenCode"
        }
    })?;
    thread::scope(move |scope| {
        // Drop the process before joining readers. An interrupted turn can leave
        // any of its three pipes blocked until the process tree is stopped.
        let mut process = process;
        let mut stdin = process
            .child
            .stdin
            .take()
            .ok_or("OpenCode input pipe is unavailable")?;
        let stdout = process
            .child
            .stdout
            .take()
            .ok_or("OpenCode output pipe is unavailable")?;
        let stderr = process
            .child
            .stderr
            .take()
            .ok_or("OpenCode error pipe is unavailable")?;
        let (messages, incoming) = mpsc::sync_channel(2);
        thread::Builder::new()
            .spawn_scoped(scope, move || {
                // Exit status and provider events determine whether the turn succeeded.
                let _ = stdin.write_all(prompt.as_bytes());
            })
            .map_err(|_| "Could not start the OpenCode input writer")?;
        thread::Builder::new()
            .spawn_scoped(scope, move || read_events(stdout, messages))
            .map_err(|_| "Could not start the OpenCode output reader")?;
        let diagnostics = thread::Builder::new()
            .spawn_scoped(scope, move || read_tail(stderr, STDERR_LIMIT))
            .map_err(|_| "Could not start the OpenCode error reader")?;
        let status = await_turn(&mut process, incoming, cancel, turn, update)?;
        let stderr = diagnostics
            .join()
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();
        Ok((status, stderr))
    })
}

enum Output {
    Event(OpenCodeEvent),
    End { oversized: bool },
    Unreadable,
}

fn read_events(input: impl Read, messages: SyncSender<Output>) {
    let mut events = OpenCodeEvents::new(BufReader::new(input));
    loop {
        match events.read_next() {
            Ok(Some(event)) => {
                if messages.send(Output::Event(event)).is_err() {
                    return;
                }
            }
            Ok(None) => {
                let _ = messages.send(Output::End {
                    oversized: events.had_oversized_event(),
                });
                return;
            }
            Err(_) => {
                let _ = messages.send(Output::Unreadable);
                return;
            }
        }
    }
}

fn await_turn(
    process: &mut ManagedProcess,
    incoming: Receiver<Output>,
    cancel: &AtomicBool,
    turn: &mut TurnOutput,
    update: &mut impl FnMut(OpenCodeUpdate<'_>),
) -> Result<ExitStatus, String> {
    let mut status = None;
    let mut drained = false;
    let mut stopping = false;
    loop {
        if !stopping && cancelled(cancel) {
            stopping = true;
            // Read any already buffered session event after stopping the child.
            process.stop();
        }
        if !stopping && status.is_none() {
            status = process
                .try_finish()
                .map_err(|_| "Could not read the OpenCode process status")?;
        }
        if drained {
            if stopping {
                return Err(CANCELLED.into());
            }
            if let Some(status) = status {
                return Ok(status);
            }
            thread::sleep(CANCEL_POLL);
            continue;
        }
        match incoming.recv_timeout(CANCEL_POLL) {
            Ok(Output::Event(event)) if stopping => {
                turn.observe_session(event.session_id.as_deref(), update);
            }
            Ok(Output::Event(event)) => turn.observe(event, update)?,
            Ok(Output::End { oversized }) => {
                turn.oversized = oversized;
                drained = true;
            }
            Ok(Output::Unreadable) | Err(RecvTimeoutError::Disconnected) => {
                return Err("Could not read OpenCode output".into());
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

#[derive(Default)]
struct TurnOutput {
    session_id: Option<String>,
    text: String,
    error: Option<String>,
    oversized: bool,
}

impl TurnOutput {
    fn observe(
        &mut self,
        event: OpenCodeEvent,
        update: &mut impl FnMut(OpenCodeUpdate<'_>),
    ) -> Result<(), String> {
        self.observe_session(event.session_id.as_deref(), update);
        if let Some(progress) = event.progress() {
            update(OpenCodeUpdate::Progress(progress));
        }
        match event.kind {
            OpenCodeEventKind::Text(text) => {
                if self.text.len() + text.len() > REPLY_LIMIT {
                    return Err("OpenCode response exceeds the chat history limit".into());
                }
                self.text.push_str(&text);
            }
            OpenCodeEventKind::Error(error) => self.error = Some(error),
            _ => {}
        }
        Ok(())
    }

    fn observe_session(
        &mut self,
        session_id: Option<&str>,
        update: &mut impl FnMut(OpenCodeUpdate<'_>),
    ) {
        if let Some(id) = session_id
            && self.session_id.as_deref() != Some(id)
        {
            self.session_id = Some(id.to_owned());
            update(OpenCodeUpdate::Session(id));
        }
    }

    fn check_result(&mut self, status: ExitStatus, stderr: &[u8]) -> Result<(), String> {
        if let Some(error) = self.error.take() {
            return Err(error);
        }
        if !status.success() {
            let diagnostics = String::from_utf8_lossy(stderr);
            if !diagnostics.trim().is_empty() {
                return Err(diagnostics.trim().to_owned());
            }
            let exit = status.code().map_or_else(
                || "OpenCode stopped unexpectedly".into(),
                |code| format!("OpenCode exited with code {code}"),
            );
            return Err(exit);
        }
        if self.text.is_empty() {
            let message = if self.oversized {
                "OpenCode returned an oversized event"
            } else {
                "OpenCode returned no response"
            };
            return Err(message.into());
        }
        Ok(())
    }
}

fn check_cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancelled(cancel) {
        Err(CANCELLED.into())
    } else {
        Ok(())
    }
}

fn read_tail(mut input: impl Read, limit: usize) -> io::Result<Vec<u8>> {
    let mut buffer = [0; 8192];
    let mut bytes = VecDeque::new();
    loop {
        let count = input.read(&mut buffer[..limit.min(8192)])?;
        if count == 0 {
            return Ok(bytes.into());
        }
        let discard = (bytes.len() + count).saturating_sub(limit);
        bytes.drain(..discard);
        bytes.extend(&buffer[..count]);
    }
}

struct ManagedProcess {
    child: Child,
    tree: Option<ProcessTree>,
}

impl ManagedProcess {
    fn spawn(mut command: Command, cancel: &AtomicBool) -> io::Result<Self> {
        if cancelled(cancel) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, CANCELLED));
        }
        configure_process(&mut command);
        let mut child = command.spawn()?;
        match ProcessTree::attach(&child) {
            Ok(tree) => Ok(Self {
                child,
                tree: Some(tree),
            }),
            Err(_) if child.try_wait().is_ok_and(|status| status.is_some()) => {
                // A short probe can exit before Windows assigns it to the job.
                Ok(Self { child, tree: None })
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(error)
            }
        }
    }

    #[cfg(windows)]
    fn try_finish(&mut self) -> io::Result<Option<ExitStatus>> {
        let status = self.child.try_wait()?;
        if status.is_some() {
            // Closing the owned job also ends descendants that kept a pipe open.
            self.tree = None;
        }
        Ok(status)
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn try_finish(&mut self) -> io::Result<Option<ExitStatus>> {
        use rustix::{
            io::Errno,
            process::{Pid, WaitId, WaitIdOptions, waitid},
        };
        let pid = Pid::from_raw(self.child.id() as i32)
            .ok_or_else(|| io::Error::other("Invalid OpenCode process ID"))?;
        let options = WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT | WaitIdOptions::EXITED;
        match waitid(WaitId::Pid(pid), options) {
            Ok(Some(_)) => {
                // Stop the remaining group before reaping the leader. Its pid
                // cannot be recycled to another process while it is waitable.
                let tree = self
                    .tree
                    .take()
                    .expect("an unreaped process still owns its tree");
                tree.stop(&mut self.child);
                self.child.wait().map(Some)
            }
            Ok(None) | Err(Errno::INTR) => Ok(None),
            Err(error @ Errno::CHILD) => {
                self.tree = None;
                Err(error.into())
            }
            Err(error) => Err(error.into()),
        }
    }

    fn wait_bounded(&mut self, timeout: Duration, cancel: &AtomicBool) -> io::Result<ExitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            if cancelled(cancel) {
                return Err(io::Error::new(io::ErrorKind::Interrupted, CANCELLED));
            }
            if let Some(status) = self.try_finish()? {
                return Ok(status);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "OpenCode command timed out",
                ));
            }
            thread::sleep(CANCEL_POLL.min(remaining));
        }
    }

    fn stop(&mut self) {
        if let Some(tree) = self.tree.take() {
            tree.stop(&mut self.child);
            let _ = self.child.wait();
        }
    }
}

impl Drop for ManagedProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_inherited_configuration_in_both_provider_modes() {
        let config = r#"{"model":"existing","mcp":{"timeout":{"execution":90000},"servers":{"current":{"type":"remote","url":"https://example.com"}},"legacy":{"type":"local","command":["example"]}}}"#;
        let tools = OpenCodeToolConnection {
            url: "http://127.0.0.1:3000/mcp",
            token: "fixture-token",
        };
        let v1: Value =
            serde_json::from_str(&tools_config(config, &tools, false).unwrap()).unwrap();
        assert_eq!(v1["mcp"]["legacy"]["command"][0], "example");
        assert_eq!(
            v1["mcp"]["refract"]["headers"]["Authorization"],
            "Bearer fixture-token"
        );
        let v2: Value = serde_json::from_str(&tools_config(config, &tools, true).unwrap()).unwrap();
        assert_eq!(v2["model"], "existing");
        assert_eq!(v2["mcp"]["timeout"]["execution"], 90000);
        assert_eq!(v2["mcp"]["servers"]["legacy"]["command"][0], "example");
        assert_eq!(
            v2["mcp"]["servers"]["current"]["url"],
            "https://example.com"
        );
        assert_eq!(v2["mcp"]["servers"]["refract"]["codemode"], false);
        assert!(v2["mcp"].get("legacy").is_none());
        assert!(tools_config("broken", &tools, true).is_err());
        assert_eq!(
            redact_token("Provider error: fixture-token".into(), Some(&tools)),
            "Provider error: [redacted]"
        );
    }

    #[test]
    fn accumulates_text_in_order_and_only_reports_new_session_ids() {
        let wire = concat!(
            "{\"sessionID\":\"session-1\",\"type\":\"step_start\"}\n",
            "{\"sessionID\":\"session-1\",\"type\":\"text\",\"part\":{\"text\":\"The figure\"}}\n",
            "{\"type\":\"tool_use\"}\n",
            "{\"type\":\"text\",\"part\":{\"text\":\" is ready.\"}}\n",
        );
        let mut events = OpenCodeEvents::new(wire.as_bytes());
        let mut turn = TurnOutput::default();
        let mut updates = Vec::new();
        while let Some(event) = events.read_next().unwrap() {
            turn.observe(event, &mut |update| {
                updates.push(match update {
                    OpenCodeUpdate::Session(id) => id.to_owned(),
                    OpenCodeUpdate::Progress(progress) => progress.to_owned(),
                });
            })
            .unwrap();
        }
        assert_eq!(turn.text, "The figure is ready.");
        assert_eq!(turn.session_id.as_deref(), Some("session-1"));
        assert_eq!(updates, ["session-1", "Thinking…", "Using a tool…"]);
    }

    #[test]
    fn drains_without_retaining_unbounded_diagnostics() {
        let mut input = io::Cursor::new(b"many diagnostic bytes before the final line");
        assert_eq!(read_tail(&mut input, 10).unwrap().as_slice(), b"final line");
        assert_eq!(input.position(), input.get_ref().len() as u64);
    }
}

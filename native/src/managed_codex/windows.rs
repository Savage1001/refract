use super::{
    RUNTIME_FILES, ReleaseAsset, RuntimeProgress, VERSION, complete, directory, executable_in,
    runtime_directory,
};
use crate::{project_management::is_link, tex::process::configure_process};
use sha2::{Digest, Sha256};
use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

pub(super) fn install(
    account: &Path,
    asset: ReleaseAsset,
    cancel: &AtomicBool,
    mut progress: impl FnMut(RuntimeProgress),
) -> io::Result<PathBuf> {
    check_cancelled(cancel)?;
    let profile = account.parent().ok_or_else(invalid_path)?;
    fs::create_dir_all(profile)?;
    let revision = runtime_directory(account, asset);
    for root in [account, &account.join("runtime"), &revision] {
        match fs::create_dir(root) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        if !directory(root) {
            return Err(invalid_path());
        }
    }
    let lock_path = revision.join("setup.lock");
    if lock_path
        .symlink_metadata()
        .is_ok_and(|info| !info.is_file() || is_link(&info))
    {
        return Err(invalid_path());
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    loop {
        check_cancelled(cancel)?;
        match lock.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) => thread::sleep(Duration::from_millis(100)),
            Err(std::fs::TryLockError::Error(error)) => return Err(error),
        }
    }
    if let Some(executable) = executable_in(account, asset) {
        return Ok(executable);
    }

    let stage = tempfile::Builder::new()
        .prefix("setup-")
        .tempdir_in(&revision)?;
    let archive = stage.path().join("runtime.tar.zst");
    progress(RuntimeProgress::Downloading(0));
    download(asset, &archive, cancel, &mut progress)?;
    progress(RuntimeProgress::Installing);
    let package = stage.path().join("package");
    fs::create_dir(&package)?;
    extract(&archive, &package, cancel)?;
    if !complete(&package) {
        return Err(io::Error::other("The Codex runtime package is incomplete."));
    }
    fs::remove_file(&archive)?;
    check_cancelled(cancel)?;
    let mut marker = tempfile::NamedTempFile::new_in(&revision)?;
    let stage_name = stage
        .path()
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(invalid_path)?;
    marker.write_all(stage_name.as_bytes())?;
    marker.as_file().sync_all()?;
    marker
        .persist(revision.join("active"))
        .map_err(|error| error.error)?;
    let executable = stage.keep().join("package").join(RUNTIME_FILES[0]);
    Ok(executable)
}

fn invalid_path() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Invalid Refract Codex runtime directory",
    )
}

fn download(
    asset: ReleaseAsset,
    archive: &Path,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(RuntimeProgress),
) -> io::Result<()> {
    let client = zed_reqwest::blocking::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(15 * 60))
        .build()
        .map_err(io::Error::other)?;
    let url = format!(
        "https://github.com/openai/codex/releases/download/rust-v{VERSION}/codex-app-server-package-{}-pc-windows-msvc.tar.zst",
        asset.target
    );
    let mut response = client
        .get(url)
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(io::Error::other)?;
    if response
        .content_length()
        .is_some_and(|length| length != asset.bytes)
    {
        return Err(checksum_error());
    }
    let mut file = File::create_new(archive)?;
    let mut digest = Sha256::new();
    let mut total = 0;
    let mut percent = 0;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        check_cancelled(cancel)?;
        let count = response.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > asset.bytes {
            return Err(checksum_error());
        }
        digest.update(&buffer[..count]);
        file.write_all(&buffer[..count])?;
        let next = (total * 100 / asset.bytes).min(99) as u8;
        if next != percent {
            percent = next;
            progress(RuntimeProgress::Downloading(percent));
        }
    }
    if total != asset.bytes || format!("{:x}", digest.finalize()) != asset.sha256 {
        return Err(checksum_error());
    }
    file.sync_all()
}

fn extract(archive: &Path, package: &Path, cancel: &AtomicBool) -> io::Result<()> {
    check_cancelled(cancel)?;
    let root = env::var_os("SystemRoot")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| io::Error::other("Couldn’t locate Windows tar.exe"))?;
    let mut command = Command::new(root.join("System32/tar.exe"));
    configure_process(&mut command);
    let mut child = command
        .args(["--no-same-owner", "-xf"])
        .arg(dunce::simplified(archive))
        .arg("-C")
        .arg(dunce::simplified(package))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(120);
    let result = loop {
        if let Err(error) = check_cancelled(cancel) {
            break Err(error);
        }
        if Instant::now() >= deadline {
            break Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Codex runtime extraction timed out.",
            ));
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(_)) => return Err(io::Error::other("Could not extract the Codex runtime.")),
            Ok(None) => thread::sleep(Duration::from_millis(100)),
            Err(error) => break Err(error),
        }
    };
    let _ = child.kill();
    let _ = child.wait();
    result
}

fn check_cancelled(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Codex setup was cancelled.",
        ));
    }
    Ok(())
}

fn checksum_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Codex runtime checksum verification failed.",
    )
}

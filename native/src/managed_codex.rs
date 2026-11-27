use std::{
    io,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

pub const UNSUPPORTED_PLATFORM: &str = "ChatGPT account mode requires Windows on x64 or ARM64.";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeProgress {
    Downloading(u8),
    Installing,
}

#[derive(Clone, Copy)]
struct ReleaseAsset {
    architecture: &'static str,
    target: &'static str,
    bytes: u64,
    sha256: &'static str,
}

const VERSION: &str = "0.170.0";
const X64: ReleaseAsset = ReleaseAsset {
    architecture: "x64",
    target: "x86_64",
    bytes: 127_199_313,
    sha256: "a409c0bf32e35ba0b0902d235048957503a534153ba3b0a9e5caaef53ad574e0",
};
const ARM64: ReleaseAsset = ReleaseAsset {
    architecture: "arm64",
    target: "aarch64",
    bytes: 118_098_720,
    sha256: "b17d1377a84a1a5b695da414ecb843b6c1951d6b1523e1dd3c07d15077ec61f2",
};

fn platform_asset() -> Option<ReleaseAsset> {
    if cfg!(all(windows, target_arch = "x86_64")) {
        Some(X64)
    } else if cfg!(all(windows, target_arch = "aarch64")) {
        Some(ARM64)
    } else {
        None
    }
}

pub fn executable(account: &Path) -> Option<PathBuf> {
    let asset = platform_asset()?;
    executable_in(account, asset)
}

pub fn install(
    account: &Path,
    cancel: &AtomicBool,
    progress: impl FnMut(RuntimeProgress),
) -> io::Result<PathBuf> {
    let asset = platform_asset()
        .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, UNSUPPORTED_PLATFORM))?;
    #[cfg(all(windows, feature = "codex-account"))]
    {
        windows::install(account, asset, cancel, progress)
    }
    #[cfg(not(all(windows, feature = "codex-account")))]
    {
        let _ = (account, asset, cancel, progress);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            UNSUPPORTED_PLATFORM,
        ))
    }
}

use crate::project_management::is_link;
use std::{fs::File, io::Read};

const RUNTIME_FILES: &[&str] = &[
    "bin/codex-app-server.exe",
    "bin/codex-code-mode-host.exe",
    "bin/codex-exec-server.exe",
    "codex-path/rg.exe",
    "codex-resources/codex-command-runner.exe",
    "codex-resources/codex-windows-sandbox-setup.exe",
];

fn runtime_directory(account: &Path, asset: ReleaseAsset) -> PathBuf {
    account
        .join("runtime")
        .join(format!("{VERSION}-{}", asset.architecture))
}

fn executable_in(account: &Path, asset: ReleaseAsset) -> Option<PathBuf> {
    let revision = runtime_directory(account, asset);
    for root in [account, &account.join("runtime"), &revision] {
        if !directory(root) {
            return None;
        }
    }
    let marker = revision.join("active");
    if !regular_file(&marker) {
        return None;
    }
    let mut name = String::new();
    File::open(marker)
        .ok()?
        .take(129)
        .read_to_string(&mut name)
        .ok()?;
    let suffix = name.strip_prefix("setup-")?;
    if name.len() > 128
        || suffix.is_empty()
        || !suffix.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return None;
    }
    let setup = revision.join(name);
    let package = setup.join("package");
    (directory(&setup) && complete(&package)).then(|| package.join(RUNTIME_FILES[0]))
}

fn complete(package: &Path) -> bool {
    directory(package)
        && ["bin", "codex-path", "codex-resources"]
            .into_iter()
            .all(|folder| directory(&package.join(folder)))
        && RUNTIME_FILES
            .iter()
            .all(|file| regular_file(&package.join(file)))
}

fn directory(path: &Path) -> bool {
    path.symlink_metadata()
        .is_ok_and(|metadata| metadata.is_dir() && !is_link(&metadata))
}

fn regular_file(path: &Path) -> bool {
    path.symlink_metadata()
        .is_ok_and(|metadata| metadata.is_file() && !is_link(&metadata) && metadata.len() > 0)
}

#[cfg(all(windows, feature = "codex-account"))]
mod windows;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn reuses_complete_electron_runtime_layouts_and_rejects_invalid_markers() {
        let profile = tempfile::tempdir().unwrap();
        let account = profile.path().join("codex-account");
        for asset in [X64, ARM64] {
            assert!(
                !asset.target.is_empty()
                    && asset.bytes > 1_000_000
                    && asset.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
                    && asset.sha256.len() == 64
            );
            let revision = runtime_directory(&account, asset);
            let package = revision.join("setup-ElectronABC").join("package");
            for tool in RUNTIME_FILES {
                let path = package.join(tool);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, b"synthetic executable").unwrap();
            }
            let executable = package.join(RUNTIME_FILES[0]);
            let marker = revision.join("active");
            fs::write(&marker, "setup-ElectronABC").unwrap();
            assert_eq!(executable_in(&account, asset), Some(executable.clone()));
            fs::write(&marker, "setup-ElectronABC/../setup-foreign").unwrap();
            assert!(executable_in(&account, asset).is_none());
            fs::write(&marker, "setup-ElectronABC").unwrap();
            fs::remove_file(package.join("codex-path/rg.exe")).unwrap();
            assert!(executable_in(&account, asset).is_none());
        }
    }
}

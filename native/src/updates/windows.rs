use super::{
    Release, UpdateCandidate, UpdateTrack, installed_version, select_windows_update, version_order,
};
use crate::project_management::is_link;
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tempfile::TempPath;
use zed_reqwest::blocking::Client;

const RELEASES_API: &str = "https://api.github.com/repos/Savage1001/refract/releases";
const RELEASE_DOWNLOAD: &str = "https://github.com/Savage1001/refract/releases/download";
const MAX_RELEASE_PAGE_BYTES: u64 = 16 * 1024 * 1024;
const INSTALL_MARKER: &str = "refract-native-installed";

#[derive(Clone)]
pub struct WindowsUpdater {
    profile: PathBuf,
}

pub struct PreparedUpdate {
    candidate: UpdateCandidate,
    installer: TempPath,
}

impl WindowsUpdater {
    pub fn installed(profile: &Path) -> Option<Self> {
        if !cfg!(all(windows, target_arch = "x86_64")) || !is_installed(&env::current_exe().ok()?) {
            return None;
        }
        Some(Self {
            profile: profile.to_path_buf(),
        })
    }

    pub fn check(
        &self,
        track: UpdateTrack,
        cancel: &AtomicBool,
    ) -> io::Result<Option<UpdateCandidate>> {
        let client = client(Duration::from_secs(30))?;
        let current = installed_version();
        let mut latest: Option<UpdateCandidate> = None;
        for page in 1..=100 {
            check_cancelled(cancel)?;
            let response = client
                .get(format!("{RELEASES_API}?per_page=100&page={page}"))
                .header("Accept", "application/vnd.github+json")
                .send()
                .and_then(|response| response.error_for_status())
                .map_err(io::Error::other)?;
            let more = response.headers().get_all("link").iter().any(|value| {
                value
                    .to_str()
                    .is_ok_and(|text| text.contains("rel=\"next\""))
            });
            let mut releases = response.take(MAX_RELEASE_PAGE_BYTES + 1);
            let mut data = Vec::new();
            releases.read_to_end(&mut data)?;
            if data.len() as u64 > MAX_RELEASE_PAGE_BYTES {
                return Err(io::Error::other("GitHub release page is too large"));
            }
            let releases: Vec<Release> = serde_json::from_slice(&data)?;
            if let Some(candidate) = select_windows_update(&releases, track, &current)
                && latest.as_ref().is_none_or(|latest| {
                    version_order(candidate.version(), latest.version()).is_gt()
                })
            {
                latest = Some(candidate);
            }
            if !more {
                return Ok(latest);
            }
        }
        Err(io::Error::other("GitHub release history is too large"))
    }

    pub fn download(
        &self,
        candidate: UpdateCandidate,
        cancel: &AtomicBool,
        progress: impl FnMut(u8),
    ) -> io::Result<PreparedUpdate> {
        check_cancelled(cancel)?;
        fs::create_dir_all(&self.profile)?;
        let cache = self.profile.join("native-updates");
        match fs::create_dir(&cache) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        let metadata = cache.symlink_metadata()?;
        if is_link(&metadata) || !metadata.is_dir() {
            return Err(io::Error::other("Invalid Refract update directory"));
        }
        let mut stage = tempfile::Builder::new()
            .prefix("Refract-Setup-")
            .suffix(".exe")
            .tempfile_in(cache)?;
        let client = client(Duration::from_secs(10 * 60))?;
        let tag = &candidate.tag;
        let name = &candidate.asset_name;
        let response = client
            .get(format!("{RELEASE_DOWNLOAD}/{tag}/{name}"))
            .send()
            .and_then(|response| response.error_for_status())
            .map_err(io::Error::other)?;
        if response
            .content_length()
            .is_some_and(|size| size != candidate.size)
        {
            return Err(io::Error::other(
                "Update installer size does not match its release",
            ));
        }
        copy_verified(response, &mut stage, &candidate, cancel, progress)?;
        stage.as_file().sync_all()?;
        Ok(PreparedUpdate {
            candidate,
            installer: stage.into_temp_path(),
        })
    }
}

impl PreparedUpdate {
    pub fn candidate(&self) -> &UpdateCandidate {
        &self.candidate
    }

    pub fn launch(self) -> io::Result<()> {
        let installer = self.installer.keep().map_err(|error| error.error)?;
        let result = Command::new(&installer)
            .args(["--updated", "--force-run"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if let Err(error) = result {
            let _ = fs::remove_file(&installer);
            return Err(error);
        }
        Ok(())
    }
}

fn is_installed(executable: &Path) -> bool {
    if !executable
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("Refract.exe"))
    {
        return false;
    }
    let marker = executable.with_file_name(INSTALL_MARKER);
    marker
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.is_file() && !is_link(&metadata) && metadata.len() <= 128)
        && fs::read_to_string(marker)
            .is_ok_and(|version| version.trim() == installed_version().to_string())
}

fn client(timeout: Duration) -> io::Result<Client> {
    Client::builder()
        .https_only(true)
        .user_agent("Refract-updater")
        .connect_timeout(Duration::from_secs(15))
        .timeout(timeout)
        .build()
        .map_err(io::Error::other)
}

fn check_cancelled(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Update cancelled",
        ));
    }
    Ok(())
}

fn copy_verified(
    mut source: impl Read,
    mut output: impl Write,
    candidate: &UpdateCandidate,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u8),
) -> io::Result<()> {
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut read = 0;
    let mut last_percent = 0;
    loop {
        check_cancelled(cancel)?;
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        read += count as u64;
        if read > candidate.size {
            return Err(io::Error::other(
                "Update installer exceeds its expected size",
            ));
        }
        let bytes = &buffer[..count];
        digest.update(bytes);
        output.write_all(bytes)?;
        let percent = (read * 100 / candidate.size) as u8;
        if percent > last_percent {
            progress(percent);
            last_percent = percent;
        }
    }
    if read != candidate.size || format!("{:x}", digest.finalize()) != candidate.sha256 {
        return Err(io::Error::other(
            "Update installer checksum or size mismatch",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use semver::Version;

    #[test]
    fn native_updates_are_only_enabled_for_packaged_refract() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("Refract.exe");
        assert!(!is_installed(&executable));
        fs::write(
            root.path().join(INSTALL_MARKER),
            format!("{}\n", installed_version()),
        )
        .unwrap();
        assert!(is_installed(&executable));
        assert!(!is_installed(&root.path().join("refract-native.exe")));
        fs::write(root.path().join(INSTALL_MARKER), b"0.0.0\n").unwrap();
        assert!(!is_installed(&executable));
    }

    #[test]
    fn verifies_the_complete_installer_and_refuses_a_partial_or_cancelled_download() {
        let contents = b"fixture installer";
        let candidate = UpdateCandidate {
            tag: "v1.0.0".into(),
            version: Version::new(1, 0, 0),
            asset_name: "Refract-Native-Setup-1.0.0.exe".into(),
            size: contents.len() as u64,
            sha256: format!("{:x}", Sha256::digest(contents)),
        };
        let cancel = AtomicBool::new(false);
        let mut output = Vec::new();
        let mut percents = Vec::new();
        copy_verified(
            contents.as_slice(),
            &mut output,
            &candidate,
            &cancel,
            |percent| {
                percents.push(percent);
            },
        )
        .unwrap();
        assert_eq!(output, contents);
        assert_eq!(percents, [100]);
        assert!(copy_verified(&contents[..1], io::sink(), &candidate, &cancel, |_| {}).is_err());
        let mut changed = contents.to_vec();
        changed[0] = b'X';
        assert!(
            copy_verified(changed.as_slice(), io::sink(), &candidate, &cancel, |_| {}).is_err()
        );
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(
            copy_verified(contents.as_slice(), io::sink(), &candidate, &cancel, |_| {})
                .unwrap_err()
                .kind(),
            io::ErrorKind::Interrupted
        );
    }
}

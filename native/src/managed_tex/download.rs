use super::{Archive, MAC, Platform, WINDOWS, current_platform, dependency_root, install};
use crate::tex::process::configure_process;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

const MAX_ARCHIVE: u64 = 1024 * 1024 * 1024;

pub fn setup() -> io::Result<PathBuf> {
    let platform = current_platform()?;
    let distribution = match platform {
        Platform::Windows => &WINDOWS,
        Platform::Mac => &MAC,
    };
    let client = zed_reqwest::blocking::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(io::Error::other)?;
    install(
        &dependency_root()?,
        distribution,
        |archive, path| download(&client, archive, path),
        |archive, directory, member| extract(platform, archive, directory, member),
    )
}

fn download(
    client: &zed_reqwest::blocking::Client,
    archive: &Archive,
    path: &Path,
) -> io::Result<()> {
    let mut last = io::Error::other("No TeX download source configured");
    for url in archive.urls {
        match download_from(client, archive, url, path) {
            Ok(()) => return Ok(()),
            Err(error) => last = error,
        }
    }
    Err(last)
}

fn download_from(
    client: &zed_reqwest::blocking::Client,
    archive: &Archive,
    url: &str,
    path: &Path,
) -> io::Result<()> {
    let response = client
        .get(url)
        .send()
        .and_then(zed_reqwest::blocking::Response::error_for_status)
        .map_err(|error| {
            io::Error::other(format!("Couldn’t download {}: {error}", archive.name))
        })?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_ARCHIVE)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "TeX archive is too large",
        ));
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    copy_verified(response, &mut file, archive.sha256)?;
    file.sync_all()
}

fn copy_verified(mut input: impl Read, mut file: impl Write, expected: &str) -> io::Result<()> {
    let mut digest = Sha256::new();
    let mut total = 0;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > MAX_ARCHIVE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "TeX archive is too large",
            ));
        }
        digest.update(&buffer[..count]);
        file.write_all(&buffer[..count])?;
    }
    let actual = format!("{:x}", digest.finalize());
    if actual != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "TeX archive checksum mismatch",
        ));
    }
    Ok(())
}

fn extract(
    platform: Platform,
    archive: &Path,
    directory: &Path,
    member: Option<&str>,
) -> io::Result<()> {
    let program = match platform {
        Platform::Windows => {
            let root = std::env::var_os("SystemRoot")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, "Couldn’t locate Windows tar.exe")
                })?;
            root.join("System32/tar.exe")
        }
        Platform::Mac => PathBuf::from("/usr/bin/tar"),
    };
    if !fs::metadata(&program)?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "System archive tool is unavailable",
        ));
    }
    let mut command = Command::new(program);
    configure_process(&mut command);
    #[cfg(windows)]
    let (archive, directory) = (dunce::simplified(archive), dunce::simplified(directory));
    command
        .arg("--no-same-owner")
        .arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(directory)
        .args(member)
        .stdin(Stdio::null());
    let output = command.output()?;
    if output.status.success() {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "TeX archive extraction failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_accepts_a_matching_archive_checksum() {
        let bytes = b"test archive";
        let checksum = format!("{:x}", Sha256::digest(bytes));
        let mut copied = Vec::new();
        copy_verified(bytes.as_slice(), &mut copied, &checksum).unwrap();
        assert_eq!(copied, bytes);
        assert!(copy_verified(bytes.as_slice(), io::sink(), &"0".repeat(64)).is_err());
    }
}

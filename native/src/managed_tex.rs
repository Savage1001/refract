use crate::project_management::is_link;
use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

#[cfg(feature = "installer")]
mod download;
#[cfg(feature = "installer")]
pub use download::setup;

const REQUIRED_TOOLS: &[&str] = &[
    "pdflatex",
    "xelatex",
    "lualatex",
    "kpsewhich",
    "synctex",
    "bibtex",
    "biber",
];

#[derive(Clone, Copy)]
enum Platform {
    Windows,
    Mac,
}

struct Archive {
    urls: &'static [&'static str],
    sha256: &'static str,
    name: &'static str,
}

struct Distribution {
    platform: Platform,
    bin: &'static str,
    extension: &'static str,
    tex: Archive,
    sync: Archive,
}

const WINDOWS: Distribution = Distribution {
    platform: Platform::Windows,
    bin: "bin/windows",
    extension: ".exe",
    tex: Archive {
        urls: &[
            "https://github.com/rstudio/tinytex-releases/releases/download/v2026.10/TinyTeX-v2026.10.zip",
        ],
        sha256: "dc1d6e63b21af62130ede1dcc4113bffdc184544690a23e3f489d75340f4ff61",
        name: "tex.zip",
    },
    sync: Archive {
        urls: &[
            "https://ctan.math.illinois.edu/systems/texlive/tlnet/archive/synctex.windows.r78097.tar.xz",
            "https://mirrors.mit.edu/CTAN/systems/texlive/tlnet/archive/synctex.windows.r78097.tar.xz",
            "https://mirrors.ctan.org/systems/texlive/tlnet/archive/synctex.windows.r78097.tar.xz",
        ],
        sha256: "09ff70bf3a0de6b1c81c5aa3eb136473fce83314dc407a9ad6c4592376a6a83c",
        name: "sync.tar.xz",
    },
};

#[cfg(feature = "installer")]
const MAC: Distribution = Distribution {
    platform: Platform::Mac,
    bin: "bin/universal-darwin",
    extension: "",
    tex: Archive {
        urls: &[
            "https://github.com/rstudio/tinytex-releases/releases/download/v2026.10/TinyTeX-darwin-v2026.10.tar.xz",
        ],
        sha256: "9962e445b56f06feb2c28f73561e523e89d550e98abf910ba60b0ffb3856921f",
        name: "tex.tar.xz",
    },
    sync: Archive {
        urls: &[
            "https://ctan.math.illinois.edu/systems/texlive/tlnet/archive/synctex.universal-darwin.r78096.tar.xz",
            "https://mirrors.mit.edu/CTAN/systems/texlive/tlnet/archive/synctex.universal-darwin.r78096.tar.xz",
            "https://mirrors.ctan.org/systems/texlive/tlnet/archive/synctex.universal-darwin.r78096.tar.xz",
        ],
        sha256: "1f247bb78b7ddd43e32b4adc7995c41c594b0eef012a2abc545deadf9c2f7f47",
        name: "sync.tar.xz",
    },
};

pub fn write_setup_error(message: &str) -> io::Result<PathBuf> {
    let platform = current_platform()?;
    let root = checked_root(&dependency_root()?, platform)?;
    let mut staged = tempfile::NamedTempFile::new_in(&root)?;
    writeln!(staged, "{message}")?;
    let path = root.join("setup-error.log");
    staged.persist(&path).map_err(|error| error.error)?;
    Ok(path)
}

fn current_platform() -> io::Result<Platform> {
    if cfg!(windows) {
        Ok(Platform::Windows)
    } else if cfg!(target_os = "macos") {
        Ok(Platform::Mac)
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Install TeX through your Linux package manager",
        ))
    }
}

fn dependency_root() -> io::Result<PathBuf> {
    match current_platform()? {
        Platform::Windows => {
            let local = env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        "Couldn’t locate the per-user Refract directory",
                    )
                })?;
            Ok(local.canonicalize()?.join("Refract/dependencies"))
        }
        Platform::Mac => Ok(PathBuf::from("/Library/Refract/dependencies")),
    }
}

fn checked_root(root: &Path, platform: Platform) -> io::Result<PathBuf> {
    let parent = root.parent().ok_or_else(unsafe_root)?;
    for directory in [parent, root] {
        match fs::create_dir(directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        let info = directory.symlink_metadata()?;
        if !info.is_dir() || is_link(&info) {
            return Err(unsafe_root());
        }
        #[cfg(unix)]
        if matches!(platform, Platform::Mac) {
            use std::os::unix::fs::MetadataExt;
            if info.uid() != 0 || info.mode() & 0o022 != 0 {
                return Err(unsafe_root());
            }
        }
        #[cfg(not(unix))]
        let _ = platform;
    }
    root.canonicalize()
}

fn unsafe_root() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "Unsafe Refract dependency directory",
    )
}

fn complete(target: &Path, distribution: &Distribution) -> bool {
    if !target
        .symlink_metadata()
        .is_ok_and(|info| info.is_dir() && !is_link(&info))
    {
        return false;
    }
    let Ok(canonical) = target.canonicalize() else {
        return false;
    };
    REQUIRED_TOOLS.iter().all(|tool| {
        let file = target
            .join(distribution.bin)
            .join(format!("{tool}{}", distribution.extension));
        let Ok(file) = file.canonicalize() else {
            return false;
        };
        if !file.starts_with(&canonical) || !file.is_file() {
            return false;
        }
        #[cfg(unix)]
        if matches!(distribution.platform, Platform::Mac) {
            use std::os::unix::fs::PermissionsExt;
            return file
                .metadata()
                .is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0);
        }
        true
    })
}

fn lock_setup(root: &Path) -> io::Result<File> {
    let path = root.join("setup.lock");
    if path
        .symlink_metadata()
        .is_ok_and(|info| !info.is_file() || is_link(&info))
    {
        return Err(unsafe_root());
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.lock()?;
    Ok(file)
}

fn install(
    root: &Path,
    distribution: &Distribution,
    mut download: impl FnMut(&Archive, &Path) -> io::Result<()>,
    mut extract: impl FnMut(&Path, &Path, Option<&str>) -> io::Result<()>,
) -> io::Result<PathBuf> {
    let root = checked_root(root, distribution.platform)?;
    let _lock = lock_setup(&root)?;
    let target = root.join("TinyTeX");
    if complete(&target, distribution) {
        return Ok(target.join(distribution.bin));
    }
    let stage = tempfile::Builder::new()
        .prefix("setup-")
        .tempdir_in(&root)?;
    let tex = stage.path().join(distribution.tex.name);
    download(&distribution.tex, &tex)?;
    extract(&tex, stage.path(), None)?;
    let extracted = stage.path().join("TinyTeX");
    let metadata = extracted.symlink_metadata()?;
    if !metadata.is_dir() || is_link(&metadata) {
        return Err(unsafe_root());
    }
    let sync = stage.path().join(distribution.sync.name);
    download(&distribution.sync, &sync)?;
    let member = format!("{}/synctex{}", distribution.bin, distribution.extension);
    extract(&sync, &extracted, Some(&member))?;
    if !complete(&extracted, distribution) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Downloaded TeX is missing required tools",
        ));
    }

    let recovery = root.join(format!(
        "previous-{}",
        stage
            .path()
            .file_name()
            .expect("stage directory has a name")
            .to_string_lossy()
    ));
    let preserved = match rename_directory(&target, &recovery) {
        Ok(()) => true,
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error),
    };
    if let Err(error) = rename_directory(&extracted, &target) {
        if preserved && !target.exists() {
            let _ = rename_directory(&recovery, &target);
        }
        return Err(error);
    }
    Ok(target.join(distribution.bin))
}

fn rename_directory(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(windows)]
    for delay in [100, 200, 400, 800, 1600, 3200, 5000] {
        match fs::rename(from, to) {
            Err(error) if matches!(error.raw_os_error(), Some(5 | 32 | 33)) => {
                std::thread::sleep(std::time::Duration::from_millis(delay))
            }
            result => return result,
        }
    }
    fs::rename(from, to)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn installs_verified_archives_and_reuses_the_completed_toolchain() {
        let profile = tempfile::tempdir().unwrap();
        let root = profile.path().join("Refract/dependencies");
        let target = root.join("TinyTeX");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("user-notes"), b"retained").unwrap();
        let downloads = RefCell::new(Vec::new());
        let result = install(
            &root,
            &WINDOWS,
            |archive, path| {
                assert_eq!(archive.sha256.len(), 64);
                assert!(archive.urls.iter().all(|url| url.starts_with("https://")));
                downloads.borrow_mut().push(archive.name);
                fs::write(path, b"fixture")
            },
            |_, destination, member| {
                if let Some(member) = member {
                    assert_eq!(member, "bin/windows/synctex.exe");
                    fs::write(destination.join(member), b"fixture")?;
                } else {
                    let bin = destination.join("TinyTeX").join(WINDOWS.bin);
                    fs::create_dir_all(&bin)?;
                    for tool in REQUIRED_TOOLS.iter().filter(|name| **name != "synctex") {
                        fs::write(bin.join(format!("{tool}.exe")), b"fixture")?;
                    }
                }
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(result, target.canonicalize().unwrap().join(WINDOWS.bin));
        assert_eq!(*downloads.borrow(), vec!["tex.zip", "sync.tar.xz"]);
        assert!(
            root.read_dir().unwrap().any(|entry| entry
                .unwrap()
                .path()
                .join("user-notes")
                .is_file())
        );
        assert_eq!(
            install(
                &root,
                &WINDOWS,
                |_, _| panic!("must reuse"),
                |_, _, _| panic!("must reuse")
            )
            .unwrap(),
            result,
        );
    }

    #[test]
    fn keeps_existing_files_after_a_failed_download() {
        let profile = tempfile::tempdir().unwrap();
        let root = profile.path().join("Refract/dependencies");
        let target = root.join("TinyTeX");
        fs::create_dir_all(&target).unwrap();
        let notes = target.join("user-notes");
        fs::write(&notes, b"retained").unwrap();
        assert!(
            install(
                &root,
                &WINDOWS,
                |_, _| Err(io::Error::other("Checksum mismatch")),
                |_, _, _| unreachable!()
            )
            .is_err()
        );
        assert_eq!(fs::read(notes).unwrap(), b"retained");
        assert!(!root.read_dir().unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("setup-")
        }));
    }
}

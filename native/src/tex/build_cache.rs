use super::{BuildResult, BuildSettings, build_file, check_preview_cancel, invalid, recorder_path};
use crate::{ProjectEntry, read_project_entries, source_path};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    env,
    ffi::OsString,
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{Mutex, atomic::AtomicBool},
    time::{Duration, Instant},
};

const MAX_ENTRIES: usize = 3;
const MAX_FILES: usize = 10_000;
const MAX_SOURCE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_RECORDER_BYTES: u64 = 8 * 1024 * 1024;
const MAX_PDF_BYTES: u64 = 12 * 1024 * 1024;
const LIFETIME: Duration = Duration::from_secs(60);

static CACHE: Mutex<VecDeque<(PathBuf, CachedBuild)>> = Mutex::new(VecDeque::new());

#[derive(PartialEq)]
pub(super) struct BuildKey {
    executable: PathBuf,
    settings: BuildSettings,
    minute: i64,
    environment: Vec<(OsString, OsString)>,
    files: BTreeMap<String, (FileStamp, [u8; 32])>,
}

type Dependencies = BTreeMap<PathBuf, (PathBuf, FileStamp)>;

pub(super) struct CachedBuild {
    key: BuildKey,
    result: BuildResult,
    dependencies: Dependencies,
    created: Instant,
}

pub(super) fn take(dir: &Path) -> Option<CachedBuild> {
    let mut cache = CACHE.lock().ok()?;
    let index = cache.iter().position(|(path, _)| path == dir)?;
    cache.remove(index).map(|(_, build)| build)
}

pub(super) fn remember(dir: &Path, build: CachedBuild) {
    if let Ok(mut cache) = CACHE.lock() {
        cache.retain(|(path, entry)| path != dir && entry.created.elapsed() < LIFETIME);
        if cache.len() == MAX_ENTRIES {
            cache.pop_front();
        }
        cache.push_back((dir.to_owned(), build));
    }
}

impl CachedBuild {
    pub(super) fn try_reuse(
        &self,
        dir: &Path,
        output: &Path,
        key: &BuildKey,
        cancel: &AtomicBool,
    ) -> Option<BuildResult> {
        if self.created.elapsed() >= LIFETIME || &self.key != key {
            return None;
        }
        let current = dependencies(dir, output, &key.executable, cancel).ok()?;
        if current != self.dependencies
            || self.created.elapsed() >= LIFETIME
            || key.minute != minute()
            || check_preview_cancel(cancel).is_err()
        {
            return None;
        }
        let mut result = self.result.clone();
        result.cached = true;
        Some(result)
    }
}

pub(super) fn record(
    dir: &Path,
    output: &Path,
    settings: &BuildSettings,
    key: BuildKey,
    build: &BuildResult,
    cancel: &AtomicBool,
) {
    let Some(pdf) = build.pdf.as_deref() else {
        return;
    };
    if build.exit_code != Some(0)
        || !pdf.metadata().is_ok_and(|info| info.len() < MAX_PDF_BYTES)
        || check_preview_cancel(cancel).is_err()
    {
        return;
    }
    if let Ok(dependencies) = dependencies(dir, output, &key.executable, cancel)
        && build_key(dir, settings, &key.executable, cancel).as_ref() == Some(&key)
    {
        remember(
            dir,
            CachedBuild {
                key,
                result: build.clone(),
                dependencies,
                created: Instant::now(),
            },
        );
    }
}

fn minute() -> i64 {
    if cfg!(test) {
        return 0;
    }
    chrono::Utc::now().timestamp().div_euclid(60)
}

pub(super) fn build_key(
    dir: &Path,
    settings: &BuildSettings,
    executable: &Path,
    cancel: &AtomicBool,
) -> Option<BuildKey> {
    let minute = minute();
    let mut environment: Vec<_> = env::vars_os()
        .filter(|(name, _)| {
            let name = name.to_string_lossy().to_ascii_uppercase();
            [
                "PATH",
                "BIBINPUTS",
                "BSTINPUTS",
                "TEX",
                "LUAINPUTS",
                "OSFONTDIR",
                "FONTCONFIG",
                "SOURCE_DATE_EPOCH",
                "FORCE_SOURCE_DATE",
                "TZ",
            ]
            .iter()
            .any(|prefix| name.starts_with(prefix))
        })
        .collect();
    environment.sort_unstable();
    let entries = read_project_entries(dir, "").ok()?;
    let mut files = BTreeMap::new();
    let mut remaining = MAX_SOURCE_BYTES;
    read_sources(dir, &entries, &mut files, &mut remaining, cancel).ok()?;
    Some(BuildKey {
        executable: executable.to_owned(),
        settings: settings.clone(),
        minute,
        environment,
        files,
    })
}

fn read_sources(
    dir: &Path,
    entries: &[ProjectEntry],
    files: &mut BTreeMap<String, (FileStamp, [u8; 32])>,
    remaining: &mut u64,
    cancel: &AtomicBool,
) -> io::Result<()> {
    for entry in entries {
        check_preview_cancel(cancel)?;
        if let Some(children) = &entry.children {
            read_sources(dir, children, files, remaining, cancel)?;
            continue;
        }
        let path = source_path(dir, &entry.path)?;
        if files.len() == MAX_FILES || path.canonicalize()? != path {
            return Err(invalid("Cannot verify the project input set"));
        }
        let mut file = build_file(&path)?;
        let before = file_stamp(&file)?;
        if before.size > *remaining {
            return Err(invalid("The project is too large to cache"));
        }
        let mut hash = Sha256::new();
        let mut chunk = [0; 32 * 1024];
        loop {
            check_preview_cancel(cancel)?;
            let bytes = file.read(&mut chunk)?;
            if bytes == 0 {
                break;
            }
            *remaining = remaining
                .checked_sub(bytes as u64)
                .ok_or_else(|| invalid("The project is too large to cache"))?;
            hash.update(&chunk[..bytes]);
        }
        if before != file_stamp(&build_file(&path)?)? {
            return Err(invalid("A project input changed during verification"));
        }
        files.insert(entry.path.clone(), (before, hash.finalize().into()));
    }
    Ok(())
}

fn dependencies(
    dir: &Path,
    output: &Path,
    executable: &Path,
    cancel: &AtomicBool,
) -> io::Result<Dependencies> {
    // TeX does not record all inputs read by BibTeX or Biber.
    for name in ["main.bbl", "main.bcf"] {
        match output.join(name).symlink_metadata() {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            _ => return Err(invalid("Bibliography dependencies cannot be verified")),
        }
    }
    let recorder = output.join("main.fls");
    let mut text = String::new();
    build_file(&recorder)?
        .take(MAX_RECORDER_BYTES + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > MAX_RECORDER_BYTES {
        return Err(invalid("The compiler dependency record is too large"));
    }
    let mut paths = BTreeSet::from([
        executable.to_owned(),
        recorder,
        output.join("main.pdf"),
        output.join("main.log"),
        output.join("main.synctex.gz"),
    ]);
    let mut saw_input = false;
    for line in text.lines().filter_map(|line| line.strip_prefix("INPUT ")) {
        check_preview_cancel(cancel)?;
        saw_input = true;
        paths.insert(recorder_path(dir, line));
        if paths.len() > MAX_FILES {
            return Err(invalid("Too many compiler dependencies"));
        }
    }
    if !saw_input {
        return Err(invalid("No compiler dependencies were recorded"));
    }
    paths
        .into_iter()
        .map(|path| {
            check_preview_cancel(cancel)?;
            // System TeX distributions can use symlinks for engines and fonts.
            // Include the target path in the stamp so a retargeted link misses.
            let resolved = path.canonicalize()?;
            let stamp = file_stamp(&build_file(&resolved)?)?;
            Ok((path, (resolved, stamp)))
        })
        .collect()
}

#[derive(Eq, PartialEq)]
struct FileStamp {
    size: u64,
    #[cfg(unix)]
    identity: (u64, u64, u32),
    #[cfg(unix)]
    times: [(i64, i64); 2],
    #[cfg(windows)]
    identity: (u64, [u8; 16], u32),
    #[cfg(windows)]
    times: [i64; 3],
}

#[cfg(unix)]
fn file_stamp(file: &File) -> io::Result<FileStamp> {
    use std::os::unix::fs::MetadataExt;
    let info = file.metadata()?;
    Ok(FileStamp {
        size: info.len(),
        identity: (info.dev(), info.ino(), info.mode()),
        times: [
            (info.mtime(), info.mtime_nsec()),
            (info.ctime(), info.ctime_nsec()),
        ],
    })
}

#[cfg(windows)]
fn file_stamp(file: &File) -> io::Result<FileStamp> {
    use std::{mem::MaybeUninit, os::windows::io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_BASIC_INFO, FILE_ID_INFO, FileBasicInfo, FileIdInfo, GetFileInformationByHandleEx,
    };
    let mut basic = MaybeUninit::<FILE_BASIC_INFO>::uninit();
    let mut id = MaybeUninit::<FILE_ID_INFO>::uninit();
    let handle = file.as_raw_handle();
    // Windows initializes each buffer only when the corresponding call succeeds.
    let (basic, id) = unsafe {
        if GetFileInformationByHandleEx(
            handle,
            FileBasicInfo,
            basic.as_mut_ptr().cast(),
            size_of::<FILE_BASIC_INFO>() as u32,
        ) == 0
            || GetFileInformationByHandleEx(
                handle,
                FileIdInfo,
                id.as_mut_ptr().cast(),
                size_of::<FILE_ID_INFO>() as u32,
            ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        (basic.assume_init(), id.assume_init())
    };
    Ok(FileStamp {
        size: file.metadata()?.len(),
        identity: (
            id.VolumeSerialNumber,
            id.FileId.Identifier,
            basic.FileAttributes,
        ),
        times: [basic.CreationTime, basic.LastWriteTime, basic.ChangeTime],
    })
}

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    fs::{self, File, Metadata, TryLockError},
    io::{self, ErrorKind},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

const INSTANCE_DIRECTORY: &str = ".refract-native-instance";
const ACTIVATION_REQUEST: &str = "activate";
const HANDOFF_TIMEOUT: Duration = Duration::from_secs(1);
const HANDOFF_INTERVAL: Duration = Duration::from_millis(25);

pub(super) struct Instance {
    _watcher: RecommendedWatcher,
    _lock: File,
    activations: Activations,
}

impl Instance {
    pub(super) fn claim(profile: &Path) -> io::Result<Option<Self>> {
        let directory = instance_directory(profile)?;
        let request = directory.join(ACTIVATION_REQUEST);
        let lock = open_lock(&directory.join("lock"))?;
        if !try_claim(&lock)? && !handoff(&lock, &request)? {
            return Ok(None);
        }

        let (send, receive) = async_channel::bounded(1);
        let observed_request = request.clone();
        let observer = send.clone();
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if event.is_ok() && observed_request.symlink_metadata().is_ok() {
                    let _ = observer.try_send(());
                }
            })
            .map_err(io::Error::other)?;
        watcher
            .watch(&directory, RecursiveMode::NonRecursive)
            .map_err(io::Error::other)?;

        // The launcher can request activation after we lock and before notifications start.
        if request_exists(&request)? {
            let _ = send.try_send(());
        }
        Ok(Some(Self {
            _watcher: watcher,
            _lock: lock,
            activations: Activations { receive, request },
        }))
    }

    pub(super) fn activations(&self) -> Activations {
        self.activations.clone()
    }
}

#[derive(Clone)]
pub(super) struct Activations {
    receive: async_channel::Receiver<()>,
    request: PathBuf,
}

impl Activations {
    pub(super) async fn wait(&self) -> bool {
        while self.receive.recv().await.is_ok() {
            if self.request.symlink_metadata().is_ok() {
                return true;
            }
        }
        false
    }

    pub(super) fn acknowledge(&self) {
        let _ = fs::remove_file(&self.request);
    }
}

fn instance_directory(profile: &Path) -> io::Result<PathBuf> {
    fs::create_dir_all(profile)?;
    let directory = profile.join(INSTANCE_DIRECTORY);
    #[cfg(unix)]
    let create = {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(&directory)
    };
    #[cfg(windows)]
    let create = fs::create_dir(&directory);
    match create {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let metadata = directory.symlink_metadata()?;
    if !metadata.is_dir() || is_link(&metadata) {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "Invalid Refract instance directory",
        ));
    }
    directory.canonicalize()
}

fn open_lock(path: &Path) -> io::Result<File> {
    let mut options = File::options();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use rustix::fs::OFlags;
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags((OFlags::NOFOLLOW | OFlags::NONBLOCK).bits() as i32);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || is_link(&metadata) {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "Invalid Refract instance lock",
        ));
    }
    Ok(file)
}

fn is_link(metadata: &Metadata) -> bool {
    #[cfg(unix)]
    {
        metadata.is_symlink()
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
}

fn try_claim(lock: &File) -> io::Result<bool> {
    match lock.try_lock() {
        Ok(()) => Ok(true),
        Err(TryLockError::WouldBlock) => Ok(false),
        Err(TryLockError::Error(error)) => Err(error),
    }
}

fn handoff(lock: &File, request: &Path) -> io::Result<bool> {
    match File::create_new(request) {
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let expires = Instant::now() + HANDOFF_TIMEOUT;
    loop {
        // If the former owner closes before acknowledging, this launcher can take its place.
        if try_claim(lock)? {
            return Ok(true);
        }
        if !request_exists(request)? || Instant::now() >= expires {
            return Ok(false);
        }
        thread::sleep(HANDOFF_INTERVAL);
    }
}

fn request_exists(request: &Path) -> io::Result<bool> {
    match request.symlink_metadata() {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signals_the_running_profile_without_locking_another() {
        let scratch = tempfile::tempdir().unwrap();
        let path = scratch.path().join("first");
        let primary = Instance::claim(&path).unwrap().unwrap();
        let another = Instance::claim(&scratch.path().join("second"))
            .unwrap()
            .unwrap();
        let secondary = thread::spawn(move || Instance::claim(&path).unwrap().is_none());
        let activations = primary.activations();
        let expires = Instant::now() + Duration::from_secs(5);
        while activations.receive.try_recv().is_err() {
            assert!(
                Instant::now() < expires,
                "No profile activation event arrived"
            );
            thread::sleep(HANDOFF_INTERVAL);
        }
        assert!(request_exists(&activations.request).unwrap());
        assert!(!request_exists(&another.activations.request).unwrap());
        activations.acknowledge();
        assert!(secondary.join().unwrap());
    }

    #[test]
    fn handles_a_request_left_by_a_previous_owner() {
        let scratch = tempfile::tempdir().unwrap();
        let primary = Instance::claim(scratch.path()).unwrap().unwrap();
        let path = primary.activations.request.clone();
        File::create_new(&path).unwrap();
        drop(primary);

        let next = Instance::claim(scratch.path()).unwrap().unwrap();
        let activations = next.activations();
        activations.receive.try_recv().unwrap();
        activations.acknowledge();
        assert!(!request_exists(&path).unwrap());
    }
}

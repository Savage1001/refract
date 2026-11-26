use super::{ProjectEntryKind, checked_project_directory, invalid_destination};
use crate::{
    METADATA_LIMIT, Project, comments,
    project_management::{is_link, read_project_metadata},
    read_local_file, stage_file, valid_project_component,
    versions::snapshot_project,
};
use serde_json::{Map, Value};
use std::{
    fs, io,
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;

#[derive(Clone)]
pub struct ProjectEntryMove {
    source: String,
    destination: String,
    kind: ProjectEntryKind,
}

impl ProjectEntryMove {
    pub fn new(source: &str, destination: &str, kind: ProjectEntryKind) -> io::Result<Self> {
        let source = source.replace('\\', "/");
        let destination = destination.replace('\\', "/");
        if !source
            .split('/')
            .chain(destination.split('/'))
            .all(valid_project_component)
        {
            return Err(invalid_destination());
        }
        let entry = Self {
            source,
            destination,
            kind,
        };
        if entry.source == entry.destination
            || kind == ProjectEntryKind::Folder && entry.remap_path(&entry.destination).is_some()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Choose a destination outside the original path",
            ));
        }
        Ok(entry)
    }

    pub fn destination(&self) -> &str {
        &self.destination
    }

    pub fn remap_path(&self, path: &str) -> Option<String> {
        let path = path.replace('\\', "/");
        let mut remaining = path.as_str();
        for source in self.source.split('/') {
            let (component, rest) = remaining.split_once('/').unwrap_or((remaining, ""));
            if component.to_lowercase() != source.to_lowercase() {
                return None;
            }
            remaining = rest;
        }
        if remaining.is_empty() {
            Some(self.destination.clone())
        } else if self.kind == ProjectEntryKind::Folder {
            Some(format!("{}/{remaining}", self.destination))
        } else {
            None
        }
    }
}

pub fn move_project_entry(
    projects_dir: &Path,
    project: &Project,
    entry: &ProjectEntryMove,
) -> io::Result<Project> {
    let folder = checked_project_directory(projects_dir, project)?;
    let source = checked_target(&folder, &entry.source)?;
    let destination = checked_target(&folder, &entry.destination)?;
    check_entry(&source, entry.kind)?;
    match destination.symlink_metadata() {
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "A file or folder already exists at that path",
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    if entry.kind == ProjectEntryKind::Folder {
        check_children(&source)?;
    }

    let metadata_path = folder.join(".refract.json");
    let metadata = read_project_metadata(&folder, &metadata_path)?;
    let current: Project = serde_json::from_slice(&metadata)?;
    if current.id != project.id {
        return Err(invalid_destination());
    }
    let (updated, root_edit) = if let Some(path) = entry.remap_path(current.root_file()) {
        let mut fields: Map<String, Value> = serde_json::from_slice(&metadata)?;
        fields.insert("rootFile".into(), Value::String(path));
        let next = serde_json::to_vec_pretty(&fields)?;
        if next.len() > METADATA_LIMIT {
            return Err(invalid_destination());
        }
        (
            serde_json::from_slice(&next)?,
            Some(RecordEdit::prepare(metadata_path, metadata, next)?),
        )
    } else {
        (current.clone(), None)
    };
    let comment_edit = if let Some(comments) = comments::read_comments_file(&folder)? {
        let path = folder.join(".refract-comments.json");
        comments::remap_files(&comments, |path| entry.remap_path(path))?
            .map(|next| RecordEdit::prepare(path, comments, next))
            .transpose()?
    } else {
        None
    };
    let mut edits: Vec<RecordEdit> = [root_edit, comment_edit].into_iter().flatten().collect();
    let recovery = snapshot_project(projects_dir, &current)?;
    for edit in &edits {
        edit.check(&folder, &edit.original)?;
    }

    move_checked(&folder, &entry.source, &entry.destination, entry.kind)?;
    for index in 0..edits.len() {
        if let Err(error) = edits[index].apply(&folder) {
            let rollback = move_checked(&folder, &entry.destination, &entry.source, entry.kind)
                .and_then(|()| {
                    for edit in edits[..index].iter_mut().rev() {
                        edit.rollback(&folder)?;
                    }
                    Ok(())
                });
            return match rollback {
                Ok(()) => Err(error),
                Err(rollback) => Err(io::Error::other(format!(
                    "Move failed. {error}. Recovery needs attention. {rollback}. Recovery version {}.",
                    recovery.id
                ))),
            };
        }
    }
    Ok(updated)
}

struct RecordEdit {
    path: PathBuf,
    original: Vec<u8>,
    updated: Vec<u8>,
    write: Option<NamedTempFile>,
    undo: Option<NamedTempFile>,
}

impl RecordEdit {
    fn prepare(path: PathBuf, original: Vec<u8>, updated: Vec<u8>) -> io::Result<Self> {
        let write = stage_file(&path, &updated)?;
        let undo = stage_file(&path, &original)?;
        Ok(Self {
            path,
            original,
            updated,
            write: Some(write),
            undo: Some(undo),
        })
    }

    fn check(&self, folder: &Path, expected: &[u8]) -> io::Result<()> {
        let info = self.path.symlink_metadata()?;
        if !info.is_file()
            || is_link(&info)
            || read_local_file(folder, &self.path, expected.len())? != expected
        {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Project records changed during the move",
            ));
        }
        Ok(())
    }

    fn apply(&mut self, folder: &Path) -> io::Result<()> {
        self.check(folder, &self.original)?;
        self.write
            .take()
            .expect("write is used once")
            .persist(&self.path)
            .map(|_| ())
            .map_err(|error| error.error)
    }

    fn rollback(&mut self, folder: &Path) -> io::Result<()> {
        self.check(folder, &self.updated)?;
        self.undo
            .take()
            .expect("undo is used once")
            .persist(&self.path)
            .map(|_| ())
            .map_err(|error| error.error)
    }
}

pub(super) fn checked_target(folder: &Path, relative: &str) -> io::Result<PathBuf> {
    let mut target = folder.to_owned();
    let mut parts = relative.split('/').peekable();
    while let Some(part) = parts.next() {
        target.push(part);
        if parts.peek().is_some() {
            let info = target.symlink_metadata()?;
            if !info.is_dir() || is_link(&info) || !target.canonicalize()?.starts_with(folder) {
                return Err(invalid_destination());
            }
        }
    }
    Ok(target)
}

pub(super) fn check_entry(path: &Path, kind: ProjectEntryKind) -> io::Result<()> {
    let info = path.symlink_metadata()?;
    let wanted = match kind {
        ProjectEntryKind::File => info.is_file(),
        ProjectEntryKind::Folder => info.is_dir(),
    };
    if is_link(&info) || !wanted {
        return Err(invalid_destination());
    }
    Ok(())
}

pub(super) fn check_children(folder: &Path) -> io::Result<()> {
    for entry in fs::read_dir(folder)? {
        let entry = entry?.path();
        let info = entry.symlink_metadata()?;
        if is_link(&info) || !(info.is_dir() || info.is_file()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Folder contains a link or unsupported entry",
            ));
        }
        if info.is_dir() {
            check_children(&entry)?;
        }
    }
    Ok(())
}

fn move_checked(
    folder: &Path,
    source: &str,
    destination: &str,
    kind: ProjectEntryKind,
) -> io::Result<()> {
    let source = checked_target(folder, source)?;
    let destination = checked_target(folder, destination)?;
    check_entry(&source, kind)?;
    rename_without_replacement(&source, &destination)
}

#[cfg(windows)]
pub(super) fn rename_without_replacement(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::MoveFileExW;
    let source: Vec<u16> = source.as_os_str().encode_wide().chain([0]).collect();
    let destination: Vec<u16> = destination.as_os_str().encode_wide().chain([0]).collect();
    // Both path buffers are null-terminated and remain alive for this call.
    if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub(super) fn rename_without_replacement(_source: &Path, _destination: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Safe project entry moves are unavailable on this platform",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        comments::{CommentUpdate, read_comments, update_comments},
        create_project,
        versions::list_versions,
    };

    #[test]
    fn renames_the_default_root_and_remaps_only_its_comments() {
        let root = tempfile::tempdir().unwrap();
        let project = create_project(root.path(), "Paper").unwrap();
        let folder = root.path().join(&project.id);
        update_comments(
            root.path(),
            &project,
            CommentUpdate::Create {
                file: "main.tex".into(),
                text: "Review root".into(),
                selection: None,
            },
        )
        .unwrap();
        update_comments(
            root.path(),
            &project,
            CommentUpdate::Create {
                file: "notes.tex".into(),
                text: "Review notes".into(),
                selection: None,
            },
        )
        .unwrap();
        let entry = ProjectEntryMove::new("main.tex", "paper.tex", ProjectEntryKind::File).unwrap();
        let result = move_project_entry(root.path(), &project, &entry).unwrap();
        assert_eq!(result.root_file(), "paper.tex");
        assert!(folder.join("paper.tex").is_file());
        assert!(!folder.join("main.tex").exists());
        let comments = read_comments(root.path(), &result).unwrap();
        assert_eq!(comments[0].file, "paper.tex");
        assert_eq!(comments[1].file, "notes.tex");
        assert_eq!(
            list_versions(root.path(), &result).unwrap()[0]
                .root_file
                .as_deref(),
            Some("main.tex")
        );
    }

    #[test]
    fn moves_a_folder_with_empty_and_hidden_members_and_updates_nested_paths() {
        let root = tempfile::tempdir().unwrap();
        let project = create_project(root.path(), "Paper").unwrap();
        let folder = root.path().join(&project.id);
        fs::create_dir_all(folder.join("chapters/empty")).unwrap();
        fs::create_dir(folder.join("draft")).unwrap();
        fs::write(folder.join("chapters/paper.tex"), "Article").unwrap();
        fs::write(folder.join("chapters/.keep"), "Hidden").unwrap();
        let metadata_path = folder.join(".refract.json");
        let mut metadata: Value =
            serde_json::from_slice(&fs::read(&metadata_path).unwrap()).unwrap();
        metadata["rootFile"] = "chapters/paper.tex".into();
        metadata["engine"] = "xelatex".into();
        fs::write(&metadata_path, serde_json::to_vec(&metadata).unwrap()).unwrap();
        update_comments(
            root.path(),
            &project,
            CommentUpdate::Create {
                file: "chapters/paper.tex".into(),
                text: "Review".into(),
                selection: None,
            },
        )
        .unwrap();
        let entry =
            ProjectEntryMove::new("chapters", "draft\\sections", ProjectEntryKind::Folder).unwrap();
        let result = move_project_entry(root.path(), &project, &entry).unwrap();
        assert_eq!(result.root_file(), "draft/sections/paper.tex");
        assert!(!folder.join("chapters").exists());
        assert!(folder.join("draft/sections/empty").is_dir());
        assert_eq!(
            fs::read_to_string(folder.join("draft/sections/.keep")).unwrap(),
            "Hidden"
        );
        assert_eq!(
            read_comments(root.path(), &result).unwrap()[0].file,
            "draft/sections/paper.tex"
        );
        assert_eq!(entry.remap_path("chapters-old/file.tex"), None);
    }

    #[test]
    fn preserves_existing_files_when_the_destination_or_metadata_is_invalid() {
        let root = tempfile::tempdir().unwrap();
        let project = create_project(root.path(), "Paper").unwrap();
        let folder = root.path().join(&project.id);
        let original = fs::read(folder.join("main.tex")).unwrap();
        fs::write(folder.join("existing.tex"), "Existing").unwrap();
        let collision =
            ProjectEntryMove::new("main.tex", "existing.tex", ProjectEntryKind::File).unwrap();
        assert_eq!(
            move_project_entry(root.path(), &project, &collision)
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(folder.join("existing.tex")).unwrap(), b"Existing");
        fs::write(folder.join(".refract-comments.json"), "invalid JSON").unwrap();
        let entry = ProjectEntryMove::new("main.tex", "paper.tex", ProjectEntryKind::File).unwrap();
        assert!(move_project_entry(root.path(), &project, &entry).is_err());
        assert_eq!(fs::read(folder.join("main.tex")).unwrap(), original);
        assert!(!folder.join("paper.tex").exists());
        assert!(
            ProjectEntryMove::new("chapters", "CHAPTERS/sections", ProjectEntryKind::Folder)
                .is_err()
        );
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn rename_without_replacement(source: &Path, destination: &Path) -> io::Result<()> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};
    renameat_with(CWD, source, CWD, destination, RenameFlags::NOREPLACE).map_err(Into::into)
}

use crate::{Project, project_directory, project_management::is_link, valid_project_component};
use std::{
    fs::{self, File},
    io,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectEntryKind {
    File,
    Folder,
}

pub fn create_project_entry(
    projects_dir: &Path,
    project: &Project,
    path: &str,
    kind: ProjectEntryKind,
) -> io::Result<String> {
    let path = path.replace('\\', "/");
    if !path.split('/').all(valid_project_component) {
        return Err(invalid_destination());
    }
    let folder = project_directory(projects_dir, project)?;
    let mut new_parent = CreatedFolders::new(&folder)?;
    let (parent, name) = path.rsplit_once('/').unwrap_or(("", &path));
    let destination = new_parent.ensure(parent)?.join(name);
    match kind {
        ProjectEntryKind::File => {
            File::create_new(destination)?;
        }
        ProjectEntryKind::Folder => fs::create_dir(destination)?,
    }
    new_parent.keep();
    Ok(path)
}

pub(crate) struct CreatedFolders<'a> {
    root: &'a Path,
    created: Vec<PathBuf>,
}

impl<'a> CreatedFolders<'a> {
    pub(crate) fn new(root: &'a Path) -> io::Result<Self> {
        let info = root.symlink_metadata()?;
        if !info.is_dir() || is_link(&info) || root.canonicalize()? != root {
            return Err(invalid_destination());
        }
        Ok(Self {
            root,
            created: Vec::new(),
        })
    }

    pub(crate) fn ensure(&mut self, relative: &str) -> io::Result<PathBuf> {
        let mut folder = self.root.to_path_buf();
        if relative.is_empty() {
            return Ok(folder);
        }
        if !relative.split('/').all(valid_project_component) {
            return Err(invalid_destination());
        }
        for name in relative.split('/') {
            folder.push(name);
            match fs::create_dir(&folder) {
                Ok(()) => self.created.push(folder.clone()),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
            let info = folder.symlink_metadata()?;
            if !info.is_dir() || is_link(&info) || !folder.canonicalize()?.starts_with(self.root) {
                return Err(invalid_destination());
            }
        }
        Ok(folder)
    }

    pub(crate) fn keep(mut self) {
        self.created.clear();
    }
}

impl Drop for CreatedFolders<'_> {
    fn drop(&mut self) {
        for folder in self.created.iter().rev() {
            if folder.canonicalize().is_ok_and(|path| path == *folder) {
                let _ = fs::remove_dir(folder);
            }
        }
    }
}

fn invalid_destination() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "Invalid file or folder path")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{create_project, project_tree};

    #[test]
    fn creates_empty_files_and_folders_in_nested_paths_without_overwriting() {
        let root = tempfile::tempdir().unwrap();
        let project = create_project(root.path(), "Paper").unwrap();
        let folder = root.path().join(&project.id);
        let file = create_project_entry(
            root.path(),
            &project,
            "sections\\notes.tex",
            ProjectEntryKind::File,
        )
        .unwrap();
        assert_eq!(file, "sections/notes.tex");
        assert!(fs::read(folder.join(&file)).unwrap().is_empty());
        let empty = create_project_entry(
            root.path(),
            &project,
            "sections/assets/empty",
            ProjectEntryKind::Folder,
        )
        .unwrap();
        assert!(folder.join(empty).is_dir());
        fs::write(folder.join(&file), "Existing notes").unwrap();
        assert_eq!(
            create_project_entry(root.path(), &project, &file, ProjectEntryKind::File)
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert!(
            create_project_entry(root.path(), &project, &file, ProjectEntryKind::Folder).is_err()
        );
        assert_eq!(
            fs::read_to_string(folder.join(&file)).unwrap(),
            "Existing notes"
        );
        let tree = project_tree(root.path(), &project).unwrap();
        assert_eq!(tree[0].path, "sections");
        assert_eq!(tree[0].children.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn rejects_hidden_absolute_and_traversal_paths_before_creating_parents() {
        let root = tempfile::tempdir().unwrap();
        let project = create_project(root.path(), "Paper").unwrap();
        for bad in [
            "../outside.tex",
            "/outside.tex",
            "C:\\outside.tex",
            "sections/.hidden/file.tex",
            "sections//file.tex",
        ] {
            assert_eq!(
                create_project_entry(root.path(), &project, bad, ProjectEntryKind::File)
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidInput
            );
        }
        let folder = root.path().join(&project.id);
        assert!(!folder.join("sections").exists());
        assert!(!root.path().join("outside.tex").exists());
    }

    #[test]
    fn refuses_to_create_files_through_a_linked_folder() {
        let root = tempfile::tempdir().unwrap();
        let project = create_project(root.path(), "Paper").unwrap();
        let link = root.path().join(&project.id).join("linked");
        let outside = root.path().join("outside");
        fs::create_dir(&outside).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        #[cfg(windows)]
        if let Err(error) = std::os::windows::fs::symlink_dir(&outside, &link) {
            if error.kind() == io::ErrorKind::PermissionDenied || error.raw_os_error() == Some(1314)
            {
                return;
            }
            panic!("Could not create test link. {error}");
        }
        assert!(
            create_project_entry(
                root.path(),
                &project,
                "linked/notes.tex",
                ProjectEntryKind::File
            )
            .is_err()
        );
        assert!(!outside.join("notes.tex").exists());
    }
}

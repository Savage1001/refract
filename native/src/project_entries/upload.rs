use super::{
    ProjectEntryKind, checked_project_directory, invalid_destination,
    relocate::{check_entry, checked_target},
    rename_without_replacement,
};
use crate::{Project, project_management::is_link, valid_project_component};
use std::{
    fs::{self, File},
    io,
    path::{Path, PathBuf},
};

pub fn upload_project_files(
    projects_dir: &Path,
    project: &Project,
    parent: &str,
    sources: &[PathBuf],
) -> io::Result<Vec<String>> {
    let root = checked_project_directory(projects_dir, project)?;
    let (parent, destination) = upload_destination(&root, parent)?;
    let mut copied = Vec::new();
    for source in sources {
        match upload_file(source, &parent, &destination) {
            Ok(path) => copied.push(path),
            Err(error) if copied.is_empty() => return Err(error),
            Err(error) => {
                return Err(io::Error::new(
                    error.kind(),
                    format!(
                        "Upload stopped after copying {}: {error}",
                        copied.join(", ")
                    ),
                ));
            }
        }
    }
    Ok(copied)
}

pub fn upload_project_folder(
    projects_dir: &Path,
    project: &Project,
    parent: &str,
    source: &Path,
) -> io::Result<String> {
    let name = upload_name(source)?;
    let root = checked_project_directory(projects_dir, project)?;
    let (parent, destination) = upload_destination(&root, parent)?;
    check_entry(source, ProjectEntryKind::Folder)?;
    let source = source.canonicalize()?;
    if source.starts_with(&root) || root.starts_with(&source) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Choose a folder outside this project",
        ));
    }
    let target = destination.join(name);
    ensure_upload_available(&target, name)?;
    let stage = tempfile::Builder::new()
        .prefix(".refract-upload-")
        .tempdir_in(&destination)?;
    copy_folder(&source, stage.path())?;
    rename_without_replacement(stage.path(), &target)?;
    let _ = stage.keep();
    Ok(upload_path(&parent, name))
}

fn upload_destination(root: &Path, parent: &str) -> io::Result<(String, PathBuf)> {
    let parent = parent.replace('\\', "/");
    let destination = if parent.is_empty() {
        root.to_owned()
    } else {
        if !parent.split('/').all(valid_project_component) {
            return Err(invalid_destination());
        }
        let path = checked_target(root, &parent)?;
        check_entry(&path, ProjectEntryKind::Folder)?;
        path
    };
    Ok((parent, destination))
}

fn upload_file(source: &Path, parent: &str, destination: &Path) -> io::Result<String> {
    let name = upload_name(source)?;
    let target = destination.join(name);
    ensure_upload_available(&target, name)?;
    let mut output = tempfile::Builder::new()
        .prefix(".refract-upload-")
        .tempfile_in(destination)?;
    copy_file_to(source, output.as_file_mut())?;
    output
        .persist_noclobber(target)
        .map_err(|error| error.error)?;
    Ok(upload_path(parent, name))
}

fn upload_name(source: &Path) -> io::Result<&str> {
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(invalid_destination)?;
    if !valid_project_component(name) {
        return Err(invalid_destination());
    }
    Ok(name)
}

fn upload_path(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}/{name}")
    }
}

fn ensure_upload_available(target: &Path, name: &str) -> io::Result<()> {
    match target.symlink_metadata() {
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{name} already exists in the destination folder"),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn copy_folder(source: &Path, destination: &Path) -> io::Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_string_lossy().starts_with(".refract") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Reserved file name in folder",
            ));
        }
        let source = entry.path();
        let output = destination.join(name);
        let info = source.symlink_metadata()?;
        if is_link(&info) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Folders containing links cannot be imported",
            ));
        }
        if info.is_dir() {
            fs::create_dir(&output)?;
            copy_folder(&source, &output)?;
        } else if info.is_file() {
            copy_file_to(&source, &mut File::create_new(output)?)?;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Unsupported file in folder",
            ));
        }
    }
    Ok(())
}

fn copy_file_to(path: &Path, output: &mut File) -> io::Result<()> {
    if !fs::metadata(path)?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Only regular files can be uploaded",
        ));
    }
    let mut source = File::open(path)?;
    let permissions = source.metadata()?.permissions();
    io::copy(&mut source, output)?;
    output.set_permissions(permissions)?;
    output.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_project;

    #[test]
    fn uploads_multiple_files_until_conflict_and_preserves_the_existing_file() {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let project = create_project(&projects, "Paper").unwrap();
        let folder = projects.join(&project.id).join("chapters/figures");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("diagram.png"), "Original").unwrap();
        let notes = profile.path().join("notes.tex");
        let image = profile.path().join("diagram.png");
        fs::write(&notes, "Notes").unwrap();
        fs::write(&image, [0, 255]).unwrap();
        let error = upload_project_files(
            &projects,
            &project,
            r"chapters\figures",
            &[notes, image.clone()],
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert!(
            error
                .to_string()
                .contains("Upload stopped after copying chapters/figures/notes.tex")
        );
        assert_eq!(
            fs::read_to_string(folder.join("notes.tex")).unwrap(),
            "Notes"
        );
        assert_eq!(
            fs::read_to_string(folder.join("diagram.png")).unwrap(),
            "Original"
        );
        assert!(
            upload_project_files(
                &projects,
                &project,
                "../outside",
                std::slice::from_ref(&image)
            )
            .is_err()
        );
        assert!(upload_project_files(&projects, &project, "missing", &[image]).is_err());
        assert!(!projects.join(&project.id).join("missing").exists());
    }

    #[test]
    fn uploads_folders_with_empty_directories_and_keeps_failed_uploads_out_of_the_tree() {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let project = create_project(&projects, "Paper").unwrap();
        let dir = projects.join(&project.id);
        fs::create_dir(dir.join("chapters")).unwrap();
        let assets = profile.path().join("assets");
        fs::create_dir_all(assets.join("empty/subfolder")).unwrap();
        fs::write(assets.join(".refract-settings"), "Private").unwrap();
        fs::write(assets.join("image.png"), [1, 255]).unwrap();
        fs::write(assets.join(".gitignore"), "ignored\n").unwrap();
        assert!(upload_project_folder(&projects, &project, "chapters", &assets).is_err());
        assert!(fs::read_dir(dir.join("chapters")).unwrap().next().is_none());
        fs::remove_file(assets.join(".refract-settings")).unwrap();
        let imported = upload_project_folder(&projects, &project, "chapters", &assets).unwrap();
        assert_eq!(imported, "chapters/assets");
        let folder = dir.join(imported);
        assert!(folder.join("empty/subfolder").is_dir());
        assert_eq!(fs::read(folder.join("image.png")).unwrap(), [1, 255]);
        assert!(folder.join(".gitignore").exists());
        assert!(upload_project_folder(&projects, &project, "", &dir).is_err());
        assert!(upload_project_folder(&projects, &project, "", profile.path()).is_err());
        assert_eq!(
            upload_project_folder(&projects, &project, "chapters", &assets)
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
    }
}

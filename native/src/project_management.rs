use crate::{
    METADATA_LIMIT, Project, is_project_id, open_local_file, project_directory, read_local_file,
    stage_file,
};
use chrono::{SecondsFormat, Utc};
use serde_json::{Map, Value};
use std::{
    fs::{self, File, Metadata},
    io::{self, Write},
    path::Path,
};
use uuid::Uuid;

mod trash;
pub use trash::{DeletedProject, list_deleted_projects, restore_deleted_project, trash_project};

const DEFAULT_SOURCE: &str = r"\documentclass[11pt]{article}
\usepackage[margin=1in]{geometry}
\usepackage{amsmath,amssymb}

\title{Untitled}
\author{Author Name}
\date{\today}

\begin{document}
\maketitle

\section{Introduction}

\end{document}
";

pub fn create_project(projects_dir: &Path, name: &str) -> io::Result<Project> {
    let name = name.trim();
    let fields = new_metadata(if name.is_empty() { "New Project" } else { name });
    publish_project(projects_dir, &fields, |stage| {
        write_new_file(&stage.join("main.tex"), DEFAULT_SOURCE.as_bytes())
    })
}

pub fn duplicate_project(projects_dir: &Path, project: &Project) -> io::Result<Project> {
    let source = project_directory(projects_dir, project)?;
    let metadata_path = source.join(".refract.json");
    let bytes = read_project_metadata(&source, &metadata_path)?;
    let current: Project = serde_json::from_slice(&bytes)?;
    if current.id != project.id {
        return Err(invalid_metadata());
    }
    let original: Map<String, Value> = serde_json::from_slice(&bytes)?;
    let mut fields = new_metadata(&format!("{} (copy)", current.name));
    for key in ["rootFile", "engine"] {
        if let Some(value) = original
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            fields.insert(key.to_owned(), Value::String(value.to_owned()));
        }
    }
    if original.get("draftMode").and_then(Value::as_bool) == Some(true) {
        fields.insert("draftMode".to_owned(), Value::Bool(true));
    }
    publish_project(projects_dir, &fields, |stage| {
        copy_visible_files(&source, &source, stage)?;
        copy_project_history(&source, stage)?;
        if read_project_metadata(&source, &metadata_path)? != bytes {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Project metadata changed during duplication",
            ));
        }
        Ok(())
    })
}

fn new_metadata(name: &str) -> Map<String, Value> {
    Map::from_iter([
        ("id".into(), Value::String(Uuid::new_v4().to_string())),
        ("name".into(), Value::String(name.into())),
        (
            "created".into(),
            Value::String(Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)),
        ),
    ])
}

fn publish_project(
    projects_dir: &Path,
    fields: &Map<String, Value>,
    prepare: impl FnOnce(&Path) -> io::Result<()>,
) -> io::Result<Project> {
    let (project, metadata) = encode_metadata(fields)?;
    fs::create_dir_all(projects_dir)?;
    let root = projects_dir.canonicalize()?;
    let stage = tempfile::Builder::new()
        .prefix(".refract-create-")
        .tempdir_in(&root)?;
    prepare(stage.path())?;
    write_new_file(&stage.path().join(".refract.json"), &metadata)?;
    let target = root.join(&project.id);
    match target.symlink_metadata() {
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "Project identifier already exists",
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    fs::rename(stage.path(), target)?;
    let _ = stage.keep();
    Ok(project)
}

fn write_new_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = File::create_new(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn copy_visible_files(root: &Path, source: &Path, target: &Path) -> io::Result<()> {
    if source.canonicalize()? != source {
        return Err(invalid_metadata());
    }
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let from = entry.path();
        let to = target.join(name);
        let info = from.symlink_metadata()?;
        if is_link(&info) {
            continue;
        }
        if info.is_dir() {
            fs::create_dir(&to)?;
            copy_visible_files(root, &from, &to)?;
        } else if info.is_file() {
            copy_project_file(root, &from, &to, false)?;
        }
    }
    Ok(())
}

fn copy_project_history(source: &Path, target: &Path) -> io::Result<()> {
    let comments = source.join(".refract-comments.json");
    match comments.symlink_metadata() {
        Ok(_) => copy_project_file(
            source,
            &comments,
            &target.join(".refract-comments.json"),
            true,
        )?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let history = source.join(".refract-versions");
    match history.symlink_metadata() {
        Ok(info) if info.is_dir() && !is_link(&info) && history.canonicalize()? == history => {}
        Ok(_) => return Err(invalid_metadata()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    }
    let destination = target.join(".refract-versions");
    for entry in fs::read_dir(&history)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some((id, ext)) = name.to_str().and_then(|name| name.rsplit_once('.')) else {
            continue;
        };
        if is_project_id(id) && matches!(ext, "json" | "zip") {
            if !destination.try_exists()? {
                fs::create_dir(&destination)?;
            }
            copy_project_file(source, &entry.path(), &destination.join(name), true)?;
        }
    }
    Ok(())
}

fn copy_project_file(root: &Path, source: &Path, target: &Path, private: bool) -> io::Result<()> {
    let info = source.symlink_metadata()?;
    if !info.is_file() || is_link(&info) || source.canonicalize()? != source {
        return Err(invalid_metadata());
    }
    let mut file = open_local_file(root, source)?;
    if private && !has_one_link(&file)? {
        return Err(invalid_metadata());
    }
    let mut destination = File::create_new(target)?;
    io::copy(&mut file, &mut destination).map(|_| ())
}

fn is_link(info: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        info.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(unix)]
    {
        info.is_symlink()
    }
}

#[cfg(windows)]
fn has_one_link(file: &File) -> io::Result<bool> {
    use std::{mem::MaybeUninit, os::windows::io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };
    let mut info = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // The open file owns a valid handle. Windows initializes the output on success.
    let result = unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) };
    if result == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { info.assume_init() }.nNumberOfLinks == 1)
}

#[cfg(unix)]
fn has_one_link(file: &File) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    Ok(file.metadata()?.nlink() == 1)
}

pub fn rename_project(projects_dir: &Path, project: &Project, name: &str) -> io::Result<Project> {
    let name = name.trim();
    if name.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Project name is required",
        ));
    }
    let folder = project_directory(projects_dir, project)?;
    let path = folder.join(".refract.json");
    let original = read_project_metadata(&folder, &path)?;
    let mut fields: Map<String, Value> = serde_json::from_slice(&original)?;
    if fields.get("id").and_then(Value::as_str) != Some(project.id.as_str()) {
        return Err(invalid_metadata());
    }
    fields.insert("name".into(), Value::String(name.into()));
    let (renamed, metadata) = encode_metadata(&fields)?;
    let staged = stage_file(&path, &metadata)?;
    if read_project_metadata(&folder, &path)? != original {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Project metadata changed during rename",
        ));
    }
    staged.persist(path).map_err(|error| error.error)?;
    Ok(renamed)
}

fn read_project_metadata(folder: &Path, path: &Path) -> io::Result<Vec<u8>> {
    if !path.symlink_metadata()?.is_file() {
        return Err(invalid_metadata());
    }
    read_local_file(folder, path, METADATA_LIMIT)
}

fn encode_metadata(fields: &Map<String, Value>) -> io::Result<(Project, Vec<u8>)> {
    let bytes = serde_json::to_vec_pretty(fields)?;
    if bytes.len() > METADATA_LIMIT {
        return Err(invalid_metadata());
    }
    Ok((serde_json::from_slice(&bytes)?, bytes))
}

fn invalid_metadata() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Invalid project metadata")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        comments::{CommentUpdate, read_comments, update_comments},
        list_projects, read_source,
        versions::{list_versions, snapshot_project},
    };
    use serde_json::json;

    #[test]
    fn creates_the_default_electron_project_in_a_new_profile() {
        let profile = tempfile::tempdir().unwrap();
        let root = profile.path().join("projects");
        let project = create_project(&root, "   ").unwrap();
        let folder = root.join(&project.id);
        let fields: Value =
            serde_json::from_slice(&fs::read(folder.join(".refract.json")).unwrap()).unwrap();
        assert_eq!(fields["id"], project.id);
        assert_eq!(fields["name"], "New Project");
        assert!(fields.get("rootFile").is_none());
        assert!(
            fields["created"]
                .as_str()
                .is_some_and(|date| date.ends_with('Z'))
        );
        assert_eq!(project.root_file(), "main.tex");
        assert_eq!(
            read_source(&root, &project, "main.tex").unwrap(),
            DEFAULT_SOURCE
        );
        assert_eq!(list_projects(&root).unwrap().len(), 1);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    }

    #[test]
    fn renames_only_the_name_in_existing_project_metadata() {
        let root = tempfile::tempdir().unwrap();
        let project = create_project(root.path(), "Paper").unwrap();
        let path = root.path().join(&project.id).join(".refract.json");
        let mut original: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        original["rootFile"] = json!("sections/paper.tex");
        original["engine"] = json!("xelatex");
        original["draftMode"] = json!(true);
        original["otherMetadata"] = json!({"preserve": ["a", "b"]});
        let bytes = serde_json::to_vec(&original).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(rename_project(root.path(), &project, " \t ").is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let renamed = rename_project(root.path(), &project, "  Spectral methods  ").unwrap();
        assert_eq!(renamed.name, "Spectral methods");
        assert_eq!(renamed.root_file(), "sections/paper.tex");
        original["name"] = json!("Spectral methods");
        let updated: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(updated, original);
    }

    #[test]
    fn duplicates_visible_app_data_with_independent_comments_and_history() {
        let root = tempfile::tempdir().unwrap();
        let project = create_project(root.path(), "Paper").unwrap();
        let folder = root.path().join(&project.id);
        let path = folder.join(".refract.json");
        let mut metadata: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        metadata["rootFile"] = json!("section/paper.tex");
        metadata["engine"] = json!("lualatex");
        metadata["draftMode"] = json!(true);
        metadata["extra"] = json!("source only");
        fs::write(&path, serde_json::to_vec(&metadata).unwrap()).unwrap();
        fs::create_dir_all(folder.join("section/empty")).unwrap();
        fs::write(folder.join("section/paper.tex"), "Paper").unwrap();
        fs::write(folder.join("section/figure.bin"), [1, 255, 0]).unwrap();
        fs::write(folder.join(".refract-chat.json"), "Chat state").unwrap();
        fs::create_dir(folder.join(".refract-trash")).unwrap();
        fs::write(folder.join("section/.hidden"), "Hidden data").unwrap();
        let project = list_projects(root.path()).unwrap().pop().unwrap();
        let comments = update_comments(
            root.path(),
            &project,
            CommentUpdate::Create {
                file: "section/paper.tex".into(),
                text: "Review this paragraph".into(),
                selection: None,
            },
        )
        .unwrap();
        let version = snapshot_project(root.path(), &project).unwrap();
        fs::write(
            folder.join(".refract-versions/staging.tmp"),
            "Temporary data",
        )
        .unwrap();

        let copy = duplicate_project(root.path(), &project).unwrap();
        let copy_folder = root.path().join(&copy.id);
        assert_ne!(project.id, copy.id);
        assert_eq!(copy.name, "Paper (copy)");
        assert_eq!(copy.root_file(), "section/paper.tex");
        assert_eq!(
            fs::read(copy_folder.join("section/figure.bin")).unwrap(),
            [1, 255, 0]
        );
        assert!(copy_folder.join("section/empty").is_dir());
        for excluded in [
            ".refract-chat.json",
            ".refract-trash",
            "section/.hidden",
            ".refract-versions/staging.tmp",
        ] {
            assert!(!copy_folder.join(excluded).exists());
        }
        let copy_metadata: Value =
            serde_json::from_slice(&fs::read(copy_folder.join(".refract.json")).unwrap()).unwrap();
        assert_eq!(copy_metadata["engine"], "lualatex");
        assert_eq!(copy_metadata["draftMode"], true);
        assert!(copy_metadata.get("extra").is_none());
        assert_eq!(list_versions(root.path(), &copy).unwrap()[0].id, version.id);
        let id = comments[0].id.clone();
        assert_eq!(read_comments(root.path(), &copy).unwrap()[0].id, id);
        update_comments(
            root.path(),
            &copy,
            CommentUpdate::SetResolved { id, resolved: true },
        )
        .unwrap();
        assert!(!read_comments(root.path(), &project).unwrap()[0].resolved);
    }

    #[test]
    fn a_hard_linked_private_record_aborts_the_copy_without_publishing_it() {
        let root = tempfile::tempdir().unwrap();
        let project = create_project(root.path(), "Paper").unwrap();
        let folder = root.path().join(&project.id);
        let outside = root.path().join("outside.json");
        fs::write(&outside, "[]").unwrap();
        let linked = folder.join(".refract-comments.json");
        fs::hard_link(&outside, &linked).unwrap();
        assert!(duplicate_project(root.path(), &project).is_err());
        assert_eq!(list_projects(root.path()).unwrap().len(), 1);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
        assert_eq!(fs::read(&linked).unwrap(), b"[]");
        fs::remove_file(linked).unwrap();
        assert!(duplicate_project(root.path(), &project).is_ok());
    }
}

use serde::Deserialize;
use std::{
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

pub mod comments;
mod project_entries;
mod project_management;
pub mod versions;
pub use project_entries::{ProjectEntryKind, create_project_entry};
pub use project_management::{
    DeletedProject, create_project, duplicate_project, list_deleted_projects, rename_project,
    restore_deleted_project, trash_project,
};

const METADATA_LIMIT: usize = 64 * 1024;
const SOURCE_LIMIT: usize = 128 * 1024;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub created: Option<String>,
    root_file: Option<String>,
}

impl Project {
    pub fn root_file(&self) -> &str {
        self.root_file
            .as_deref()
            .filter(|path| !path.is_empty())
            .unwrap_or("main.tex")
    }
}

pub struct ProjectEntry {
    pub name: String,
    pub path: String,
    pub children: Option<Vec<ProjectEntry>>,
}

pub fn list_projects(projects_dir: &Path) -> io::Result<Vec<Project>> {
    let entries = match fs::read_dir(projects_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };

    let mut projects = Vec::new();
    for entry in entries {
        let entry = entry?;
        let id = entry.file_name();
        let Some(id) = id.to_str().filter(|id| is_project_id(id)) else {
            continue;
        };
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let metadata_path = entry.path().join(".refract.json");
        let Ok(metadata) = read_local_file(&entry.path(), &metadata_path, METADATA_LIMIT) else {
            continue;
        };
        if let Ok(project) = serde_json::from_slice::<Project>(&metadata)
            && project.id == id
        {
            projects.push(project);
        }
    }
    projects.sort_by(|left, right| {
        right
            .created
            .cmp(&left.created)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then(left.id.cmp(&right.id))
    });
    Ok(projects)
}

pub fn project_tree(projects_dir: &Path, project: &Project) -> io::Result<Vec<ProjectEntry>> {
    read_project_entries(&project_directory(projects_dir, project)?, "")
}

fn read_project_entries(root: &Path, parent: &str) -> io::Result<Vec<ProjectEntry>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(root.join(parent))? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str().filter(|name| valid_project_component(name)) else {
            continue;
        };
        let path = if parent.is_empty() {
            name.to_owned()
        } else {
            format!("{parent}/{name}")
        };
        let file_type = entry.file_type()?;
        let children = if file_type.is_dir() {
            Some(read_project_entries(root, &path)?)
        } else if file_type.is_file() {
            None
        } else {
            continue;
        };
        entries.push(ProjectEntry {
            name: name.to_owned(),
            path,
            children,
        });
    }
    entries.sort_by(|left, right| {
        right
            .children
            .is_some()
            .cmp(&left.children.is_some())
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(entries)
}

pub fn read_source(projects_dir: &Path, project: &Project, file: &str) -> io::Result<String> {
    let project_dir = project_directory(projects_dir, project)?;
    let target = source_path(&project_dir, file)?;
    let source = read_local_file(&project_dir, &target, SOURCE_LIMIT)?;
    if !is_source_text(&source) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "This file cannot be previewed as text.",
        ));
    }
    String::from_utf8(source).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "This file cannot be previewed as text.",
        )
    })
}

fn source_path(project_dir: &Path, file: &str) -> io::Result<PathBuf> {
    let parts = file.split(['/', '\\']);
    if !parts.clone().all(valid_project_component) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid project file path",
        ));
    }
    let mut target = project_dir.to_owned();
    for part in parts {
        target.push(part);
        if fs::symlink_metadata(&target)?.is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Linked files cannot be opened",
            ));
        }
    }
    Ok(target)
}

fn is_source_text(source: &[u8]) -> bool {
    !(source.starts_with(b"%PDF-")
        || source.starts_with(b"\x7fELF")
        || source.starts_with(b"MZ")
        || source.starts_with(b"PK\x03\x04")
        || source
            .iter()
            .any(|&byte| matches!(byte, 0..=8 | 11 | 14..=31 | 127)))
}

pub fn save_source(
    projects_dir: &Path,
    project: &Project,
    file: &str,
    expected: &str,
    edited: &str,
) -> io::Result<()> {
    let folder = project_directory(projects_dir, project)?;
    let target = source_path(&folder, file)?;
    let conflict = || {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "The file changed on disk after it was opened.",
        )
    };
    if read_local_file(&folder, &target, SOURCE_LIMIT)? != expected.as_bytes() {
        return Err(conflict());
    }
    if expected == edited {
        return Ok(());
    }
    if edited.len() > SOURCE_LIMIT
        || !is_source_text(edited.as_bytes())
        || !is_source_text(expected.as_bytes())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "The draft must be text within the preview size limit.",
        ));
    }

    let comment_path = folder.join(".refract-comments.json");
    let original_comments = comments::read_comments_file(&folder)?;
    let next_comments = match &original_comments {
        Some(bytes) => comments::plan_edit(
            bytes,
            &file.replace('\\', "/"),
            Some(expected),
            Some(edited),
        )?,
        None => None,
    };
    let staged = stage_file(&target, edited.as_bytes())?;
    let staged_comments = next_comments
        .as_deref()
        .map(|bytes| stage_file(&comment_path, bytes))
        .transpose()?;
    let rollback = if staged_comments.is_some() {
        Some(stage_file(&target, expected.as_bytes())?)
    } else {
        None
    };
    let comments_conflict = || {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Comments changed on disk during the save.",
        )
    };
    if comments::read_comments_file(&folder)? != original_comments {
        return Err(comments_conflict());
    }
    let target = source_path(&folder, file)?;
    if read_local_file(&folder, &target, SOURCE_LIMIT)? != expected.as_bytes() {
        return Err(conflict());
    }

    staged.persist(&target).map_err(|error| error.error)?;
    if let (Some(staged_comments), Some(rollback)) = (staged_comments, rollback) {
        let commit_comments = (|| {
            if comments::read_comments_file(&folder)? != original_comments {
                return Err(comments_conflict());
            }
            staged_comments
                .persist(comment_path)
                .map(|_| ())
                .map_err(|error| error.error)
        })();
        if let Err(error) = commit_comments {
            if source_path(&folder, file)
                .and_then(|target| read_local_file(&folder, &target, SOURCE_LIMIT))
                .is_ok_and(|bytes| bytes == edited.as_bytes())
                && rollback.persist(target).is_ok()
            {
                return Err(error);
            }
            return Err(io::Error::other(format!(
                "Source was saved, but the comment update and source rollback failed: {error}"
            )));
        }
    }
    Ok(())
}

fn stage_file(target: &Path, content: &[u8]) -> io::Result<tempfile::NamedTempFile> {
    let parent = target
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Missing project folder"))?;
    let permissions = fs::metadata(target)?.permissions();
    if permissions.readonly() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "The file is read-only.",
        ));
    }
    let mut file = tempfile::Builder::new()
        .prefix(".refract-save-")
        .tempfile_in(parent)?;
    file.write_all(content)?;
    file.as_file().set_permissions(permissions)?;
    file.as_file().sync_all()?;
    Ok(file)
}

fn project_directory(projects_dir: &Path, project: &Project) -> io::Result<PathBuf> {
    if !is_project_id(&project.id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid project ID",
        ));
    }
    let projects_dir = projects_dir.canonicalize()?;
    let project_dir = projects_dir.join(&project.id);
    if !fs::symlink_metadata(&project_dir)?.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid project folder",
        ));
    }
    Ok(project_dir)
}

fn is_project_id(id: &str) -> bool {
    // Match the directory names the TypeScript project store accepts.
    id.len() == 36
        && id
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f' | b'-'))
}

fn valid_project_component(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !name.contains(['/', '\\', ':', '\0'])
}

fn read_local_file(folder: &Path, path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    open_local_file(folder, path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("File exceeds the {} KiB preview limit", limit / 1024),
        ));
    }
    Ok(bytes)
}

fn open_local_file(folder: &Path, path: &Path) -> io::Result<File> {
    let folder = folder.canonicalize()?;
    let path = path.canonicalize()?;
    if !path.starts_with(&folder) || !fs::metadata(&path)?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "File is outside the project or is not a regular file",
        ));
    }
    File::open(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const LEGACY_ID: &str = "b3b9ffb5-13c0-4d22-b427-54520347684a";
    const IMPORTED_ID: &str = "cd220285-8dd2-57d7-b847-aec2740c249a";

    fn write_metadata(root: &Path, folder_id: &str, metadata: serde_json::Value) -> PathBuf {
        let folder = root.join(folder_id);
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join(".refract.json"), metadata.to_string()).unwrap();
        folder
    }

    #[test]
    fn opens_legacy_and_imported_projects_without_creating_files() {
        let root = tempfile::tempdir().unwrap();
        let absent = root.path().join("absent");
        assert!(list_projects(&absent).unwrap().is_empty());
        assert!(!absent.exists());

        let legacy = write_metadata(
            root.path(),
            LEGACY_ID,
            json!({"id": LEGACY_ID, "name": "A paper"}),
        );
        fs::write(legacy.join("main.tex"), "Legacy source").unwrap();
        let imported = write_metadata(
            root.path(),
            IMPORTED_ID,
            json!({"id": IMPORTED_ID, "name": "B paper", "rootFile": r"paper\draft.TeX", "engine": "xelatex"}),
        );
        fs::create_dir(imported.join("paper")).unwrap();
        fs::write(imported.join("paper/draft.TeX"), "Imported source").unwrap();
        let projects = list_projects(root.path()).unwrap();
        assert_eq!(projects.len(), 2);
        assert_eq!(
            read_source(root.path(), &projects[0], projects[0].root_file()).unwrap(),
            "Legacy source"
        );
        assert_eq!(
            read_source(root.path(), &projects[1], projects[1].root_file()).unwrap(),
            "Imported source"
        );
    }

    #[test]
    fn opens_nested_text_from_the_visible_project_tree() {
        let root = tempfile::tempdir().unwrap();
        let folder = write_metadata(
            root.path(),
            LEGACY_ID,
            json!({"id": LEGACY_ID, "name": "Paper"}),
        );
        fs::create_dir(folder.join("sections")).unwrap();
        fs::create_dir(folder.join(".refract-build")).unwrap();
        fs::write(folder.join("main.tex"), "Main document").unwrap();
        fs::write(folder.join("sections/abstract.tex"), "Nested document").unwrap();
        fs::write(folder.join("references.bib"), "@book{example}").unwrap();
        fs::write(folder.join(".refract-build/output.pdf"), b"%PDF-1.7").unwrap();
        let project = list_projects(root.path()).unwrap().pop().unwrap();
        let tree = project_tree(root.path(), &project).unwrap();
        assert_eq!(
            tree.iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["sections", "main.tex", "references.bib"]
        );
        let child = &tree[0].children.as_ref().unwrap()[0];
        assert_eq!(child.path, "sections/abstract.tex");
        assert_eq!(
            read_source(root.path(), &project, &child.path).unwrap(),
            "Nested document"
        );
        assert_eq!(
            read_source(root.path(), &project, "references.bib").unwrap(),
            "@book{example}"
        );
    }

    #[test]
    fn lists_the_newest_projects_first() {
        let root = tempfile::tempdir().unwrap();
        write_metadata(
            root.path(),
            LEGACY_ID,
            json!({"id": LEGACY_ID, "name": "A paper", "created": "2026-01-01T19:25:30.000Z"}),
        );
        write_metadata(
            root.path(),
            IMPORTED_ID,
            json!({"id": IMPORTED_ID, "name": "B paper", "created": "2026-04-08T08:25:30.000Z"}),
        );
        let projects = list_projects(root.path()).unwrap();
        assert_eq!(projects[0].id, IMPORTED_ID);
        assert_eq!(projects[1].id, LEGACY_ID);
    }

    #[test]
    fn skips_trash_malformed_records_and_mismatched_ids() {
        let root = tempfile::tempdir().unwrap();
        write_metadata(
            root.path(),
            ".refract-project-trash",
            json!({"id": LEGACY_ID, "name": "Deleted"}),
        );
        write_metadata(
            root.path(),
            IMPORTED_ID,
            json!({"id": LEGACY_ID, "name": "Mismatched"}),
        );
        let broken = root.path().join(LEGACY_ID);
        fs::create_dir(&broken).unwrap();
        fs::write(broken.join(".refract.json"), "{broken").unwrap();
        assert!(list_projects(root.path()).unwrap().is_empty());
    }

    #[test]
    fn rejects_paths_outside_the_project_and_bounds_large_files() {
        let root = tempfile::tempdir().unwrap();
        let folder = write_metadata(
            root.path(),
            LEGACY_ID,
            json!({"id": LEGACY_ID, "name": "Paper"}),
        );
        let project = list_projects(root.path()).unwrap().pop().unwrap();
        for path in [
            "../outside.tex",
            r"..\outside.tex",
            "/absolute.tex",
            r"C:\outside.tex",
            "main.tex:stream",
            ".refract.json",
            ".refract-build/output.pdf",
        ] {
            assert_eq!(
                read_source(root.path(), &project, path).unwrap_err().kind(),
                io::ErrorKind::InvalidInput,
                "{path}"
            );
        }
        fs::write(folder.join("main.tex"), vec![b'x'; SOURCE_LIMIT + 1]).unwrap();
        assert_eq!(
            read_source(root.path(), &project, "main.tex")
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        fs::write(folder.join("main.tex"), [0xff]).unwrap();
        assert_eq!(
            read_source(root.path(), &project, "main.tex")
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        fs::write(folder.join("document.pdf"), b"%PDF-1.7").unwrap();
        assert_eq!(
            read_source(root.path(), &project, "document.pdf")
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn saves_source_and_updates_existing_comment_anchors() {
        let root = tempfile::tempdir().unwrap();
        let folder = write_metadata(
            root.path(),
            LEGACY_ID,
            json!({"id": LEGACY_ID, "name": "Paper"}),
        );
        let source = folder.join("main.tex");
        let comments_path = folder.join(".refract-comments.json");
        let before = "😀 An abstract\r\n";
        let after = "Added\r\n😀 An abstract\r\n";
        fs::write(&source, before).unwrap();
        fs::write(
            &comments_path,
            json!([{
                "id": "one", "file": "main.tex", "quote": "abstract", "text": "Review",
                "extraMetadata": {"keep": true},
                "anchor": {
                    "start": 6, "end": 14, "prefix": "😀 An ", "suffix": "\n",
                    "sourceHash": "", "status": "attached",
                }
            }])
            .to_string(),
        )
        .unwrap();
        let project = list_projects(root.path()).unwrap().pop().unwrap();

        save_source(root.path(), &project, "main.tex", before, after).unwrap();
        assert_eq!(fs::read_to_string(&source).unwrap(), after);
        let raw_comments = fs::read(&comments_path).unwrap();
        let metadata: serde_json::Value = serde_json::from_slice(&raw_comments).unwrap();
        assert_eq!(metadata[0]["anchor"]["start"], 12);
        assert_eq!(metadata[0]["extraMetadata"]["keep"], true);
        let comments = comments::read_comments(root.path(), &project).unwrap();
        assert_eq!(comments[0].source_position(after), Some((1, 5)));

        fs::write(&source, "An external edit").unwrap();
        assert_eq!(
            save_source(root.path(), &project, "main.tex", after, "A stale draft")
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read_to_string(source).unwrap(), "An external edit");
        assert_eq!(fs::read(comments_path).unwrap(), raw_comments);
    }

    #[test]
    fn leaves_source_untouched_when_comments_cannot_be_loaded() {
        let root = tempfile::tempdir().unwrap();
        let folder = write_metadata(
            root.path(),
            LEGACY_ID,
            json!({"id": LEGACY_ID, "name": "Paper"}),
        );
        let source = folder.join("main.tex");
        fs::write(&source, "Original").unwrap();
        fs::write(folder.join(".refract-comments.json"), "invalid JSON").unwrap();
        let project = list_projects(root.path()).unwrap().pop().unwrap();
        assert!(save_source(root.path(), &project, "main.tex", "Original", "Edited").is_err());
        assert_eq!(fs::read_to_string(source).unwrap(), "Original");
    }
}

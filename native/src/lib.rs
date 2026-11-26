use serde::Deserialize;
use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};

pub mod comments;

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
    let parts = file.split(['/', '\\']);
    if !parts.clone().all(valid_project_component) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid project file path",
        ));
    }
    let project_dir = project_directory(projects_dir, project)?;
    let mut target = project_dir.clone();
    for part in parts {
        target.push(part);
        if fs::symlink_metadata(&target)?.is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Linked files cannot be previewed",
            ));
        }
    }
    let source = read_local_file(&project_dir, &target, SOURCE_LIMIT)?;
    let invalid_text = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "This file cannot be previewed as text.",
        )
    };
    if source.starts_with(b"%PDF-")
        || source.starts_with(b"\x7fELF")
        || source.starts_with(b"MZ")
        || source.starts_with(b"PK\x03\x04")
        || source
            .iter()
            .any(|&byte| matches!(byte, 0..=8 | 11 | 14..=31 | 127))
    {
        return Err(invalid_text());
    }
    String::from_utf8(source).map_err(|_| invalid_text())
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
    let folder = folder.canonicalize()?;
    let path = path.canonicalize()?;
    if !path.starts_with(&folder) || !fs::metadata(&path)?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "File is outside the project or is not a regular file",
        ));
    }
    let mut bytes = Vec::new();
    File::open(path)?
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
}

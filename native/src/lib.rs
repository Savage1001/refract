use serde::Deserialize;
use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};

const METADATA_LIMIT: usize = 64 * 1024;
const SOURCE_LIMIT: usize = 128 * 1024;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
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
    projects.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
    Ok(projects)
}

pub fn read_source(projects_dir: &Path, project: &Project) -> io::Result<String> {
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
    let relative = relative_source_path(project.root_file())?;
    let source = read_local_file(&project_dir, &project_dir.join(relative), SOURCE_LIMIT)?;
    String::from_utf8(source)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "The source is not UTF-8 text"))
}

fn is_project_id(id: &str) -> bool {
    // Match the directory names the TypeScript project store accepts.
    id.len() == 36
        && id
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f' | b'-'))
}

fn relative_source_path(name: &str) -> io::Result<PathBuf> {
    let invalid_path = || io::Error::new(io::ErrorKind::InvalidInput, "Invalid root source path");
    let mut path = PathBuf::new();
    for part in name.split(['/', '\\']) {
        if part.is_empty() || part == "." || part == ".." || part.contains([':', '\0']) {
            return Err(invalid_path());
        }
        path.push(part);
    }
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("tex"))
    {
        return Err(invalid_path());
    }
    Ok(path)
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
            read_source(root.path(), &projects[0]).unwrap(),
            "Legacy source"
        );
        assert_eq!(
            read_source(root.path(), &projects[1]).unwrap(),
            "Imported source"
        );
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
        let mut project = list_projects(root.path()).unwrap().pop().unwrap();
        for path in [
            "../outside.tex",
            r"..\outside.tex",
            "/absolute.tex",
            r"C:\outside.tex",
            "main.tex:stream",
            "image.png",
        ] {
            project.root_file = Some(path.to_owned());
            assert_eq!(
                read_source(root.path(), &project).unwrap_err().kind(),
                io::ErrorKind::InvalidInput,
                "{path}"
            );
        }
        project.root_file = None;
        fs::write(folder.join("main.tex"), vec![b'x'; SOURCE_LIMIT + 1]).unwrap();
        assert_eq!(
            read_source(root.path(), &project).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        fs::write(folder.join("main.tex"), [0xff]).unwrap();
        assert_eq!(
            read_source(root.path(), &project).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}

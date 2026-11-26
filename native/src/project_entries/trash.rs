use super::{
    CreatedFolders, ProjectEntryKind, checked_project_directory, invalid_destination,
    relocate::{check_children, check_entry, checked_target, rename_without_replacement},
};
use crate::{
    METADATA_LIMIT, Project, is_project_id, project_management::is_link, read_local_file,
    valid_project_component,
};
use chrono::{DateTime, FixedOffset, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
use uuid::Uuid;

const TRASH_DIR: &str = ".refract-trash";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeletedEntry {
    pub id: String,
    #[serde(rename = "name")]
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<ProjectEntryKind>,
    deleted: Option<String>,
}

impl DeletedEntry {
    pub fn kind(&self) -> ProjectEntryKind {
        self.kind.unwrap_or(ProjectEntryKind::File)
    }

    pub fn deleted_at(&self) -> Option<DateTime<FixedOffset>> {
        DateTime::parse_from_rfc3339(self.deleted.as_deref()?).ok()
    }
}

pub fn trash_project_entry(
    projects_dir: &Path,
    project: &Project,
    path: &str,
    kind: ProjectEntryKind,
) -> io::Result<DeletedEntry> {
    let path = normalize_path(path)?;
    let folder = checked_project_directory(projects_dir, project)?;
    let source = checked_target(&folder, &path)?;
    check_entry(&source, kind)?;
    if kind == ProjectEntryKind::Folder {
        check_children(&source)?;
    }

    let trash = folder.join(TRASH_DIR);
    match fs::create_dir(&trash) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    check_trash_directory(&trash)?;
    let record = DeletedEntry {
        id: Uuid::new_v4().to_string(),
        path,
        kind: (kind == ProjectEntryKind::Folder).then_some(kind),
        deleted: Some(Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)),
    };
    let record_path = trash.join(format!("{}.json", record.id));
    let bytes = serde_json::to_vec(&record)?;
    if bytes.len() > METADATA_LIMIT {
        return Err(invalid_destination());
    }
    let mut temporary = tempfile::Builder::new()
        .prefix(".refract-trash-")
        .tempfile_in(&trash)?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(&record_path)
        .map_err(|error| error.error)?;
    // The record is only visible to readers once its payload has moved into place.
    if let Err(error) = rename_without_replacement(&source, &trash.join(&record.id)) {
        remove_matching_record(&trash, &record_path, &bytes);
        return Err(error);
    }
    Ok(record)
}

pub fn list_deleted_entries(
    projects_dir: &Path,
    project: &Project,
) -> io::Result<Vec<DeletedEntry>> {
    let trash = checked_project_directory(projects_dir, project)?.join(TRASH_DIR);
    match check_trash_directory(&trash) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    }
    let mut records = Vec::new();
    for entry in fs::read_dir(&trash)? {
        let name = entry?.file_name();
        if let Some(id) = name
            .to_str()
            .and_then(|name| name.strip_suffix(".json"))
            .filter(|id| is_project_id(id))
            && let Some((record, _)) = read_record(&trash, id)?
        {
            records.push(record);
        }
    }
    records.sort_by(|left, right| {
        right
            .deleted
            .cmp(&left.deleted)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(records)
}

pub fn restore_deleted_entry(
    projects_dir: &Path,
    project: &Project,
    id: &str,
) -> io::Result<DeletedEntry> {
    if !is_project_id(id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid deleted file",
        ));
    }
    let folder = checked_project_directory(projects_dir, project)?;
    let trash = folder.join(TRASH_DIR);
    check_trash_directory(&trash)?;
    let (record, bytes) = read_record(&trash, id)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Deleted file not found"))?;
    let source = trash.join(id);
    if record.kind() == ProjectEntryKind::Folder {
        check_children(&source)?;
    }
    let mut parents = CreatedFolders::new(&folder)?;
    let (parent, name) = record.path.rsplit_once('/').unwrap_or(("", &record.path));
    let target = parents.ensure(parent)?.join(name);
    if let Err(error) = rename_without_replacement(&source, &target) {
        if target.symlink_metadata().is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "A file or folder already exists at the original path",
            ));
        }
        return Err(error);
    }
    parents.keep();
    remove_matching_record(&trash, &trash.join(format!("{id}.json")), &bytes);
    Ok(record)
}

fn normalize_path(path: &str) -> io::Result<String> {
    let path = path.replace('\\', "/");
    if !path.split('/').all(valid_project_component) {
        return Err(invalid_destination());
    }
    Ok(path)
}

fn check_trash_directory(path: &Path) -> io::Result<()> {
    let info = path.symlink_metadata()?;
    if !info.is_dir() || is_link(&info) || path.canonicalize()? != path {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid deleted files directory",
        ));
    }
    Ok(())
}

fn read_record(trash: &Path, id: &str) -> io::Result<Option<(DeletedEntry, Vec<u8>)>> {
    let record_path = trash.join(format!("{id}.json"));
    let bytes = match check_entry(&record_path, ProjectEntryKind::File)
        .and_then(|()| read_local_file(trash, &record_path, METADATA_LIMIT))
    {
        Ok(bytes) => bytes,
        Err(error) if is_missing_or_invalid(&error) => return Ok(None),
        Err(error) => return Err(error),
    };
    let Ok(mut record) = serde_json::from_slice::<DeletedEntry>(&bytes) else {
        return Ok(None);
    };
    if record.id != id {
        return Ok(None);
    }
    let Ok(path) = normalize_path(&record.path) else {
        return Ok(None);
    };
    record.path = path;
    match check_entry(&trash.join(id), record.kind()) {
        Ok(()) => Ok(Some((record, bytes))),
        Err(error) if is_missing_or_invalid(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

fn is_missing_or_invalid(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData
    )
}

fn remove_matching_record(trash: &Path, path: &Path, bytes: &[u8]) {
    if check_entry(path, ProjectEntryKind::File)
        .and_then(|()| read_local_file(trash, path, METADATA_LIMIT))
        .is_ok_and(|current| current == bytes)
    {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{create_project, project_tree};
    use serde_json::{Value, json};

    const LEGACY_ID: &str = "cc208c11-54a0-466b-9e6c-9f71513e001b";

    #[test]
    fn round_trips_root_files_and_folders_with_the_electron_record_format() {
        let root = tempfile::tempdir().unwrap();
        let project = create_project(root.path(), "Paper").unwrap();
        let folder = root.path().join(&project.id);
        let metadata = fs::read(folder.join(".refract.json")).unwrap();
        fs::create_dir_all(folder.join("chapters/empty")).unwrap();
        fs::write(folder.join("chapters/.source"), [0, 255]).unwrap();
        let source = fs::read(folder.join("main.tex")).unwrap();
        let main =
            trash_project_entry(root.path(), &project, "main.tex", ProjectEntryKind::File).unwrap();
        let chapters =
            trash_project_entry(root.path(), &project, "chapters", ProjectEntryKind::Folder)
                .unwrap();
        let trash = folder.join(TRASH_DIR);
        let read_json = |id: &str| -> Value {
            serde_json::from_slice(&fs::read(trash.join(format!("{id}.json"))).unwrap()).unwrap()
        };
        let main_json = read_json(&main.id);
        assert!(main_json.get("kind").is_none());
        assert_eq!(main_json["name"], "main.tex");
        assert!(main.deleted_at().is_some());
        assert_eq!(read_json(&chapters.id)["kind"], "folder");
        assert!(project_tree(root.path(), &project).unwrap().is_empty());
        assert_eq!(
            list_deleted_entries(root.path(), &project).unwrap().len(),
            2
        );
        assert_eq!(
            restore_deleted_entry(root.path(), &project, &main.id).unwrap(),
            main
        );
        assert_eq!(
            restore_deleted_entry(root.path(), &project, &chapters.id).unwrap(),
            chapters
        );
        assert_eq!(fs::read(folder.join("main.tex")).unwrap(), source);
        assert_eq!(fs::read(folder.join("chapters/.source")).unwrap(), [0, 255]);
        assert!(folder.join("chapters/empty").is_dir());
        assert_eq!(fs::read(folder.join(".refract.json")).unwrap(), metadata);
        assert!(
            list_deleted_entries(root.path(), &project)
                .unwrap()
                .is_empty()
        );
        assert!(fs::read_dir(trash).unwrap().next().is_none());
    }

    #[test]
    fn restores_legacy_paths_and_preserves_deleted_items_on_conflict() {
        let root = tempfile::tempdir().unwrap();
        let project = create_project(root.path(), "Paper").unwrap();
        let folder = root.path().join(&project.id);
        let trash = folder.join(TRASH_DIR);
        let record = trash.join(format!("{LEGACY_ID}.json"));
        fs::create_dir(&trash).unwrap();
        fs::write(trash.join(LEGACY_ID), "Deleted notes").unwrap();
        fs::write(
            &record,
            json!({"id": LEGACY_ID, "name": "chapters\\notes.tex"}).to_string(),
        )
        .unwrap();
        let entry = list_deleted_entries(root.path(), &project)
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(entry.path, "chapters/notes.tex");
        assert_eq!(entry.kind(), ProjectEntryKind::File);
        fs::create_dir(folder.join("chapters")).unwrap();
        let destination = folder.join(&entry.path);
        fs::write(&destination, "Existing notes").unwrap();
        let conflict = restore_deleted_entry(root.path(), &project, LEGACY_ID).unwrap_err();
        assert_eq!(conflict.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&destination).unwrap(), "Existing notes");
        assert!(record.exists());
        assert_eq!(
            fs::read_to_string(trash.join(LEGACY_ID)).unwrap(),
            "Deleted notes"
        );
        fs::remove_file(destination).unwrap();
        fs::remove_dir(folder.join("chapters")).unwrap();
        restore_deleted_entry(root.path(), &project, LEGACY_ID).unwrap();
        assert_eq!(
            fs::read_to_string(folder.join(entry.path)).unwrap(),
            "Deleted notes"
        );
    }

    #[test]
    fn skips_unpublished_records_and_rejects_paths_outside_the_project() {
        let root = tempfile::tempdir().unwrap();
        let project = create_project(root.path(), "Paper").unwrap();
        let trash = root.path().join(&project.id).join(TRASH_DIR);
        fs::create_dir(&trash).unwrap();
        let record = trash.join(format!("{LEGACY_ID}.json"));
        fs::write(
            &record,
            json!({"id": LEGACY_ID, "name": "main.tex"}).to_string(),
        )
        .unwrap();
        assert!(
            list_deleted_entries(root.path(), &project)
                .unwrap()
                .is_empty()
        );
        fs::write(trash.join(LEGACY_ID), "Keep me").unwrap();
        fs::write(
            record,
            json!({"id": LEGACY_ID, "name": "../paper.tex"}).to_string(),
        )
        .unwrap();
        assert!(
            list_deleted_entries(root.path(), &project)
                .unwrap()
                .is_empty()
        );
        assert!(restore_deleted_entry(root.path(), &project, LEGACY_ID).is_err());
        assert!(!root.path().join("paper.tex").exists());
        assert_eq!(
            fs::read_to_string(trash.join(LEGACY_ID)).unwrap(),
            "Keep me"
        );
    }
}

use super::{
    METADATA_LIMIT, Project, invalid_metadata, is_link, is_project_id, read_local_file,
    read_project_metadata,
};
use chrono::{DateTime, FixedOffset, SecondsFormat, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};
use uuid::Uuid;

const TRASH_DIR: &str = ".refract-project-trash";
const RETENTION: TimeDelta = TimeDelta::days(30);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletedProject {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub deleted: Option<String>,
}

impl DeletedProject {
    pub fn deleted_at(&self) -> Option<DateTime<FixedOffset>> {
        DateTime::parse_from_rfc3339(self.deleted.as_deref()?).ok()
    }

    pub fn days_remaining(&self, now: DateTime<Utc>) -> Option<i64> {
        let elapsed = now.signed_duration_since(self.deleted_at()?);
        let remaining = (RETENTION - elapsed).num_milliseconds().max(0);
        let day = TimeDelta::days(1).num_milliseconds();
        Some(remaining.saturating_add(day - 1) / day)
    }

    fn expired(&self, now: DateTime<Utc>) -> bool {
        self.deleted_at()
            .is_some_and(|date| now.signed_duration_since(date) >= RETENTION)
    }
}

pub fn trash_project(projects_dir: &Path, project: &Project) -> io::Result<DeletedProject> {
    if !is_project_id(&project.id) {
        return Err(invalid_metadata());
    }
    let root = project_root(projects_dir)?;
    let source = root.join(&project.id);
    let stored = project_identity(&source, &project.id)?;
    let trash = root.join(TRASH_DIR);
    match fs::create_dir(&trash) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    check_directory(&trash)?;
    let record = DeletedProject {
        id: Uuid::new_v4().to_string(),
        project_id: stored.id,
        name: stored.name,
        deleted: Some(Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)),
    };
    let record_path = trash.join(format!("{}.json", record.id));
    let bytes = serde_json::to_vec(&record)?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".refract-trash-")
        .tempfile_in(&trash)?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(&record_path)
        .map_err(|error| error.error)?;
    if let Err(error) = move_project(&source, &trash.join(&record.id), &record.project_id) {
        if read_local_file(&trash, &record_path, METADATA_LIMIT)
            .is_ok_and(|current| current == bytes)
        {
            let _ = fs::remove_file(&record_path);
        }
        return Err(error);
    }
    Ok(record)
}

pub fn list_deleted_projects(projects_dir: &Path) -> io::Result<Vec<DeletedProject>> {
    let Some((_, trash)) = find_trash(projects_dir)? else {
        return Ok(Vec::new());
    };
    let now = Utc::now();
    let mut records = Vec::new();
    for entry in fs::read_dir(&trash)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(id) = name
            .to_str()
            .and_then(|name| name.strip_suffix(".json"))
            .filter(|id| is_project_id(id))
        else {
            continue;
        };
        if let Some((record, _)) = read_entry(&trash, id)? {
            if record.expired(now) {
                expire_entry(&trash, &record)?;
            } else {
                records.push(record);
            }
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

pub fn restore_deleted_project(projects_dir: &Path, id: &str) -> io::Result<Project> {
    if !is_project_id(id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid deleted project",
        ));
    }
    let Some((root, trash)) = find_trash(projects_dir)? else {
        return Err(missing_project());
    };
    let (record, project) = read_entry(&trash, id)?.ok_or_else(missing_project)?;
    if record.expired(Utc::now()) {
        expire_entry(&trash, &record)?;
        return Err(missing_project());
    }
    move_project(
        &trash.join(id),
        &root.join(&record.project_id),
        &record.project_id,
    )?;
    remove_record(&trash.join(format!("{id}.json")))?;
    Ok(project)
}

fn project_root(projects_dir: &Path) -> io::Result<PathBuf> {
    let info = projects_dir.symlink_metadata()?;
    if !info.is_dir() || is_link(&info) {
        return Err(invalid_metadata());
    }
    projects_dir.canonicalize()
}

fn find_trash(projects_dir: &Path) -> io::Result<Option<(PathBuf, PathBuf)>> {
    let root = match project_root(projects_dir) {
        Ok(root) => root,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let trash = root.join(TRASH_DIR);
    match check_directory(&trash) {
        Ok(()) => Ok(Some((root, trash))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn read_entry(trash: &Path, id: &str) -> io::Result<Option<(DeletedProject, Project)>> {
    let path = trash.join(format!("{id}.json"));
    let record = (|| {
        let info = path.symlink_metadata()?;
        if !info.is_file() || is_link(&info) || path.canonicalize()? != path {
            return Err(invalid_metadata());
        }
        let record: DeletedProject =
            serde_json::from_slice(&read_local_file(trash, &path, METADATA_LIMIT)?)?;
        if record.id != id || !is_project_id(&record.project_id) {
            return Err(invalid_metadata());
        }
        let project = project_identity(&trash.join(id), &record.project_id)?;
        Ok((record, project))
    })();
    match record {
        Ok(record) => Ok(Some(record)),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::InvalidData | io::ErrorKind::NotFound
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn project_identity(directory: &Path, id: &str) -> io::Result<Project> {
    check_directory(directory)?;
    let path = directory.join(".refract.json");
    let project: Project = serde_json::from_slice(&read_project_metadata(directory, &path)?)?;
    if project.id != id {
        return Err(invalid_metadata());
    }
    Ok(project)
}

fn check_directory(path: &Path) -> io::Result<()> {
    let info = path.symlink_metadata()?;
    if !info.is_dir() || is_link(&info) || path.canonicalize()? != path {
        return Err(invalid_metadata());
    }
    Ok(())
}

fn move_project(source: &Path, target: &Path, id: &str) -> io::Result<()> {
    let target_parent = target.parent().ok_or_else(invalid_metadata)?;
    let mut delays = [80, 160, 320, 640].into_iter();
    loop {
        project_identity(source, id)?;
        check_directory(target_parent)?;
        match target.symlink_metadata() {
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "A project with this ID already exists",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        match fs::rename(source, target) {
            Err(error) if cfg!(windows) && matches!(error.raw_os_error(), Some(5 | 32 | 33)) => {
                let Some(delay) = delays.next() else {
                    return Err(error);
                };
                thread::sleep(Duration::from_millis(delay));
            }
            result => return result,
        }
    }
}

fn expire_entry(trash: &Path, record: &DeletedProject) -> io::Result<()> {
    let source = trash.join(&record.id);
    check_directory(trash)?;
    let current = read_entry(trash, &record.id)?.map(|(current, _)| current);
    if current.as_ref() != Some(record) || !record.expired(Utc::now()) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Deleted project changed during cleanup",
        ));
    }
    fs::remove_dir_all(source)?;
    remove_record(&trash.join(format!("{}.json", record.id)))
}

fn remove_record(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn missing_project() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "Deleted project not found")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        create_project, list_projects, save_source,
        versions::{list_versions, snapshot_project},
    };

    #[test]
    fn restores_a_project_with_its_original_id_files_and_private_data() {
        let root = tempfile::tempdir().unwrap();
        let project = create_project(root.path(), "Paper").unwrap();
        let active = root.path().join(&project.id);
        fs::write(active.join(".refract-chat.json"), "Private chat data").unwrap();
        fs::write(active.join("figure.bin"), [0, 255, 9]).unwrap();
        let version = snapshot_project(root.path(), &project).unwrap();
        let record = trash_project(root.path(), &project).unwrap();
        assert!(list_projects(root.path()).unwrap().is_empty());
        assert_eq!(
            list_deleted_projects(root.path()).unwrap(),
            [record.clone()]
        );
        assert_eq!(record.days_remaining(Utc::now()), Some(30));
        assert!(!active.exists());
        assert!(save_source(root.path(), &project, "main.tex", "", "Late edit").is_err());
        fs::create_dir(&active).unwrap();
        fs::write(active.join("other.txt"), "Occupied directory").unwrap();
        assert_eq!(
            restore_deleted_project(root.path(), &record.id)
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(
            fs::read_to_string(active.join("other.txt")).unwrap(),
            "Occupied directory"
        );
        fs::remove_file(active.join("other.txt")).unwrap();
        fs::remove_dir(&active).unwrap();
        let restored = restore_deleted_project(root.path(), &record.id).unwrap();
        assert_eq!(restored.id, project.id);
        assert_eq!(fs::read(active.join("figure.bin")).unwrap(), [0, 255, 9]);
        assert_eq!(
            fs::read_to_string(active.join(".refract-chat.json")).unwrap(),
            "Private chat data"
        );
        assert_eq!(
            list_versions(root.path(), &restored).unwrap()[0].id,
            version.id
        );
        assert!(list_deleted_projects(root.path()).unwrap().is_empty());
    }

    #[test]
    fn expires_only_deleted_projects_with_valid_records_after_thirty_days() {
        let root = tempfile::tempdir().unwrap();
        let old = trash_project(root.path(), &create_project(root.path(), "Old").unwrap()).unwrap();
        let later =
            trash_project(root.path(), &create_project(root.path(), "Later").unwrap()).unwrap();
        let unverified = trash_project(
            root.path(),
            &create_project(root.path(), "Unverified").unwrap(),
        )
        .unwrap();
        let trash = root.path().join(TRASH_DIR);
        let old_record = trash.join(format!("{}.json", old.id));
        let unverified_record = trash.join(format!("{}.json", unverified.id));
        let mut expired = old.clone();
        expired.deleted =
            Some((Utc::now() - RETENTION).to_rfc3339_opts(SecondsFormat::Millis, true));
        fs::write(&old_record, serde_json::to_vec(&expired).unwrap()).unwrap();
        let mut mismatched = unverified.clone();
        mismatched.project_id = old.project_id.clone();
        mismatched.deleted = expired.deleted;
        fs::write(&unverified_record, serde_json::to_vec(&mismatched).unwrap()).unwrap();
        assert_eq!(list_deleted_projects(root.path()).unwrap(), [later]);
        assert!(!old_record.exists());
        assert!(!trash.join(&old.id).exists());
        assert!(trash.join(&unverified.id).join("main.tex").is_file());
        assert!(unverified_record.exists());
        assert_eq!(
            restore_deleted_project(root.path(), &old.id)
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
        assert!(restore_deleted_project(root.path(), "../main").is_err());
    }
}

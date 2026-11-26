use super::{
    MANIFEST_LIMIT, Project, copy_and_hash, is_project_id, open_local_file, open_version,
    project_directory, read_local_file, valid_project_component, valid_version_path, version_file,
};
use serde::Deserialize;
use std::{
    fs::{self, File, Metadata},
    io::{self, Read},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};
use zip::ZipArchive;

const STAGE_PREFIX: &str = ".refract-restore-";
const CLEANUP_PREFIX: &str = ".refract-cleanup-";
const OWNER_FILE: &str = ".refract-stage-owner";

#[derive(Deserialize)]
struct CleanupRecord {
    stages: Vec<RestoreStage>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RestoreStage {
    id: String,
    owner: String,
    target_id: String,
    before_id: String,
    files: Vec<String>,
    blockers: Vec<String>,
}

pub fn retry_retained_restore_stages(projects_dir: &Path, project: &Project) -> io::Result<usize> {
    let folder = project_directory(projects_dir, project)?;
    for entry in fs::read_dir(&folder)? {
        let entry = entry?;
        if record_id(
            &entry.file_name().to_string_lossy(),
            CLEANUP_PREFIX,
            ".json",
        )
        .is_some()
        {
            // A damaged cleanup record must not prevent access to the project.
            let _ = retry_cleanup_record(&folder, &entry.path());
        }
    }
    let mut retained = 0;
    for entry in fs::read_dir(&folder)? {
        if record_id(&entry?.file_name().to_string_lossy(), STAGE_PREFIX, "").is_some() {
            retained += 1;
        }
    }
    Ok(retained)
}

fn record_id<'a>(name: &'a str, prefix: &str, suffix: &str) -> Option<&'a str> {
    name.strip_prefix(prefix)?
        .strip_suffix(suffix)
        .filter(|id| is_project_id(id))
}

fn retry_cleanup_record(folder: &Path, path: &Path) -> io::Result<()> {
    let original = checked_info(path)?;
    if !original.is_file() {
        return Err(invalid_stage());
    }
    let bytes = read_local_file(folder, path, MANIFEST_LIMIT)?;
    let record: CleanupRecord = serde_json::from_slice(&bytes)?;
    let mut retained = false;
    for stage in record.stages {
        retained |= retry_stage(folder, &stage).is_err();
    }
    if retained
        || !same_entry(&checked_info(path)?, &original)
        || read_local_file(folder, path, MANIFEST_LIMIT)? != bytes
    {
        return Err(invalid_stage());
    }
    fs::remove_file(path)
}

fn retry_stage(folder: &Path, record: &RestoreStage) -> io::Result<()> {
    for delay in [50, 100] {
        match clean_stage(folder, record) {
            Ok(()) => return Ok(()),
            Err(error)
                if error.kind() == io::ErrorKind::PermissionDenied
                    || cfg!(windows) && matches!(error.raw_os_error(), Some(32 | 33)) =>
            {
                thread::sleep(Duration::from_millis(delay));
            }
            Err(error) => return Err(error),
        }
    }
    clean_stage(folder, record)
}

fn clean_stage(folder: &Path, record: &RestoreStage) -> io::Result<()> {
    if ![
        &record.id,
        &record.owner,
        &record.target_id,
        &record.before_id,
    ]
    .into_iter()
    .all(|id| is_project_id(id))
    {
        return Err(invalid_stage());
    }
    let stage = folder.join(format!("{STAGE_PREFIX}{}", record.id));
    let info = match checked_info(&stage) {
        Ok(info) if info.is_dir() => info,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Ok(_) => return Err(invalid_stage()),
        Err(error) => return Err(error),
    };
    let marker = stage.join(OWNER_FILE);
    let marker_info = checked_info(&marker)?;
    if !marker_info.is_file() || read_local_file(folder, &marker, 36)? != record.owner.as_bytes() {
        return Err(invalid_stage());
    }
    let (target, mut target_archive) = open_version(folder, &record.target_id)?;
    let (before, mut before_archive) = open_version(folder, &record.before_id)?;
    let Some(before_dirs) = before.directories.as_deref() else {
        return Err(invalid_stage());
    };
    if !record.files.iter().all(|file| target.files.contains(file))
        || !record.blockers.iter().all(|name| valid_version_path(name))
    {
        return Err(invalid_stage());
    }

    let mut removals = Vec::new();
    for entry in fs::read_dir(&stage)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(invalid_stage)?;
        if name == OWNER_FILE {
            continue;
        }
        if let Some(index) = name.strip_prefix("blocked-").and_then(stage_index) {
            let original = index
                .checked_sub(record.files.len())
                .and_then(|index| record.blockers.get(index))
                .ok_or_else(invalid_stage)?;
            inspect_entry(
                folder,
                entry.path(),
                original,
                &before.files,
                before_dirs,
                &mut before_archive,
                &mut removals,
            )?;
        } else {
            let original = stage_index(name)
                .and_then(|index| record.files.get(index))
                .ok_or_else(invalid_stage)?;
            inspect_entry(
                folder,
                entry.path(),
                original,
                &target.files,
                &[],
                &mut target_archive,
                &mut removals,
            )?;
        }
    }
    removals.push((marker, marker_info));
    removals.push((stage, info));
    // Verify the entire stage before removing any entry, then remove directories bottom up.
    for (path, original) in &removals {
        if !same_entry(&checked_info(path)?, original) {
            return Err(invalid_stage());
        }
    }
    for (path, original) in removals {
        if original.is_dir() {
            fs::remove_dir(path)?;
        } else {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn stage_index(name: &str) -> Option<usize> {
    let index: usize = name.parse().ok()?;
    (index.to_string() == name).then_some(index)
}

fn inspect_entry(
    folder: &Path,
    path: PathBuf,
    name: &str,
    files: &[String],
    directories: &[String],
    archive: &mut ZipArchive<File>,
    removals: &mut Vec<(PathBuf, Metadata)>,
) -> io::Result<()> {
    let info = checked_info(&path)?;
    if info.is_file() && files.iter().any(|file| file == name) {
        let entry = version_file(archive, name)?;
        let size = entry.size();
        if size != info.len() {
            return Err(invalid_stage());
        }
        let expected = copy_and_hash(entry.take(size.saturating_add(1)), &mut io::sink())?;
        let actual = copy_and_hash(
            open_local_file(folder, &path)?.take(size.saturating_add(1)),
            &mut io::sink(),
        )?;
        if expected != actual || actual.0 != size {
            return Err(invalid_stage());
        }
    } else if info.is_dir() && directories.iter().any(|directory| directory == name) {
        for entry in fs::read_dir(&path)? {
            let entry = entry?;
            let child = entry.file_name();
            let child = child
                .to_str()
                .filter(|name| valid_project_component(name))
                .ok_or_else(invalid_stage)?;
            inspect_entry(
                folder,
                entry.path(),
                &format!("{name}/{child}"),
                files,
                directories,
                archive,
                removals,
            )?;
        }
    } else {
        return Err(invalid_stage());
    }
    removals.push((path, info));
    Ok(())
}

fn checked_info(path: &Path) -> io::Result<Metadata> {
    let info = path.symlink_metadata()?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if info.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(invalid_stage());
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if info.is_file() && info.nlink() > 1 {
            return Err(invalid_stage());
        }
    }
    if info.is_symlink() || path.canonicalize()? != path {
        return Err(invalid_stage());
    }
    Ok(info)
}

fn same_entry(left: &Metadata, right: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if left.dev() != right.dev() || left.ino() != right.ino() || left.nlink() != right.nlink() {
            return false;
        }
    }
    left.file_type() == right.file_type()
        && left.len() == right.len()
        && left.created().ok() == right.created().ok()
        && left.modified().ok() == right.modified().ok()
}

fn invalid_stage() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Restore stage could not be verified",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{list_projects, versions::snapshot_project};
    use serde_json::json;

    const PROJECT_ID: &str = "b3b9ffb5-13c0-4d22-b427-54520347684a";
    const STAGE_ID: &str = "11111111-1111-1111-1111-111111111111";
    const OWNER: &str = "22222222-2222-2222-2222-222222222222";
    const CLEANUP_ID: &str = "33333333-3333-3333-3333-333333333333";
    const UNOWNED_ID: &str = "44444444-4444-4444-4444-444444444444";

    struct Fixture {
        root: tempfile::TempDir,
        project: Project,
        folder: PathBuf,
        stage: PathBuf,
        record: PathBuf,
    }

    fn retained_stage() -> Fixture {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join(PROJECT_ID);
        fs::create_dir_all(folder.join("chapter")).unwrap();
        fs::write(
            folder.join(".refract.json"),
            json!({"id": PROJECT_ID, "name": "Paper"}).to_string(),
        )
        .unwrap();
        fs::write(folder.join("chapter/text.tex"), "Chapter").unwrap();
        let project = list_projects(root.path()).unwrap().pop().unwrap();
        let before = snapshot_project(root.path(), &project).unwrap();
        let stage = folder.join(format!("{STAGE_PREFIX}{STAGE_ID}"));
        fs::create_dir(&stage).unwrap();
        fs::write(stage.join(OWNER_FILE), OWNER).unwrap();
        fs::rename(folder.join("chapter"), stage.join("blocked-1")).unwrap();
        fs::write(folder.join("chapter"), "Replacement").unwrap();
        let target = snapshot_project(root.path(), &project).unwrap();
        fs::write(stage.join("0"), "Replacement").unwrap();
        let record = folder.join(format!("{CLEANUP_PREFIX}{CLEANUP_ID}.json"));
        fs::write(
            &record,
            json!({"recoveryId": before.id, "stages": [{
                "id": STAGE_ID, "owner": OWNER, "targetId": target.id, "beforeId": before.id,
                "files": ["chapter"], "blockers": ["chapter"]
            }]})
            .to_string(),
        )
        .unwrap();
        Fixture {
            root,
            project,
            folder,
            stage,
            record,
        }
    }

    #[test]
    fn removes_a_verified_electron_cleanup_stage_and_its_record() {
        let fixture = retained_stage();
        assert_eq!(
            retry_retained_restore_stages(fixture.root.path(), &fixture.project).unwrap(),
            0
        );
        assert!(!fixture.stage.exists());
        assert!(!fixture.record.exists());
        assert_eq!(
            fs::read_to_string(fixture.folder.join("chapter")).unwrap(),
            "Replacement"
        );
    }

    #[test]
    fn preserves_an_entire_stage_if_any_entry_cannot_be_verified() {
        for changed_path in [
            "blocked-1/text.tex",
            "blocked-1/.hidden",
            "unexpected",
            OWNER_FILE,
        ] {
            let fixture = retained_stage();
            let changed = fixture.stage.join(changed_path);
            fs::write(&changed, "Do not discard").unwrap();
            assert_eq!(
                retry_retained_restore_stages(fixture.root.path(), &fixture.project).unwrap(),
                1
            );
            assert_eq!(fs::read_to_string(changed).unwrap(), "Do not discard");
            assert_eq!(
                fs::read_to_string(fixture.stage.join("0")).unwrap(),
                "Replacement"
            );
            assert!(fixture.record.exists());
        }
    }

    #[test]
    fn leaves_unrecorded_stages_and_an_active_restore_alone() {
        let fixture = retained_stage();
        let unowned = fixture.folder.join(format!("{STAGE_PREFIX}{UNOWNED_ID}"));
        fs::create_dir(&unowned).unwrap();
        fs::write(unowned.join("content"), "Unrecorded").unwrap();
        let active = fixture.folder.join(".refract-restore-journal.json");
        fs::rename(&fixture.record, &active).unwrap();
        assert_eq!(
            retry_retained_restore_stages(fixture.root.path(), &fixture.project).unwrap(),
            2
        );
        assert!(active.exists());
        assert!(fixture.stage.join("0").exists());
        assert_eq!(
            fs::read_to_string(unowned.join("content")).unwrap(),
            "Unrecorded"
        );
    }
}

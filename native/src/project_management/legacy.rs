use super::{
    DEFAULT_SOURCE, is_link, new_metadata, publish_project, read_project_metadata, write_new_file,
};
use crate::read_local_file;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::{self, ErrorKind, Write},
    path::Path,
};
use tempfile::NamedTempFile;
use uuid::Uuid;

const SOURCE_FILE: &str = "projects.json";
const PROGRESS_FILE: &str = "legacy-migration-progress.json";
const COMPLETION_FILE: &str = "legacy-migrated.json";
const IMPORT_KEY: &str = "legacyMigrationKey";
const LEGACY_DATA_LIMIT: usize = 128 * 1024 * 1024;
const PROGRESS_LIMIT: usize = 1024 * 1024;

/// Resume the prototype project import recorded by Electron. Leave its source
/// intact, and write the Electron markers so opening the old app after the
/// native import does not create additional copies.
pub fn migrate_legacy_projects(projects_dir: &Path) -> io::Result<()> {
    let profile = projects_dir
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let marker = profile.join(COMPLETION_FILE);
    if regular_file_exists(&marker)? {
        return Ok(());
    }
    let original = profile.join(SOURCE_FILE);
    if !regular_file_exists(&original)? {
        return Ok(());
    }
    let legacy: Vec<LegacyProject> =
        serde_json::from_slice(&read_local_file(profile, &original, LEGACY_DATA_LIMIT)?)?;
    let progress = profile.join(PROGRESS_FILE);
    let mut done: BTreeSet<usize> = if regular_file_exists(&progress)? {
        serde_json::from_slice(&read_local_file(profile, &progress, PROGRESS_LIMIT)?)?
    } else {
        BTreeSet::new()
    };
    fs::create_dir_all(projects_dir)?;
    let info = projects_dir.symlink_metadata()?;
    if !info.is_dir() || is_link(&info) {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "Invalid project storage directory",
        ));
    }
    for (index, project) in legacy.iter().enumerate() {
        if !done.contains(&index) {
            import_project(projects_dir, index, project)?;
            done.insert(index);
            save_json(&progress, &serde_json::to_vec(&done)?, true)?;
        }
    }
    save_json(
        &marker,
        &serde_json::to_vec(&json!({"count": done.len()}))?,
        false,
    )
}

#[derive(Deserialize)]
struct LegacyProject {
    name: Option<String>,
    source: Option<Value>,
}

fn import_project(projects_dir: &Path, index: usize, old: &LegacyProject) -> io::Result<()> {
    let name = old
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("New Project");
    let source = old
        .source
        .as_ref()
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_SOURCE);
    let (id, fingerprint) = project_identity(index, name, source);
    let target = projects_dir.join(&id);
    if matches_import(&target, &id, &fingerprint)? {
        return Ok(());
    }
    let mut metadata = new_metadata(name);
    metadata.insert("id".to_owned(), Value::String(id.clone()));
    metadata.insert(IMPORT_KEY.to_owned(), Value::String(fingerprint.clone()));
    match publish_project(projects_dir, &metadata, |stage| {
        write_new_file(&stage.join("main.tex"), source.as_bytes())
    }) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            if matches_import(&target, &id, &fingerprint)? {
                Ok(())
            } else {
                Err(error)
            }
        }
        Err(error) => Err(error),
    }
}

fn project_identity(index: usize, name: &str, source: &str) -> (String, String) {
    let mut hash = Sha256::new();
    hash.update(b"Refract prototype project v1");
    hash.update((index as u64).to_be_bytes());
    hash.update((name.len() as u64).to_be_bytes());
    hash.update(name.as_bytes());
    hash.update(source.as_bytes());
    let hash = hash.finalize();
    let mut id = [0; 16];
    id.copy_from_slice(&hash[..16]);
    id[6] = (id[6] & 0x0f) | 0x80;
    id[8] = (id[8] & 0x3f) | 0x80;
    (Uuid::from_bytes(id).to_string(), format!("{hash:x}"))
}

fn matches_import(target: &Path, id: &str, fingerprint: &str) -> io::Result<bool> {
    match target.symlink_metadata() {
        Ok(info) if info.is_dir() && !is_link(&info) => {}
        Ok(_) => return Ok(false),
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    }
    match read_project_metadata(target, &target.join(".refract.json")) {
        Ok(bytes) => {
            let data: Value = serde_json::from_slice(&bytes)?;
            Ok(data["id"].as_str() == Some(id) && data[IMPORT_KEY].as_str() == Some(fingerprint))
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn regular_file_exists(path: &Path) -> io::Result<bool> {
    match path.symlink_metadata() {
        Ok(info) if info.is_file() && !is_link(&info) => Ok(true),
        Ok(_) => Err(io::Error::new(
            ErrorKind::InvalidInput,
            "Invalid prototype migration file",
        )),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn save_json(path: &Path, bytes: &[u8], replace: bool) -> io::Result<()> {
    let profile = path
        .parent()
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "Missing profile directory"))?;
    let mut stage = NamedTempFile::new_in(profile)?;
    stage.write_all(bytes)?;
    stage.as_file().sync_all()?;
    if replace {
        stage.persist(path).map_err(|error| error.error)?;
    } else if let Err(error) = stage.persist_noclobber(path)
        && (error.error.kind() != ErrorKind::AlreadyExists || !regular_file_exists(path)?)
    {
        return Err(error.error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{create_project, list_projects, read_source, rename_project, save_source};

    #[test]
    fn imports_prototype_documents_without_changing_the_original() {
        let home = tempfile::tempdir().unwrap();
        let projects = home.path().join("projects");
        let original = br#"[{"name":" Paper ","source":"\u03bc: exact source\n"},{"name":" "}]"#;
        fs::write(home.path().join(SOURCE_FILE), original).unwrap();

        migrate_legacy_projects(&projects).unwrap();
        migrate_legacy_projects(&projects).unwrap();
        let all = list_projects(&projects).unwrap();
        assert_eq!(all.len(), 2);
        let paper = all.iter().find(|project| project.name == "Paper").unwrap();
        let new = all
            .iter()
            .find(|project| project.name == "New Project")
            .unwrap();
        assert_eq!(
            read_source(&projects, paper, "main.tex").unwrap(),
            "μ: exact source\n"
        );
        assert_eq!(
            read_source(&projects, new, "main.tex").unwrap(),
            DEFAULT_SOURCE
        );
        assert_eq!(fs::read(home.path().join(SOURCE_FILE)).unwrap(), original);
        assert_eq!(
            fs::read_to_string(home.path().join(COMPLETION_FILE)).unwrap(),
            r#"{"count":2}"#
        );
    }

    #[test]
    fn resumes_electron_progress_without_duplicating_existing_projects() {
        let home = tempfile::tempdir().unwrap();
        let projects = home.path().join("projects");
        let saved = create_project(&projects, "Paper").unwrap();
        fs::write(
            home.path().join(SOURCE_FILE),
            r#"[{"name":"Paper"},{"name":"Notebook"}]"#,
        )
        .unwrap();
        fs::write(home.path().join(PROGRESS_FILE), "[0]").unwrap();

        migrate_legacy_projects(&projects).unwrap();
        let all = list_projects(&projects).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(
            all.iter()
                .find(|project| project.name == "Paper")
                .unwrap()
                .id,
            saved.id
        );
        assert_eq!(
            fs::read_to_string(home.path().join(PROGRESS_FILE)).unwrap(),
            "[0,1]"
        );
    }

    #[test]
    fn accepts_an_electron_completion_marker_even_if_original_data_is_damaged() {
        let home = tempfile::tempdir().unwrap();
        let projects = home.path().join("projects");
        fs::write(home.path().join(SOURCE_FILE), "interrupted JSON").unwrap();
        fs::write(home.path().join(COMPLETION_FILE), r#"{"count":1}"#).unwrap();

        migrate_legacy_projects(&projects).unwrap();
        assert!(!projects.exists());
    }

    #[test]
    fn retries_a_published_project_without_overwriting_edits() {
        let home = tempfile::tempdir().unwrap();
        let projects = home.path().join("projects");
        let original = r#"[{"name":"Paper","source":"Original"}]"#;
        fs::write(home.path().join(SOURCE_FILE), original).unwrap();
        let legacy: Vec<LegacyProject> = serde_json::from_str(original).unwrap();
        import_project(&projects, 0, &legacy[0]).unwrap();
        let paper = list_projects(&projects).unwrap().pop().unwrap();
        save_source(&projects, &paper, "main.tex", "Original", "User edits").unwrap();
        rename_project(&projects, &paper, "Renamed").unwrap();

        migrate_legacy_projects(&projects).unwrap();
        let all = list_projects(&projects).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "Renamed");
        assert_eq!(
            read_source(&projects, &all[0], "main.tex").unwrap(),
            "User edits"
        );
    }

    #[test]
    fn leaves_a_conflicting_directory_alone_and_records_other_imports() {
        let home = tempfile::tempdir().unwrap();
        let projects = home.path().join("projects");
        let original = r#"[{"name":"Paper"},{"name":"Notebook"}]"#;
        fs::write(home.path().join(SOURCE_FILE), original).unwrap();
        let (id, _) = project_identity(1, "Notebook", DEFAULT_SOURCE);
        let blocked = projects.join(id);
        fs::create_dir_all(&blocked).unwrap();
        fs::write(blocked.join("main.tex"), "Separate project").unwrap();

        assert!(migrate_legacy_projects(&projects).is_err());
        assert!(!home.path().join(COMPLETION_FILE).exists());
        assert_eq!(
            fs::read_to_string(home.path().join(PROGRESS_FILE)).unwrap(),
            "[0]"
        );
        assert_eq!(
            fs::read_to_string(home.path().join(SOURCE_FILE)).unwrap(),
            original
        );
        assert_eq!(
            fs::read_to_string(blocked.join("main.tex")).unwrap(),
            "Separate project"
        );
    }
}

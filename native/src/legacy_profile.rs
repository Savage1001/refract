//! Imports Electron's file-origin application preferences from a closed profile.

use crate::{
    Project,
    conversation::{Attachment, ConversationDraft, ConversationId, MAX_ATTACHMENTS},
    is_project_id, list_projects, open_local_file,
    project_catalog::{LibraryPreferences, ProjectSort},
    project_entries::checked_project_directory,
    project_management::{has_one_link, is_link},
    workspace_layout::PaneVisibility,
};
use rusty_leveldb::{DB, LdbIterator, Options, in_memory};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::{self, Read, Write},
    path::Path,
    time::SystemTime,
};

const MARKER: &str = ".refract-native-electron-migration-v1";
const FILE_ORIGIN: &[u8] = b"_file://\x00";
const DRAFT_PREFIX: &str = "refract-chat-draft:";
const MAX_DATABASE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DATABASE_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_DATABASE_FILES: usize = 256;
const MAX_STORAGE_ENTRIES: usize = 8_192;
const MAX_DRAFT_BYTES: usize = 6 * 1024 * 1024;

type FileStamp = (u64, SystemTime);

/// Import preferences once, preserving any files already written by the native app.
///
/// Chromium's original database remains unopened by LevelDB. The snapshot and
/// recovery logs exist only in a bounded in-memory LevelDB environment.
pub fn migrate_electron_profile(projects_dir: &Path) -> io::Result<()> {
    let profile = projects_dir
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let marker = profile.join(MARKER);
    if target_exists(&marker)? {
        return Ok(());
    }
    let source = profile.join("Local Storage");
    if !existing_directory(&source)? {
        return Ok(());
    }
    let source = source.join("leveldb");
    if !existing_directory(&source)? {
        return Ok(());
    }
    let imported = std::panic::catch_unwind(|| read_source(&source))
        .unwrap_or_else(|_| Err(invalid_database()))?;
    let Some(imported) = imported else {
        return Ok(());
    };
    imported.save(projects_dir, profile)?;
    save_if_absent(&marker, b"1\n")
}

fn read_source(source: &Path) -> io::Result<Option<Preferences>> {
    let Some(mut database) = snapshot_database(source)? else {
        return Ok(None);
    };
    if database.get(b"VERSION").as_deref() != Some(b"1".as_slice()) {
        return Err(invalid_database());
    }
    read_preferences(&mut database).map(Some)
}

fn existing_directory(path: &Path) -> io::Result<bool> {
    match path.symlink_metadata() {
        Ok(info) if info.is_dir() && !is_link(&info) => Ok(true),
        Ok(_) => Err(invalid_database()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn target_exists(path: &Path) -> io::Result<bool> {
    match path.symlink_metadata() {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn snapshot_database(source: &Path) -> io::Result<Option<DB>> {
    let files = database_files(source)?;
    let Some(&current) = files.get("CURRENT") else {
        return Ok(None);
    };
    if current.0 > 128 {
        return Err(invalid_database());
    }
    let mut current_data = Vec::new();
    copy_file(source, "CURRENT", current, &mut current_data)?;
    let manifest = std::str::from_utf8(&current_data)
        .ok()
        .and_then(|text| text.strip_suffix('\n'))
        .filter(|name| manifest_name(name) && files.contains_key(*name))
        .ok_or_else(invalid_database)?;

    let database = Path::new("refract-legacy-storage");
    let options = Options {
        create_if_missing: false,
        paranoid_checks: true,
        max_open_files: 64,
        block_cache_capacity_bytes: 2 * 1024 * 1024,
        ..in_memory()
    };
    options
        .env
        .mkdir(database)
        .map_err(|_| invalid_database())?;
    for (name, stamp) in &files {
        if manifest_name(name) && name != manifest {
            continue;
        }
        let mut destination = options
            .env
            .open_writable_file(&database.join(name))
            .map_err(|_| invalid_database())?;
        if name == "CURRENT" {
            destination.write_all(&current_data)?;
        } else {
            copy_file(source, name, *stamp, &mut destination)?;
        }
    }
    if files != database_files(source)? {
        return Err(profile_changed());
    }
    DB::open(database, options)
        .map(Some)
        .map_err(|_| invalid_database())
}

fn database_files(source: &Path) -> io::Result<BTreeMap<String, FileStamp>> {
    if !existing_directory(source)? {
        return Err(profile_changed());
    }
    let mut files = BTreeMap::new();
    let mut total = 0;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if name != "CURRENT" && !manifest_name(&name) && !table_or_log_name(&name) {
            continue;
        }
        let info = entry.path().symlink_metadata()?;
        if !info.is_file() || is_link(&info) || info.len() > MAX_DATABASE_FILE_BYTES {
            return Err(invalid_database());
        }
        total += info.len();
        if total > MAX_DATABASE_BYTES || files.len() >= MAX_DATABASE_FILES {
            return Err(invalid_database());
        }
        files.insert(name, (info.len(), info.modified()?));
    }
    Ok(files)
}

fn manifest_name(name: &str) -> bool {
    name.strip_prefix("MANIFEST-").is_some_and(file_number)
}

fn table_or_log_name(name: &str) -> bool {
    name.split_once('.').is_some_and(|(number, suffix)| {
        file_number(number) && matches!(suffix, "ldb" | "sst" | "log")
    })
}

fn file_number(digits: &str) -> bool {
    !digits.is_empty() && digits.len() <= 20 && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn copy_file(
    source: &Path,
    name: &str,
    expected: FileStamp,
    destination: &mut impl Write,
) -> io::Result<()> {
    let path = source.join(name);
    let info = path.symlink_metadata()?;
    if !info.is_file() || is_link(&info) {
        return Err(invalid_database());
    }
    let mut file = open_local_file(source, &path)?;
    if !has_one_link(&file)? {
        return Err(invalid_database());
    }
    let info = file.metadata()?;
    if (info.len(), info.modified()?) != expected
        || io::copy(&mut (&mut file).take(expected.0 + 1), destination)? != expected.0
    {
        return Err(profile_changed());
    }
    let info = file.metadata()?;
    if (info.len(), info.modified()?) != expected {
        return Err(profile_changed());
    }
    Ok(())
}

#[derive(Default)]
struct Preferences {
    sort: Option<ProjectSort>,
    grid: Option<bool>,
    panes: Option<PaneVisibility>,
    outline_height: Option<f32>,
    drafts: Vec<SavedDraft>,
}

struct SavedDraft {
    project: String,
    id: ConversationId,
    draft: ConversationDraft,
}

impl Preferences {
    fn save(self, projects_dir: &Path, profile: &Path) -> io::Result<()> {
        if self.sort.is_some() || self.grid.is_some() {
            let library = LibraryPreferences {
                sort: self.sort.unwrap_or_default(),
                grid: self.grid.unwrap_or_default(),
            };
            save_if_absent(
                &profile.join(".refract-native-library.json"),
                &serde_json::to_vec(&library)?,
            )?;
        }
        if let Some(panes) = self.panes {
            save_if_absent(
                &profile.join(".refract-native-workspace.json"),
                &serde_json::to_vec(&panes)?,
            )?;
        }
        if let Some(height) = self.outline_height {
            save_if_absent(
                &profile.join(".refract-native-outline-height"),
                height.to_string().as_bytes(),
            )?;
        }
        if !self.drafts.is_empty() {
            let projects = list_projects(projects_dir)?;
            let projects: HashMap<&str, &Project> = projects
                .iter()
                .map(|project| (project.id.as_str(), project))
                .collect();
            for saved in self.drafts {
                if let Some(project) = projects.get(saved.project.as_str()) {
                    save_draft(projects_dir, project, saved)?;
                }
            }
        }
        Ok(())
    }

    fn read(&mut self, key: &str, value: &[u8]) {
        if let Some(ids) = key.strip_prefix(DRAFT_PREFIX) {
            if let Some(draft) = read_draft(ids, value) {
                self.drafts.push(draft);
            }
            return;
        }
        if !matches!(
            key,
            "refract-project-sort"
                | "refract-project-view"
                | "refract.workspace-panes.v1"
                | "refract-outline-list-height"
        ) {
            return;
        }
        let Some(value) = decode_storage_string(value, 512) else {
            return;
        };
        match key {
            "refract-project-sort" => {
                self.sort = match value.as_str() {
                    "newest" => Some(ProjectSort::Newest),
                    "oldest" => Some(ProjectSort::Oldest),
                    "name" => Some(ProjectSort::Name),
                    "edited" => Some(ProjectSort::Edited),
                    _ => None,
                };
            }
            "refract-project-view" => {
                self.grid = match value.as_str() {
                    "list" => Some(false),
                    "grid" => Some(true),
                    _ => None,
                };
            }
            "refract.workspace-panes.v1" => {
                if let Ok(value) = serde_json::from_str::<Value>(&value)
                    && let (Some(agent), Some(editor), Some(preview)) = (
                        value["agent"].as_bool(),
                        value["editor"].as_bool(),
                        value["preview"].as_bool(),
                    )
                {
                    self.panes = Some(PaneVisibility {
                        agent,
                        editor,
                        preview,
                    });
                }
            }
            "refract-outline-list-height" => {
                self.outline_height = value
                    .trim()
                    .parse::<f32>()
                    .ok()
                    .filter(|height| height.is_finite() && *height >= 32.);
            }
            _ => {}
        }
    }
}

fn read_preferences(database: &mut DB) -> io::Result<Preferences> {
    let mut preferences = Preferences::default();
    let mut records = database.new_iter().map_err(|_| invalid_database())?;
    records.seek(FILE_ORIGIN);
    let mut scanned = 0;
    while let Some((key, value)) = records.current() {
        let Some(key) = key.strip_prefix(FILE_ORIGIN) else {
            break;
        };
        scanned += 1;
        if scanned > MAX_STORAGE_ENTRIES {
            return Err(invalid_database());
        }
        if let Some(key) = decode_storage_string(key, 256) {
            preferences.read(&key, &value);
        }
        if !records.advance() {
            break;
        }
    }
    Ok(preferences)
}

fn decode_storage_string(bytes: &[u8], limit: usize) -> Option<String> {
    let (&encoding, bytes) = bytes.split_first()?;
    if bytes.len() > limit {
        return None;
    }
    match encoding {
        1 => Some(bytes.iter().map(|&byte| char::from(byte)).collect()),
        0 if bytes.len().is_multiple_of(2) => {
            let units: Vec<_> = bytes
                .chunks_exact(2)
                .map(|unit| u16::from_le_bytes([unit[0], unit[1]]))
                .collect();
            String::from_utf16(&units).ok()
        }
        _ => None,
    }
}

fn read_draft(ids: &str, value: &[u8]) -> Option<SavedDraft> {
    let (project, conversation) = ids.split_once(':')?;
    if !is_project_id(project) {
        return None;
    }
    let id = ConversationId::parse(conversation).ok()?;
    let value = decode_storage_string(value, MAX_DRAFT_BYTES)?;
    let mut value: Value = serde_json::from_str(&value).ok()?;
    let text = value["text"].as_str()?.to_owned();
    let attachments = value["attachments"]
        .as_array_mut()?
        .drain(..)
        .filter_map(|item| serde_json::from_value::<Attachment>(item).ok())
        .take(MAX_ATTACHMENTS)
        .collect();
    let provider = value["provider"]
        .as_str()
        .filter(|name| matches!(*name, "codex" | "opencode"))
        .map(str::to_owned);
    Some(SavedDraft {
        project: project.to_owned(),
        id,
        draft: ConversationDraft {
            text,
            attachments,
            provider,
        },
    })
}

fn save_draft(projects_dir: &Path, project: &Project, saved: SavedDraft) -> io::Result<()> {
    let contents = serde_json::to_vec(&saved.draft)?;
    if contents.len() > MAX_DRAFT_BYTES {
        return Ok(());
    }
    let root = checked_project_directory(projects_dir, project)?;
    let directory = root.join(".refract-native-drafts");
    if !existing_directory(&directory)? {
        match fs::create_dir(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        if !existing_directory(&directory)? {
            return Err(profile_changed());
        }
    }
    let file = directory.join(format!("{}.json", saved.id.as_str()));
    save_if_absent(&file, &contents)
}

fn save_if_absent(path: &Path, contents: &[u8]) -> io::Result<()> {
    if target_exists(path)? {
        return Ok(());
    }
    let directory = path.parent().ok_or_else(invalid_database)?;
    let mut stage = tempfile::NamedTempFile::new_in(directory)?;
    stage.write_all(contents)?;
    stage.as_file().sync_all()?;
    match stage.persist_noclobber(path) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.error),
    }
}

fn invalid_database() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Could not read the Electron preferences database.",
    )
}

fn profile_changed() -> io::Error {
    io::Error::new(
        io::ErrorKind::Interrupted,
        "The Electron profile changed during migration.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        conversation::{load_conversation_draft, save_conversation_draft},
        create_project,
    };

    fn storage_key(key: &str) -> Vec<u8> {
        [FILE_ORIGIN, b"\x01", key.as_bytes()].concat()
    }

    fn write_fixture(profile: &Path, project: &Project) -> std::path::PathBuf {
        let name = Path::new("legacy-fixture");
        let options = Options {
            compressor: 1,
            ..in_memory()
        };
        let env = options.env.clone();
        let mut db = DB::open(name, options).unwrap();
        db.put(b"VERSION", b"1").unwrap();
        db.put(&storage_key("refract-project-sort"), b"\x01edited")
            .unwrap();
        db.put(&storage_key("refract-project-view"), b"\x01grid")
            .unwrap();
        db.put(
            &storage_key("refract.workspace-panes.v1"),
            b"\x01{\"agent\":false,\"editor\":true,\"preview\":false}",
        )
        .unwrap();
        db.put(&storage_key("refract-outline-list-height"), b"\x01144")
            .unwrap();
        let draft = serde_json::json!({
            "text": "Résumé π 🧪",
            "attachments": [false, {"token":"fixture.png", "name":"Sketch.png"}],
            "provider": "opencode"
        })
        .to_string();
        let value: Vec<_> = std::iter::once(0)
            .chain(draft.encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        db.put(
            &storage_key(&format!("{DRAFT_PREFIX}{}:default", project.id)),
            &value,
        )
        .unwrap();
        let deleted = storage_key(&format!(
            "{DRAFT_PREFIX}{}:00000000-0000-4000-8000-000000000000",
            project.id
        ));
        db.put(&deleted, &value).unwrap();
        db.delete(&deleted).unwrap();
        db.put(
            b"_https://example.org\x00\x01refract-project-sort",
            b"\x01oldest",
        )
        .unwrap();
        db.compact_range(b"", b"\xff").unwrap();
        db.close().unwrap();
        drop(db);

        let source = profile.join("Local Storage").join("leveldb");
        fs::create_dir_all(&source).unwrap();
        for child in env.children(name).unwrap() {
            let file = child.file_name().unwrap();
            let mut reader = env.open_sequential_file(&name.join(file)).unwrap();
            let mut output = fs::File::create(source.join(file)).unwrap();
            io::copy(&mut reader, &mut output).unwrap();
        }
        source
    }

    fn assert_preferences(profile: &Path, projects: &Path, project: &Project) {
        assert_eq!(
            LibraryPreferences::read(&profile.join(".refract-native-library.json")),
            LibraryPreferences {
                sort: ProjectSort::Edited,
                grid: true
            },
        );
        assert_eq!(
            PaneVisibility::read(&profile.join(".refract-native-workspace.json")),
            PaneVisibility {
                agent: false,
                editor: true,
                preview: false
            },
        );
        assert_eq!(
            fs::read_to_string(profile.join(".refract-native-outline-height")).unwrap(),
            "144"
        );
        let draft = load_conversation_draft(projects, project, &ConversationId::default()).unwrap();
        assert_eq!(draft.text, "Résumé π 🧪");
        assert_eq!(draft.provider.as_deref(), Some("opencode"));
        assert_eq!(draft.attachments.len(), 1);
        assert_eq!(draft.attachments[0].name, "Sketch.png");
    }

    #[test]
    fn imports_chromium_values_from_a_snapshot_without_touching_the_original() {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let project = create_project(&projects, "Fixture").unwrap();
        let source = write_fixture(profile.path(), &project);
        let before = database_files(&source).unwrap();
        migrate_electron_profile(&projects).unwrap();
        assert_preferences(profile.path(), &projects, &project);
        let after = database_files(&source).unwrap();
        assert_eq!(before, after);
        assert!(
            !projects
                .join(project.id)
                .join(".refract-native-drafts")
                .join("00000000-0000-4000-8000-000000000000.json")
                .exists()
        );
        assert!(profile.path().join(MARKER).is_file());
    }

    #[test]
    fn retains_existing_native_files_and_only_imports_once() {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let project = create_project(&projects, "Fixture").unwrap();
        let source = write_fixture(profile.path(), &project);
        let library = profile.path().join(".refract-native-library.json");
        LibraryPreferences::default().save(&library).unwrap();
        let id = ConversationId::default();
        let draft = ConversationDraft {
            text: "Native edit".into(),
            ..ConversationDraft::default()
        };
        save_conversation_draft(&projects, &project, &id, &draft).unwrap();

        migrate_electron_profile(&projects).unwrap();
        assert_eq!(
            LibraryPreferences::read(&library),
            LibraryPreferences::default()
        );
        assert_eq!(
            load_conversation_draft(&projects, &project, &id)
                .unwrap()
                .text,
            draft.text
        );

        fs::write(source.join("CURRENT"), b"invalid manifest\n").unwrap();
        migrate_electron_profile(&projects).unwrap();
        assert_eq!(
            fs::read(source.join("CURRENT")).unwrap(),
            b"invalid manifest\n"
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires the Electron fixture runner built by Windows CI"]
    fn imports_local_storage_written_by_the_packaged_electron_version() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let fixture = repository.join("build/scripts/legacy-profile-fixture.cjs");
        let electron = repository.join("node_modules/electron/dist/electron.exe");
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let project = create_project(&projects, "Fixture").unwrap();
        let mut command = std::process::Command::new(&electron);
        crate::tex::process::configure_process(&mut command);
        let result = command
            .args(["--disable-gpu", "--disable-renderer-backgrounding"])
            .arg(fixture)
            .env("REFRACT_TEST_LEGACY_PROFILE_DIR", profile.path())
            .env("REFRACT_TEST_LEGACY_PROFILE_PROJECT", &project.id)
            .env_remove("ELECTRON_RUN_AS_NODE")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "Electron local storage fixture failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        migrate_electron_profile(&projects).unwrap();
        assert_preferences(profile.path(), &projects, &project);
    }
}

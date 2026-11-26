use super::{
    Project, Version, VersionTree, comments, copy_and_hash, open_local_file, open_version,
    project_directory, read_anchor_text, read_local_file,
    restore_stage::{RestoreJournal, checked_info},
    snapshot_project, source_path, stage_file, valid_project_component, valid_version_path,
    version_file,
};
use crate::METADATA_LIMIT;
use serde_json::Value;
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs::{self, File, Metadata},
    io::{self, Read, Write},
    path::Path,
};
use tempfile::NamedTempFile;
use zip::ZipArchive;

pub struct ProjectRestore {
    pub project: Project,
    pub cleanup_pending: usize,
    pub comment_warning: Option<String>,
}

pub fn restore_project_version(
    projects_dir: &Path,
    project: &Project,
    version_id: &str,
) -> io::Result<ProjectRestore> {
    let folder = project_directory(projects_dir, project)?;
    let mut target = ArchivedProject::open(&folder, version_id)?;
    let current = read_metadata(&folder)?.1;
    let before = snapshot_project(projects_dir, &current)?;
    let comments = prepare_comments(&folder, &mut target)?;
    let journal = RestoreJournal::create(&folder, before.id.clone())?;
    let mut restored = complete_restore(projects_dir, &folder, &before, target, journal, false)?;
    if let Some((original, staged)) = comments {
        let update = comments::read_comments_file(&folder).and_then(|current| {
            if current.as_deref() != Some(original.as_slice()) {
                return Err(io::Error::other("Comments changed during restore."));
            }
            staged
                .persist(folder.join(".refract-comments.json"))
                .map(|_| ())
                .map_err(|error| error.error)
        });
        restored.comment_warning = update.err().map(|error| error.to_string());
    }
    Ok(restored)
}

pub fn recover_project_restore(
    projects_dir: &Path,
    project: &Project,
) -> io::Result<Option<ProjectRestore>> {
    let folder = project_directory(projects_dir, project)?;
    let Some(journal) = RestoreJournal::read(&folder)? else {
        return Ok(None);
    };
    let target = ArchivedProject::open(&folder, journal.recovery_id())?;
    let current = read_metadata(&folder)?.1;
    let before = snapshot_project(projects_dir, &current)?;
    complete_restore(projects_dir, &folder, &before, target, journal, true).map(Some)
}

fn complete_restore(
    projects_dir: &Path,
    folder: &Path,
    before: &Version,
    mut target: ArchivedProject,
    mut journal: RestoreJournal,
    recovering: bool,
) -> io::Result<ProjectRestore> {
    let mut modified = false;
    let apply = target
        .apply(folder, before, &mut journal, &mut modified)
        .and_then(|project| {
            snapshot_project(projects_dir, &project)?;
            Ok(project)
        });
    match apply {
        Ok(project) => Ok(ProjectRestore {
            project,
            cleanup_pending: journal.finish(folder)?,
            comment_warning: None,
        }),
        Err(error) if recovering => Err(error),
        Err(error) => {
            let recovery_id = journal.recovery_id().to_owned();
            if modified {
                let rollback = (|| {
                    let current = read_metadata(folder)?.1;
                    let partial = snapshot_project(projects_dir, &current)?;
                    let mut recovery = ArchivedProject::open(folder, &recovery_id)?;
                    let mut rolled_back = false;
                    let recovered =
                        recovery.apply(folder, &partial, &mut journal, &mut rolled_back)?;
                    snapshot_project(projects_dir, &recovered)?;
                    Ok::<_, io::Error>(())
                })();
                if let Err(rollback) = rollback {
                    return Err(io::Error::other(format!(
                        "Restore failed: {error}. Recovery also failed: {rollback}. Recovery version: {recovery_id}"
                    )));
                }
            }
            journal.finish(folder)?;
            if modified {
                Err(io::Error::other(format!(
                    "Restore failed and the previous project was recovered: {error}"
                )))
            } else {
                Err(error)
            }
        }
    }
}

struct ArchivedProject {
    version: Version,
    archive: ZipArchive<File>,
    files: HashMap<String, String>,
    directories: BTreeSet<String>,
}

impl ArchivedProject {
    fn open(folder: &Path, id: &str) -> io::Result<Self> {
        let (version, archive) = open_version(folder, id)?;
        let mut files = HashMap::new();
        let mut directories = BTreeSet::new();
        let mut seen = HashSet::new();
        for name in &version.files {
            let path = normalized_path(name)?;
            if !seen.insert(path.to_lowercase()) {
                return Err(invalid_layout());
            }
            files.insert(lookup_key(&path), name.clone());
            add_parents(&mut directories, &path);
        }
        for name in version.directories.iter().flatten() {
            let path = normalized_path(name)?;
            add_parents(&mut directories, &path);
            directories.insert(path);
        }
        for path in &directories {
            if !seen.insert(path.to_lowercase()) {
                return Err(invalid_layout());
            }
        }
        Ok(Self {
            version,
            archive,
            files,
            directories,
        })
    }

    fn apply(
        &mut self,
        folder: &Path,
        before: &Version,
        journal: &mut RestoreJournal,
        modified: &mut bool,
    ) -> io::Result<Project> {
        let (metadata, current) = read_metadata(folder)?;
        if before.root_file.as_deref() != Some(current.root_file()) {
            return Err(restore_conflict());
        }
        let next_metadata = restored_metadata(&metadata, &current, &self.version)?;
        let metadata_path = folder.join(".refract.json");
        let next = serde_json::from_slice::<Project>(&next_metadata)?;
        let staged_metadata = stage_file(&metadata_path, &next_metadata)?;
        let tree = VersionTree::from_project(folder)?;
        let blockers = self.blockers(folder)?;
        let stage = journal.stage(
            folder,
            self.version.id.clone(),
            before.id.clone(),
            self.version.files.clone(),
            blockers.iter().cloned().collect(),
        )?;
        let mut staged = Vec::with_capacity(self.version.files.len());
        for (index, name) in self.version.files.iter().enumerate() {
            let mut output = File::create_new(stage.join(index.to_string()))?;
            staged.push(copy_archive_file(&mut self.archive, name, &mut output)?);
            output.sync_all()?;
        }

        let originals = verify_backup(folder, before, &tree)?;
        if VersionTree::from_project(folder)? != tree || read_metadata(folder)?.0 != metadata {
            return Err(restore_conflict());
        }
        for (index, blocker) in blockers.iter().enumerate() {
            let path = folder.join(blocker);
            let info = checked_info(&path)?;
            if info.is_dir() {
                ensure_visible_directory(&path, info)?;
            }
            fs::rename(
                path,
                stage.join(format!("blocked-{}", self.version.files.len() + index)),
            )?;
            *modified = true;
        }
        for name in &self.directories {
            let path = folder.join(name);
            match destination_info(&path)? {
                Some(info) if info.is_dir() => {}
                None => {
                    fs::create_dir(path)?;
                    *modified = true;
                }
                _ => return Err(restore_conflict()),
            }
        }
        for (index, (name, fingerprint)) in self.version.files.iter().zip(staged).enumerate() {
            let name = normalized_path(name)?;
            let path = folder.join(&name);
            let existing = destination_info(&path)?;
            if existing.as_ref().is_some_and(|info| !info.is_file()) {
                return Err(restore_conflict());
            }
            if existing.is_some() && originals.get(name.as_str()) == Some(&fingerprint) {
                continue;
            }
            fs::rename(stage.join(index.to_string()), path)?;
            *modified = true;
        }
        for name in &tree.files {
            if self.files.contains_key(&lookup_key(name)) || covered_by(&blockers, name) {
                continue;
            }
            let path = folder.join(name);
            if !checked_info(&path)?.is_file() {
                return Err(restore_conflict());
            }
            fs::remove_file(path)?;
            *modified = true;
        }
        if self.version.directories.is_some() {
            for name in tree.directories.iter().rev() {
                if self.directories.contains(name) || covered_by(&blockers, name) {
                    continue;
                }
                let path = folder.join(name);
                if !checked_info(&path)?.is_dir() {
                    return Err(restore_conflict());
                }
                match fs::remove_dir(path) {
                    Ok(()) => *modified = true,
                    Err(error) if error.kind() == io::ErrorKind::DirectoryNotEmpty => {}
                    Err(error) => return Err(error),
                }
            }
        }
        if metadata != next_metadata {
            staged_metadata
                .persist(metadata_path)
                .map_err(|error| error.error)?;
            *modified = true;
        }
        Ok(next)
    }

    fn blockers(&self, folder: &Path) -> io::Result<BTreeSet<String>> {
        let mut blockers = BTreeSet::new();
        for directory in &self.directories {
            if !covered_by(&blockers, directory)
                && let Some(info) = destination_info(&folder.join(directory))?
                && !info.is_dir()
            {
                if !info.is_file() {
                    return Err(invalid_layout());
                }
                blockers.insert(directory.clone());
            }
        }
        for name in &self.version.files {
            let name = normalized_path(name)?;
            if covered_by(&blockers, &name) {
                continue;
            }
            let path = folder.join(&name);
            if let Some(info) = destination_info(&path)?
                && !info.is_file()
            {
                ensure_visible_directory(&path, info)?;
                blockers.insert(name);
            }
        }
        Ok(blockers)
    }
}

fn add_parents(paths: &mut BTreeSet<String>, name: &str) {
    paths.extend(
        name.match_indices('/')
            .map(|(index, _)| name[..index].to_owned()),
    );
}

fn covered_by(paths: &BTreeSet<String>, name: &str) -> bool {
    paths.contains(name)
        || name
            .match_indices('/')
            .any(|(index, _)| paths.contains(&name[..index]))
}

fn normalized_path(name: &str) -> io::Result<String> {
    if !valid_version_path(name) {
        return Err(invalid_layout());
    }
    Ok(name.replace('\\', "/"))
}

fn lookup_key(name: &str) -> String {
    if cfg!(windows) {
        name.to_lowercase()
    } else {
        name.to_owned()
    }
}

fn destination_info(path: &Path) -> io::Result<Option<Metadata>> {
    match checked_info(path) {
        Ok(info) => Ok(Some(info)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn ensure_visible_directory(dir: &Path, info: Metadata) -> io::Result<()> {
    if !info.is_dir() {
        return Err(invalid_layout());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if !entry
            .file_name()
            .to_str()
            .is_some_and(valid_project_component)
        {
            return Err(io::Error::other(
                "Hidden data prevents replacing a project folder.",
            ));
        }
        let path = entry.path();
        let info = checked_info(&path)?;
        if !info.is_file() {
            ensure_visible_directory(&path, info)?;
        }
    }
    Ok(())
}

type Fingerprint = (u64, String);

fn copy_archive_file(
    archive: &mut ZipArchive<File>,
    name: &str,
    output: &mut impl Write,
) -> io::Result<Fingerprint> {
    let entry = version_file(archive, name)?;
    let size = entry.size();
    let actual = copy_and_hash(entry.take(size.saturating_add(1)), output)?;
    if actual.0 != size {
        return Err(invalid_layout());
    }
    Ok(actual)
}

fn verify_backup<'a>(
    folder: &Path,
    before: &Version,
    current: &'a VersionTree,
) -> io::Result<HashMap<&'a str, Fingerprint>> {
    if before.files != current.files || before.directories.as_ref() != Some(&current.directories) {
        return Err(restore_conflict());
    }
    let (_, mut archive) = open_version(folder, &before.id)?;
    let mut originals = HashMap::new();
    for name in &current.files {
        let path = source_path(folder, name)?;
        checked_info(&path)?;
        let bytes = copy_and_hash(open_local_file(folder, &path)?, &mut io::sink())?;
        if bytes != copy_archive_file(&mut archive, name, &mut io::sink())? {
            return Err(restore_conflict());
        }
        originals.insert(name.as_str(), bytes);
    }
    Ok(originals)
}

fn read_metadata(folder: &Path) -> io::Result<(Vec<u8>, Project)> {
    let path = folder.join(".refract.json");
    if !checked_info(&path)?.is_file() {
        return Err(invalid_layout());
    }
    let bytes = read_local_file(folder, &path, METADATA_LIMIT)?;
    let project: Project = serde_json::from_slice(&bytes)?;
    if folder.file_name().and_then(|name| name.to_str()) != Some(project.id.as_str()) {
        return Err(invalid_layout());
    }
    Ok((bytes, project))
}

fn restored_metadata(bytes: &[u8], project: &Project, version: &Version) -> io::Result<Vec<u8>> {
    let root = version
        .root_file
        .as_deref()
        .filter(|name| !name.is_empty())
        .or_else(|| {
            if version.files.iter().any(|file| file == project.root_file()) {
                return None;
            }
            version
                .files
                .iter()
                .find(|file| file.as_str() == "main.tex")
                .or_else(|| {
                    version
                        .files
                        .iter()
                        .find(|file| file.to_ascii_lowercase().ends_with(".tex"))
                })
                .map(String::as_str)
        });
    let Some(root) = root else {
        return Ok(bytes.to_vec());
    };
    let mut fields: serde_json::Map<String, Value> = serde_json::from_slice(bytes)?;
    if fields.get("rootFile").and_then(Value::as_str) == Some(root) {
        return Ok(bytes.to_vec());
    }
    fields.insert("rootFile".into(), Value::String(root.into()));
    let encoded = serde_json::to_vec_pretty(&fields)?;
    if encoded.len() > METADATA_LIMIT {
        return Err(invalid_layout());
    }
    Ok(encoded)
}

fn prepare_comments(
    folder: &Path,
    target: &mut ArchivedProject,
) -> io::Result<Option<(Vec<u8>, NamedTempFile)>> {
    let Some(original) = comments::read_comments_file(folder)? else {
        return Ok(None);
    };
    let mut updates = None;
    for name in comments::attached_files(&original)? {
        if !valid_version_path(&name) {
            continue;
        }
        let current = match source_path(folder, &name) {
            Ok(path) if checked_info(&path)?.is_file() => {
                read_anchor_text(open_local_file(folder, &path)?)?
            }
            Ok(_) => None,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                None
            }
            Err(error) => return Err(error),
        };
        let next = if let Some(path) = target.files.get(&lookup_key(&name.replace('\\', "/"))) {
            let mut data = Vec::new();
            copy_archive_file(&mut target.archive, path, &mut data)?;
            if data.contains(&0) {
                None
            } else {
                String::from_utf8(data).ok()
            }
        } else {
            None
        };
        if let Some(updated) = comments::plan_edit(
            updates.as_deref().unwrap_or(&original),
            &name,
            current.as_deref(),
            next.as_deref(),
        )? {
            updates = Some(updated);
        }
    }
    updates
        .map(|bytes| {
            stage_file(&folder.join(".refract-comments.json"), &bytes).map(|file| (original, file))
        })
        .transpose()
}

fn invalid_layout() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Invalid project layout for restore",
    )
}

fn restore_conflict() -> io::Error {
    io::Error::new(
        io::ErrorKind::Interrupted,
        "The project changed during restore.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{list_projects, versions::list_versions};
    use std::path::PathBuf;

    const ID: &str = "b3b9ffb5-13c0-4d22-b427-54520347684a";

    struct Fixture {
        root: tempfile::TempDir,
        folder: PathBuf,
        initial: Version,
        current: Project,
    }

    fn changed_project() -> Fixture {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join(ID)).unwrap();
        let folder = root.path().canonicalize().unwrap().join(ID);
        let metadata = serde_json::json!({"id": ID, "name": "Paper", "rootFile": "main.tex", "engine": "xelatex"});
        fs::write(folder.join(".refract.json"), metadata.to_string()).unwrap();
        fs::write(folder.join("main.tex"), "The result.\n").unwrap();
        fs::write(folder.join("item"), [0, 255, 1]).unwrap();
        fs::create_dir_all(folder.join("sections/empty")).unwrap();
        let current = list_projects(root.path()).unwrap().pop().unwrap();
        let initial = snapshot_project(root.path(), &current).unwrap();
        let mut metadata = metadata;
        metadata["rootFile"] = "paper.tex".into();
        fs::write(folder.join(".refract.json"), metadata.to_string()).unwrap();
        fs::write(folder.join("main.tex"), "Draft\nThe result.\n").unwrap();
        fs::write(folder.join("paper.tex"), "New root\n").unwrap();
        fs::remove_file(folder.join("item")).unwrap();
        fs::create_dir(folder.join("item")).unwrap();
        fs::write(folder.join("item/part.tex"), "New nested file").unwrap();
        fs::remove_dir(folder.join("sections/empty")).unwrap();
        fs::create_dir(folder.join("sections/new")).unwrap();
        fs::write(folder.join(".unrelated"), "Keep").unwrap();
        let current = read_metadata(&folder).unwrap().1;
        Fixture {
            root,
            folder,
            initial,
            current,
        }
    }

    #[test]
    fn restores_files_empty_folders_root_metadata_and_comment_anchors() {
        let fixture = changed_project();
        let comments_path = fixture.folder.join(".refract-comments.json");
        fs::write(&comments_path, serde_json::json!([{
            "id": "comment", "file": "main.tex", "quote": "result", "text": "Verify",
            "anchor": {"start": 10, "end": 16, "prefix": "Draft\nThe ", "suffix": ".\n", "sourceHash": "", "status": "attached"}
        }]).to_string()).unwrap();
        let restored =
            restore_project_version(fixture.root.path(), &fixture.current, &fixture.initial.id)
                .unwrap();
        assert_eq!(restored.project.root_file(), "main.tex");
        assert_eq!(restored.cleanup_pending, 0);
        assert!(restored.comment_warning.is_none());
        assert_eq!(fs::read(fixture.folder.join("item")).unwrap(), [0, 255, 1]);
        assert!(!fixture.folder.join("paper.tex").exists());
        assert!(!fixture.folder.join("sections/new").exists());
        assert!(fixture.folder.join("sections/empty").is_dir());
        assert_eq!(
            fs::read_to_string(fixture.folder.join(".unrelated")).unwrap(),
            "Keep"
        );
        let metadata: Value =
            serde_json::from_slice(&read_metadata(&fixture.folder).unwrap().0).unwrap();
        assert_eq!(metadata["engine"], "xelatex");
        let comments: Value = serde_json::from_slice(&fs::read(comments_path).unwrap()).unwrap();
        assert_eq!(comments[0]["anchor"]["start"], 4);
        let history = list_versions(fixture.root.path(), &restored.project).unwrap();
        assert_eq!(history.len(), 3);
        assert_eq!(history[1].root_file.as_deref(), Some("paper.tex"));
        assert!(history[1].files.contains(&"item/part.tex".into()));
    }

    #[test]
    fn recovers_an_interruption_from_the_published_restore_journal() {
        let fixture = changed_project();
        let before = snapshot_project(fixture.root.path(), &fixture.current).unwrap();
        let mut target = ArchivedProject::open(&fixture.folder, &fixture.initial.id).unwrap();
        let mut journal = RestoreJournal::create(&fixture.folder, before.id.clone()).unwrap();
        assert!(RestoreJournal::create(&fixture.folder, fixture.initial.id.clone()).is_err());
        let mut modified = false;
        target
            .apply(&fixture.folder, &before, &mut journal, &mut modified)
            .unwrap();
        assert!(modified);
        drop(journal);
        assert_eq!(fs::read(fixture.folder.join("item")).unwrap(), [0, 255, 1]);
        let recovered = recover_project_restore(fixture.root.path(), &fixture.current)
            .unwrap()
            .unwrap();
        assert_eq!(recovered.project.root_file(), "paper.tex");
        assert_eq!(recovered.cleanup_pending, 0);
        assert_eq!(
            fs::read_to_string(fixture.folder.join("item/part.tex")).unwrap(),
            "New nested file"
        );
        assert!(fixture.folder.join("sections/new").is_dir());
        assert!(!fixture.folder.join("sections/empty").exists());
        assert!(
            recover_project_restore(fixture.root.path(), &recovered.project)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn protects_hidden_content_in_a_folder_that_blocks_restore() {
        let fixture = changed_project();
        let hidden = fixture.folder.join("item/.keep");
        fs::write(&hidden, "User data").unwrap();
        let error =
            restore_project_version(fixture.root.path(), &fixture.current, &fixture.initial.id)
                .err()
                .unwrap();
        assert!(error.to_string().contains("Hidden data"));
        assert_eq!(fs::read_to_string(hidden).unwrap(), "User data");
        assert_eq!(
            fs::read_to_string(fixture.folder.join("paper.tex")).unwrap(),
            "New root\n"
        );
        assert!(RestoreJournal::read(&fixture.folder).unwrap().is_none());
    }

    #[test]
    fn a_legacy_snapshot_keeps_existing_empty_folders_and_selects_the_root() {
        let fixture = changed_project();
        let manifest = fixture
            .folder
            .join(".refract-versions")
            .join(format!("{}.json", fixture.initial.id));
        let mut version = fixture.initial.clone();
        version.root_file = None;
        version.directories = None;
        fs::write(manifest, serde_json::to_vec(&version).unwrap()).unwrap();
        let restored =
            restore_project_version(fixture.root.path(), &fixture.current, &version.id).unwrap();
        assert_eq!(restored.project.root_file(), "main.tex");
        assert_eq!(restored.cleanup_pending, 0);
        assert!(fixture.folder.join("sections/new").is_dir());
    }
}

use crate::{
    Project, ProjectEntry, comments, is_project_id, open_local_file, project_directory,
    read_local_file, read_project_entries, source_path, stage_file, valid_project_component,
};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

mod project_restore;
mod restore_stage;
pub use project_restore::{ProjectRestore, recover_project_restore, restore_project_version};
pub use restore_stage::retry_retained_restore_stages;

const MANIFEST_LIMIT: usize = 4 * 1024 * 1024;
const COMPARISON_LIMIT: u64 = 100_000;
const LINE_ALIGNMENT_LIMIT: usize = 1_000_000;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    pub id: String,
    pub created: String,
    #[serde(default)]
    pub sequence: u64,
    #[serde(default)]
    pub digest: String,
    pub files: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directories: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_file: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Deleted,
    Changed,
    FolderAdded,
    FolderDeleted,
    RootDocument,
}

impl ChangeKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Deleted => "deleted",
            Self::Changed | Self::RootDocument => "changed",
            Self::FolderAdded => "folder added",
            Self::FolderDeleted => "folder deleted",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Difference {
    pub name: String,
    pub kind: ChangeKind,
}

pub struct DifferenceLines {
    pub previous: Option<ComparisonSide>,
    pub current: Option<ComparisonSide>,
    pub skipped_alignment: bool,
}

pub struct ComparisonSide {
    pub lines: Vec<ComparisonLine>,
    pub changed_indices: Vec<usize>,
}

pub struct ComparisonLine {
    pub text: String,
    pub changed: bool,
}

#[derive(Default, PartialEq, Eq)]
struct VersionTree {
    files: Vec<String>,
    directories: Vec<String>,
}

impl VersionTree {
    fn from_project(folder: &Path) -> io::Result<Self> {
        fn visit(nodes: Vec<ProjectEntry>, tree: &mut VersionTree) {
            for node in nodes {
                if let Some(children) = node.children {
                    tree.directories.push(node.path);
                    visit(children, tree);
                } else {
                    tree.files.push(node.path);
                }
            }
        }

        let mut tree = Self::default();
        visit(read_project_entries(folder, "")?, &mut tree);
        // JavaScript's default sort compares string code units.
        tree.files
            .sort_by(|left, right| left.encode_utf16().cmp(right.encode_utf16()));
        tree.directories
            .sort_by(|left, right| left.encode_utf16().cmp(right.encode_utf16()));
        Ok(tree)
    }
}

fn version_folder(folder: &Path, create: bool) -> io::Result<PathBuf> {
    let target = folder.join(".refract-versions");
    if create
        && let Err(error) = fs::create_dir(&target)
        && error.kind() != io::ErrorKind::AlreadyExists
    {
        return Err(error);
    }
    if !target.symlink_metadata()?.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid version history folder",
        ));
    }
    Ok(target)
}

pub fn list_versions(projects_dir: &Path, project: &Project) -> io::Result<Vec<Version>> {
    list_project_versions(&project_directory(projects_dir, project)?)
}

pub fn restore_version_file(
    projects_dir: &Path,
    project: &Project,
    version_id: &str,
    file: &str,
) -> io::Result<()> {
    let folder = project_directory(projects_dir, project)?;
    let (version, mut archive) = open_version(&folder, version_id)?;
    if !version.files.iter().any(|name| name == file) {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "File not found in version",
        ));
    }
    let name = file.replace('\\', "/");
    let target = folder.join(&name);
    let current = open_restore_target(&folder, &name)?;
    let before = current
        .as_ref()
        .map(|file| copy_and_hash(file, &mut io::sink()))
        .transpose()?;
    let mut staged = tempfile::Builder::new()
        .prefix(".refract-restore-file-")
        .tempfile_in(&folder)?;
    let entry = version_file(&mut archive, file)?;
    let size = entry.size();
    let after = copy_and_hash(entry.take(size.saturating_add(1)), &mut staged)?;
    if after.0 != size {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Version archive entry has an invalid size",
        ));
    }
    if before.as_ref() == Some(&after) {
        return snapshot_project(projects_dir, project).map(|_| ());
    }
    if let Some(current) = &current {
        let permissions = current.metadata()?.permissions();
        if permissions.readonly() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "The file is read-only.",
            ));
        }
        staged.as_file().set_permissions(permissions)?;
    }
    staged.as_file().sync_all()?;
    drop(current);

    let comment_path = folder.join(".refract-comments.json");
    let original_comments = comments::read_comments_file(&folder)?;
    let mut staged_comments = None;
    if let Some(metadata) = &original_comments
        && comments::has_attached_anchors(metadata, &name)?
    {
        let old_text = open_restore_target(&folder, &name)?
            .map(read_anchor_text)
            .transpose()?
            .flatten();
        let new_text = read_anchor_text(staged.reopen()?)?;
        if let Some(updated) =
            comments::plan_edit(metadata, &name, old_text.as_deref(), new_text.as_deref())?
        {
            staged_comments = Some(stage_file(&comment_path, &updated)?);
        }
    }

    let recovery = snapshot_project(projects_dir, project)?;
    if let Some(expected) = &before {
        let (_, mut backup) = open_version(&folder, &recovery.id)?;
        let entry = version_file(&mut backup, &name)?;
        let fingerprint = copy_and_hash(entry.take(expected.0.saturating_add(1)), &mut io::sink())?;
        if &fingerprint != expected {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "The recovery version does not match the current file.",
            ));
        }
    }
    let mut created_folders = CreatedFolders::default();
    if let Some((parent, _)) = name.rsplit_once('/') {
        created_folders.ensure(&folder, parent)?;
    }
    let current = open_restore_target(&folder, &name)?
        .map(|file| copy_and_hash(file, &mut io::sink()))
        .transpose()?;
    if current != before || comments::read_comments_file(&folder)? != original_comments {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "The file or comments changed during restore.",
        ));
    }
    let result = match before {
        Some(_) => staged.persist(&target),
        None => staged.persist_noclobber(&target),
    };
    result.map_err(|error| error.error)?;
    created_folders.0.clear();
    if let Some(staged_comments) = staged_comments {
        let update = comments::read_comments_file(&folder).and_then(|current| {
            if current != original_comments {
                return Err(io::Error::other("Comments changed during restore."));
            }
            staged_comments
                .persist(comment_path)
                .map(|_| ())
                .map_err(|error| error.error)
        });
        if let Err(error) = update {
            return Err(io::Error::other(format!(
                "The file was restored, but comment anchors could not be updated. Recovery version: {}. {error}",
                recovery.id
            )));
        }
    }
    snapshot_project(projects_dir, project).map_err(|error| io::Error::other(format!(
        "The file was restored, but the new version could not be saved. Recovery version: {}. {error}", recovery.id
    )))?;
    Ok(())
}

fn open_restore_target(folder: &Path, name: &str) -> io::Result<Option<File>> {
    match source_path(folder, name) {
        Ok(target) => open_local_file(folder, &target).map(Some),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn read_anchor_text(mut file: File) -> io::Result<Option<String>> {
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    if bytes.contains(&0) {
        return Ok(None);
    }
    Ok(String::from_utf8(bytes).ok())
}

#[derive(Default)]
struct CreatedFolders(Vec<PathBuf>);

impl CreatedFolders {
    fn ensure(&mut self, folder: &Path, parent: &str) -> io::Result<()> {
        let mut path = folder.to_owned();
        for name in parent.split('/') {
            path.push(name);
            match fs::create_dir(&path) {
                Ok(()) => self.0.push(path.clone()),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
            if !path.symlink_metadata()?.is_dir() || !path.canonicalize()?.starts_with(folder) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Invalid folder for version restore",
                ));
            }
        }
        Ok(())
    }
}

impl Drop for CreatedFolders {
    fn drop(&mut self) {
        for path in self.0.iter().rev() {
            let _ = fs::remove_dir(path);
        }
    }
}

pub fn compare_project(
    projects_dir: &Path,
    project: &Project,
    version_id: &str,
) -> io::Result<Vec<Difference>> {
    let folder = project_directory(projects_dir, project)?;
    let (version, mut archive) = open_version(&folder, version_id)?;
    let current = VersionTree::from_project(&folder)?;
    let previous_files: HashSet<&str> = version.files.iter().map(String::as_str).collect();
    let current_files: HashSet<&str> = current.files.iter().map(String::as_str).collect();
    let mut names: Vec<&str> = previous_files.union(&current_files).copied().collect();
    names.sort_by(|left, right| left.encode_utf16().cmp(right.encode_utf16()));
    let mut changes = Vec::new();
    for name in names {
        let previous = previous_files
            .contains(name)
            .then(|| version_file(&mut archive, name))
            .transpose()?;
        let now = current_files
            .contains(name)
            .then(|| open_local_file(&folder, &source_path(&folder, name)?))
            .transpose()?;
        let kind = match (previous, now) {
            (Some(previous), Some(now)) => {
                let size = previous.size();
                if size == now.metadata()?.len() {
                    let before =
                        copy_and_hash(previous.take(size.saturating_add(1)), &mut io::sink())?;
                    if before.0 != size {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "Version archive entry has an invalid size",
                        ));
                    }
                    if before == copy_and_hash(now, &mut io::sink())? {
                        continue;
                    }
                }
                ChangeKind::Changed
            }
            (Some(_), None) => ChangeKind::Deleted,
            (None, Some(_)) => ChangeKind::Added,
            (None, None) => continue,
        };
        changes.push(Difference {
            name: name.into(),
            kind,
        });
    }
    if let Some(previous_dirs) = &version.directories {
        let previous: HashSet<&str> = previous_dirs.iter().map(String::as_str).collect();
        let now: HashSet<&str> = current.directories.iter().map(String::as_str).collect();
        let mut folders: Vec<&str> = previous.symmetric_difference(&now).copied().collect();
        folders.sort_by(|left, right| left.encode_utf16().cmp(right.encode_utf16()));
        changes.extend(folders.into_iter().map(|name| Difference {
            name: format!("{name}/"),
            kind: if now.contains(name) {
                ChangeKind::FolderAdded
            } else {
                ChangeKind::FolderDeleted
            },
        }));
    }
    if version
        .root_file
        .as_deref()
        .is_some_and(|name| !name.is_empty() && name != project.root_file())
    {
        changes.push(Difference {
            name: "Root document".into(),
            kind: ChangeKind::RootDocument,
        });
    }
    Ok(changes)
}

pub fn compare_lines(
    projects_dir: &Path,
    project: &Project,
    version_id: &str,
    difference: &Difference,
) -> io::Result<DifferenceLines> {
    let folder = project_directory(projects_dir, project)?;
    let (version, mut archive) = open_version(&folder, version_id)?;
    let (previous, now) = match difference.kind {
        ChangeKind::FolderAdded => (None, Some("Folder exists".into())),
        ChangeKind::FolderDeleted => (Some("Folder exists".into()), None),
        ChangeKind::RootDocument => (version.root_file, Some(project.root_file().into())),
        kind => {
            let previous = if kind != ChangeKind::Added && version.files.contains(&difference.name)
            {
                let entry = version_file(&mut archive, &difference.name)?;
                let size = entry.size();
                read_comparison_text(entry, size)?
            } else {
                None
            };
            let now = if kind == ChangeKind::Deleted {
                None
            } else {
                let source = open_local_file(&folder, &source_path(&folder, &difference.name)?)?;
                let size = source.metadata()?.len();
                read_comparison_text(source, size)?
            };
            (previous, now)
        }
    };
    Ok(align_lines(previous.as_deref(), now.as_deref()))
}

fn open_version(folder: &Path, version_id: &str) -> io::Result<(Version, ZipArchive<File>)> {
    if !is_project_id(version_id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid version ID",
        ));
    }
    let versions_dir = version_folder(folder, false)?;
    let version = read_manifest(&versions_dir, version_id)?;
    let mut seen = HashSet::new();
    for name in &version.files {
        if !valid_version_path(name) || !seen.insert(name.to_lowercase()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid or duplicate file in version",
            ));
        }
    }
    if !version
        .directories
        .iter()
        .flatten()
        .chain(version.root_file.iter().filter(|name| !name.is_empty()))
        .all(|name| valid_version_path(name))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid folder or root document in version",
        ));
    }
    let path = versions_dir.join(format!("{version_id}.zip"));
    if !path.symlink_metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid version archive",
        ));
    }
    let archive =
        ZipArchive::new(open_local_file(&versions_dir, &path)?).map_err(io::Error::other)?;
    Ok((version, archive))
}

fn valid_version_path(name: &str) -> bool {
    name.split(['/', '\\']).all(valid_project_component)
}

fn version_file<'a>(
    archive: &'a mut ZipArchive<File>,
    name: &str,
) -> io::Result<zip::read::ZipFile<'a, File>> {
    let entry = archive.by_name(name).map_err(io::Error::other)?;
    if !entry.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Expected a file in the version archive",
        ));
    }
    Ok(entry)
}

fn read_comparison_text(reader: impl Read, size: u64) -> io::Result<Option<String>> {
    if size > COMPARISON_LIMIT {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    reader.take(COMPARISON_LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > COMPARISON_LIMIT || bytes.contains(&0) {
        return Ok(None);
    }
    Ok(String::from_utf8(bytes).ok().map(|mut text| {
        if text.starts_with('\u{feff}') {
            text.drain(..'\u{feff}'.len_utf8());
        }
        text
    }))
}

fn align_lines(previous: Option<&str>, now: Option<&str>) -> DifferenceLines {
    let left: Vec<&str> = previous.unwrap_or("").split('\n').collect();
    let right: Vec<&str> = now.unwrap_or("").split('\n').collect();
    let mut old_changed = vec![true; left.len()];
    let mut new_changed = vec![true; right.len()];
    let skipped_alignment = left.len().saturating_mul(right.len()) > LINE_ALIGNMENT_LIMIT;
    if !skipped_alignment {
        let width = right.len() + 1;
        let mut lengths = vec![0u32; (left.len() + 1) * width];
        for i in (0..left.len()).rev() {
            for j in (0..right.len()).rev() {
                lengths[i * width + j] = if left[i] == right[j] {
                    1 + lengths[(i + 1) * width + j + 1]
                } else {
                    lengths[(i + 1) * width + j].max(lengths[i * width + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < left.len() && j < right.len() {
            if left[i] == right[j] {
                old_changed[i] = false;
                new_changed[j] = false;
                i += 1;
                j += 1;
            } else if lengths[(i + 1) * width + j] >= lengths[i * width + j + 1] {
                i += 1;
            } else {
                j += 1;
            }
        }
    }
    let side = |lines: Vec<&str>, changed: Vec<bool>| ComparisonSide {
        changed_indices: changed
            .iter()
            .enumerate()
            .filter_map(|(index, &changed)| changed.then_some(index))
            .collect(),
        lines: lines
            .into_iter()
            .zip(changed)
            .map(|(text, changed)| ComparisonLine {
                text: text.into(),
                changed,
            })
            .collect(),
    };
    DifferenceLines {
        previous: previous.map(|_| side(left, old_changed)),
        current: now.map(|_| side(right, new_changed)),
        skipped_alignment,
    }
}

fn list_project_versions(folder: &Path) -> io::Result<Vec<Version>> {
    let versions_dir = match version_folder(folder, false) {
        Ok(path) => path,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut versions = Vec::new();
    for file in fs::read_dir(&versions_dir)? {
        let file = file?;
        let name = file.file_name();
        let Some(id) = name
            .to_str()
            .and_then(|name| name.strip_suffix(".json"))
            .filter(|id| is_project_id(id))
        else {
            continue;
        };
        versions.push(read_manifest(&versions_dir, id)?);
    }
    versions.sort_by(|left, right| {
        right
            .sequence
            .cmp(&left.sequence)
            .then_with(|| right.created.cmp(&left.created))
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(versions)
}

fn read_manifest(folder: &Path, id: &str) -> io::Result<Version> {
    let path = folder.join(format!("{id}.json"));
    if !path.symlink_metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid version manifest",
        ));
    }
    let version: Version =
        serde_json::from_slice(&read_local_file(folder, &path, MANIFEST_LIMIT)?)?;
    if version.id != id {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Version ID does not match its manifest",
        ));
    }
    Ok(version)
}

pub fn snapshot_project(projects_dir: &Path, project: &Project) -> io::Result<Version> {
    let folder = project_directory(projects_dir, project)?;
    let versions = list_project_versions(&folder)?;
    let tree = VersionTree::from_project(&folder)?;
    let layout =
        serde_json::json!({ "directories": tree.directories, "rootFile": project.root_file() });
    let mut digest = Sha256::new();
    digest.update(b"refract-snapshot-v2:");
    digest.update(serde_json::to_vec(&layout)?);

    let mut fingerprints = Vec::with_capacity(tree.files.len());
    for name in &tree.files {
        let file = open_local_file(&folder, &source_path(&folder, name)?)?;
        let fingerprint = copy_and_hash(file, &mut io::sink())?;
        digest.update(serde_json::to_vec(&(name, fingerprint.0, &fingerprint.1))?);
        fingerprints.push(fingerprint);
    }
    let digest = format!("{:x}", digest.finalize());
    if let Some(latest) = versions.first().filter(|latest| latest.digest == digest)
        && version_folder(&folder, false)?
            .join(format!("{}.zip", latest.id))
            .symlink_metadata()
            .is_ok_and(|info| info.is_file())
    {
        return Ok(latest.clone());
    }
    let sequence = versions
        .first()
        .map_or(0, |latest| latest.sequence)
        .checked_add(1)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid version sequence"))?;
    let versions_dir = version_folder(&folder, true)?;
    let mut archive = tempfile::Builder::new()
        .prefix(".refract-version-")
        .tempfile_in(&versions_dir)?;
    let mut zip = ZipWriter::new(archive.as_file_mut());
    for (name, expected) in tree.files.iter().zip(fingerprints) {
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .compression_level(Some(3))
            .large_file(expected.0 >= u64::from(u32::MAX));
        zip.start_file(name.as_str(), options)
            .map_err(io::Error::other)?;
        let file = open_local_file(&folder, &source_path(&folder, name)?)?;
        if copy_and_hash(file, &mut zip)? != expected {
            return Err(project_changed());
        }
    }
    zip.finish().map_err(io::Error::other)?;
    if VersionTree::from_project(&folder)? != tree {
        return Err(project_changed());
    }
    archive.as_file().sync_all()?;
    let version = Version {
        id: Uuid::new_v4().to_string(),
        created: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        sequence,
        digest,
        files: tree.files,
        directories: Some(tree.directories),
        root_file: Some(project.root_file().to_owned()),
    };
    let records = serde_json::to_vec(&version)?;
    if records.len() > MANIFEST_LIMIT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Version manifest exceeds the size limit",
        ));
    }
    let mut manifest = tempfile::Builder::new()
        .prefix(".refract-version-")
        .tempfile_in(&versions_dir)?;
    manifest.write_all(&records)?;
    manifest.as_file().sync_all()?;
    let target = versions_dir.join(format!("{}.zip", version.id));
    archive
        .persist_noclobber(&target)
        .map_err(|error| error.error)?;
    if let Err(error) =
        manifest.persist_noclobber(versions_dir.join(format!("{}.json", version.id)))
    {
        let _ = fs::remove_file(&target);
        return Err(error.error);
    }
    Ok(version)
}

fn project_changed() -> io::Error {
    io::Error::new(
        io::ErrorKind::Interrupted,
        "Project files changed while a version was being captured",
    )
}

fn copy_and_hash(mut input: impl Read, output: &mut impl Write) -> io::Result<(u64, String)> {
    let mut digest = Sha256::new();
    let mut size = 0;
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let read = input.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        let bytes = &chunk[..read];
        output.write_all(bytes)?;
        digest.update(bytes);
        size += read as u64;
    }
    Ok((size, format!("{:x}", digest.finalize())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::list_projects;
    use std::fs::File;
    use zip::ZipArchive;

    #[test]
    fn restores_binary_files_and_uses_the_pre_restore_version_as_recovery() {
        let root = tempfile::tempdir().unwrap();
        let id = "b3b9ffb5-13c0-4d22-b427-54520347684a";
        let folder = root.path().join(id);
        fs::create_dir_all(folder.join("sections/figures")).unwrap();
        fs::write(
            folder.join(".refract.json"),
            serde_json::json!({"id":id,"name":"Paper"}).to_string(),
        )
        .unwrap();
        let image = "sections/figures/figure.bin";
        fs::write(folder.join(image), [0, 255, 10]).unwrap();
        fs::write(folder.join("main.tex"), "Draft\n").unwrap();
        let project = list_projects(root.path()).unwrap().pop().unwrap();
        let original = snapshot_project(root.path(), &project).unwrap();
        fs::write(folder.join(image), [0, 15, 20]).unwrap();
        fs::write(folder.join("main.tex"), "Keep this edit\n").unwrap();
        restore_version_file(root.path(), &project, &original.id, image).unwrap();
        assert_eq!(fs::read(folder.join(image)).unwrap(), [0, 255, 10]);
        assert_eq!(
            fs::read_to_string(folder.join("main.tex")).unwrap(),
            "Keep this edit\n"
        );
        let history = list_versions(root.path(), &project).unwrap();
        assert_eq!(history.len(), 3);
        restore_version_file(root.path(), &project, &history[1].id, image).unwrap();
        assert_eq!(fs::read(folder.join(image)).unwrap(), [0, 15, 20]);

        fs::remove_file(folder.join(image)).unwrap();
        fs::remove_dir(folder.join("sections/figures")).unwrap();
        fs::remove_dir(folder.join("sections")).unwrap();
        restore_version_file(root.path(), &project, &original.id, image).unwrap();
        assert_eq!(fs::read(folder.join(image)).unwrap(), [0, 255, 10]);
        fs::write(
            folder
                .join(".refract-versions")
                .join(format!("{}.zip", original.id)),
            "damaged archive",
        )
        .unwrap();
        let before = list_versions(root.path(), &project).unwrap().len();
        fs::write(folder.join(image), [0, 1]).unwrap();
        assert!(restore_version_file(root.path(), &project, &original.id, image).is_err());
        assert_eq!(fs::read(folder.join(image)).unwrap(), [0, 1]);
        assert_eq!(list_versions(root.path(), &project).unwrap().len(), before);
    }

    #[test]
    fn restores_source_and_updates_anchored_comments() {
        let root = tempfile::tempdir().unwrap();
        let id = "b3b9ffb5-13c0-4d22-b427-54520347684a";
        let folder = root.path().join(id);
        fs::create_dir(&folder).unwrap();
        fs::write(
            folder.join(".refract.json"),
            serde_json::json!({"id":id,"name":"Paper"}).to_string(),
        )
        .unwrap();
        fs::write(folder.join("main.tex"), "Check this.\n").unwrap();
        let project = list_projects(root.path()).unwrap().pop().unwrap();
        let original = snapshot_project(root.path(), &project).unwrap();
        fs::write(folder.join("main.tex"), "Draft\nCheck this.\n").unwrap();
        fs::write(folder.join(".refract-comments.json"), serde_json::json!([{
            "id":"read", "file":"main.tex", "quote":"Check", "text":"Clarify", "anchor":{
                "start":6,"end":11,"prefix":"Draft\n","suffix":" this.\n","sourceHash":"","status":"attached"
            }
        }]).to_string()).unwrap();
        restore_version_file(root.path(), &project, &original.id, "main.tex").unwrap();
        let comments = comments::read_comments(root.path(), &project).unwrap();
        assert_eq!(comments[0].source_position("Check this.\n"), Some((0, 0)));
        let bytes = comments::read_comments_file(&folder).unwrap().unwrap();
        let records: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(records[0]["anchor"]["start"], 0);
        assert_eq!(records[0]["anchor"]["status"], "attached");
    }

    #[test]
    fn captures_electron_compatible_versions_and_deduplicates_unchanged_projects() {
        let root = tempfile::tempdir().unwrap();
        let id = "b3b9ffb5-13c0-4d22-b427-54520347684a";
        let folder = root.path().join(id);
        fs::create_dir_all(folder.join("sections/empty")).unwrap();
        fs::write(
            folder.join(".refract.json"),
            serde_json::json!({"id": id, "name": "Paper"}).to_string(),
        )
        .unwrap();
        fs::write(folder.join("main.tex"), "Main LaTeX\r\n").unwrap();
        fs::write(folder.join("sections/abstract.tex"), "Nested 😀 text\n").unwrap();
        fs::write(folder.join("sections/picture.bin"), [0, 255, 13, 10]).unwrap();
        let project = list_projects(root.path()).unwrap().pop().unwrap();

        let first = snapshot_project(root.path(), &project).unwrap();
        assert_eq!(first.sequence, 1);
        // Generated using the Electron snapshot hash input and Node's SHA-256.
        assert_eq!(
            first.digest,
            "8d3519008ef42836fdac6b1ed720a87ace69a2423370292f38a556962db6846e"
        );
        assert_eq!(
            first.directories.as_deref().unwrap(),
            ["sections", "sections/empty"]
        );
        assert_eq!(first.root_file.as_deref(), Some("main.tex"));
        let path = folder
            .join(".refract-versions")
            .join(format!("{}.zip", first.id));
        let mut archive = ZipArchive::new(File::open(path).unwrap()).unwrap();
        let mut bytes = Vec::new();
        archive
            .by_name("sections/picture.bin")
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(bytes, [0, 255, 13, 10]);
        let mut source = String::new();
        archive
            .by_name("main.tex")
            .unwrap()
            .read_to_string(&mut source)
            .unwrap();
        assert_eq!(source, "Main LaTeX\r\n");

        assert_eq!(
            snapshot_project(root.path(), &project).unwrap().id,
            first.id
        );
        fs::write(folder.join("main.tex"), "Changed").unwrap();
        let second = snapshot_project(root.path(), &project).unwrap();
        assert_eq!(second.sequence, 2);
        assert_ne!(second.digest, first.digest);
        let versions = list_versions(root.path(), &project).unwrap();
        assert_eq!(
            versions
                .iter()
                .map(|version| &version.id)
                .collect::<Vec<_>>(),
            [&second.id, &first.id]
        );
    }

    #[test]
    fn opens_legacy_manifests_with_no_sequence_or_directory_list() {
        let root = tempfile::tempdir().unwrap();
        let id = "b3b9ffb5-13c0-4d22-b427-54520347684a";
        let version_id = "cd220285-8dd2-57d7-b847-aec2740c249a";
        let folder = root.path().join(id);
        let versions = folder.join(".refract-versions");
        fs::create_dir_all(&versions).unwrap();
        fs::write(
            folder.join(".refract.json"),
            serde_json::json!({"id": id, "name": "Paper"}).to_string(),
        )
        .unwrap();
        fs::write(
            versions.join(format!("{version_id}.json")),
            serde_json::json!({
                "id": version_id,
                "created": "2025-08-29T00:00:00.000Z",
                "files": ["main.tex"]
            })
            .to_string(),
        )
        .unwrap();
        let project = list_projects(root.path()).unwrap().pop().unwrap();
        let versions = list_versions(root.path(), &project).unwrap();
        assert_eq!(versions[0].sequence, 0);
        assert_eq!(versions[0].files, ["main.tex"]);
        assert!(versions[0].directories.is_none());
    }

    #[test]
    fn compares_files_binary_content_folders_and_the_root_document() {
        let root = tempfile::tempdir().unwrap();
        let id = "b3b9ffb5-13c0-4d22-b427-54520347684a";
        let folder = root.path().join(id);
        fs::create_dir_all(folder.join("empty")).unwrap();
        fs::write(
            folder.join(".refract.json"),
            serde_json::json!({"id": id, "name": "Paper"}).to_string(),
        )
        .unwrap();
        fs::write(folder.join("main.tex"), "Intro\nRepeated\n").unwrap();
        fs::write(folder.join("asset.bin"), [0, 1, 2, 3]).unwrap();
        fs::write(folder.join("old.tex"), "Deleted source").unwrap();
        let project = list_projects(root.path()).unwrap().pop().unwrap();
        let version = snapshot_project(root.path(), &project).unwrap();

        fs::write(folder.join("main.tex"), "Intro\nUpdated\n").unwrap();
        fs::write(folder.join("asset.bin"), [0, 1, 3, 3]).unwrap();
        fs::remove_file(folder.join("old.tex")).unwrap();
        fs::remove_dir(folder.join("empty")).unwrap();
        fs::create_dir(folder.join("new")).unwrap();
        fs::write(folder.join("new/root.tex"), "New root").unwrap();
        fs::write(
            folder.join(".refract.json"),
            serde_json::json!({"id": id, "name": "Paper", "rootFile": "new/root.tex"}).to_string(),
        )
        .unwrap();
        let project = list_projects(root.path()).unwrap().pop().unwrap();

        let changes = compare_project(root.path(), &project, &version.id).unwrap();
        assert_eq!(
            changes
                .iter()
                .map(|change| (change.name.as_str(), change.kind))
                .collect::<Vec<_>>(),
            [
                ("asset.bin", ChangeKind::Changed),
                ("main.tex", ChangeKind::Changed),
                ("new/root.tex", ChangeKind::Added),
                ("old.tex", ChangeKind::Deleted),
                ("empty/", ChangeKind::FolderDeleted),
                ("new/", ChangeKind::FolderAdded),
                ("Root document", ChangeKind::RootDocument),
            ]
        );
        let binary = compare_lines(root.path(), &project, &version.id, &changes[0]).unwrap();
        assert!(binary.previous.is_none() && binary.current.is_none());
        let text = compare_lines(root.path(), &project, &version.id, &changes[1]).unwrap();
        assert_eq!(text.previous.unwrap().lines[1].text, "Repeated");
        let current = text.current.unwrap();
        assert_eq!(current.lines[1].text, "Updated");
        assert_eq!(current.changed_indices, [1]);
    }

    #[test]
    fn aligns_repeated_lines_in_order_and_skips_large_alignment_matrices() {
        let diff = align_lines(Some("A\nB\nA"), Some("A\nA\nB"));
        assert_eq!(diff.previous.unwrap().changed_indices, [1]);
        assert_eq!(diff.current.unwrap().changed_indices, [2]);
        let long = "A\n".repeat(1001);
        let diff = align_lines(Some(&long), Some(&long));
        assert!(diff.skipped_alignment);
        assert_eq!(diff.current.unwrap().changed_indices.len(), 1002);
    }
}

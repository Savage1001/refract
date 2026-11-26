use crate::{
    Project, ProjectEntry, is_project_id, open_local_file, project_directory, read_local_file,
    read_project_entries, source_path,
};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

const MANIFEST_LIMIT: usize = 4 * 1024 * 1024;

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
        if !file.file_type()?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid version manifest",
            ));
        }
        let version: Version = serde_json::from_slice(&read_local_file(
            &versions_dir,
            &file.path(),
            MANIFEST_LIMIT,
        )?)?;
        if version.id != id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Version ID does not match its manifest",
            ));
        }
        versions.push(version);
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
}

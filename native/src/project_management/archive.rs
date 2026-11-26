use super::{Project, encode_metadata, new_metadata, write_new_file};
use crate::project_entries::rename_without_replacement;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet, hash_map::Entry},
    fs::{self, File},
    io::{self, BufRead, BufReader, Read},
    path::Path,
};
use zip::{CompressionMethod, ZipArchive};

const MAX_ARCHIVE_BYTES: u64 = 500 * 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;

pub struct ProjectArchive {
    source: ZipArchive<File>,
    title: String,
    entries: Vec<ArchiveEntry>,
    roots: Vec<String>,
    separate_projects: bool,
}

struct ArchiveEntry {
    index: usize,
    path: String,
    directory: bool,
    size: u64,
}

struct ArchivePath {
    position: usize,
    spelling: String,
    directory: bool,
}

impl ProjectArchive {
    pub fn open(path: &Path) -> io::Result<Self> {
        let info = fs::metadata(path)?;
        if !info.is_file() || info.len() > MAX_ARCHIVE_BYTES {
            return Err(invalid("Project archive is too large or is not a file"));
        }
        let mut source = ZipArchive::new(File::open(path)?).map_err(io::Error::other)?;
        let mut entries = read_entries(&mut source)?;
        if let Some(wrapper) = common_wrapper(&entries) {
            let prefix = format!("{wrapper}/");
            entries.retain(|entry| entry.path != wrapper);
            for entry in &mut entries {
                entry.path.drain(..prefix.len());
            }
        }
        let roots = find_roots(&mut source, &entries)?;
        let mut folders = HashSet::new();
        let separate_projects = roots.len() > 1
            && roots.iter().all(|name| {
                name.split_once('/')
                    .is_some_and(|(folder, _)| folders.insert(folder))
            });
        let name = path.file_stem().unwrap_or_default().to_string_lossy();
        let title = if name.trim().is_empty() {
            "Imported Project"
        } else {
            name.trim()
        }
        .to_owned();
        Ok(Self {
            source,
            title,
            entries,
            roots,
            separate_projects,
        })
    }

    pub fn roots(&self) -> &[String] {
        &self.roots
    }

    pub fn import(mut self, projects_dir: &Path, roots: &[String]) -> io::Result<Project> {
        if roots.is_empty()
            || roots.iter().collect::<HashSet<_>>().len() != roots.len()
            || roots.iter().any(|root| !self.roots.contains(root))
        {
            return Err(invalid("Invalid LaTeX root selection"));
        }
        if roots.len() > 1 && !self.separate_projects {
            return Err(invalid(
                "Cannot separate projects in this archive: each root document must be in its own top-level folder",
            ));
        }
        fs::create_dir_all(projects_dir)?;
        let projects_dir = projects_dir.canonicalize()?;
        let scratch = tempfile::Builder::new()
            .prefix(".refract-import-")
            .tempdir_in(&projects_dir)?;
        let mut projects = Vec::new();
        for root in roots {
            let (title, relative_root, prefix) = if self.separate_projects {
                let (folder, relative_root) =
                    root.split_once('/').expect("a separate root has a folder");
                (folder, relative_root, format!("{folder}/"))
            } else {
                (self.title.as_str(), root.as_str(), String::new())
            };
            let mut fields = new_metadata(title);
            fields.insert(
                "rootFile".to_owned(),
                Value::String(relative_root.to_owned()),
            );
            let (project, metadata) = encode_metadata(&fields)?;
            let staged = scratch.path().join(&project.id);
            fs::create_dir(&staged)?;
            extract_project(&mut self.source, &self.entries, &prefix, &staged)?;
            write_new_file(&staged.join(".refract.json"), &metadata)?;
            projects.push(project);
        }
        for (index, project) in projects.iter().enumerate() {
            let from = scratch.path().join(&project.id);
            let to = projects_dir.join(&project.id);
            if let Err(error) = rename_without_replacement(&from, &to) {
                let failed_rollback: Vec<_> = projects[..index]
                    .iter()
                    .filter_map(|published| {
                        rename_without_replacement(
                            &projects_dir.join(&published.id),
                            &scratch.path().join(&published.id),
                        )
                        .err()
                        .map(|rollback| format!("{}: {rollback}", published.name))
                    })
                    .collect();
                if !failed_rollback.is_empty() {
                    return Err(io::Error::other(format!(
                        "Import failed. {error}. Couldn’t undo the import of {}",
                        failed_rollback.join(", ")
                    )));
                }
                return Err(error);
            }
        }
        Ok(projects.remove(0))
    }
}

fn read_entries(source: &mut ZipArchive<File>) -> io::Result<Vec<ArchiveEntry>> {
    if source.is_empty() || source.len() > MAX_ENTRIES {
        return Err(invalid("Archive has no files or too many files"));
    }
    if source.has_duplicate_names() {
        return Err(invalid("Archive contains duplicate file names"));
    }
    let mut entries = Vec::new();
    let mut paths = HashMap::new();
    let mut total = 0;
    for index in 0..source.len() {
        let entry = source.by_index_raw(index).map_err(io::Error::other)?;
        let directory = entry.is_dir();
        let size = entry.size();
        if size > MAX_ARCHIVE_BYTES - total {
            return Err(invalid("Project archive is too large"));
        }
        total += size;
        if !directory
            && entry.compression() == CompressionMethod::Stored
            && entry.compressed_size() != size
        {
            return Err(invalid("Invalid stored file size in project archive"));
        }
        let name = entry.name().map_err(io::Error::other)?;
        let name = name.strip_suffix('/').unwrap_or(&name);
        if entry.is_symlink() {
            return Err(invalid("Invalid path in project archive"));
        }
        let mut parent = 0;
        let mut parts = name.split('/').peekable();
        while let Some(part) = parts.next() {
            let lower = part.to_lowercase();
            if !valid_archive_component(part, &lower) {
                return Err(invalid("Invalid path in project archive"));
            }
            let is_directory = parts.peek().is_some() || directory;
            let position = paths.len() + 1;
            match paths.entry((parent, lower)) {
                Entry::Occupied(previous) => {
                    let existing: &ArchivePath = previous.get();
                    if !is_directory || !existing.directory || existing.spelling != part {
                        return Err(invalid("Archive contains conflicting paths"));
                    }
                    parent = existing.position;
                }
                Entry::Vacant(entry) => {
                    entry.insert(ArchivePath {
                        position,
                        spelling: part.to_owned(),
                        directory: is_directory,
                    });
                    parent = position;
                }
            }
        }
        entries.push(ArchiveEntry {
            index,
            path: name.to_owned(),
            directory,
            size,
        });
    }
    if entries.iter().all(|entry| entry.directory) {
        return Err(invalid("Archive has no files or too many files"));
    }
    Ok(entries)
}

fn valid_archive_component(part: &str, lower: &str) -> bool {
    if part.is_empty()
        || lower.starts_with(".refract")
        || part.ends_with(['.', ' '])
        || part
            .chars()
            .any(|c| c <= '\u{1f}' || "<>:\"|?*\\".contains(c))
    {
        return false;
    }
    let device = lower.split('.').next().unwrap_or(lower);
    if matches!(device, "con" | "prn" | "aux" | "nul") {
        return false;
    }
    !device
        .strip_prefix("com")
        .or_else(|| device.strip_prefix("lpt"))
        .is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
}

fn common_wrapper(entries: &[ArchiveEntry]) -> Option<String> {
    let first_file = entries.iter().find(|entry| !entry.directory)?;
    let (wrapper, _) = first_file.path.split_once('/')?;
    let prefix = format!("{wrapper}/");
    entries
        .iter()
        .all(|entry| entry.path.starts_with(&prefix) || entry.directory && entry.path == wrapper)
        .then(|| wrapper.to_owned())
}

fn find_roots(source: &mut ZipArchive<File>, entries: &[ArchiveEntry]) -> io::Result<Vec<String>> {
    let mut roots = Vec::new();
    let mut default = None;
    for entry in entries.iter().filter(|entry| !entry.directory) {
        if !entry
            .path
            .rsplit_once('.')
            .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("tex"))
        {
            continue;
        }
        if entry.path.eq_ignore_ascii_case("main.tex") {
            default = Some(&entry.path);
        }
        let file = source.by_index(entry.index).map_err(io::Error::other)?;
        let mut reader = BufReader::new(file.take(entry.size + 1));
        let mut bytes_read = 0;
        let mut line = Vec::new();
        let mut has_document_class = false;
        loop {
            line.clear();
            let read = reader.read_until(b'\n', &mut line)?;
            if read == 0 {
                break;
            }
            bytes_read += read as u64;
            if !has_document_class {
                has_document_class = document_class_on_line(&String::from_utf8_lossy(&line));
            }
        }
        if bytes_read != entry.size {
            return Err(invalid("Invalid file data in project archive"));
        }
        if has_document_class {
            roots.push(entry.path.clone());
        }
    }
    if let Some(default) = default
        && !roots.contains(default)
    {
        roots.push(default.clone());
    }
    if roots.is_empty() {
        return Err(invalid("Project does not contain a LaTeX root file"));
    }
    Ok(roots)
}

fn document_class_on_line(line: &str) -> bool {
    let mut backslashes = 0;
    let end = line
        .bytes()
        .position(|byte| {
            let comment = byte == b'%' && backslashes % 2 == 0;
            backslashes = if byte == b'\\' { backslashes + 1 } else { 0 };
            comment
        })
        .unwrap_or(line.len());
    line[..end]
        .match_indices(r"\documentclass")
        .any(|(start, command)| {
            let arguments = line[start + command.len()..end].trim_start();
            if let Some(options) = arguments.strip_prefix('[') {
                options
                    .split_once(']')
                    .is_some_and(|(_, rest)| rest.trim_start().starts_with('{'))
            } else {
                arguments.starts_with('{')
            }
        })
}

fn extract_project(
    source: &mut ZipArchive<File>,
    entries: &[ArchiveEntry],
    prefix: &str,
    directory: &Path,
) -> io::Result<()> {
    for entry in entries {
        let Some(relative) = entry.path.strip_prefix(prefix) else {
            continue;
        };
        let path = directory.join(relative);
        if entry.directory {
            fs::create_dir_all(path)?;
        } else {
            fs::create_dir_all(path.parent().expect("a file in the import has a parent"))?;
            let file = source.by_index(entry.index).map_err(io::Error::other)?;
            let mut target = File::create_new(path)?;
            if io::copy(&mut file.take(entry.size + 1), &mut target)? != entry.size {
                return Err(invalid("Invalid file data in project archive"));
            }
            target.sync_all()?;
        }
    }
    Ok(())
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{list_projects, read_source};
    use std::io::{Seek, SeekFrom, Write};
    use zip::{ZipWriter, write::SimpleFileOptions};

    fn write_zip(path: &Path, files: &[(&str, &[u8])]) {
        let mut zip = ZipWriter::new(File::create(path).unwrap());
        for (name, content) in files {
            if name.ends_with('/') {
                zip.add_directory(*name, SimpleFileOptions::default())
                    .unwrap();
            } else {
                zip.start_file(*name, SimpleFileOptions::default()).unwrap();
                zip.write_all(content).unwrap();
            }
        }
        zip.finish().unwrap();
    }

    #[test]
    fn unwraps_single_projects_and_preserves_nested_empty_folders() {
        let profile = tempfile::tempdir().unwrap();
        let archive = profile.path().join("Thesis.zip");
        let projects = profile.path().join("projects");
        let paper = br"\% text \documentclass [11pt] {article}";
        write_zip(
            &archive,
            &[
                ("wrapper/paper.tex", paper),
                ("wrapper/chapter.tex", br"% \documentclass{book}"),
                ("wrapper/empty/nested/", b""),
            ],
        );
        let archive = ProjectArchive::open(&archive).unwrap();
        assert_eq!(archive.roots(), ["paper.tex"]);
        assert!(!projects.exists());
        let roots = archive.roots().to_vec();
        let project = archive.import(&projects, &roots).unwrap();
        assert_eq!(project.name, "Thesis");
        assert_eq!(project.root_file(), "paper.tex");
        assert_eq!(
            read_source(&projects, &project, "paper.tex")
                .unwrap()
                .as_bytes(),
            paper
        );
        assert!(projects.join(&project.id).join("empty/nested").is_dir());
        assert!(!projects.join(&project.id).join("wrapper").exists());
    }

    #[test]
    fn imports_all_distinct_project_folders_and_validates_root_choice() {
        let profile = tempfile::tempdir().unwrap();
        let archive = profile.path().join("bundle.zip");
        let projects = profile.path().join("projects");
        write_zip(
            &archive,
            &[
                ("bundle/One/main.tex", br"\documentclass{article}"),
                ("bundle/One/empty/", b""),
                ("bundle/Two/nested/paper.tex", br"\documentclass{book}"),
                ("bundle/Two/picture.bin", &[0, 255]),
            ],
        );
        assert!(
            ProjectArchive::open(&archive)
                .unwrap()
                .import(&projects, &["../outside.tex".into()])
                .is_err()
        );
        assert!(!projects.exists());
        let archive = ProjectArchive::open(&archive).unwrap();
        assert_eq!(archive.roots(), ["One/main.tex", "Two/nested/paper.tex"]);
        let roots = archive.roots().to_vec();
        let first = archive.import(&projects, &roots).unwrap();
        assert_eq!(first.name, "One");
        assert!(projects.join(&first.id).join("empty").is_dir());
        assert!(!projects.join(&first.id).join("picture.bin").exists());
        let all = list_projects(&projects).unwrap();
        assert_eq!(all.len(), 2);
        let second = all.iter().find(|project| project.name == "Two").unwrap();
        assert_eq!(second.root_file(), "nested/paper.tex");
        assert_eq!(
            fs::read(projects.join(&second.id).join("picture.bin")).unwrap(),
            [0, 255]
        );
    }

    #[test]
    fn accepts_main_tex_fallback_and_rejects_ambiguous_import_all() {
        let profile = tempfile::tempdir().unwrap();
        let archive = profile.path().join("papers.zip");
        let projects = profile.path().join("projects");
        write_zip(
            &archive,
            &[
                ("main.TEX", b"Main text"),
                ("second.tex", br"\documentclass{book}"),
            ],
        );
        let plan = ProjectArchive::open(&archive).unwrap();
        assert_eq!(plan.roots(), ["second.tex", "main.TEX"]);
        let roots = plan.roots().to_vec();
        assert!(plan.import(&projects, &roots).is_err());
        assert!(!projects.exists());
        let project = ProjectArchive::open(&archive)
            .unwrap()
            .import(&projects, &["main.TEX".into()])
            .unwrap();
        assert_eq!(project.root_file(), "main.TEX");
        assert!(projects.join(&project.id).join("second.tex").exists());
    }

    #[test]
    fn rejects_unsafe_archive_paths_and_windows_aliases() {
        let profile = tempfile::tempdir().unwrap();
        let archive = profile.path().join("paper.zip");
        for invalid in [
            "../outside.txt",
            "assets/CON.dat",
            "assets/file. ",
            "assets/.Refract-settings",
            "main.TEX",
        ] {
            write_zip(&archive, &[("main.tex", b"Root"), (invalid, b"Invalid")]);
            assert!(
                ProjectArchive::open(&archive).is_err(),
                "accepted {invalid}"
            );
        }
    }

    #[test]
    fn leaves_no_partially_imported_projects_if_later_zip_data_is_corrupt() {
        let profile = tempfile::tempdir().unwrap();
        let archive = profile.path().join("bundle.zip");
        let projects = profile.path().join("projects");
        write_zip(
            &archive,
            &[
                ("One/main.tex", br"\documentclass{article}"),
                ("Two/main.tex", br"\documentclass{book}"),
                ("Two/broken.bin", b"Valid data"),
            ],
        );
        let mut zip = ZipArchive::new(File::open(&archive).unwrap()).unwrap();
        let offset = zip.by_name("Two/broken.bin").unwrap().data_start().unwrap();
        drop(zip);
        let mut corrupt = File::options().write(true).open(&archive).unwrap();
        corrupt.seek(SeekFrom::Start(offset)).unwrap();
        corrupt.write_all(b"X").unwrap();
        drop(corrupt);
        let archive = ProjectArchive::open(&archive).unwrap();
        let roots = archive.roots().to_vec();
        assert!(archive.import(&projects, &roots).is_err());
        assert!(fs::read_dir(&projects).unwrap().next().is_none());
    }
}

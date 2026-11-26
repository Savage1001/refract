use crate::{
    Project, open_local_file, project_directory, project_management, valid_project_component,
};
use regex::{Regex, RegexBuilder};
use std::{
    fs,
    io::{self, Read},
    ops::Range,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

const MAX_SEARCH_BYTES: u64 = 2 * 1024 * 1024;
const MAX_RESULTS: usize = 200;
const MAX_EXCERPT_UNITS: usize = 240;

#[derive(Clone)]
pub struct SearchHit {
    pub file: String,
    pub line: u32,
    pub column: usize,
    pub length: usize,
    pub excerpt: String,
    pub highlight: Option<Range<usize>>,
}

/// Search saved UTF-8 files. Coordinates use UTF-16 so they agree with the existing editor.
pub fn search_project(
    projects_dir: &Path,
    project: &Project,
    query: &str,
    cancelled: &AtomicBool,
) -> io::Result<Vec<SearchHit>> {
    if query.trim().is_empty() || query.len() as u64 > MAX_SEARCH_BYTES {
        return Ok(Vec::new());
    }
    let needle = RegexBuilder::new(&regex::escape(query))
        .case_insensitive(true)
        .build()
        .map_err(io::Error::other)?;
    let root = project_directory(projects_dir, project)?;
    if project_management::is_link(&fs::symlink_metadata(&root)?) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Linked project folder",
        ));
    }
    let mut search = Search {
        needle,
        cancelled,
        results: Vec::new(),
        bytes: Vec::new(),
    };
    search.visit(&root, "")?;
    Ok(search.results)
}

struct Search<'a> {
    needle: Regex,
    cancelled: &'a AtomicBool,
    results: Vec<SearchHit>,
    bytes: Vec<u8>,
}

impl Search<'_> {
    fn finished(&self) -> bool {
        self.results.len() == MAX_RESULTS || self.cancelled.load(Ordering::Relaxed)
    }

    fn visit(&mut self, root: &Path, folder: &str) -> io::Result<()> {
        for entry in fs::read_dir(root.join(folder))? {
            if self.finished() {
                break;
            }
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str().filter(|name| valid_project_component(name)) else {
                continue;
            };
            let info = match entry.metadata() {
                Ok(info) => info,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            if project_management::is_link(&info) {
                continue;
            }
            let path = if folder.is_empty() {
                name.to_owned()
            } else {
                format!("{folder}/{name}")
            };
            let result = if info.is_dir() {
                self.visit(root, &path)
            } else if info.is_file() && info.len() <= MAX_SEARCH_BYTES {
                self.search_file(root, &path)
            } else {
                continue;
            };
            if let Err(error) = result
                && error.kind() != io::ErrorKind::NotFound
            {
                return Err(error);
            }
        }
        Ok(())
    }

    fn search_file(&mut self, root: &Path, path: &str) -> io::Result<()> {
        self.bytes.clear();
        open_local_file(root, &root.join(path))?
            .take(MAX_SEARCH_BYTES + 1)
            .read_to_end(&mut self.bytes)?;
        if self.bytes.len() as u64 > MAX_SEARCH_BYTES || self.bytes.contains(&0) {
            return Ok(());
        }
        let Ok(text) = std::str::from_utf8(&self.bytes) else {
            return Ok(());
        };
        for (index, line) in text.split('\n').enumerate() {
            if self.finished() {
                break;
            }
            let line = line.strip_suffix('\r').unwrap_or(line);
            // Electron's search decoder strips the BOM. Keep it in offsets because the editor
            // opens the same source with the BOM preserved.
            let start =
                usize::from(index == 0 && line.starts_with('\u{feff}')) * '\u{feff}'.len_utf8();
            let Some(found) = self.needle.find(&line[start..]) else {
                continue;
            };
            let excerpt = truncate_utf16(&line[start..]);
            let highlight = (found.start() < excerpt.len())
                .then(|| found.start()..found.end().min(excerpt.len()));
            self.results.push(SearchHit {
                file: path.to_owned(),
                line: index as u32 + 1,
                column: line[..start + found.start()].encode_utf16().count(),
                length: found.as_str().encode_utf16().count(),
                excerpt: excerpt.to_owned(),
                highlight,
            });
        }
        Ok(())
    }
}

fn truncate_utf16(text: &str) -> &str {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        units += ch.len_utf16();
        if units > MAX_EXCERPT_UNITS {
            return &text[..byte];
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_project;

    #[test]
    fn finds_literal_unicode_matches_in_nested_files() {
        let projects = tempfile::tempdir().unwrap();
        let project = create_project(projects.path(), "Paper").unwrap();
        let folder = projects.path().join(&project.id);
        fs::create_dir(folder.join("sections")).unwrap();
        fs::write(
            folder.join("sections/paper.tex"),
            "\u{feff}İ A.b axb\r\n𐐀 FINDING finding\n",
        )
        .unwrap();
        fs::write(folder.join("binary.bin"), b"finding\0").unwrap();
        fs::write(folder.join("invalid.txt"), b"finding\xff").unwrap();
        fs::write(folder.join(".hidden"), "finding").unwrap();
        let cancelled = AtomicBool::new(false);

        let matches = search_project(projects.path(), &project, "a.b", &cancelled).unwrap();
        assert_eq!(matches.len(), 1);
        let hit = &matches[0];
        assert_eq!(
            (hit.file.as_str(), hit.line, hit.column, hit.length),
            ("sections/paper.tex", 1, 3, 3)
        );
        assert_eq!(&hit.excerpt[hit.highlight.clone().unwrap()], "A.b");
        let matches = search_project(projects.path(), &project, "finding", &cancelled).unwrap();
        assert_eq!(matches.len(), 1);
        let hit = &matches[0];
        assert_eq!((hit.line, hit.column, hit.length), (2, 3, 7));
        assert_eq!(&hit.excerpt[hit.highlight.clone().unwrap()], "FINDING");
    }

    #[test]
    fn limits_results_and_skips_oversized_files() {
        let projects = tempfile::tempdir().unwrap();
        let project = create_project(projects.path(), "Paper").unwrap();
        let folder = projects.path().join(&project.id);
        let long_line = format!("{}🚀 needle", "x".repeat(MAX_EXCERPT_UNITS - 1));
        fs::write(
            folder.join("main.tex"),
            format!("{long_line}\n{}", "needle\n".repeat(MAX_RESULTS)),
        )
        .unwrap();
        fs::write(
            folder.join("oversized.tex"),
            "oversized needle\n".repeat(125_000),
        )
        .unwrap();
        let cancelled = AtomicBool::new(false);

        let hits = search_project(projects.path(), &project, "needle", &cancelled).unwrap();
        assert_eq!(hits.len(), MAX_RESULTS);
        assert!(hits.iter().all(|hit| hit.file == "main.tex"));
        assert_eq!(hits[0].column, MAX_EXCERPT_UNITS + 2);
        assert_eq!(hits[0].excerpt.len(), MAX_EXCERPT_UNITS - 1);
        assert_eq!(hits[0].highlight, None);
        assert!(
            search_project(projects.path(), &project, "oversized needle", &cancelled)
                .unwrap()
                .is_empty()
        );
        cancelled.store(true, Ordering::Relaxed);
        assert!(
            search_project(projects.path(), &project, "needle", &cancelled)
                .unwrap()
                .is_empty()
        );
    }
}

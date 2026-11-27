use crate::{Project, project_entries::checked_project_directory, project_management::is_link};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::HashMap,
    fs::{self, File},
    io::{self, Read, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering as AtomicOrdering},
    time::SystemTime,
};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectSort {
    #[default]
    Newest,
    Oldest,
    Name,
    Edited,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct LibraryPreferences {
    pub sort: ProjectSort,
    pub grid: bool,
}

impl LibraryPreferences {
    pub fn read(path: &Path) -> Self {
        File::open(path)
            .ok()
            .and_then(|file| serde_json::from_reader(file.take(4096)).ok())
            .unwrap_or_default()
    }

    pub fn save(self, path: &Path) -> io::Result<()> {
        let root = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(root)?;
        let mut stage = tempfile::NamedTempFile::new_in(root)?;
        serde_json::to_writer(&mut stage, &self)?;
        stage.flush()?;
        stage.persist(path).map(|_| ()).map_err(|error| error.error)
    }
}

pub fn matches_query(project: &Project, query: &str) -> bool {
    query.is_empty() || project.name.to_lowercase().contains(query)
}

pub fn visible_projects<'a>(
    projects: &'a [Project],
    query: &str,
    sort: ProjectSort,
    edited: &HashMap<String, SystemTime>,
    compare_names: impl Fn(&str, &str) -> Ordering,
) -> Vec<&'a Project> {
    let mut matching: Vec<_> = projects
        .iter()
        .filter(|project| matches_query(project, query))
        .map(|project| {
            let created = project
                .created
                .as_deref()
                .and_then(|date| DateTime::parse_from_rfc3339(date).ok())
                .map(|date| date.with_timezone(&Utc));
            let edited = edited.get(&project.id).copied().map(DateTime::<Utc>::from);
            (project, created, edited.or(created))
        })
        .collect();
    matching.sort_by(|(a, a_created, a_edited), (b, b_created, b_edited)| {
        let names = || compare_names(&a.name, &b.name);
        match sort {
            ProjectSort::Newest => b_created.cmp(a_created).then_with(names),
            ProjectSort::Oldest => a_created.cmp(b_created).then_with(names),
            ProjectSort::Name => names().then_with(|| b_created.cmp(a_created)),
            ProjectSort::Edited => b_edited.cmp(a_edited).then_with(names),
        }
        .then_with(|| a.id.cmp(&b.id))
    });
    matching
        .into_iter()
        .map(|(project, _, _)| project)
        .collect()
}

pub fn latest_edits(
    projects_dir: &Path,
    projects: &[Project],
    cancelled: &AtomicBool,
) -> HashMap<String, SystemTime> {
    let mut dates = HashMap::new();
    for project in projects {
        if cancelled.load(AtomicOrdering::Relaxed) {
            break;
        }
        if let Ok(root) = checked_project_directory(projects_dir, project)
            && let Some(date) = last_file_edit(&root, cancelled)
        {
            dates.insert(project.id.clone(), date);
        }
    }
    dates
}

fn last_file_edit(folder: &Path, cancelled: &AtomicBool) -> Option<SystemTime> {
    let mut pending = vec![fs::read_dir(folder).ok()?];
    let mut latest = None;
    while let Some(directory) = pending.last_mut() {
        if cancelled.load(AtomicOrdering::Relaxed) {
            return None;
        }
        let Some(entry) = directory.next() else {
            pending.pop();
            continue;
        };
        let Ok(entry) = entry else { continue };
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if let Ok(info) = entry.path().symlink_metadata()
            && !is_link(&info)
        {
            if info.is_dir() {
                if let Ok(children) = fs::read_dir(entry.path()) {
                    pending.push(children);
                }
            } else if info.is_file() {
                latest = latest.max(info.modified().ok());
            }
        }
    }
    latest
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_project;
    use std::time::Duration;

    #[test]
    fn sorts_by_all_reference_choices_and_uses_visible_file_edit_times() {
        let home = tempfile::tempdir().unwrap();
        let older = create_project(home.path(), "Zebra").unwrap();
        let newer = create_project(home.path(), "Atlas").unwrap();
        let duplicate = create_project(home.path(), "Atlas").unwrap();
        let mut projects = [older, newer, duplicate];
        let epoch = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        for (index, project) in projects.iter_mut().enumerate() {
            project.created =
                Some(DateTime::<Utc>::from(epoch + Duration::from_secs(index as u64)).to_rfc3339());
            let modified = epoch + Duration::from_secs(u64::from(index == 1));
            File::options()
                .write(true)
                .open(home.path().join(&project.id).join("main.tex"))
                .unwrap()
                .set_modified(modified)
                .unwrap();
        }
        let folder = home.path().join(&projects[0].id).join("sections");
        fs::create_dir(&folder).unwrap();
        let edited = epoch + Duration::from_secs(120);
        File::create(folder.join("child.tex"))
            .unwrap()
            .set_modified(edited)
            .unwrap();
        // Newer compiler output and project metadata do not affect the sort.
        let build = home.path().join(&projects[1].id).join(".refract-build");
        fs::create_dir(&build).unwrap();
        File::create(build.join("main.pdf"))
            .unwrap()
            .set_modified(edited + Duration::from_secs(120))
            .unwrap();
        let times = latest_edits(home.path(), &projects, &AtomicBool::new(false));
        assert_eq!(times[&projects[0].id], edited);
        assert_eq!(times[&projects[1].id], epoch + Duration::from_secs(1));
        let ordered = |sort, query| {
            visible_projects(&projects, query, sort, &times, str::cmp)
                .into_iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>()
        };
        let [a, b, c] = projects.each_ref().map(|p| p.id.as_str());
        assert_eq!(ordered(ProjectSort::Newest, ""), [c, b, a]);
        assert_eq!(ordered(ProjectSort::Oldest, ""), [a, b, c]);
        assert_eq!(ordered(ProjectSort::Name, ""), [c, b, a]);
        assert_eq!(ordered(ProjectSort::Edited, ""), [a, b, c]);
        assert_eq!(ordered(ProjectSort::Name, "atlas"), [c, b]);
        assert!(latest_edits(home.path(), &projects, &AtomicBool::new(true)).is_empty());
    }

    #[test]
    fn restores_library_preferences_and_defaults_when_the_file_is_damaged() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("prefs.json");
        assert_eq!(
            LibraryPreferences::read(&path),
            LibraryPreferences::default()
        );
        let preferences = LibraryPreferences {
            sort: ProjectSort::Edited,
            grid: true,
        };
        preferences.save(&path).unwrap();
        assert_eq!(LibraryPreferences::read(&path), preferences);
        fs::write(&path, b"{").unwrap();
        assert_eq!(
            LibraryPreferences::read(&path),
            LibraryPreferences::default()
        );
    }
}

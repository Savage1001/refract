use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

/// Resolve the configuration directory used by Electron and early native builds.
/// Prefer the profile with projects and use Electron's spelling for ties.
pub fn unix_profile_dir(config_dir: &Path) -> PathBuf {
    let electron = config_dir.join("refract");
    let preview = config_dir.join("Refract");
    preferred_profile(&electron, &preview).to_path_buf()
}

fn preferred_profile<'a>(electron: &'a Path, preview: &'a Path) -> &'a Path {
    if contents(preview) > contents(electron) {
        preview
    } else {
        electron
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum ProfileContents {
    Missing,
    Directory,
    Preferences,
    LegacyProjects,
    Projects,
}

fn contents(profile: &Path) -> ProfileContents {
    let has_project_entries = match fs::read_dir(profile.join("projects")) {
        Ok(mut entries) => entries.next().is_some(),
        // Keep inaccessible data on the same path so startup can report the error.
        Err(error) => !matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory),
    };
    if has_project_entries {
        return ProfileContents::Projects;
    }
    if exists(&profile.join("projects.json")) {
        return ProfileContents::LegacyProjects;
    }
    let has_preferences = [
        "Local Storage/leveldb",
        "codex-account",
        "codex-preferences.json",
        "codex-account-preferences.json",
        "opencode-preferences.json",
        "update-settings.json",
        ".refract-native-library.json",
        ".refract-native-workspace.json",
        ".refract-native-outline-height",
    ]
    .iter()
    .any(|file| exists(&profile.join(file)));
    if has_preferences {
        ProfileContents::Preferences
    } else if exists(profile) {
        ProfileContents::Directory
    } else {
        ProfileContents::Missing
    }
}

fn exists(path: &Path) -> bool {
    !fs::metadata(path)
        .is_err_and(|error| matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_electrons_default_spelling_for_a_new_configuration() {
        let config = tempfile::tempdir().unwrap();
        assert_eq!(
            unix_profile_dir(config.path()),
            config.path().join("refract")
        );
    }

    #[test]
    fn keeps_populated_projects_when_the_other_profile_contains_only_preferences() {
        let config = tempfile::tempdir().unwrap();
        let electron = config.path().join("electron");
        let preview = config.path().join("preview");
        fs::create_dir_all(electron.join("Local Storage/leveldb")).unwrap();
        fs::create_dir_all(preview.join("projects/.refract-project-trash")).unwrap();
        assert_eq!(preferred_profile(&electron, &preview), preview);

        fs::create_dir(electron.join("projects")).unwrap();
        assert_eq!(preferred_profile(&electron, &preview), preview);
    }

    #[test]
    fn uses_electrons_profile_if_both_contain_projects() {
        let config = tempfile::tempdir().unwrap();
        let electron = config.path().join("electron");
        let preview = config.path().join("preview");
        fs::create_dir_all(electron.join("projects/fixture")).unwrap();
        fs::create_dir_all(preview.join("projects/fixture")).unwrap();
        assert_eq!(preferred_profile(&electron, &preview), electron);
        assert!(preview.join("projects/fixture").is_dir());
    }

    #[test]
    fn keeps_a_preview_profile_with_preferences_when_electrons_is_empty() {
        let config = tempfile::tempdir().unwrap();
        let electron = config.path().join("electron");
        let preview = config.path().join("preview");
        fs::create_dir(&electron).unwrap();
        fs::create_dir(&preview).unwrap();
        fs::write(preview.join("update-settings.json"), "{\"track\":\"beta\"}").unwrap();
        assert_eq!(preferred_profile(&electron, &preview), preview);
    }
}

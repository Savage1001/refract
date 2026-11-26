use crate::{METADATA_LIMIT, Project, project_directory, read_local_file, stage_file};
use chrono::{SecondsFormat, Utc};
use serde_json::{Map, Value, json};
use std::{
    fs::{self, File},
    io::{self, Write},
    path::Path,
};
use uuid::Uuid;

const DEFAULT_SOURCE: &str = r"\documentclass[11pt]{article}
\usepackage[margin=1in]{geometry}
\usepackage{amsmath,amssymb}

\title{Untitled}
\author{Author Name}
\date{\today}

\begin{document}
\maketitle

\section{Introduction}

\end{document}
";

pub fn create_project(projects_dir: &Path, name: &str) -> io::Result<Project> {
    let id = Uuid::new_v4().to_string();
    let name = name.trim();
    let fields = json!({
        "id": id,
        "name": if name.is_empty() { "New Project" } else { name },
        "created": Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
    });
    let (project, metadata) = encode_metadata(&fields)?;
    fs::create_dir_all(projects_dir)?;
    let root = projects_dir.canonicalize()?;
    let stage = tempfile::Builder::new()
        .prefix(".refract-create-")
        .tempdir_in(&root)?;
    for (name, data) in [
        ("main.tex", DEFAULT_SOURCE.as_bytes()),
        (".refract.json", metadata.as_slice()),
    ] {
        let mut file = File::create_new(stage.path().join(name))?;
        file.write_all(data)?;
        file.sync_all()?;
    }
    let target = root.join(&project.id);
    match target.symlink_metadata() {
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "Project identifier already exists",
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    fs::rename(stage.path(), target)?;
    let _ = stage.keep();
    Ok(project)
}

pub fn rename_project(projects_dir: &Path, project: &Project, name: &str) -> io::Result<Project> {
    let name = name.trim();
    if name.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Project name is required",
        ));
    }
    let folder = project_directory(projects_dir, project)?;
    let path = folder.join(".refract.json");
    let original = read_project_metadata(&folder, &path)?;
    let mut fields: Map<String, Value> = serde_json::from_slice(&original)?;
    if fields.get("id").and_then(Value::as_str) != Some(project.id.as_str()) {
        return Err(invalid_metadata());
    }
    fields.insert("name".into(), Value::String(name.into()));
    let (renamed, metadata) = encode_metadata(&fields)?;
    let staged = stage_file(&path, &metadata)?;
    if read_project_metadata(&folder, &path)? != original {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Project metadata changed during rename",
        ));
    }
    staged.persist(path).map_err(|error| error.error)?;
    Ok(renamed)
}

fn read_project_metadata(folder: &Path, path: &Path) -> io::Result<Vec<u8>> {
    if !path.symlink_metadata()?.is_file() {
        return Err(invalid_metadata());
    }
    read_local_file(folder, path, METADATA_LIMIT)
}

fn encode_metadata(fields: &impl serde::Serialize) -> io::Result<(Project, Vec<u8>)> {
    let bytes = serde_json::to_vec_pretty(fields)?;
    if bytes.len() > METADATA_LIMIT {
        return Err(invalid_metadata());
    }
    Ok((serde_json::from_slice(&bytes)?, bytes))
}

fn invalid_metadata() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Invalid project metadata")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{list_projects, read_source};

    #[test]
    fn creates_the_default_electron_project_in_a_new_profile() {
        let profile = tempfile::tempdir().unwrap();
        let root = profile.path().join("projects");
        let project = create_project(&root, "   ").unwrap();
        let folder = root.join(&project.id);
        let fields: Value =
            serde_json::from_slice(&fs::read(folder.join(".refract.json")).unwrap()).unwrap();
        assert_eq!(fields["id"], project.id);
        assert_eq!(fields["name"], "New Project");
        assert!(fields.get("rootFile").is_none());
        assert!(
            fields["created"]
                .as_str()
                .is_some_and(|date| date.ends_with('Z'))
        );
        assert_eq!(project.root_file(), "main.tex");
        assert_eq!(
            read_source(&root, &project, "main.tex").unwrap(),
            DEFAULT_SOURCE
        );
        assert_eq!(list_projects(&root).unwrap().len(), 1);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    }

    #[test]
    fn renames_only_the_name_in_existing_project_metadata() {
        let root = tempfile::tempdir().unwrap();
        let project = create_project(root.path(), "Paper").unwrap();
        let path = root.path().join(&project.id).join(".refract.json");
        let mut original: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        original["rootFile"] = json!("sections/paper.tex");
        original["engine"] = json!("xelatex");
        original["draftMode"] = json!(true);
        original["otherMetadata"] = json!({"preserve": ["a", "b"]});
        let bytes = serde_json::to_vec(&original).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(rename_project(root.path(), &project, " \t ").is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let renamed = rename_project(root.path(), &project, "  Spectral methods  ").unwrap();
        assert_eq!(renamed.name, "Spectral methods");
        assert_eq!(renamed.root_file(), "sections/paper.tex");
        original["name"] = json!("Spectral methods");
        let updated: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(updated, original);
    }
}

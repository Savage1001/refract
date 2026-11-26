use super::{
    ProjectEntryKind, checked_project_directory, invalid_destination,
    relocate::{check_entry, checked_target},
};
use crate::{
    Project, ProjectEntry, open_local_file, project_management::is_link, read_project_entries,
    valid_project_component,
};
use std::{fs::File, io, path::Path};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

pub fn download_project_file(
    projects_dir: &Path,
    project: &Project,
    source: &str,
    destination: &Path,
) -> io::Result<()> {
    let folder = checked_project_directory(projects_dir, project)?;
    let source = source.replace('\\', "/");
    let mut file = open_project_file(&folder, &source)?;
    write_transfer(destination, |output| {
        io::copy(&mut file, output).map(|_| ())
    })
}

pub fn export_project_zip(
    projects_dir: &Path,
    project: &Project,
    destination: &Path,
) -> io::Result<()> {
    let folder = checked_project_directory(projects_dir, project)?;
    let entries = read_project_entries(&folder, "")?;
    write_transfer(destination, |output| {
        let mut zip = ZipWriter::new(output);
        zip_entries(&folder, &entries, &mut zip)?;
        zip.finish().map_err(io::Error::other)?;
        Ok(())
    })
}

fn open_project_file(folder: &Path, name: &str) -> io::Result<File> {
    if !name.split('/').all(valid_project_component) {
        return Err(invalid_destination());
    }
    let path = checked_target(folder, name)?;
    check_entry(&path, ProjectEntryKind::File)?;
    open_local_file(folder, &path)
}

fn zip_entries(
    folder: &Path,
    entries: &[ProjectEntry],
    zip: &mut ZipWriter<&mut File>,
) -> io::Result<()> {
    for entry in entries {
        if let Some(children) = &entry.children {
            let path = checked_target(folder, &entry.path)?;
            check_entry(&path, ProjectEntryKind::Folder)?;
            zip.add_directory(format!("{}/", entry.path), SimpleFileOptions::default())
                .map_err(io::Error::other)?;
            zip_entries(folder, children, zip)?;
        } else {
            let mut file = open_project_file(folder, &entry.path)?;
            let options = SimpleFileOptions::default()
                .compression_method(CompressionMethod::Deflated)
                .compression_level(Some(3))
                .large_file(file.metadata()?.len() >= u64::from(u32::MAX));
            zip.start_file(&entry.path, options)
                .map_err(io::Error::other)?;
            io::copy(&mut file, zip)?;
        }
    }
    Ok(())
}

fn write_transfer(
    destination: &Path,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<()> {
    if destination.file_name().is_none() {
        return Err(invalid_destination());
    }
    let existing = match destination.symlink_metadata() {
        Ok(info) => {
            if !info.is_file() || is_link(&info) {
                return Err(invalid_destination());
            }
            if info.permissions().readonly() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "The destination is read-only.",
                ));
            }
            Some(info.permissions())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut output = tempfile::Builder::new()
        .prefix(".refract-transfer-")
        .tempfile_in(parent)?;
    write(output.as_file_mut())?;
    let replace = existing.is_some();
    if let Some(permissions) = existing {
        output.as_file().set_permissions(permissions)?;
    }
    output.as_file().sync_all()?;
    if replace {
        output.persist(destination).map_err(|error| error.error)?;
    } else {
        output
            .persist_noclobber(destination)
            .map_err(|error| error.error)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_project;
    use std::{fs, io::Read};
    use zip::ZipArchive;

    #[test]
    fn exports_visible_files_and_empty_directories_without_private_records() {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let project = create_project(&projects, "Paper").unwrap();
        let dir = projects.join(&project.id);
        fs::create_dir_all(dir.join("figures/empty")).unwrap();
        fs::write(dir.join("figures/logo.png"), [0, 255]).unwrap();
        fs::write(dir.join("figures/.private"), "Private note").unwrap();
        let output = profile.path().join("project.zip");
        export_project_zip(&projects, &project, &output).unwrap();
        let mut archive = ZipArchive::new(File::open(output).unwrap()).unwrap();
        assert_eq!(archive.len(), 4);
        assert!(archive.by_name("figures/").unwrap().is_dir());
        assert!(archive.by_name("figures/empty/").unwrap().is_dir());
        let mut bytes = Vec::new();
        archive
            .by_name("figures/logo.png")
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(bytes, [0, 255]);
        bytes.clear();
        archive
            .by_name("main.tex")
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(bytes, fs::read(dir.join("main.tex")).unwrap());
        assert!(archive.by_name(".refract.json").is_err());
        assert!(archive.by_name("figures/.private").is_err());
    }

    #[test]
    fn downloads_binary_files_and_leaves_the_destination_intact_on_failure() {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let project = create_project(&projects, "Paper").unwrap();
        let dir = projects.join(&project.id);
        fs::create_dir(dir.join("figures")).unwrap();
        let bytes = vec![0xEA; 256 * 1024];
        fs::write(dir.join("figures/photo.bin"), &bytes).unwrap();
        let output = profile.path().join("photo.bin");
        fs::write(&output, "Previous download").unwrap();
        for source in ["../main.tex", ".refract.json", "figures", "missing.tex"] {
            assert!(download_project_file(&projects, &project, source, &output).is_err());
        }
        assert_eq!(fs::read_to_string(&output).unwrap(), "Previous download");
        download_project_file(&projects, &project, r"figures\photo.bin", &output).unwrap();
        assert_eq!(fs::read(output).unwrap(), bytes);
    }
}

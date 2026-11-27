use super::private_directory;
use crate::{
    Project,
    conversation::Attachment,
    file_preview::ImageKind,
    is_project_id, open_local_file,
    project_entries::checked_project_directory,
    project_management::{has_one_link, is_link},
};
use std::{
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

pub const MAX_ATTACHMENTS: usize = 10;
const MAX_ATTACHMENT_BYTES: u64 = 20 * 1024 * 1024;
const ATTACHMENTS_DIRECTORY: &str = ".refract-attachments";
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

pub fn attach_conversation_files(
    projects_dir: &Path,
    project: &Project,
    sources: &[PathBuf],
    remaining: usize,
) -> io::Result<Vec<Attachment>> {
    if remaining > MAX_ATTACHMENTS || sources.len() > remaining {
        return Err(invalid(format!(
            "Select at most {} more attachments",
            remaining.min(MAX_ATTACHMENTS)
        )));
    }
    let root = checked_project_directory(projects_dir, project)?;
    if sources.is_empty() {
        return Ok(Vec::new());
    }
    let attachments = private_directory(&root, ATTACHMENTS_DIRECTORY, true)?;
    let mut staged = Vec::with_capacity(sources.len());
    for source in sources {
        let extension = source
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase)
            .filter(|value| {
                matches!(
                    value.as_str(),
                    "png"
                        | "jpg"
                        | "jpeg"
                        | "webp"
                        | "pdf"
                        | "txt"
                        | "md"
                        | "tex"
                        | "bib"
                        | "csv"
                        | "docx"
                )
            })
            .ok_or_else(|| invalid("Unsupported file format"))?;
        let name = source
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .ok_or_else(|| invalid("Only files can be attached"))?;
        if !source.metadata()?.is_file() {
            return Err(invalid("Only files can be attached"));
        }
        let mut input = File::open(source)?;
        let before = input.metadata()?;
        if !before.is_file() || before.len() > MAX_ATTACHMENT_BYTES {
            return Err(invalid("Attachment is too large or is not a regular file"));
        }
        let mut temporary = tempfile::NamedTempFile::new_in(&attachments)?;
        let size = io::copy(
            &mut Read::by_ref(&mut input).take(MAX_ATTACHMENT_BYTES + 1),
            temporary.as_file_mut(),
        )?;
        if size > MAX_ATTACHMENT_BYTES {
            return Err(invalid("Attachment is too large"));
        }
        let after = input.metadata()?;
        if size != before.len() || size != after.len() || before.modified()? != after.modified()? {
            return Err(invalid("File changed while being attached"));
        }
        staged.push((
            Attachment {
                token: new_token(&extension),
                name,
            },
            temporary,
        ));
    }

    let mut copied = Vec::<Attachment>::with_capacity(staged.len());
    for (item, temporary) in staged {
        let result = commit_attachment(&root, &item, temporary);
        if let Err(error) = result {
            let cleanup = private_directory(&root, ATTACHMENTS_DIRECTORY, false).and_then(|dir| {
                let mut error = Ok(());
                for item in &copied {
                    if let Err(failure) = fs::remove_file(dir.join(&item.token))
                        && failure.kind() != io::ErrorKind::NotFound
                    {
                        error = Err(failure);
                    }
                }
                error
            });
            return match cleanup {
                Ok(()) => Err(error),
                Err(cleanup) => Err(io::Error::other(format!(
                    "{error}. Some copied attachments could not be cleaned up: {cleanup}"
                ))),
            };
        }
        copied.push(item);
    }
    Ok(copied)
}

pub fn attach_conversation_image(
    projects_dir: &Path,
    project: &Project,
    png: &[u8],
) -> io::Result<Attachment> {
    if png.len() as u64 > MAX_ATTACHMENT_BYTES || !png.starts_with(PNG_SIGNATURE) {
        return Err(invalid("Pasted image is not a PNG under 20 MiB"));
    }
    let root = checked_project_directory(projects_dir, project)?;
    let dir = private_directory(&root, ATTACHMENTS_DIRECTORY, true)?;
    let mut temporary = tempfile::NamedTempFile::new_in(dir)?;
    temporary.write_all(png)?;
    let image = Attachment {
        token: new_token("png"),
        name: "Pasted screenshot.png".into(),
    };
    commit_attachment(&root, &image, temporary)?;
    Ok(image)
}

pub fn read_conversation_image(
    projects_dir: &Path,
    project: &Project,
    token: &str,
) -> io::Result<(ImageKind, Vec<u8>)> {
    let (id, extension) = token
        .split_once('.')
        .filter(|(id, _)| is_project_id(id))
        .ok_or_else(|| invalid("Invalid image attachment"))?;
    let kind = match extension {
        "png" => ImageKind::Png,
        "jpg" | "jpeg" => ImageKind::Jpeg,
        "webp" => ImageKind::WebP,
        _ => return Err(invalid("Invalid image attachment")),
    };
    let root = checked_project_directory(projects_dir, project)?;
    let dir = private_directory(&root, ATTACHMENTS_DIRECTORY, false)?;
    let path = dir.join(format!("{id}.{extension}"));
    let metadata = path.symlink_metadata()?;
    if !metadata.is_file() || is_link(&metadata) {
        return Err(invalid("Invalid image attachment"));
    }
    let file = open_local_file(&dir, &path)?;
    if !has_one_link(&file)? || file.metadata()?.len() > MAX_ATTACHMENT_BYTES {
        return Err(invalid("Invalid image attachment"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_ATTACHMENT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_ATTACHMENT_BYTES {
        return Err(invalid("Image is too large"));
    }
    Ok((kind, bytes))
}

fn commit_attachment(
    root: &Path,
    item: &Attachment,
    temporary: tempfile::NamedTempFile,
) -> io::Result<()> {
    let dir = private_directory(root, ATTACHMENTS_DIRECTORY, false)?;
    temporary
        .persist_noclobber(dir.join(&item.token))
        .map(|_| ())
        .map_err(|error| error.error)
}

fn new_token(extension: &str) -> String {
    format!("{}.{extension}", uuid::Uuid::new_v4())
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_project;

    #[test]
    fn stores_the_reference_attachment_names_and_preserves_existing_files() {
        let profile = tempfile::tempdir().unwrap();
        let root = profile.path();
        let project = create_project(root, "Paper").unwrap();
        let image = root.join("Figure.PNG");
        fs::write(&image, PNG_SIGNATURE).unwrap();
        let text = root.join("Notes.tex");
        fs::write(&text, "LaTeX source").unwrap();
        let copied = attach_conversation_files(root, &project, &[image, text.clone()], 2).unwrap();
        assert_eq!(copied[0].name, "Figure.PNG");
        assert!(copied[0].token.ends_with(".png"));
        let dir = root.join(&project.id).join(ATTACHMENTS_DIRECTORY);
        assert_eq!(
            fs::read(dir.join(&copied[1].token)).unwrap(),
            b"LaTeX source"
        );
        let (kind, bytes) = read_conversation_image(root, &project, &copied[0].token).unwrap();
        assert!(matches!(kind, ImageKind::Png));
        assert_eq!(bytes, PNG_SIGNATURE);
        let pasted = attach_conversation_image(root, &project, PNG_SIGNATURE).unwrap();
        assert_eq!(pasted.name, "Pasted screenshot.png");
        let unsupported = root.join("Example.exe");
        fs::write(&unsupported, b"Unsupported").unwrap();
        assert!(attach_conversation_files(root, &project, &[text, unsupported], 2).is_err());
        assert_eq!(dir.read_dir().unwrap().count(), 3);
        assert!(read_conversation_image(root, &project, "../Figure.PNG").is_err());
        fs::hard_link(dir.join(&pasted.token), root.join("linked.png")).unwrap();
        assert!(read_conversation_image(root, &project, &pasted.token).is_err());
    }
}

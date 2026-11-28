use crate::{Project, SOURCE_LIMIT, project_directory, read_local_file, source_path};
use std::{io, path::Path};

pub const NOT_TEXT: &str = "This file cannot be previewed as text.";
const TOO_LARGE: &str = "This file is too large to preview.";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageKind {
    Bmp,
    Gif,
    Jpeg,
    Png,
    Svg,
    WebP,
}

#[derive(Debug, Eq, PartialEq)]
pub enum Preview {
    Text(String),
    Image(ImageKind, Vec<u8>),
    Pdf(Vec<u8>),
    Unsupported(&'static str),
}

#[derive(PartialEq)]
enum Kind {
    Text,
    Image(ImageKind),
    Pdf,
    Binary,
}

pub fn inspect_file(projects_dir: &Path, project: &Project, name: &str) -> io::Result<Preview> {
    let root = project_directory(projects_dir, project)?;
    let file = source_path(&root, name)?;
    let bytes = match read_local_file(&root, &file, SOURCE_LIMIT) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            return Ok(Preview::Unsupported(TOO_LARGE));
        }
        Err(error) => return Err(error),
    };
    Ok(match kind(name, &bytes) {
        Kind::Text => {
            String::from_utf8(bytes).map_or(Preview::Unsupported(NOT_TEXT), Preview::Text)
        }
        Kind::Image(kind) => Preview::Image(kind, bytes),
        Kind::Pdf => Preview::Pdf(bytes),
        Kind::Binary => Preview::Unsupported(NOT_TEXT),
    })
}

pub fn is_text_asset(name: &str, bytes: &[u8]) -> bool {
    kind(name, bytes) == Kind::Text
}

fn kind(name: &str, bytes: &[u8]) -> Kind {
    let extension = Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if extension == "pdf" || bytes.starts_with(b"%PDF-") {
        return Kind::Pdf;
    }
    match extension.as_str() {
        "bmp" => Kind::Image(ImageKind::Bmp),
        "png" => Kind::Image(ImageKind::Png),
        "jpg" | "jpeg" => Kind::Image(ImageKind::Jpeg),
        "gif" => Kind::Image(ImageKind::Gif),
        "webp" => Kind::Image(ImageKind::WebP),
        "svg" => Kind::Image(ImageKind::Svg),
        "bin" | "sqlite" | "sqlite3" | "db" | "class" | "wasm" | "npy" | "pkl" | "rds" | "zip"
        | "gz" | "tar" | "7z" | "rar" | "exe" | "dll" | "so" | "dylib" | "woff" | "woff2"
        | "ttf" | "otf" | "eot" | "docx" | "xlsx" | "pptx" | "mp3" | "mp4" | "mov" | "avi"
        | "eps" => Kind::Binary,
        _ if bytes.starts_with(b"\x7fELF")
            || bytes.starts_with(b"MZ")
            || matches!(
                bytes,
                [b'P', b'K', 3, 4, ..] | [b'P', b'K', 5, 6, ..] | [b'P', b'K', 7, 8, ..]
            )
            || bytes
                .iter()
                .any(|&byte| matches!(byte, 0..=8 | 11 | 14..=31 | 127)) =>
        {
            Kind::Binary
        }
        _ => Kind::Text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_project;
    use std::fs::{self, File};

    #[test]
    fn opens_the_same_asset_types_as_the_desktop_file_store() {
        let projects = tempfile::tempdir().unwrap();
        let project = create_project(projects.path(), "Assets").unwrap();
        let root = projects.path().join(&project.id);
        for (path, value, expected) in [
            (
                "note.txt",
                "\u{feff}Plain UTF-8 ✓".as_bytes(),
                Preview::Text("\u{feff}Plain UTF-8 ✓".into()),
            ),
            (
                "bad.tex",
                b"\xff\x80".as_slice(),
                Preview::Unsupported(NOT_TEXT),
            ),
            ("binary.txt", b"A\0B", Preview::Unsupported(NOT_TEXT)),
            (
                "program.EXE",
                b"printable bytes",
                Preview::Unsupported(NOT_TEXT),
            ),
            (
                "empty-archive.txt",
                b"PK\x05\x06 printable",
                Preview::Unsupported(NOT_TEXT),
            ),
            (
                "paper.dat",
                b"%PDF-1.7\n",
                Preview::Pdf(b"%PDF-1.7\n".to_vec()),
            ),
            (
                "diagram.SVG",
                b"<svg/>",
                Preview::Image(ImageKind::Svg, b"<svg/>".to_vec()),
            ),
        ] {
            fs::write(root.join(path), value).unwrap();
            assert_eq!(
                inspect_file(projects.path(), &project, path).unwrap(),
                expected,
                "{path}"
            );
        }
        File::create(root.join("large.txt"))
            .unwrap()
            .set_len(SOURCE_LIMIT as u64 + 1)
            .unwrap();
        assert_eq!(
            inspect_file(projects.path(), &project, "large.txt").unwrap(),
            Preview::Unsupported(TOO_LARGE)
        );
        assert!(inspect_file(projects.path(), &project, "../escape").is_err());
    }
}

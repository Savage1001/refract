use super::{
    TexToolchain, build_file, invalid, lock_build_directory, process::query_process, recorder_path,
    tool_path, validate_input,
};
use crate::{Project, project_entries::checked_project_directory, valid_project_component};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use std::{
    collections::HashMap,
    ffi::OsString,
    fs::File,
    io::{self, BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::AtomicBool,
    time::Duration,
};

const NO_MATCH: &str = "No matching source or PDF position";
const MAX_SYNC_BYTES: u64 = 64 * 1024 * 1024;
const UNICODE_ALIAS: &str = "refract-sync-source.tex";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PdfPosition {
    pub page: u32,
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourcePosition {
    pub file: String,
    pub line: u32,
}

pub fn forward(
    projects_dir: &Path,
    project: &Project,
    toolchain: &TexToolchain,
    file: &str,
    line: u32,
    cancel: &AtomicBool,
) -> io::Result<PdfPosition> {
    let file = file.replace('\\', "/");
    if !file.split('/').all(valid_project_component)
        || file.contains(['\r', '\n'])
        || !(1..=10_000_000).contains(&line)
    {
        return Err(invalid("Invalid source position"));
    }
    let dir = checked_project_directory(projects_dir, project)?;
    validate_input(&dir, &file)?;
    let output = dir.join(".refract-build");
    let _lock = lock_build_directory(&output)?;
    let pdf = sync_output(&output)?;
    let selection = match forward_query(toolchain, &dir, &file, line, &pdf, cancel) {
        Err(error)
            if !file.is_ascii()
                && error.kind() == io::ErrorKind::NotFound
                && error.to_string() == NO_MATCH =>
        {
            unicode_forward(toolchain, &dir, &file, line, &pdf, cancel)?
        }
        result => result?,
    };
    pdf_position(&selection)
}

pub fn reverse(
    projects_dir: &Path,
    project: &Project,
    toolchain: &TexToolchain,
    position: PdfPosition,
    cancel: &AtomicBool,
) -> io::Result<SourcePosition> {
    if !(1..=1_000_000).contains(&position.page)
        || !(0. ..=1_000_000.).contains(&position.x)
        || !(0. ..=1_000_000.).contains(&position.y)
    {
        return Err(invalid("Invalid PDF position"));
    }
    let dir = checked_project_directory(projects_dir, project)?;
    let output = dir.join(".refract-build");
    let _lock = lock_build_directory(&output)?;
    let pdf = sync_output(&output)?;
    let mut argument = OsString::from(format!("{}:{}:{}:", position.page, position.x, position.y));
    argument.push(tool_path(&pdf));
    let fields = query(
        toolchain,
        &dir,
        ["edit".into(), "-o".into(), argument],
        cancel,
    )?;
    source_position(&dir, &fields)
}

fn sync_output(dir: &Path) -> io::Result<PathBuf> {
    let pdf = dir.join("main.pdf");
    for path in [&pdf, &dir.join("main.synctex.gz")] {
        build_file(path).map_err(|_| invalid("Invalid synchronization data"))?;
    }
    Ok(pdf)
}

fn forward_query(
    toolchain: &TexToolchain,
    dir: &Path,
    file: &str,
    line: u32,
    pdf: &Path,
    cancel: &AtomicBool,
) -> io::Result<HashMap<String, String>> {
    query(
        toolchain,
        dir,
        [
            "view".into(),
            "-i".into(),
            format!("{line}:0:{file}").into(),
            "-o".into(),
            tool_path(pdf).as_os_str().to_owned(),
        ],
        cancel,
    )
}

fn query(
    toolchain: &TexToolchain,
    dir: &Path,
    arguments: impl IntoIterator<Item = OsString>,
    cancel: &AtomicBool,
) -> io::Result<HashMap<String, String>> {
    let executable = toolchain.sync_executable();
    let mut command = Command::new(tool_path(&executable));
    command.current_dir(tool_path(dir)).args(arguments);
    let output = query_process(command, cancel, Duration::from_secs(15), 1024 * 1024)?;
    fields(&output)
}

fn fields(output: &[u8]) -> io::Result<HashMap<String, String>> {
    let mut found = false;
    let mut result = HashMap::new();
    for bytes in output.split(|byte| *byte == b'\n') {
        let text = decode_line(bytes.strip_suffix(b"\r").unwrap_or(bytes))?;
        if text == "SyncTeX result begin" {
            found = true;
        } else if found {
            if text == "SyncTeX result end" {
                return Ok(result);
            }
            if let Some((key, value)) = text.split_once(':')
                && !key.is_empty()
            {
                result
                    .entry(key.to_owned())
                    .or_insert_with(|| value.to_owned());
            }
        }
    }
    Err(io::Error::new(io::ErrorKind::NotFound, NO_MATCH))
}

fn pdf_position(fields: &HashMap<String, String>) -> io::Result<PdfPosition> {
    let page = fields
        .get("Page")
        .and_then(|page| page.trim().parse::<u32>().ok())
        .filter(|page| *page >= 1);
    let coordinate = |key| {
        fields
            .get(key)
            .and_then(|number| number.trim().parse::<f64>().ok())
            .filter(|number| number.is_finite())
    };
    match (page, coordinate("x"), coordinate("y")) {
        (Some(page), Some(x), Some(y)) => Ok(PdfPosition { page, x, y }),
        _ => Err(invalid("Invalid SyncTeX result")),
    }
}

fn source_position(dir: &Path, fields: &HashMap<String, String>) -> io::Result<SourcePosition> {
    let input = fields
        .get("Input")
        .filter(|name| !name.is_empty())
        .ok_or_else(|| invalid("Invalid SyncTeX result"))?;
    let line = fields
        .get("Line")
        .and_then(|line| line.trim().parse::<u32>().ok())
        .filter(|line| *line >= 1)
        .ok_or_else(|| invalid("Invalid SyncTeX result"))?;
    let source = recorder_path(dir, input);
    let root = tool_path(dir);
    let relative = if let Ok(relative) = source.strip_prefix(root) {
        relative.to_path_buf()
    } else if cfg!(windows)
        && root.components().count() < source.components().count()
        && root.components().zip(source.components()).all(|(a, b)| {
            a.as_os_str().to_string_lossy().to_lowercase()
                == b.as_os_str().to_string_lossy().to_lowercase()
        })
    {
        source
            .components()
            .skip(root.components().count())
            .collect()
    } else {
        return Err(invalid("SyncTeX source is outside the project"));
    };
    let file = relative
        .to_str()
        .ok_or_else(|| invalid("Invalid SyncTeX result"))?
        .replace('\\', "/");
    if !file.split('/').all(valid_project_component) || file.contains(['\r', '\n']) {
        return Err(invalid("SyncTeX source is outside the project"));
    }
    validate_input(dir, &file)?;
    Ok(SourcePosition { file, line })
}

fn unicode_forward(
    toolchain: &TexToolchain,
    dir: &Path,
    file: &str,
    line: u32,
    pdf: &Path,
    cancel: &AtomicBool,
) -> io::Result<HashMap<String, String>> {
    let source = build_file(&pdf.with_extension("synctex.gz"))?;
    if source.metadata()?.len() > MAX_SYNC_BYTES {
        return Err(invalid("SyncTeX data is too large"));
    }
    let scratch = tempfile::Builder::new()
        .prefix("refract-synctex-query-")
        .tempdir()?;
    let data = File::create_new(scratch.path().join("main.synctex.gz"))?;
    let mut compressed = GzEncoder::new(data, Compression::fast());
    let input = BufReader::new(GzDecoder::new(source).take(MAX_SYNC_BYTES + 1));
    rewrite_unicode_source(input, &mut compressed, dir, file)?;
    compressed.finish()?;
    let pdf_path = scratch.path().join("main.pdf");
    File::create_new(&pdf_path)?;
    let canonical_pdf = pdf_path.canonicalize()?;
    forward_query(toolchain, dir, UNICODE_ALIAS, line, &canonical_pdf, cancel)
}

fn rewrite_unicode_source(
    mut reader: impl BufRead,
    mut writer: impl Write,
    dir: &Path,
    file: &str,
) -> io::Result<()> {
    let wanted = recorder_path(dir, file).to_string_lossy().to_lowercase();
    let mut rewritten = false;
    let mut line = Vec::new();
    let mut total = 0u64;
    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line)?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > MAX_SYNC_BYTES {
            return Err(invalid("SyncTeX data is too large"));
        }
        if let Some(data) = line.strip_prefix(b"Input:")
            && let Some(separator) = data.iter().position(|byte| *byte == b':')
            && separator > 0
            && data[..separator].iter().all(u8::is_ascii_digit)
        {
            let path_start = 6 + separator + 1;
            let path_end = line
                .iter()
                .rposition(|byte| !matches!(byte, b'\r' | b'\n'))
                .map_or(path_start, |last| (last + 1).max(path_start));
            let input = decode_line(&line[path_start..path_end])?;
            if recorder_path(dir, &input).to_string_lossy().to_lowercase() == wanted {
                writer.write_all(&line[..path_start])?;
                writer.write_all(UNICODE_ALIAS.as_bytes())?;
                writer.write_all(&line[path_end..])?;
                rewritten = true;
                continue;
            }
        }
        writer.write_all(&line)?;
    }
    if rewritten {
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::NotFound, NO_MATCH))
    }
}

fn decode_line(bytes: &[u8]) -> io::Result<String> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text.to_owned()),
        #[cfg(windows)]
        Err(_) => decode_windows_ansi(bytes),
        #[cfg(not(windows))]
        Err(error) => Err(io::Error::new(io::ErrorKind::InvalidData, error)),
    }
}

#[cfg(windows)]
fn decode_windows_ansi(bytes: &[u8]) -> io::Result<String> {
    use std::ptr::null_mut;
    use windows_sys::Win32::Globalization::{CP_ACP, MB_ERR_INVALID_CHARS, MultiByteToWideChar};

    let length = i32::try_from(bytes.len()).map_err(|_| invalid("SyncTeX output is too large"))?;
    // Windows first measures the converted line, then writes to that many UTF-16 code units.
    let count = unsafe {
        MultiByteToWideChar(
            CP_ACP,
            MB_ERR_INVALID_CHARS,
            bytes.as_ptr(),
            length,
            null_mut(),
            0,
        )
    };
    if count == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut wide = vec![0; count as usize];
    if unsafe {
        MultiByteToWideChar(
            CP_ACP,
            MB_ERR_INVALID_CHARS,
            bytes.as_ptr(),
            length,
            wide.as_mut_ptr(),
            count,
        )
    } != count
    {
        return Err(io::Error::last_os_error());
    }
    String::from_utf16(&wide).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn takes_the_first_result_and_rejects_incomplete_output() {
        let output =
            b"SyncTeX result begin\r\nPage:2\r\nx:21.5\r\ny:34\r\nPage:3\r\nSyncTeX result end\r\n";
        assert_eq!(
            pdf_position(&fields(output).unwrap()).unwrap(),
            PdfPosition {
                page: 2,
                x: 21.5,
                y: 34.
            }
        );
        assert_eq!(
            fields(b"SyncTeX result begin\nPage:1\n")
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
        let bad = fields(b"SyncTeX result begin\nPage:1\nx:NaN\ny:32\nSyncTeX result end").unwrap();
        assert!(pdf_position(&bad).is_err());
    }

    #[test]
    fn resolves_unicode_source_paths_and_rejects_external_files() {
        let profile = tempfile::tempdir().unwrap();
        let dir = profile.path().join("project");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("résumé.tex"), "Paper").unwrap();
        fs::write(profile.path().join("outside.tex"), "Outside").unwrap();
        let local = fields(
            "SyncTeX result begin\nInput:./résumé.tex\nLine:2\nSyncTeX result end".as_bytes(),
        )
        .unwrap();
        assert_eq!(
            source_position(&dir.canonicalize().unwrap(), &local).unwrap(),
            SourcePosition {
                file: "résumé.tex".into(),
                line: 2
            }
        );
        let outside =
            fields(b"SyncTeX result begin\nInput:../outside.tex\nLine:1\nSyncTeX result end")
                .unwrap();
        assert!(source_position(&dir.canonicalize().unwrap(), &outside).is_err());
    }

    #[test]
    fn rewrites_only_the_matching_unicode_source_spelling() {
        let profile = tempfile::tempdir().unwrap();
        let input = "SyncTeX Version:1\r\nInput:1:./résumé.tex\r\nInput:2:./other.tex\nInput:3:résumé.tex\nContent:\n";
        let mut output = Vec::new();
        rewrite_unicode_source(input.as_bytes(), &mut output, profile.path(), "résumé.tex")
            .unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "SyncTeX Version:1\r\nInput:1:refract-sync-source.tex\r\nInput:2:./other.tex\nInput:3:refract-sync-source.tex\nContent:\n"
        );
    }
}

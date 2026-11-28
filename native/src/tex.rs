use crate::{
    Project,
    project_entries::{checked_project_directory, write_transfer},
    project_management::{has_one_link, is_link, read_project_metadata},
    valid_project_component,
};
use serde_json::Value;
use std::{
    env,
    ffi::{OsStr, OsString},
    fs::{self, File},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub mod diagnostics;
pub(crate) mod process;
pub mod synctex;
#[cfg(test)]
mod tests;
mod toolchain;

use process::{ProcessOutput, run_process};
pub use toolchain::{LatexEngine, TexToolchain};

const SYNC_FILES: [&str; 4] = [
    "main.synctex.gz",
    "main.synctex",
    "main.synctex(busy)",
    "main.synctex.gz(busy)",
];
const BUILD_LOG_LIMIT: usize = 250_000;
const THUMBNAIL_PDF_LIMIT: u64 = 64 * 1024 * 1024;
const IN_MEMORY_PDF_LIMIT: u64 = 128 * 1024 * 1024;

const DRAFT_WRAPPER: &str = r"\PassOptionsToPackage{draft}{graphics}
\PassOptionsToPackage{draft}{graphicx}
\newread\refractrootstream
\openin\refractrootstream=.refract-build/refract-internal/draft-root.txt
\begingroup\endlinechar=-1
\global\readline\refractrootstream to\refractroot
\endgroup
\closein\refractrootstream
\expandafter\input\expandafter{\refractroot}
";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildSettings {
    pub root_file: String,
    pub engine: LatexEngine,
    pub draft_mode: bool,
}

pub struct BuildResult {
    pub arguments: Vec<OsString>,
    pub exit_code: Option<i32>,
    pub log: String,
    pub pdf: Option<PathBuf>,
}

impl Project {
    pub fn latex_engine(&self) -> io::Result<LatexEngine> {
        match &self.engine {
            Value::String(engine) => LatexEngine::parse(engine),
            Value::Null | Value::Bool(false) => Ok(LatexEngine::PdfLatex),
            _ => Err(invalid("Unsupported LaTeX engine in project settings")),
        }
    }

    pub fn draft_mode(&self) -> bool {
        self.draft_mode == Value::Bool(true)
    }
}

pub fn build_settings(projects_dir: &Path, project: &Project) -> io::Result<BuildSettings> {
    let dir = checked_project_directory(projects_dir, project)?;
    read_build_settings(&dir, project)
}

pub fn compile_project(
    projects_dir: &Path,
    project: &Project,
    toolchain: &TexToolchain,
    cancel: &AtomicBool,
) -> io::Result<BuildResult> {
    compile_with_output(projects_dir, project, toolchain, cancel, |_| {})
}

pub fn compile_project_with_preview(
    projects_dir: &Path,
    project: &Project,
    toolchain: &TexToolchain,
    cancel: &AtomicBool,
) -> io::Result<(BuildResult, Option<io::Result<RetainedPdf>>)> {
    let mut preview = None;
    let build = compile_with_output(projects_dir, project, toolchain, cancel, |path| {
        preview = Some(retain_pdf_at(path, cancel));
    })?;
    Ok((build, preview))
}

fn compile_with_output(
    projects_dir: &Path,
    project: &Project,
    toolchain: &TexToolchain,
    cancel: &AtomicBool,
    capture: impl FnOnce(&Path),
) -> io::Result<BuildResult> {
    if cancel.load(Ordering::Relaxed) {
        return Ok(cancelled_build());
    }
    let dir = checked_project_directory(projects_dir, project)?;
    let settings = read_build_settings(&dir, project)?;
    build(&dir, &settings, toolchain, cancel, run_process, capture)
}

pub fn save_compiled_pdf(source: &Path, destination: &Path) -> io::Result<()> {
    let mut pdf = build_file(source)?;
    let mut header = [0; 5];
    if pdf.read_exact(&mut header).is_err() || &header != b"%PDF-" {
        return Err(invalid("Invalid PDF data"));
    }
    write_transfer(destination, |file| {
        file.write_all(&header)?;
        io::copy(&mut pdf, file)?;
        Ok(())
    })
}

#[derive(Debug)]
pub enum RetainedPdf {
    Bytes(Arc<Vec<u8>>),
    File(PreviewFile),
}

/// An app-owned snapshot of a PDF that is too large to keep in the heap.
#[derive(Debug)]
pub struct PreviewFile {
    #[cfg(windows)]
    path: tempfile::TempPath,
    #[cfg(not(windows))]
    file: File,
}

impl PreviewFile {
    #[cfg(windows)]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[cfg(all(
        feature = "portable-pdf",
        any(target_os = "linux", target_os = "macos")
    ))]
    pub(crate) fn map(&self) -> io::Result<memmap2::Mmap> {
        // The backing file is anonymous and this type gives out no writable handles.
        // Only snapshot_pdf writes to it, before any rendering worker can map it.
        unsafe { memmap2::Mmap::map(&self.file) }
    }
}

/// Snapshot this project's compiled output independently of the next build.
pub fn retain_compiled_pdf(
    projects_dir: &Path,
    project: &Project,
    cancel: &AtomicBool,
) -> io::Result<RetainedPdf> {
    let project_dir = checked_project_directory(projects_dir, project)?;
    let output = project_dir.join(".refract-build");
    let _lock = lock_build_directory(&output)?;
    retain_pdf_at(&output.join("main.pdf"), cancel)
}

fn retain_pdf_at(path: &Path, cancel: &AtomicBool) -> io::Result<RetainedPdf> {
    if path.canonicalize()? != path {
        return Err(invalid("Invalid compiled PDF path"));
    }
    snapshot_pdf(build_file(path)?, IN_MEMORY_PDF_LIMIT, cancel)
}

fn snapshot_pdf(mut file: File, memory_limit: u64, cancel: &AtomicBool) -> io::Result<RetainedPdf> {
    check_preview_cancel(cancel)?;
    let length = file.metadata()?.len();
    let mut header = [0; 5];
    if file.read_exact(&mut header).is_err() || &header != b"%PDF-" {
        return Err(invalid("Invalid PDF data"));
    }
    let remaining = length
        .checked_sub(header.len() as u64)
        .ok_or_else(|| invalid("Invalid PDF data"))?;
    let mut copy = |snapshot: &mut dyn Write| -> io::Result<()> {
        snapshot.write_all(&header)?;
        copy_pdf_bytes(&mut file, snapshot, remaining, cancel)?;
        if file.metadata()?.len() != length {
            return Err(invalid(
                "The compiled PDF changed before the preview was ready",
            ));
        }
        Ok(())
    };
    if length <= memory_limit {
        let mut bytes = Vec::with_capacity(length as usize);
        copy(&mut bytes)?;
        return Ok(RetainedPdf::Bytes(Arc::new(bytes)));
    }

    // Windows' PDF renderer reopens by name. Retain the cleanup guard for
    // the entire lifetime of the preview and any render work in flight.
    #[cfg(windows)]
    let snapshot = {
        let mut snapshot = tempfile::Builder::new()
            .prefix("refract-pdf-")
            .suffix(".pdf")
            .tempfile()?;
        copy(&mut snapshot)?;
        PreviewFile {
            path: snapshot.into_temp_path(),
        }
    };
    #[cfg(not(windows))]
    let snapshot = {
        let mut file = tempfile::tempfile()?;
        copy(&mut file)?;
        PreviewFile { file }
    };
    Ok(RetainedPdf::File(snapshot))
}

fn copy_pdf_bytes(
    source: &mut File,
    snapshot: &mut dyn Write,
    mut remaining: u64,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let mut buffer = [0; 64 * 1024];
    loop {
        check_preview_cancel(cancel)?;
        if remaining == 0 {
            return Ok(());
        }
        let chunk = remaining.min(buffer.len() as u64) as usize;
        source.read_exact(&mut buffer[..chunk])?;
        snapshot.write_all(&buffer[..chunk])?;
        remaining -= chunk as u64;
    }
}

fn check_preview_cancel(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "PDF preview was cancelled",
        ));
    }
    Ok(())
}

pub fn compiled_pdf_for_thumbnail(
    projects_dir: &Path,
    project: &Project,
) -> io::Result<Option<Vec<u8>>> {
    let directory = checked_project_directory(projects_dir, project)?.join(".refract-build");
    let info = match directory.symlink_metadata() {
        Ok(info) => info,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !info.is_dir() || is_link(&info) {
        return Err(invalid("Invalid compilation directory"));
    }
    let file = match build_file(&directory.join("main.pdf")) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if file.metadata()?.len() > THUMBNAIL_PDF_LIMIT {
        return Err(invalid(
            "The compiled PDF is too large for a library thumbnail",
        ));
    }
    let mut bytes = Vec::new();
    file.take(THUMBNAIL_PDF_LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > THUMBNAIL_PDF_LIMIT || !bytes.starts_with(b"%PDF-") {
        return Err(invalid("Invalid thumbnail PDF data"));
    }
    Ok(Some(bytes))
}

fn read_build_settings(dir: &Path, project: &Project) -> io::Result<BuildSettings> {
    let metadata = read_project_metadata(dir, &dir.join(".refract.json"))?;
    let current: Project = serde_json::from_slice(&metadata)?;
    if current.id != project.id {
        return Err(invalid("Project metadata changed"));
    }
    Ok(BuildSettings {
        root_file: current.root_file().replace('\\', "/"),
        engine: current.latex_engine()?,
        draft_mode: current.draft_mode(),
    })
}

fn build(
    dir: &Path,
    settings: &BuildSettings,
    toolchain: &TexToolchain,
    cancel: &AtomicBool,
    runner: impl FnMut(Command, &AtomicBool) -> io::Result<ProcessOutput>,
    capture: impl FnOnce(&Path),
) -> io::Result<BuildResult> {
    if cancel.load(Ordering::Relaxed) {
        return Ok(cancelled_build());
    }
    let root = validate_input(dir, &settings.root_file)?;
    let output = dir.join(".refract-build");
    match fs::create_dir(&output) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let _lock = lock_build_directory(&output)?;
    validate_build_directory(&output)?;
    if cancel.load(Ordering::Relaxed) {
        return Ok(cancelled_build());
    }
    remove_outputs(
        &output,
        SYNC_FILES.into_iter().chain([
            "main.pdf", "main.log", "main.aux", "main.bbl", "main.bcf", "main.fls",
        ]),
    )?;
    let result = execute_build(dir, &output, &root, settings, toolchain, cancel, runner);
    if let Some(path) = result.as_ref().ok().and_then(|build| build.pdf.as_deref()) {
        capture(path);
    } else {
        remove_outputs(&output, SYNC_FILES.into_iter().chain(["main.pdf"]))?;
    }
    result
}

fn execute_build(
    dir: &Path,
    output: &Path,
    root: &Path,
    settings: &BuildSettings,
    toolchain: &TexToolchain,
    cancel: &AtomicBool,
    runner: impl FnMut(Command, &AtomicBool) -> io::Result<ProcessOutput>,
) -> io::Result<BuildResult> {
    let input = if settings.draft_mode {
        write_draft_wrapper(output, root)?;
        "./.refract-build/refract-internal/draft.tex".to_owned()
    } else {
        format!("./{}", settings.root_file)
    };
    let mut arguments: Vec<OsString> = [
        "-interaction=nonstopmode",
        "-halt-on-error",
        "-file-line-error",
        "-no-shell-escape",
        "-recorder",
        "-synctex=1",
        "-jobname=main",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    let mut output_arg = OsString::from("-output-directory=");
    output_arg.push(tool_path(output));
    arguments.push(output_arg);
    arguments.push(input.into());
    let environment = BuildEnvironment::new(dir, output, toolchain, settings.engine)?;
    let mut build = BuildExecution {
        environment,
        toolchain,
        cancel,
        runner,
        result: BuildResult {
            arguments,
            exit_code: None,
            log: String::new(),
            pdf: None,
        },
    };
    if !build.latex(dir, settings.engine)? {
        return Ok(build.result);
    }
    let aux = inspect_aux(&output.join("main.aux"))?;
    let bcf = output.join("main.bcf").try_exists()?;
    if bcf {
        let out = tool_path(output).as_os_str();
        if !build.step(
            "biber",
            dir,
            [
                OsStr::new("--input-directory"),
                out,
                OsStr::new("--output-directory"),
                out,
                OsStr::new("main"),
            ],
        )? {
            return Ok(build.result);
        }
    } else if aux.bibliography && !build.step("bibtex", output, ["main"])? {
        return Ok(build.result);
    }
    let bibliography = bcf || aux.bibliography;
    let second_pass = bibliography
        || !first_pass_complete(dir, output, settings.engine, aux.plain, &build.result.log);
    if (second_pass && !build.latex(dir, settings.engine)?)
        || (bibliography && !build.latex(dir, settings.engine)?)
    {
        return Ok(build.result);
    }
    if cancel.load(Ordering::Relaxed) {
        build.result.exit_code = None;
        build.result.log.push_str("\nCompilation cancelled.");
        return Ok(build.result);
    }
    let pdf = output.join("main.pdf");
    let mut header = [0; 5];
    if build_file(&pdf)
        .and_then(|mut file| file.read_exact(&mut header))
        .is_ok()
        && &header == b"%PDF-"
    {
        build.result.pdf = Some(pdf);
    } else {
        build
            .result
            .log
            .insert_str(0, "Compilation finished without producing a PDF.\n");
    }
    Ok(build.result)
}

struct BuildExecution<'a, R> {
    environment: BuildEnvironment,
    toolchain: &'a TexToolchain,
    cancel: &'a AtomicBool,
    runner: R,
    result: BuildResult,
}

impl<R> BuildExecution<'_, R>
where
    R: FnMut(Command, &AtomicBool) -> io::Result<ProcessOutput>,
{
    fn latex(&mut self, dir: &Path, engine: LatexEngine) -> io::Result<bool> {
        let arguments = self.result.arguments.clone();
        self.step(engine.name(), dir, arguments)
    }

    fn step(
        &mut self,
        name: &str,
        dir: &Path,
        args: impl IntoIterator<Item = impl AsRef<OsStr>>,
    ) -> io::Result<bool> {
        let executable = self.toolchain.executable(name);
        let mut command = Command::new(tool_path(&executable));
        command.current_dir(tool_path(dir)).args(args);
        self.environment.apply(&mut command);
        let output = (self.runner)(command, self.cancel)?;
        self.result.exit_code = output.code;
        self.result.log.push('\n');
        self.result.log.push_str(&output.log);
        let mut trim = self.result.log.len().saturating_sub(BUILD_LOG_LIMIT);
        while !self.result.log.is_char_boundary(trim) {
            trim += 1;
        }
        self.result.log.drain(..trim);
        Ok(output.code == Some(0))
    }
}

struct BuildEnvironment {
    path: OsString,
    bibliography: OsString,
    font_config: Option<PathBuf>,
}

impl BuildEnvironment {
    fn new(
        dir: &Path,
        output: &Path,
        toolchain: &TexToolchain,
        engine: LatexEngine,
    ) -> io::Result<Self> {
        let separator = if cfg!(windows) { ";" } else { ":" };
        let mut path = tool_path(toolchain.bin()).as_os_str().to_owned();
        path.push(separator);
        path.push(env::var_os("PATH").unwrap_or_default());
        let mut bibliography = tool_path(dir).as_os_str().to_owned();
        bibliography.push(format!("{}//{separator}", std::path::MAIN_SEPARATOR));
        bibliography.push(env::var_os("BIBINPUTS").unwrap_or_default());
        Ok(Self {
            path,
            bibliography,
            font_config: font_config(output, toolchain, engine)?,
        })
    }

    fn apply(&self, command: &mut Command) {
        command
            .env("PATH", &self.path)
            .env("BIBINPUTS", &self.bibliography);
        if let Some(config) = &self.font_config {
            command.env("FONTCONFIG_FILE", tool_path(config));
        }
    }
}

#[cfg(windows)]
fn font_config(
    output: &Path,
    toolchain: &TexToolchain,
    engine: LatexEngine,
) -> io::Result<Option<PathBuf>> {
    if engine != LatexEngine::XeLatex {
        return Ok(None);
    }
    let Some(root) = toolchain.bin().parent().and_then(Path::parent) else {
        return Ok(None);
    };
    let fonts = root.join("texmf-dist/fonts");
    if !fonts.is_dir() {
        return Ok(None);
    }
    let cache = output.join("font-cache");
    fs::create_dir_all(&cache)?;
    let system_fonts =
        PathBuf::from(env::var_os("WINDIR").unwrap_or_else(|| r"C:\Windows".into())).join("Fonts");
    let convert = |path: &Path| {
        tool_path(path)
            .to_string_lossy()
            .replace('\\', "/")
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    let config = output.join("fontconfig.conf");
    fs::write(
        &config,
        format!(
            "<?xml version=\"1.0\"?><!DOCTYPE fontconfig SYSTEM \"fonts.dtd\"><fontconfig><dir>{}</dir><dir>{}</dir><dir>{}</dir><cachedir>{}</cachedir></fontconfig>",
            convert(&fonts.join("opentype")),
            convert(&fonts.join("truetype")),
            convert(&system_fonts),
            convert(&cache)
        ),
    )?;
    Ok(Some(config))
}

#[cfg(not(windows))]
fn font_config(_: &Path, _: &TexToolchain, _: LatexEngine) -> io::Result<Option<PathBuf>> {
    Ok(None)
}

fn tool_path(path: &Path) -> &Path {
    #[cfg(windows)]
    return dunce::simplified(path);
    #[cfg(not(windows))]
    path
}

fn validate_input(dir: &Path, relative: &str) -> io::Result<PathBuf> {
    if !relative.split('/').all(valid_project_component) {
        return Err(invalid("Invalid LaTeX root file"));
    }
    let mut target = dir.to_path_buf();
    let mut parts = relative.split('/').peekable();
    while let Some(part) = parts.next() {
        target.push(part);
        let info = target.symlink_metadata()?;
        let expected = if parts.peek().is_some() {
            info.is_dir()
        } else {
            info.is_file()
        };
        if !expected || is_link(&info) {
            return Err(invalid("Invalid LaTeX root file"));
        }
    }
    Ok(target)
}

fn validate_build_directory(dir: &Path) -> io::Result<()> {
    let info = dir.symlink_metadata()?;
    if !info.is_dir() || is_link(&info) {
        return Err(invalid("Invalid compilation directory"));
    }
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            let info = path.symlink_metadata()?;
            if is_link(&info) {
                return Err(invalid(
                    "Links are not allowed in the compilation directory",
                ));
            }
            if info.is_dir() {
                pending.push(path);
            } else {
                build_file(&path)?;
            }
        }
    }
    Ok(())
}

fn lock_build_directory(dir: &Path) -> io::Result<File> {
    let directory = dir.symlink_metadata()?;
    if !directory.is_dir() || is_link(&directory) {
        return Err(invalid("Invalid compilation directory"));
    }
    let path = dir.join(".refract-lock");
    let file = match File::options()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            build_file(&path)?;
            File::options().read(true).write(true).open(&path)?
        }
        Err(error) => return Err(error),
    };
    file.try_lock().map_err(|error| {
        let error = io::Error::from(error);
        if error.kind() == io::ErrorKind::WouldBlock {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                "A build is already running for this project",
            )
        } else {
            error
        }
    })?;
    Ok(file)
}

fn build_file(path: &Path) -> io::Result<File> {
    let info = path.symlink_metadata()?;
    if !info.is_file() || is_link(&info) {
        return Err(invalid(
            "Only unlinked regular files are allowed in the compilation directory",
        ));
    }
    let mut options = File::options();
    options.read(true);
    #[cfg(unix)]
    {
        use rustix::fs::OFlags;
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags((OFlags::NOFOLLOW | OFlags::NONBLOCK).bits() as i32);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let info = file.metadata()?;
    if !info.is_file() || is_link(&info) || !has_one_link(&file)? {
        return Err(invalid(
            "Only unlinked regular files are allowed in the compilation directory",
        ));
    }
    Ok(file)
}

fn write_draft_wrapper(output: &Path, root: &Path) -> io::Result<()> {
    let folder = output.join("refract-internal");
    fs::create_dir_all(&folder)?;
    let source = tool_path(root).to_string_lossy().replace('\\', "/");
    fs::write(folder.join("draft-root.txt"), format!("{source}\n"))?;
    fs::write(folder.join("draft.tex"), DRAFT_WRAPPER)
}

fn remove_outputs<'a>(dir: &Path, names: impl IntoIterator<Item = &'a str>) -> io::Result<()> {
    for name in names {
        match fs::remove_file(dir.join(name)) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

struct AuxInfo {
    plain: bool,
    bibliography: bool,
}

fn inspect_aux(path: &Path) -> io::Result<AuxInfo> {
    let mut file = match build_file(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(AuxInfo {
                plain: false,
                bibliography: false,
            });
        }
        Err(error) => return Err(error),
    };
    const NEEDLE: &[u8] = br"\bibdata{";
    let mut sample = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut carry = 0;
    let mut bibliography = false;
    loop {
        let read = file.read(&mut buffer[carry..])?;
        if read == 0 {
            break;
        }
        if sample.len() <= 256 {
            sample.extend_from_slice(&buffer[carry..carry + read]);
        }
        let end = carry + read;
        if buffer[..end]
            .windows(NEEDLE.len())
            .any(|part| part == NEEDLE)
        {
            bibliography = true;
            break;
        }
        carry = end.min(NEEDLE.len() - 1);
        buffer.copy_within(end - carry..end, 0);
    }
    let mut lines = std::str::from_utf8(&sample).unwrap_or_default().lines();
    let first = lines
        .next()
        .is_some_and(|line| line.trim_end_matches([' ', '\t']) == r"\relax");
    let last = lines.next().and_then(|line| {
        line.strip_prefix(r"\gdef \@abspage@last{")?
            .strip_suffix('}')
    });
    let plain = sample.len() <= 256
        && first
        && last.is_some_and(|pages| {
            matches!(pages.as_bytes().first(), Some(b'1'..=b'9'))
                && pages.bytes().all(|byte| byte.is_ascii_digit())
        })
        && lines.next().is_none();
    Ok(AuxInfo {
        plain,
        bibliography,
    })
}

fn first_pass_complete(
    dir: &Path,
    output: &Path,
    engine: LatexEngine,
    plain_aux: bool,
    log: &str,
) -> bool {
    if !plain_aux || needs_rerun(log) {
        return false;
    }
    let Ok(file) = build_file(&output.join("main.fls")) else {
        return false;
    };
    const RECORDER_LIMIT: u64 = 8 * 1024 * 1024;
    let mut text = String::new();
    if file
        .take(RECORDER_LIMIT + 1)
        .read_to_string(&mut text)
        .is_err()
        || text.len() as u64 > RECORDER_LIMIT
    {
        return false;
    }
    let output = tool_path(output);
    let aux = output.join("main.aux");
    let standard = ["main.aux", "main.log", "main.pdf", "main.xdv"].map(|name| output.join(name));
    let mut wrote_aux = false;
    for line in text.lines() {
        if let Some(input) = line.strip_prefix("INPUT ") {
            let path = recorder_path(dir, input);
            if path
                .file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("main.aux"))
                && path != aux
            {
                return false;
            }
        }
        if let Some(file) = line.strip_prefix("OUTPUT ") {
            let path = recorder_path(dir, file);
            if path == aux {
                wrote_aux = true;
            }
            if standard.contains(&path) {
                continue;
            }
            let discarded_font_probe = engine == LatexEngine::LuaLatex
                && path
                    .file_name()
                    .is_some_and(|name| name == "m_t_x_t_e_s_t.tmp")
                && !path.starts_with(tool_path(dir))
                && path
                    .symlink_metadata()
                    .is_err_and(|error| error.kind() == io::ErrorKind::NotFound);
            if !discarded_font_probe {
                return false;
            }
        }
    }
    wrote_aux
}

fn recorder_path(dir: &Path, name: &str) -> PathBuf {
    let file = tool_path(dir).join(name);
    let mut normalized = PathBuf::new();
    for part in tool_path(&file).components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            part => normalized.push(part.as_os_str()),
        }
    }
    normalized
}

fn needs_rerun(log: &str) -> bool {
    let log = log.to_ascii_lowercase();
    log.lines()
        .any(|line| line.starts_with("no file ") && line != "no file main.aux.")
        || [
            "run latex again",
            "run tex again",
            "undefined references",
            "undefined citations",
        ]
        .iter()
        .any(|phrase| log.contains(phrase))
        || log
            .split(|unit: char| !unit.is_ascii_alphanumeric() && unit != '_' && unit != '-')
            .any(|word| matches!(word, "rerun" | "re-run"))
}

fn cancelled_build() -> BuildResult {
    BuildResult {
        arguments: Vec::new(),
        exit_code: None,
        log: "Compilation cancelled.".into(),
        pdf: None,
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

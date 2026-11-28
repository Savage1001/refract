use crate::{
    Project, ProjectEntry, ProjectSetting,
    conversation::{ConversationId, Message, load_conversation},
    open_local_file,
    project_entries::checked_project_directory,
    project_management::has_one_link,
    project_tree, save_source, source_path,
    tex::{LatexEngine, TexToolchain, build_settings, compile_project, process::run_process},
    update_project_setting,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    env,
    io::{self, Read},
    path::PathBuf,
    process::Command,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicBool, Ordering},
    },
};

mod server;
pub use server::AssistantToolServer;

const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_FILES: usize = 10_000;
const MAX_BATCH: usize = 8;
const SELECT_ENGINE: &str = "Choose the fastest engine that meets the user, template, package, font, language and output requirements. Check compatibility before changing engines. For a routine build, keep a compatible current engine. After switching, build and check for compatibility warnings. If the new engine is unsuitable, restore the previous engine when available.";

pub fn claims_successful_build(text: &str) -> bool {
    static CLAIM: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r"(?i)\b(?:build|compilation) (?:succeeded|completed successfully)\b|\b(?:successfully (?:built|compiled)|(?:built|compiled) successfully)\b|\bI (?:built|compiled) (?:the (?:paper|document|project|PDF)|it|with|using)\b|\bI\b[^.!?\n]{0,200}\band (?:built|compiled) (?:(?:it|with|using)\b|the (?:paper|document|project|PDF)\b)",
        ).expect("valid build claim pattern")
    });
    CLAIM.is_match(text)
}

pub struct AssistantTools {
    projects: PathBuf,
    project: Project,
    chat: ConversationId,
    cancel: Arc<AtomicBool>,
    successful_build: Arc<AtomicBool>,
}

impl AssistantTools {
    pub fn new(
        projects: PathBuf,
        project: Project,
        chat: ConversationId,
        cancel: Arc<AtomicBool>,
    ) -> Self {
        Self {
            projects,
            project,
            chat,
            cancel,
            successful_build: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn build_status(&self) -> Arc<AtomicBool> {
        self.successful_build.clone()
    }

    pub fn engine_instructions(&self) -> String {
        let engine = self
            .project
            .latex_engine()
            .ok()
            .map(|engine| format!("The project starts this turn using {}. ", engine.name()))
            .unwrap_or_default();
        format!(
            "{engine}For requested compilation, call the Refract build tool and check its result before replying. Use project_set_engine only to change the saved engine. {SELECT_ENGINE}"
        )
    }

    pub fn instructions(&self) -> String {
        format!(
            "Prefer Refract tools. Use project_edit directly when the file and exact old text are known. Otherwise read targeted context with project_read, then edit with its SHA-256. Batch edits with project_edit_many or add disjoint replacements to one file. Reread after a conflict. Use conversation_read for omitted history. Batch edits before building and check warnings. Only claim visual inspection if performed. Preserve document content and shared TinyTeX. {engine}",
            engine = self.engine_instructions()
        )
    }

    pub fn definitions() -> Vec<Value> {
        let path = json!({"type":"string","minLength":1,"maxLength":1024});
        let edit = json!({
            "type":"object",
            "required":["path","oldText","newText"],
            "properties":{
                "path":path,
                "sha256":{"type":"string","pattern":"^[0-9a-f]{64}$"},
                "oldText":{"type":"string","minLength":1,"maxLength":MAX_BYTES},
                "newText":{"type":"string","maxLength":MAX_BYTES},
                "additional":{"type":"array","maxItems":7,"items":{
                    "type":"object","required":["oldText","newText"],"properties":{
                        "oldText":{"type":"string","minLength":1,"maxLength":MAX_BYTES},
                        "newText":{"type":"string","maxLength":MAX_BYTES}
                    }
                }}
            }
        });
        vec![
            definition(
                "project_read",
                "Read project text and its SHA-256. Use find for an exact match. Follow nextOffset for more text.",
                json!({
                    "type":"object","required":["path"],"properties":{
                        "path":path,"offset":{"type":"integer","minimum":0,"maximum":MAX_BYTES},
                        "limit":{"type":"integer","minimum":1,"maximum":65536},"find":{"type":"string","minLength":1,"maxLength":256}
                    }
                }),
            ),
            definition(
                "project_read_many",
                "Read up to eight files. Errors are reported per file.",
                json!({
                    "type":"object","required":["paths"],"properties":{
                        "paths":{"type":"array","minItems":1,"maxItems":MAX_BATCH,"items":path},
                        "limit":{"type":"integer","minimum":1,"maximum":8000}
                    }
                }),
            ),
            definition(
                "project_list",
                "List the visible files in the active project.",
                json!({"type":"object"}),
            ),
            definition(
                "project_edit",
                "Replace unique exact text and verify the save. Use SHA-256 after reading a file. Supply disjoint additional replacements together.",
                edit.clone(),
            ),
            definition(
                "project_edit_many",
                "Edit up to eight files independently. Successful edits persist if another file fails.",
                json!({
                    "type":"object","required":["edits"],"properties":{"edits":{"type":"array","minItems":1,"maxItems":MAX_BATCH,"items":edit}}
                }),
            ),
            definition(
                "conversation_read",
                "Read an earlier page of the active conversation.",
                json!({
                    "type":"object","properties":{"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":16000}}
                }),
            ),
            definition(
                "build",
                "Compile the active project with its current engine and root. The log is bounded.",
                json!({
                    "type":"object","properties":{"force":{"type":"boolean"},"logLimit":{"type":"integer","minimum":1000,"maximum":250000}}
                }),
            ),
            definition(
                "project_set_engine",
                SELECT_ENGINE,
                json!({
                    "type":"object","required":["engine"],"additionalProperties":false,
                    "properties":{"engine":{"enum":["pdflatex","xelatex","lualatex"]}}
                }),
            ),
            definition(
                "package_lookup",
                "Check availability of a package in the configured TeX environment.",
                json!({
                    "type":"object","required":["name"],"properties":{"name":{"type":"string","pattern":"^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}\\.sty$"}}
                }),
            ),
        ]
    }

    pub fn call(&mut self, name: &str, arguments: Value) -> io::Result<Value> {
        self.check_cancelled()?;
        match name {
            "project_read" => {
                let read: ReadRequest = serde_json::from_value(arguments)?;
                let file = self.read(&read.path)?;
                let offset = read.offset.unwrap_or(0);
                let limit = bounded(read.limit.unwrap_or(16000), 1, 65536, "Invalid read limit")?;
                bounded(offset, 0, MAX_BYTES, "Invalid read offset")?;
                if let Some(needle) = read.find {
                    bounded(needle.encode_utf16().count(), 1, 256, "Invalid search text")?;
                    return Ok(file.find(&needle, offset, limit));
                }
                Ok(file.excerpt(offset, limit))
            }
            "project_read_many" => {
                let read: ReadMany = serde_json::from_value(arguments)?;
                bounded(read.paths.len(), 1, MAX_BATCH, "Select up to eight files")?;
                let limit = bounded(read.limit.unwrap_or(8000), 1, 8000, "Invalid read limit")?;
                Ok(Value::Array(
                    read.paths
                        .iter()
                        .map(|path| match self.read(path) {
                            Ok(file) => file.excerpt(0, limit),
                            Err(error) => json!({"path":path,"error": error.to_string()}),
                        })
                        .collect(),
                ))
            }
            "project_list" => {
                let mut files = Vec::new();
                let tree = project_tree(&self.projects, &self.project)?;
                list_files(&tree, &mut files);
                let truncated = files.len() > MAX_FILES;
                files.truncate(MAX_FILES);
                let mut result = json!({"files":files});
                if truncated {
                    result["truncated"] = json!(true);
                }
                Ok(result)
            }
            "project_edit" => self.edit(serde_json::from_value(arguments)?),
            "project_edit_many" => {
                let request: EditMany = serde_json::from_value(arguments)?;
                bounded(
                    request.edits.len(),
                    1,
                    MAX_BATCH,
                    "Select up to eight edits",
                )?;
                Ok(Value::Array(request.edits.into_iter().map(|edit| {
                    let path = edit.path.clone();
                    self.edit(edit).unwrap_or_else(|error| json!({"path":path,"applied":false,"error":error.to_string()}))
                }).collect()))
            }
            "conversation_read" => {
                let range: ConversationRead = serde_json::from_value(arguments)?;
                let limit = bounded(
                    range.limit.unwrap_or(8000),
                    1,
                    16000,
                    "Invalid conversation read limit",
                )?;
                let history = load_conversation(&self.projects, &self.project, &self.chat)?;
                let mut lines = Vec::new();
                for message in history.messages {
                    match message {
                        Message::User { text, attachments } => {
                            let mut entry = format!("user: {text}");
                            if let Some(files) = attachments.filter(|files| !files.is_empty()) {
                                let files = files
                                    .iter()
                                    .map(|file| {
                                        format!(
                                            ".refract-attachments/{} ({})",
                                            file.token, file.name
                                        )
                                    })
                                    .collect::<Vec<_>>();
                                entry.push_str("\nAttachments: ");
                                entry.push_str(&files.join(", "));
                            }
                            lines.push(entry);
                        }
                        Message::Assistant { text } => lines.push(format!("assistant: {text}")),
                        _ => {}
                    }
                }
                Ok(excerpt(
                    &lines.join("\n\n"),
                    range.offset.unwrap_or(0),
                    limit,
                ))
            }
            "build" => {
                let request: BuildRequest = serde_json::from_value(arguments)?;
                let limit = bounded(
                    request.log_limit.unwrap_or(12000),
                    1000,
                    250000,
                    "Invalid build log limit",
                )?;
                self.build(limit, request.force.unwrap_or_default())
            }
            "project_set_engine" => {
                let request: EngineRequest = serde_json::from_value(arguments)?;
                self.set_engine(request.engine)
            }
            "package_lookup" => {
                let request: PackageRequest = serde_json::from_value(arguments)?;
                self.package(&request.name)
            }
            _ => Err(invalid("Unknown Refract tool")),
        }
    }

    fn check_cancelled(&self) -> io::Result<()> {
        if self.cancel.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Assistant operation cancelled",
            ));
        }
        Ok(())
    }

    fn read(&self, name: &str) -> io::Result<SourceFile> {
        bounded(
            name.encode_utf16().count(),
            1,
            1024,
            "Invalid project file path",
        )?;
        let root = checked_project_directory(&self.projects, &self.project)?;
        let path = source_path(&root, name)?;
        let file = open_local_file(&root, &path)?;
        let before = file.metadata()?;
        if !has_one_link(&file)? || before.len() > MAX_BYTES as u64 {
            return Err(invalid("Expected a regular text file no larger than 2 MiB"));
        }
        let mut bytes = Vec::new();
        (&file)
            .take((MAX_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        let after = file.metadata()?;
        if bytes.len() as u64 != before.len()
            || before.len() != after.len()
            || before.modified()? != after.modified()?
        {
            return Err(invalid("File changed while reading; retry"));
        }
        if bytes.contains(&0) {
            return Err(invalid("Binary files cannot be edited"));
        }
        let content =
            String::from_utf8(bytes).map_err(|_| invalid("Expected a UTF-8 text file"))?;
        Ok(SourceFile {
            path: name.to_owned(),
            content,
        })
    }

    fn edit(&mut self, edit: EditRequest) -> io::Result<Value> {
        self.check_cancelled()?;
        let file = self.read(&edit.path)?;
        if let Some(hash) = edit.sha256 {
            if hash.len() != 64
                || !hash
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
            {
                return Err(invalid("Invalid SHA-256"));
            }
            if file.hash() != hash {
                return Err(invalid(
                    "File changed since it was read; read again before editing",
                ));
            }
        }
        bounded(
            edit.additional.len(),
            0,
            7,
            "Too many additional replacements",
        )?;
        let crlf =
            file.content.contains("\r\n") && !file.content.replace("\r\n", "").contains('\n');
        let mut changes = Vec::new();
        for replacement in std::iter::once(edit.replacement).chain(edit.additional) {
            let (old, replacement) = if crlf {
                (
                    with_crlf(&replacement.old_text),
                    with_crlf(&replacement.new_text),
                )
            } else {
                (replacement.old_text, replacement.new_text)
            };
            if old.is_empty()
                || old == replacement
                || old.len() > MAX_BYTES
                || replacement.len() > MAX_BYTES
                || replacement.contains('\0')
            {
                return Err(invalid(
                    "Supply a nonempty exact match and a different Unicode replacement",
                ));
            }
            let start = file.content.find(&old).ok_or_else(no_match)?;
            let next = start + old.chars().next().map_or(1, char::len_utf8);
            if file.content[next..].contains(&old) {
                return Err(no_match());
            }
            changes.push((start, start + old.len(), replacement));
        }
        changes.sort_unstable_by_key(|change| change.0);
        let mut combined = String::new();
        let mut last = 0;
        for (start, end, replacement) in changes {
            if start < last {
                return Err(invalid("Replacements overlap; supply disjoint passages"));
            }
            combined.push_str(&file.content[last..start]);
            combined.push_str(&replacement);
            last = end;
        }
        combined.push_str(&file.content[last..]);
        bounded(combined.len(), 0, MAX_BYTES, "Edited file exceeds 2 MiB")?;
        self.check_cancelled()?;
        save_source(
            &self.projects,
            &self.project,
            &file.path,
            &file.content,
            &combined,
        )?;
        let saved = self.read(&file.path)?;
        if saved.content != combined {
            return Err(invalid("Edit verification failed; read the file again"));
        }
        self.successful_build.store(false, Ordering::Release);
        Ok(json!({"path":file.path,"applied":true,"sha256":saved.hash(),"bytes":combined.len()}))
    }

    fn build(&self, log_limit: usize, force: bool) -> io::Result<Value> {
        self.successful_build.store(false, Ordering::Release);
        let toolchain = TexToolchain::discover()?;
        let settings = build_settings(&self.projects, &self.project)?;
        let project = checked_project_directory(&self.projects, &self.project)?;
        let result = compile_project(
            &self.projects,
            &self.project,
            &toolchain,
            &self.cancel,
            force,
        )?;
        let ok = result.pdf.is_some() && result.exit_code == Some(0);
        self.successful_build.store(ok, Ordering::Release);
        let characters = result.log.encode_utf16().count();
        let (start, _) = text_boundary(&result.log, characters.saturating_sub(log_limit));
        Ok(json!({
            "id":uuid::Uuid::new_v4().to_string(),
            "cached":result.cached,"engine":settings.engine.name(),"rootFile":settings.root_file,
            "cwd":project,"outputDirectory":project.join(".refract-build"),
            "arguments":result.arguments.iter().map(|arg|arg.to_string_lossy()).collect::<Vec<_>>(),
            "environment":"Configured TeX toolchain and project-local files",
            "pdfPath":result.pdf,"ok":ok,"exitCode":result.exit_code,"log":&result.log[start..],
            "logCharacters":characters,"logTruncated":characters > log_limit,
        }))
    }

    fn set_engine(&mut self, engine: String) -> io::Result<Value> {
        let selected = match engine.as_str() {
            "pdflatex" => LatexEngine::PdfLatex,
            "xelatex" => LatexEngine::XeLatex,
            "lualatex" => LatexEngine::LuaLatex,
            _ => return Err(invalid("Unsupported LaTeX engine")),
        };
        let previous = self.project.latex_engine().ok();
        let changed = previous != Some(selected);
        if changed {
            self.project = update_project_setting(
                &self.projects,
                &self.project,
                ProjectSetting::Engine(selected),
            )?;
            self.successful_build.store(false, Ordering::Release);
        }
        Ok(
            json!({"engine":engine,"previousEngine":previous.map(LatexEngine::name),"changed":changed}),
        )
    }

    fn package(&self, name: &str) -> io::Result<Value> {
        let prefix = name.strip_suffix(".sty").ok_or_else(invalid_package)?;
        let mut bytes = prefix.bytes();
        if prefix.len() > 128
            || !bytes
                .next()
                .is_some_and(|first| first.is_ascii_alphanumeric())
            || !bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
        {
            return Err(invalid_package());
        }
        let toolchain = TexToolchain::discover()?;
        let project = checked_project_directory(&self.projects, &self.project)?;
        let path = env::var_os("PATH").unwrap_or_default();
        let paths = std::iter::once(toolchain.bin().to_owned()).chain(env::split_paths(&path));
        let path = env::join_paths(paths).map_err(io::Error::other)?;
        let mut command = Command::new(toolchain.executable("kpsewhich"));
        command.current_dir(project).arg(name).env("PATH", path);
        let output = run_process(command, &self.cancel)?;
        Ok(
            json!({"name":name,"available":output.code == Some(0),"exitCode":output.code,"output":output.log}),
        )
    }
}

#[derive(Deserialize)]
struct ReadRequest {
    path: String,
    offset: Option<usize>,
    limit: Option<usize>,
    find: Option<String>,
}

#[derive(Deserialize)]
struct ReadMany {
    paths: Vec<String>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct ConversationRead {
    offset: Option<usize>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BuildRequest {
    force: Option<bool>,
    log_limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EngineRequest {
    engine: String,
}

#[derive(Deserialize)]
struct PackageRequest {
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Replacement {
    old_text: String,
    new_text: String,
}

#[derive(Deserialize)]
struct EditRequest {
    path: String,
    sha256: Option<String>,
    #[serde(flatten)]
    replacement: Replacement,
    #[serde(default)]
    additional: Vec<Replacement>,
}

#[derive(Deserialize)]
struct EditMany {
    edits: Vec<EditRequest>,
}

struct SourceFile {
    path: String,
    content: String,
}

impl SourceFile {
    fn hash(&self) -> String {
        format!("{:x}", Sha256::digest(self.content.as_bytes()))
    }

    fn excerpt(&self, offset: usize, limit: usize) -> Value {
        let mut result = excerpt(&self.content, offset, limit);
        result["path"] = json!(self.path);
        result["sha256"] = json!(self.hash());
        result
    }

    fn find(&self, needle: &str, offset: usize, limit: usize) -> Value {
        let (from, _) = text_boundary(&self.content, offset);
        let Some(relative) = self.content[from..].find(needle) else {
            let mut result = self.excerpt(offset, 0);
            result["matchOffset"] = Value::Null;
            result["nextOffset"] = Value::Null;
            return result;
        };
        let start = from + relative;
        let match_offset = self.content[..start].encode_utf16().count();
        let needle_size = needle.encode_utf16().count();
        let before = 120.min(limit.max(needle_size).saturating_sub(needle_size) / 2);
        let mut result = self.excerpt(
            match_offset.saturating_sub(before),
            limit.min(600.max(needle_size)),
        );
        let first = self.content[start..]
            .chars()
            .next()
            .map_or(1, char::len_utf8);
        result["matchOffset"] = json!(match_offset);
        result["nextOffset"] = json!(
            self.content[start + first..]
                .contains(needle)
                .then_some(match_offset + 1)
        );
        result
    }
}

fn excerpt(text: &str, offset: usize, limit: usize) -> Value {
    let (start, units) = text_boundary(text, offset);
    let (end, count) = text_boundary(&text[start..], limit);
    json!({
        "offset":offset,"nextOffset":(start+end < text.len()).then_some(units+count),
        "totalCharacters":text.encode_utf16().count(),"content":&text[start..start+end]
    })
}

// Advance to a Unicode boundary if a requested UTF-16 offset splits a supplementary character.
fn text_boundary(text: &str, offset: usize) -> (usize, usize) {
    let mut units = 0;
    for (index, character) in text.char_indices() {
        if units >= offset {
            return (index, units);
        }
        units += character.len_utf16();
    }
    (text.len(), units)
}

fn list_files<'a>(entries: &'a [ProjectEntry], files: &mut Vec<&'a str>) {
    for entry in entries {
        if files.len() > MAX_FILES {
            return;
        }
        if let Some(children) = &entry.children {
            list_files(children, files);
        } else {
            files.push(entry.path.as_str());
        }
    }
}

fn with_crlf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}

fn definition(name: &'static str, description: &'static str, schema: Value) -> Value {
    json!({"name":name,"description":description,"inputSchema":schema})
}

fn bounded(value: usize, min: usize, max: usize, message: &'static str) -> io::Result<usize> {
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(invalid(message))
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn no_match() -> io::Error {
    invalid("Expected exactly one match; read the file and supply more context")
}

fn invalid_package() -> io::Error {
    invalid("Expected a package filename ending in .sty")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        conversation::{Provider, StartTurn, begin_conversation_turn},
        create_project,
    };
    use std::fs;

    #[test]
    fn recognizes_success_claims_that_require_a_tool_receipt() {
        assert!(claims_successful_build(
            "I fixed the citations and compiled the paper."
        ));
        assert!(claims_successful_build(
            "The compilation completed successfully."
        ));
        assert!(!claims_successful_build(
            "Compile the document again to check the citations."
        ));
    }

    #[test]
    fn reads_bounded_source_and_verifies_disjoint_edits() {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path();
        let project = create_project(projects, "Paper").unwrap();
        let path = projects.join(&project.id).join("main.tex");
        fs::write(&path, "Title A\r\nContext 😀\r\nResult B\r\n").unwrap();
        let mut tools = AssistantTools::new(
            projects.to_owned(),
            project,
            ConversationId::default(),
            Arc::new(AtomicBool::new(false)),
        );
        let match_result = tools
            .call(
                "project_read",
                json!({"path":"main.tex","find":"😀","limit":12}),
            )
            .unwrap();
        assert_eq!(
            match_result["matchOffset"],
            "Title A\r\nContext ".encode_utf16().count()
        );
        assert_eq!(match_result["nextOffset"], Value::Null);
        let expected = &match_result["sha256"];
        let edit = json!({
            "path":"main.tex","sha256":expected,"oldText":"Title A\nContext","newText":"Title C\nContext",
            "additional":[{"oldText":"Result B","newText":"Theorem B"}]
        });
        assert_eq!(
            tools.call("project_edit", edit.clone()).unwrap()["applied"],
            true
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "Title C\r\nContext 😀\r\nTheorem B\r\n"
        );
        assert!(
            tools
                .call("project_edit", edit)
                .unwrap_err()
                .to_string()
                .contains("changed")
        );
        assert!(
            tools
                .call("project_read", json!({"path":".refract.json"}))
                .is_err()
        );
        fs::hard_link(&path, path.with_file_name("alias.tex")).unwrap();
        assert!(
            tools
                .call("project_read", json!({"path":"alias.tex"}))
                .is_err()
        );
    }

    #[test]
    fn pages_the_active_conversation_and_changes_only_the_project_engine() {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path();
        let project = create_project(projects, "Paper").unwrap();
        let id = ConversationId::default();
        let (turn, _) = begin_conversation_turn(
            projects,
            &project,
            &id,
            StartTurn {
                provider: Provider::Codex,
                prompt: "Check citations",
                attachments: &[],
                codex_account: None,
                build_instructions: None,
            },
        )
        .unwrap();
        turn.finish(Ok("The citations are present."), None).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut tools = AssistantTools::new(projects.to_owned(), project, id, cancel.clone());
        let result = tools
            .call("conversation_read", json!({"offset":0,"limit":10}))
            .unwrap();
        assert_eq!(result["content"], "user: Chec");
        assert_eq!(result["nextOffset"], 10);
        let result = tools
            .call("project_set_engine", json!({"engine":"xelatex"}))
            .unwrap();
        assert_eq!(result["previousEngine"], "pdflatex");
        assert_eq!(result["changed"], true);
        assert_eq!(
            tools
                .call("project_set_engine", json!({"engine":"xelatex"}))
                .unwrap()["changed"],
            false
        );
        cancel.store(true, Ordering::Release);
        assert_eq!(
            tools.call("project_list", json!({})).unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
    }
}

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json, value::RawValue};
use std::{
    ffi::OsString,
    io::{self, BufRead, Write},
    path::PathBuf,
};

const CODEX_FRAME_LIMIT: usize = 8 * 1024 * 1024;
const OPENCODE_FRAME_LIMIT: usize = 4 * 1024 * 1024;

#[derive(Serialize)]
pub struct CodexRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<u64>,
    method: &'static str,
    params: Value,
}

#[derive(Default)]
pub struct CodexThreadOptions<'a> {
    pub model: Option<&'a str>,
    pub reasoning_effort: Option<&'a str>,
    pub fast_mode: Option<bool>,
    pub developer_instructions: Option<&'a str>,
    pub tools: Option<CodexToolConnection<'a>>,
}

pub struct CodexToolConnection<'a> {
    pub url: &'a str,
    pub token: &'a str,
}

impl CodexRequest {
    fn new(id: u64, method: &'static str, params: Value) -> Self {
        Self {
            id: Some(id),
            method,
            params,
        }
    }

    pub fn initialize(id: u64, account_connection: bool) -> Self {
        Self::new(
            id,
            "initialize",
            json!({
                "clientInfo": {
                    "name": if account_connection { "Refract" } else { "refract" },
                    "title": "Refract",
                    "version": "0.2.1",
                }
            }),
        )
    }

    pub fn initialized() -> Self {
        Self {
            id: None,
            method: "initialized",
            params: json!({}),
        }
    }

    pub fn model_list(id: u64, cursor: Option<&str>) -> Self {
        Self::new(id, "model/list", json!({"cursor": cursor, "limit": 100}))
    }

    pub fn thread(
        id: u64,
        cwd: &str,
        session_id: Option<&str>,
        options: &CodexThreadOptions<'_>,
    ) -> Self {
        let mut config = Map::from_iter([
            ("allow_login_shell".into(), json!(false)),
            ("tool_output_token_limit".into(), json!(4000)),
        ]);
        if let Some(tools) = &options.tools {
            config.insert(
                "mcp_servers.refract".into(),
                json!({
                    "url": tools.url,
                    "http_headers": {"Authorization": format!("Bearer {}", tools.token)},
                    "required": true,
                    "default_tools_approval_mode": "approve",
                    "tool_timeout_sec": 300,
                    "tools": {
                        "project_read": {"output_token_limit": 32768},
                        "project_read_many": {"output_token_limit": 32768},
                        "conversation_read": {"output_token_limit": 32768},
                    }
                }),
            );
        }
        if let Some(effort) = options.reasoning_effort {
            config.insert("model_reasoning_effort".into(), json!(effort));
        }
        if let Some(fast) = options.fast_mode {
            config.insert(
                "service_tier".into(),
                json!(if fast { "fast" } else { "default" }),
            );
            config.insert("features.fast_mode".into(), json!(true));
        }
        let mut params = json!({
            "cwd": cwd,
            "approvalPolicy": "never",
            "sandbox": "workspace-write",
            "model": options.model,
            "config": config,
        });
        if let Some(instructions) = options.developer_instructions {
            params["developerInstructions"] = json!(instructions);
        }
        if let Some(thread) = session_id {
            params["threadId"] = json!(thread);
            params["excludeTurns"] = json!(true);
            Self::new(id, "thread/resume", params)
        } else {
            Self::new(id, "thread/start", params)
        }
    }

    pub fn start_turn(
        id: u64,
        cwd: &str,
        thread_id: &str,
        prompt: &str,
        images: &[String],
        effort: Option<&str>,
    ) -> Self {
        let mut input = vec![json!({"type": "text", "text": prompt, "text_elements": []})];
        input.extend(
            images
                .iter()
                .map(|image| json!({"type": "localImage", "path": image})),
        );
        let mut params = json!({"threadId": thread_id, "cwd": cwd, "input": input});
        if let Some(effort) = effort {
            params["effort"] = json!(effort);
        }
        Self::new(id, "turn/start", params)
    }

    pub fn interrupt(id: u64, thread_id: &str, turn_id: &str) -> Self {
        Self::new(
            id,
            "turn/interrupt",
            json!({"threadId": thread_id, "turnId": turn_id}),
        )
    }

    pub fn unsubscribe(id: u64, thread_id: &str) -> Self {
        Self::new(id, "thread/unsubscribe", json!({"threadId": thread_id}))
    }

    pub fn write_to(&self, output: &mut impl Write) -> io::Result<()> {
        write_message(output, self)
    }
}

fn write_message(output: &mut impl Write, message: &impl Serialize) -> io::Result<()> {
    serde_json::to_writer(&mut *output, message)?;
    output.write_all(b"\n")?;
    output.flush()
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum CodexRpcId {
    Number(u64),
    String(String),
}

#[derive(Debug)]
pub struct CodexServerRequest {
    pub id: CodexRpcId,
    pub method: String,
}

impl CodexServerRequest {
    pub fn deny(&self, output: &mut impl Write) -> io::Result<()> {
        write_message(
            output,
            &json!({
                "id": self.id,
                "error": {"code": -32000, "message": "Interactive input is unavailable in Refract"}
            }),
        )
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexResult {
    pub thread: Option<CodexThread>,
    pub turn: Option<CodexTurn>,
    pub status: Option<String>,
    pub data: Option<Vec<Value>>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CodexThread {
    pub id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CodexTurnStatus {
    Completed,
    Interrupted,
    #[serde(other)]
    Failed,
}

#[derive(Debug, Deserialize)]
pub struct CodexTurn {
    pub id: String,
    pub status: Option<CodexTurnStatus>,
    pub error: Option<CodexRpcError>,
}

impl CodexTurn {
    pub fn outcome(&self) -> Result<(), &str> {
        match self.status {
            Some(CodexTurnStatus::Completed) => Ok(()),
            Some(CodexTurnStatus::Interrupted) => Err("Assistant operation cancelled."),
            _ => Err(self
                .error
                .as_ref()
                .map(|error| error.message.as_str())
                .filter(|message| !message.is_empty())
                .unwrap_or("Codex turn failed")),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CodexRpcError {
    pub code: Option<i32>,
    pub message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CodexItemKind {
    AgentMessage,
    CommandExecution,
    FileChange,
    McpToolCall,
    WebSearch,
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
pub struct CodexItem {
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: CodexItemKind,
    pub text: Option<String>,
}

#[derive(Debug)]
pub enum CodexEventKind {
    TurnStarted(Option<CodexTurn>),
    TurnCompleted(Option<CodexTurn>),
    ItemStarted(Option<CodexItem>),
    ItemCompleted(Option<CodexItem>),
    ThreadClosed,
    Other,
}

#[derive(Debug)]
pub struct CodexEvent {
    pub thread_id: Option<String>,
    pub kind: CodexEventKind,
}

impl CodexEvent {
    pub fn progress(&self) -> Option<&'static str> {
        match &self.kind {
            CodexEventKind::TurnStarted(_) => Some("Thinking…"),
            CodexEventKind::ItemStarted(item) => Some(match item.as_ref().map(|item| &item.kind) {
                Some(CodexItemKind::CommandExecution) => "Running a command…",
                Some(CodexItemKind::FileChange) => "Updating project files…",
                Some(CodexItemKind::McpToolCall) => "Using a tool…",
                _ => "Working…",
            }),
            CodexEventKind::ItemCompleted(item)
                if !matches!(
                    item,
                    Some(CodexItem {
                        kind: CodexItemKind::AgentMessage,
                        text: Some(_),
                        ..
                    })
                ) =>
            {
                Some("Thinking…")
            }
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum CodexMessage {
    Reply {
        id: u64,
        result: Result<CodexResult, CodexRpcError>,
    },
    ServerRequest(CodexServerRequest),
    Notification(CodexEvent),
}

#[derive(Deserialize)]
struct CodexWireMessage<'a> {
    id: Option<CodexRpcId>,
    method: Option<String>,
    #[serde(borrow)]
    params: Option<&'a RawValue>,
    #[serde(borrow)]
    result: Option<&'a RawValue>,
    error: Option<CodexRpcError>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexEventParams {
    thread_id: Option<String>,
    turn: Option<CodexTurn>,
    item: Option<CodexItem>,
}

impl CodexWireMessage<'_> {
    fn decode(self) -> serde_json::Result<CodexMessage> {
        Ok(match (self.id, self.method) {
            (Some(id), Some(method)) => {
                CodexMessage::ServerRequest(CodexServerRequest { id, method })
            }
            (Some(CodexRpcId::Number(id)), None) => CodexMessage::Reply {
                id,
                result: match self.error {
                    Some(error) => Err(error),
                    None => Ok(self
                        .result
                        .map(|result| serde_json::from_str(result.get()))
                        .transpose()?
                        .unwrap_or_default()),
                },
            },
            (_, method) => {
                let method = method.as_deref().unwrap_or_default();
                let mut params = if matches!(
                    method,
                    "turn/started"
                        | "turn/completed"
                        | "item/started"
                        | "item/completed"
                        | "thread/closed"
                ) {
                    self.params
                        .map(|params| serde_json::from_str(params.get()))
                        .transpose()?
                        .unwrap_or_default()
                } else {
                    CodexEventParams::default()
                };
                let kind = match method {
                    "turn/started" => CodexEventKind::TurnStarted(params.turn.take()),
                    "turn/completed" => CodexEventKind::TurnCompleted(params.turn.take()),
                    "item/started" => CodexEventKind::ItemStarted(params.item.take()),
                    "item/completed" => CodexEventKind::ItemCompleted(params.item.take()),
                    "thread/closed" => CodexEventKind::ThreadClosed,
                    _ => CodexEventKind::Other,
                };
                CodexMessage::Notification(CodexEvent {
                    thread_id: params.thread_id,
                    kind,
                })
            }
        })
    }
}

pub struct CodexMessages<R> {
    input: R,
    frame: Vec<u8>,
}

impl<R: BufRead> CodexMessages<R> {
    pub fn new(input: R) -> Self {
        Self {
            input,
            frame: Vec::new(),
        }
    }

    pub fn read_next(&mut self) -> io::Result<Option<CodexMessage>> {
        let invalid = || {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Codex returned an invalid protocol event",
            )
        };
        match read_frame(&mut self.input, &mut self.frame, CODEX_FRAME_LIMIT)? {
            Frame::Line => serde_json::from_slice::<CodexWireMessage>(&self.frame)
                .and_then(CodexWireMessage::decode)
                .map(Some)
                .map_err(|_| invalid()),
            Frame::Partial => Err(invalid()),
            Frame::Oversized => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Codex returned an oversized protocol event",
            )),
            Frame::End => Ok(None),
        }
    }
}

pub fn open_code_arguments(
    session_id: Option<&str>,
    images: &[PathBuf],
    model: Option<&str>,
    variant: Option<&str>,
    standalone: bool,
) -> Vec<OsString> {
    let mut arguments = vec!["run".into(), "--format".into(), "json".into()];
    if standalone {
        arguments.push("--standalone".into());
    }
    if let Some(model) = model {
        arguments.push("--model".into());
        arguments.push(
            variant
                .map_or_else(|| model.to_owned(), |variant| format!("{model}#{variant}"))
                .into(),
        );
    }
    if let Some(session_id) = session_id {
        arguments.extend([OsString::from("--session"), session_id.into()]);
    }
    for image in images {
        arguments.extend([OsString::from("--file"), image.as_os_str().to_owned()]);
    }
    arguments
}

#[derive(Debug, PartialEq, Eq)]
pub enum OpenCodeEventKind {
    Text(String),
    Error(String),
    ToolUse,
    StepStart,
    Other,
}

#[derive(Debug, PartialEq, Eq)]
pub struct OpenCodeEvent {
    pub session_id: Option<String>,
    pub kind: OpenCodeEventKind,
}

impl OpenCodeEvent {
    pub fn progress(&self) -> Option<&'static str> {
        match self.kind {
            OpenCodeEventKind::ToolUse => Some("Using a tool…"),
            OpenCodeEventKind::StepStart => Some("Thinking…"),
            _ => None,
        }
    }
}

#[derive(Deserialize)]
struct OpenCodeWireMessage<'a> {
    #[serde(rename = "sessionID")]
    session_id: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(borrow)]
    part: Option<&'a RawValue>,
    #[serde(borrow)]
    error: Option<&'a RawValue>,
}

#[derive(Deserialize)]
struct OpenCodePart {
    text: Option<String>,
}

impl From<OpenCodeWireMessage<'_>> for OpenCodeEvent {
    fn from(message: OpenCodeWireMessage<'_>) -> Self {
        let kind = match message.kind.as_deref() {
            Some("text") => message
                .part
                .and_then(|part| serde_json::from_str::<OpenCodePart>(part.get()).ok())
                .and_then(|part| part.text)
                .map(OpenCodeEventKind::Text)
                .unwrap_or(OpenCodeEventKind::Other),
            Some("error") => {
                let error: Value = message
                    .error
                    .and_then(|error| serde_json::from_str(error.get()).ok())
                    .unwrap_or_default();
                let text = [
                    error.pointer("/data/message"),
                    error.get("message"),
                    error.get("_tag"),
                ]
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .find(|text| !text.is_empty())
                .unwrap_or("OpenCode reported an error");
                OpenCodeEventKind::Error(text.to_owned())
            }
            Some("tool_use") => OpenCodeEventKind::ToolUse,
            Some("step_start") => OpenCodeEventKind::StepStart,
            _ => OpenCodeEventKind::Other,
        };
        Self {
            session_id: message.session_id.filter(|id| !id.is_empty()),
            kind,
        }
    }
}

pub struct OpenCodeEvents<R> {
    input: R,
    frame: Vec<u8>,
    oversized: bool,
}

impl<R: BufRead> OpenCodeEvents<R> {
    pub fn new(input: R) -> Self {
        Self {
            input,
            frame: Vec::new(),
            oversized: false,
        }
    }

    pub fn had_oversized_event(&self) -> bool {
        self.oversized
    }

    pub fn read_next(&mut self) -> io::Result<Option<OpenCodeEvent>> {
        loop {
            match read_frame(&mut self.input, &mut self.frame, OPENCODE_FRAME_LIMIT)? {
                Frame::Line | Frame::Partial => {
                    if let Ok(message) = serde_json::from_slice::<OpenCodeWireMessage>(&self.frame)
                    {
                        return Ok(Some(message.into()));
                    }
                }
                Frame::Oversized => self.oversized = true,
                Frame::End => return Ok(None),
            }
        }
    }
}

enum Frame {
    Line,
    Partial,
    Oversized,
    End,
}

fn read_frame(input: &mut impl BufRead, frame: &mut Vec<u8>, limit: usize) -> io::Result<Frame> {
    frame.clear();
    let mut oversized = false;
    loop {
        let available = input.fill_buf()?;
        if available.is_empty() {
            return Ok(if oversized {
                Frame::Oversized
            } else if frame.is_empty() {
                Frame::End
            } else {
                Frame::Partial
            });
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let length = newline.unwrap_or(available.len());
        if !oversized {
            if frame.len() + length > limit {
                frame.clear();
                oversized = true;
            } else {
                frame.extend_from_slice(&available[..length]);
            }
        }
        input.consume(length + usize::from(newline.is_some()));
        if newline.is_some() {
            return Ok(if oversized {
                Frame::Oversized
            } else {
                Frame::Line
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;

    #[test]
    fn codex_requests_keep_the_existing_workspace_and_resume_options() {
        let request = CodexRequest::thread(
            7,
            r"C:\Project files",
            Some("thread-7"),
            &CodexThreadOptions {
                model: Some("gpt-example"),
                reasoning_effort: Some("high"),
                fast_mode: Some(false),
                ..Default::default()
            },
        );
        let mut bytes = Vec::new();
        request.write_to(&mut bytes).unwrap();
        assert!(bytes.ends_with(b"\n"));
        let request: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(request["id"], 7);
        assert_eq!(request["method"], "thread/resume");
        assert_eq!(request["params"]["cwd"], r"C:\Project files");
        assert_eq!(request["params"]["sandbox"], "workspace-write");
        assert_eq!(request["params"]["excludeTurns"], true);
        assert_eq!(request["params"]["config"]["features.fast_mode"], true);
        assert_eq!(request["params"]["config"]["service_tier"], "default");
        assert_eq!(
            request["params"]["config"]["model_reasoning_effort"],
            "high"
        );

        let turn = CodexRequest::start_turn(
            8,
            r"C:\Project files",
            "thread-7",
            "Revise the abstract",
            &[r"C:\Project files\screenshot.png".into()],
            Some("high"),
        );
        let turn = serde_json::to_value(turn).unwrap();
        assert_eq!(turn["params"]["input"][0]["text"], "Revise the abstract");
        assert_eq!(turn["params"]["input"][1]["type"], "localImage");
        assert_eq!(turn["params"]["effort"], "high");
    }

    #[test]
    fn codex_reads_replies_and_scoped_events_and_can_reject_interactive_requests() {
        let fixture = concat!(
            "{\"id\":1,\"result\":{\"thread\":{\"id\":\"thread-1\"}}}\r\n",
            "{\"method\":\"item/started\",\"params\":{\"threadId\":\"thread-1\",\"item\":{\"id\":\"item-1\",\"type\":\"mcpToolCall\"}}}\n",
            "{\"id\":\"provider-2\",\"method\":\"item/commandExecution/requestApproval\",\"params\":{\"threadId\":\"thread-1\"}}\n",
            "{\"method\":\"item/completed\",\"params\":{\"threadId\":\"thread-1\",\"item\":{\"type\":\"agentMessage\",\"text\":\"Fixed résumé\"}}}\n",
            "{\"method\":\"turn/completed\",\"params\":{\"threadId\":\"thread-1\",\"turn\":{\"id\":\"turn-1\",\"status\":\"interrupted\"}}}\n",
        );
        let mut messages = CodexMessages::new(BufReader::with_capacity(5, fixture.as_bytes()));
        let CodexMessage::Reply { id: 1, result } = messages.read_next().unwrap().unwrap() else {
            panic!("expected the thread reply");
        };
        assert_eq!(result.unwrap().thread.unwrap().id, "thread-1");
        let CodexMessage::Notification(event) = messages.read_next().unwrap().unwrap() else {
            panic!("expected a tool progress event");
        };
        assert_eq!(event.thread_id.as_deref(), Some("thread-1"));
        assert_eq!(event.progress(), Some("Using a tool…"));
        let CodexMessage::ServerRequest(request) = messages.read_next().unwrap().unwrap() else {
            panic!("expected an approval request");
        };
        let mut answer = Vec::new();
        request.deny(&mut answer).unwrap();
        let answer: Value = serde_json::from_slice(&answer).unwrap();
        assert_eq!(answer["id"], "provider-2");
        assert_eq!(answer["error"]["code"], -32000);
        let CodexMessage::Notification(CodexEvent {
            kind: CodexEventKind::ItemCompleted(Some(item)),
            ..
        }) = messages.read_next().unwrap().unwrap()
        else {
            panic!("expected the assistant reply");
        };
        assert_eq!(item.text.as_deref(), Some("Fixed résumé"));
        let CodexMessage::Notification(CodexEvent {
            kind: CodexEventKind::TurnCompleted(Some(turn)),
            ..
        }) = messages.read_next().unwrap().unwrap()
        else {
            panic!("expected a completed turn event");
        };
        assert_eq!(turn.outcome(), Err("Assistant operation cancelled."));
        assert!(messages.read_next().unwrap().is_none());
        assert!(CodexMessages::new(&b"{invalid}\n"[..]).read_next().is_err());
    }

    #[test]
    fn open_code_keeps_each_jsonl_text_part_and_accepts_a_final_line_without_newline() {
        let fixture = concat!(
            "diagnostic message\n",
            "{\"sessionID\":\"ses_1\",\"type\":\"step_start\"}\n",
            "{\"sessionID\":\"ses_1\",\"type\":\"text\",\"part\":{\"text\":\"First résumé.\"}}\r\n",
            "{\"type\":\"text\",\"part\":{\"text\":\" Second part.\"}}\n",
            "{\"type\":\"error\",\"error\":{\"_tag\":\"UnknownError\",\"data\":{\"message\":\"Session expired\"}}}",
        );
        let mut events = OpenCodeEvents::new(BufReader::with_capacity(3, fixture.as_bytes()));
        let first = events.read_next().unwrap().unwrap();
        assert_eq!(first.session_id.as_deref(), Some("ses_1"));
        assert_eq!(first.progress(), Some("Thinking…"));
        assert_eq!(
            events.read_next().unwrap().unwrap().kind,
            OpenCodeEventKind::Text("First résumé.".into())
        );
        assert_eq!(
            events.read_next().unwrap().unwrap().kind,
            OpenCodeEventKind::Text(" Second part.".into())
        );
        assert_eq!(
            events.read_next().unwrap().unwrap().kind,
            OpenCodeEventKind::Error("Session expired".into())
        );
        assert!(events.read_next().unwrap().is_none());
        assert!(!events.had_oversized_event());
        assert_eq!(
            open_code_arguments(
                Some("ses_1"),
                &[PathBuf::from("figure one.png")],
                Some("provider/model"),
                Some("high"),
                true,
            ),
            [
                "run",
                "--format",
                "json",
                "--standalone",
                "--model",
                "provider/model#high",
                "--session",
                "ses_1",
                "--file",
                "figure one.png",
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn oversized_frame_is_discarded_without_consuming_the_next_event() {
        let mut wire = &b"oversized line\n{}\n"[..];
        let mut frame = Vec::new();
        assert!(matches!(
            read_frame(&mut wire, &mut frame, 3).unwrap(),
            Frame::Oversized
        ));
        assert!(frame.is_empty());
        assert!(matches!(
            read_frame(&mut wire, &mut frame, 3).unwrap(),
            Frame::Line
        ));
        assert_eq!(frame, b"{}");
    }
}

use crate::{
    assistant_protocol::{
        CodexEvent, CodexEventKind, CodexItemKind, CodexMessage, CodexMessages, CodexRequest,
        CodexResult, CodexServerRequest, CodexThreadOptions,
    },
    tex::process::{ProcessTree, configure_process},
};

pub mod codex_catalog;
pub mod opencode;
use std::{
    fmt,
    io::{self, BufReader, Write},
    path::PathBuf,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

const RPC_TIMEOUT: Duration = Duration::from_secs(30);
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const CANCEL_POLL: Duration = Duration::from_millis(25);
const OUTGOING_LIMIT: usize = 8 * 1024 * 1024;
const CANCELLED: &str = "Assistant operation cancelled.";

const INITIALIZE_REQUEST: u64 = 1;
const THREAD_REQUEST: u64 = 2;
const TURN_REQUEST: u64 = 3;
const INTERRUPT_REQUEST: u64 = 4;
const UNSUBSCRIBE_REQUEST: u64 = 5;

pub struct CodexTurnRequest<'a> {
    pub account_connection: bool,
    pub session_id: Option<&'a str>,
    pub prompt: &'a str,
    pub images: &'a [PathBuf],
    pub options: CodexThreadOptions<'a>,
}

pub enum CodexUpdate<'a> {
    Session(&'a str),
    Progress(&'static str),
}

#[derive(Debug)]
pub struct CodexTurnReply {
    pub session_id: String,
    pub text: String,
}

#[derive(Debug)]
pub struct CodexRunError {
    pub session_id: Option<String>,
    pub message: String,
}

impl fmt::Display for CodexRunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CodexRunError {}

/// Run one turn with a configured Codex app-server command. Call on a worker
/// thread. The command must use the project as its working directory.
pub fn run_codex(
    command: Command,
    request: CodexTurnRequest<'_>,
    cancel: &AtomicBool,
    mut update: impl FnMut(CodexUpdate<'_>),
) -> Result<CodexTurnReply, CodexRunError> {
    let mut session_id = request.session_id.map(str::to_owned);
    let result = with_connection(command, cancel, |connection, cwd| {
        run_connection(
            connection,
            cwd,
            request,
            cancel,
            &mut session_id,
            &mut update,
        )
    });
    match result {
        Ok(text) => Ok(CodexTurnReply {
            session_id: session_id.expect("completed Codex turns have a session"),
            text,
        }),
        Err(message) => Err(CodexRunError {
            session_id,
            message,
        }),
    }
}

fn with_connection<T>(
    mut command: Command,
    cancel: &AtomicBool,
    run: impl FnOnce(&Connection, &str) -> Result<T, String>,
) -> Result<T, String> {
    if cancelled(cancel) {
        return Err(CANCELLED.into());
    }
    let cwd = command
        .get_current_dir()
        .filter(|path| path.is_absolute())
        .and_then(protocol_path)
        .ok_or("Codex requires an absolute UTF-8 project directory")?
        .to_owned();
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Stderr can include request data and credentials from the provider.
        .stderr(Stdio::null());
    configure_process(&mut command);
    let mut child = command
        .spawn()
        .map_err(|_| "Could not start Codex app server")?;
    let tree = match ProcessTree::attach(&child) {
        Ok(tree) => tree,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Could not manage the Codex app-server process".into());
        }
    };
    let process = AssistantProcess { child, tree };
    thread::scope(move |scope| {
        // The process must be stopped before the scope tries to join workers
        // that may still be waiting on its pipes.
        let mut process = process;
        let stdin = process
            .child
            .stdin
            .take()
            .ok_or("Codex input pipe is unavailable")?;
        let stdout = process
            .child
            .stdout
            .take()
            .ok_or("Codex output pipe is unavailable")?;
        let (events, incoming) = mpsc::sync_channel(2);
        let (outgoing, requests) = mpsc::sync_channel(2);
        let reader_events = events.clone();
        thread::Builder::new()
            .name("refract-codex-output".into())
            .spawn_scoped(scope, move || read_pipe(stdout, reader_events))
            .map_err(|_| "Could not start the Codex output reader")?;
        thread::Builder::new()
            .name("refract-codex-input".into())
            .spawn_scoped(scope, move || write_pipe(stdin, requests, events))
            .map_err(|_| "Could not start the Codex input writer")?;
        let connection = Connection { incoming, outgoing };
        run(&connection, &cwd)
    })
}

struct AssistantProcess {
    child: Child,
    tree: ProcessTree,
}

impl Drop for AssistantProcess {
    fn drop(&mut self) {
        self.tree.stop(&mut self.child);
        let _ = self.child.wait();
    }
}

enum Incoming {
    Message(CodexMessage),
    Closed,
    Invalid(String),
}

enum Outgoing {
    Request(CodexRequest),
    Denial(CodexServerRequest),
}

fn read_pipe(stdout: ChildStdout, events: SyncSender<Incoming>) {
    let mut messages = CodexMessages::new(BufReader::new(stdout));
    loop {
        let event = match messages.read_next() {
            Ok(Some(message)) => Incoming::Message(message),
            Ok(None) => {
                let _ = events.send(Incoming::Closed);
                return;
            }
            Err(error) => {
                let _ = events.send(Incoming::Invalid(error.to_string()));
                return;
            }
        };
        if events.send(event).is_err() {
            return;
        }
    }
}

fn write_pipe(mut stdin: ChildStdin, requests: Receiver<Outgoing>, events: SyncSender<Incoming>) {
    for request in requests {
        let mut output = LimitedWriter {
            output: &mut stdin,
            remaining: OUTGOING_LIMIT,
        };
        let result = match request {
            Outgoing::Request(request) => request.write_to(&mut output),
            Outgoing::Denial(request) => request.deny(&mut output),
        };
        if result.is_err() {
            let _ = events.send(Incoming::Invalid(
                "Could not write the request to Codex".into(),
            ));
            return;
        }
    }
}

struct LimitedWriter<'a, W> {
    output: &'a mut W,
    remaining: usize,
}

impl<W: Write> Write for LimitedWriter<'_, W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if buffer.len() > self.remaining {
            return Err(io::Error::other("Codex request is too large"));
        }
        let written = self.output.write(buffer)?;
        self.remaining -= written;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.output.flush()
    }
}

struct Connection {
    incoming: Receiver<Incoming>,
    outgoing: SyncSender<Outgoing>,
}

impl Connection {
    fn send(&self, message: Outgoing) -> Result<(), String> {
        self.outgoing
            .try_send(message)
            .map_err(|_| "Codex app-server input is unavailable".into())
    }

    fn request(&self, request: CodexRequest) -> Result<(), String> {
        self.send(Outgoing::Request(request))
    }

    fn poll(&self, deadline: Option<Instant>) -> Result<Option<CodexMessage>, String> {
        let wait = deadline.map_or(CANCEL_POLL, |deadline| {
            CANCEL_POLL.min(deadline.saturating_duration_since(Instant::now()))
        });
        match self.incoming.recv_timeout(wait) {
            Ok(Incoming::Message(CodexMessage::ServerRequest(request))) => {
                self.send(Outgoing::Denial(request))?;
                Ok(None)
            }
            Ok(Incoming::Message(message)) => Ok(Some(message)),
            Ok(Incoming::Invalid(error)) => Err(error),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Ok(Incoming::Closed) | Err(RecvTimeoutError::Disconnected) => {
                Err("Codex app server stopped".into())
            }
        }
    }

    fn reply(
        &self,
        request_id: u64,
        method: &str,
        cancel: &AtomicBool,
    ) -> Result<CodexResult, String> {
        let deadline = Instant::now() + RPC_TIMEOUT;
        while Instant::now() < deadline {
            if cancelled(cancel) {
                return Err(CANCELLED.into());
            }
            if let Some(CodexMessage::Reply { id, result }) = self.poll(Some(deadline))?
                && id == request_id
            {
                return result.map_err(|error| error.message);
            }
        }
        Err(format!("Codex {method} timed out"))
    }
}

fn run_connection(
    connection: &Connection,
    cwd: &str,
    request: CodexTurnRequest<'_>,
    cancel: &AtomicBool,
    session_id: &mut Option<String>,
    update: &mut impl FnMut(CodexUpdate<'_>),
) -> Result<String, String> {
    let images = request
        .images
        .iter()
        .map(|path| {
            protocol_path(path)
                .map(str::to_owned)
                .ok_or("Codex requires UTF-8 attachment paths")
        })
        .collect::<Result<Vec<_>, _>>()?;
    connection.request(CodexRequest::initialize(
        INITIALIZE_REQUEST,
        request.account_connection,
    ))?;
    connection.reply(INITIALIZE_REQUEST, "initialize", cancel)?;
    connection.request(CodexRequest::initialized())?;
    connection.request(CodexRequest::thread(
        THREAD_REQUEST,
        cwd,
        request.session_id,
        &request.options,
    ))?;
    let thread = connection
        .reply(THREAD_REQUEST, "thread", cancel)?
        .thread
        .map(|thread| thread.id)
        .filter(|id| !id.is_empty())
        .ok_or("Codex returned no thread ID")?;
    *session_id = Some(thread.clone());
    update(CodexUpdate::Session(&thread));
    if cancelled(cancel) {
        unsubscribe(connection, &thread);
        return Err(CANCELLED.into());
    }

    connection.request(CodexRequest::start_turn(
        TURN_REQUEST,
        cwd,
        &thread,
        request.prompt,
        &images,
        request.options.reasoning_effort,
    ))?;
    let mut turn = TurnState::default();
    let result = run_turn(connection, &thread, &mut turn, cancel, update);
    if turn.outcome.is_some() {
        unsubscribe(connection, &thread);
    }
    result
}

#[derive(Default)]
struct TurnState {
    id: Option<String>,
    text: String,
    outcome: Option<Result<(), String>>,
}

impl TurnState {
    fn observe(&mut self, event: CodexEvent, update: &mut impl FnMut(CodexUpdate<'_>)) {
        if self.outcome.is_some() {
            return;
        }
        if let Some(progress) = event.progress() {
            update(CodexUpdate::Progress(progress));
        }
        match event.kind {
            CodexEventKind::TurnStarted(Some(turn)) => self.id = Some(turn.id),
            CodexEventKind::ItemCompleted(Some(item))
                if matches!(item.kind, CodexItemKind::AgentMessage) =>
            {
                if let Some(text) = item.text {
                    self.text = text;
                }
            }
            CodexEventKind::TurnCompleted(turn) => {
                if let Some(turn) = turn {
                    if self.id.as_ref().is_some_and(|id| id != &turn.id) {
                        return;
                    }
                    self.outcome = Some(turn.outcome().map_err(str::to_owned));
                } else {
                    self.outcome = Some(Err("Codex turn failed".into()));
                }
            }
            _ => {}
        }
    }
}

fn run_turn(
    connection: &Connection,
    thread_id: &str,
    turn: &mut TurnState,
    cancel: &AtomicBool,
    update: &mut impl FnMut(CodexUpdate<'_>),
) -> Result<String, String> {
    let startup_deadline = Instant::now() + RPC_TIMEOUT;
    let mut stop_deadline = None;
    let mut started = false;
    let mut interrupt_sent = false;
    loop {
        let stopping = stop_deadline.is_some() || cancelled(cancel);
        if stopping {
            let deadline = stop_deadline.get_or_insert_with(|| Instant::now() + STOP_TIMEOUT);
            if turn.outcome.is_some() || Instant::now() >= *deadline {
                return Err(CANCELLED.into());
            }
            if !interrupt_sent && let Some(turn_id) = &turn.id {
                connection
                    .request(CodexRequest::interrupt(
                        INTERRUPT_REQUEST,
                        thread_id,
                        turn_id,
                    ))
                    .map_err(|_| CANCELLED)?;
                interrupt_sent = true;
            }
        } else if started && let Some(result) = &turn.outcome {
            result.as_ref().map_err(Clone::clone)?;
            return if turn.text.is_empty() {
                Err("Codex returned no response".into())
            } else {
                Ok(std::mem::take(&mut turn.text))
            };
        } else if !started && Instant::now() >= startup_deadline {
            return Err("Codex turn/start timed out".into());
        }
        let deadline = stop_deadline.or((!started).then_some(startup_deadline));
        let message = connection.poll(deadline).map_err(|error| {
            if stopping || cancelled(cancel) {
                CANCELLED.into()
            } else {
                error
            }
        })?;
        match message {
            Some(CodexMessage::Reply {
                id: TURN_REQUEST,
                result,
            }) => {
                let result = result.map_err(|error| {
                    if stopping {
                        CANCELLED.to_owned()
                    } else {
                        error.message
                    }
                })?;
                if let Some(started) = result.turn {
                    turn.id = Some(started.id);
                }
                started = true;
            }
            Some(CodexMessage::Reply {
                id: INTERRUPT_REQUEST,
                result: Err(_),
            }) => {
                return Err(CANCELLED.into());
            }
            Some(CodexMessage::Notification(event))
                if event.thread_id.as_deref() == Some(thread_id) =>
            {
                turn.observe(event, update);
            }
            _ => {}
        }
    }
}

fn unsubscribe(connection: &Connection, thread_id: &str) {
    if connection
        .request(CodexRequest::unsubscribe(UNSUBSCRIBE_REQUEST, thread_id))
        .is_err()
    {
        return;
    }
    let deadline = Instant::now() + STOP_TIMEOUT;
    let mut closed = false;
    let mut replied = false;
    while Instant::now() < deadline && !(closed && replied) {
        match connection.poll(Some(deadline)) {
            Ok(Some(CodexMessage::Reply {
                id: UNSUBSCRIBE_REQUEST,
                result: Ok(result),
            })) => {
                replied = true;
                closed |= result.status.as_deref() == Some("notLoaded");
            }
            Ok(Some(CodexMessage::Notification(event)))
                if event.thread_id.as_deref() == Some(thread_id) =>
            {
                closed |= matches!(event.kind, CodexEventKind::ThreadClosed);
            }
            Ok(Some(CodexMessage::Reply {
                id: UNSUBSCRIBE_REQUEST,
                result: Err(_),
            }))
            | Err(_) => return,
            _ => {}
        }
    }
}

fn cancelled(cancel: &AtomicBool) -> bool {
    cancel.load(Ordering::Relaxed)
}

fn protocol_path(path: &std::path::Path) -> Option<&str> {
    #[cfg(windows)]
    let path = dunce::simplified(path);
    path.to_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn fixture(wire: &str) -> (Connection, Receiver<Outgoing>) {
        let (events, incoming) = mpsc::sync_channel(16);
        let (outgoing, requests) = mpsc::sync_channel(16);
        let mut messages = CodexMessages::new(wire.as_bytes());
        while let Some(message) = messages.read_next().unwrap() {
            events.send(Incoming::Message(message)).unwrap();
        }
        (Connection { incoming, outgoing }, requests)
    }

    fn request() -> CodexTurnRequest<'static> {
        CodexTurnRequest {
            account_connection: false,
            session_id: None,
            prompt: "Improve the introduction",
            images: &[],
            options: CodexThreadOptions::default(),
        }
    }

    fn methods(requests: Receiver<Outgoing>) -> Vec<String> {
        requests
            .try_iter()
            .map(|request| {
                let mut bytes = Vec::new();
                match request {
                    Outgoing::Request(request) => request.write_to(&mut bytes).unwrap(),
                    Outgoing::Denial(request) => request.deny(&mut bytes).unwrap(),
                }
                let message: Value = serde_json::from_slice(&bytes).unwrap();
                message["method"].as_str().unwrap_or("deny").to_owned()
            })
            .collect()
    }

    #[test]
    fn completes_a_turn_with_notifications_before_its_acknowledgement() {
        let (connection, requests) = fixture(concat!(
            "{\"id\":1,\"result\":{}}\n",
            "{\"id\":2,\"result\":{\"thread\":{\"id\":\"thread-1\"}}}\n",
            "{\"method\":\"turn/started\",\"params\":{\"threadId\":\"thread-1\",\"turn\":{\"id\":\"turn-1\"}}}\n",
            "{\"id\":\"approval\",\"method\":\"item/requestApproval\"}\n",
            "{\"method\":\"item/completed\",\"params\":{\"threadId\":\"thread-1\",\"item\":{\"type\":\"agentMessage\",\"text\":\"The introduction is ready.\"}}}\n",
            "{\"method\":\"item/completed\",\"params\":{\"threadId\":\"other-thread\",\"item\":{\"type\":\"agentMessage\",\"text\":\"Other workspace.\"}}}\n",
            "{\"method\":\"turn/completed\",\"params\":{\"threadId\":\"thread-1\",\"turn\":{\"id\":\"turn-1\",\"status\":\"completed\"}}}\n",
            "{\"id\":3,\"result\":{\"turn\":{\"id\":\"turn-1\"}}}\n",
            "{\"method\":\"thread/closed\",\"params\":{\"threadId\":\"thread-1\"}}\n",
            "{\"id\":5,\"result\":{}}\n",
        ));
        let mut session = None;
        let mut updates = Vec::new();
        let text = run_connection(
            &connection,
            "test-project",
            request(),
            &AtomicBool::new(false),
            &mut session,
            &mut |update| {
                updates.push(match update {
                    CodexUpdate::Session(id) => id.to_owned(),
                    CodexUpdate::Progress(status) => status.to_owned(),
                })
            },
        )
        .unwrap();
        assert_eq!(session.as_deref(), Some("thread-1"));
        assert_eq!(text, "The introduction is ready.");
        assert_eq!(updates, ["thread-1", "Thinking…"]);
        assert_eq!(
            methods(requests),
            [
                "initialize",
                "initialized",
                "thread/start",
                "turn/start",
                "deny",
                "thread/unsubscribe"
            ]
        );
    }

    #[test]
    fn interrupts_an_active_turn_and_keeps_its_session() {
        let (connection, requests) = fixture(concat!(
            "{\"id\":1,\"result\":{}}\n",
            "{\"id\":2,\"result\":{\"thread\":{\"id\":\"thread-1\"}}}\n",
            "{\"method\":\"turn/started\",\"params\":{\"threadId\":\"thread-1\",\"turn\":{\"id\":\"turn-1\"}}}\n",
            "{\"id\":3,\"result\":{\"turn\":{\"id\":\"turn-1\"}}}\n",
            "{\"id\":4,\"result\":{}}\n",
            "{\"method\":\"turn/completed\",\"params\":{\"threadId\":\"thread-1\",\"turn\":{\"id\":\"turn-1\",\"status\":\"interrupted\"}}}\n",
            "{\"id\":5,\"result\":{\"status\":\"notLoaded\"}}\n",
        ));
        let cancel = AtomicBool::new(false);
        let mut session = None;
        let error = run_connection(
            &connection,
            "test-project",
            request(),
            &cancel,
            &mut session,
            &mut |update| {
                if matches!(update, CodexUpdate::Progress("Thinking…")) {
                    cancel.store(true, Ordering::Relaxed);
                }
            },
        )
        .unwrap_err();
        assert_eq!(session.as_deref(), Some("thread-1"));
        assert_eq!(error, CANCELLED);
        assert!(methods(requests).contains(&"turn/interrupt".to_owned()));
    }

    #[test]
    fn pre_cancelled_run_does_not_start_the_provider() {
        let mut request = request();
        request.session_id = Some("saved-session");
        let error = run_codex(
            Command::new("missing-test-assistant"),
            request,
            &AtomicBool::new(true),
            |_| panic!("no provider updates expected"),
        )
        .unwrap_err();
        assert_eq!(error.session_id.as_deref(), Some("saved-session"));
        assert_eq!(error.message, CANCELLED);
    }
}

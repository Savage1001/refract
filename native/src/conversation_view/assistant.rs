use super::{
    ConversationPane, Library, Palette, chat_button,
    models::{CHECKING_MODELS, ModelSelection},
};
use gpui::{Context, FocusHandle, Window, div, prelude::*, px, rgb, svg};
use gpui_component::tooltip::Tooltip;
use refract_native::{
    Project, ProjectEntry,
    assistant_launch::{CodexCli, OpenCodeCli, discover_codex_cli, discover_opencode_cli},
    assistant_preferences::AssistantConnection,
    assistant_protocol::{CodexThreadOptions, CodexToolConnection},
    assistant_runtime::{
        CodexTurnRequest, CodexUpdate,
        opencode::{OpenCodeToolConnection, OpenCodeTurnRequest, run_opencode},
        run_codex,
    },
    assistant_tools::{AssistantToolServer, AssistantTools, claims_successful_build},
    codex_connection::CodexMode,
    conversation::{
        Attachment, Conversation, ConversationDraft, ConversationId, Message, Provider, StartTurn,
        begin_conversation_turn,
    },
    project_tree,
    versions::{Version, list_versions, snapshot_project},
};
use std::{
    io,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone)]
pub(super) enum InstalledProvider {
    Codex(CodexCli),
    #[cfg(windows)]
    CodexAccount(Arc<refract_native::codex_account::CodexAccount>),
    #[cfg(not(windows))]
    CodexAccount,
    OpenCode(OpenCodeCli),
}

pub(super) enum PreparedProvider<'a> {
    Codex(CodexBackend<'a>),
    OpenCode(&'a OpenCodeCli),
}

pub(super) enum CodexBackend<'a> {
    Cli(&'a CodexCli),
    #[cfg(windows)]
    Account(refract_native::codex_account::CodexConnection),
}

impl CodexBackend<'_> {
    pub(super) fn key(&self) -> Option<&str> {
        match self {
            Self::Cli(_) => None,
            #[cfg(windows)]
            Self::Account(connection) => Some(connection.account_key()),
        }
    }

    pub(super) fn command(&self, cwd: &Path) -> io::Result<Command> {
        match self {
            Self::Cli(cli) => Ok(cli.app_server_command(cwd)),
            #[cfg(windows)]
            Self::Account(connection) => connection.app_server_command(cwd),
        }
    }
}

impl InstalledProvider {
    pub(super) fn kind(&self) -> Provider {
        match self {
            Self::Codex(_) => Provider::Codex,
            #[cfg(windows)]
            Self::CodexAccount(_) => Provider::Codex,
            #[cfg(not(windows))]
            Self::CodexAccount => Provider::Codex,
            Self::OpenCode(_) => Provider::OpenCode,
        }
    }

    pub(super) fn mode(&self) -> AssistantConnection {
        match self {
            Self::Codex(_) => AssistantConnection::CodexCli,
            #[cfg(windows)]
            Self::CodexAccount(_) => AssistantConnection::CodexAccount,
            #[cfg(not(windows))]
            Self::CodexAccount => AssistantConnection::CodexAccount,
            Self::OpenCode(_) => AssistantConnection::OpenCode,
        }
    }

    pub(super) fn prepare(&self, cancel: &AtomicBool) -> io::Result<PreparedProvider<'_>> {
        #[cfg(not(windows))]
        let _ = cancel;
        match self {
            Self::Codex(cli) => Ok(PreparedProvider::Codex(CodexBackend::Cli(cli))),
            #[cfg(windows)]
            Self::CodexAccount(account) => account
                .connection(cancel)
                .map(CodexBackend::Account)
                .map(PreparedProvider::Codex),
            #[cfg(not(windows))]
            Self::CodexAccount => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                refract_native::managed_codex::UNSUPPORTED_PLATFORM,
            )),
            Self::OpenCode(cli) => Ok(PreparedProvider::OpenCode(cli)),
        }
    }
}

pub(super) fn installed_providers(
    profile: &Path,
    #[cfg(windows)] account: Arc<refract_native::codex_account::CodexAccount>,
) -> Vec<InstalledProvider> {
    let mut providers = Vec::new();
    match CodexMode::load(profile) {
        CodexMode::Account => {
            #[cfg(windows)]
            providers.push(InstalledProvider::CodexAccount(account));
            #[cfg(not(windows))]
            providers.push(InstalledProvider::CodexAccount);
        }
        CodexMode::Cli => {
            if let Ok(cli) = discover_codex_cli() {
                providers.push(InstalledProvider::Codex(cli));
            }
        }
    }
    if let Ok(cli) = discover_opencode_cli() {
        providers.push(InstalledProvider::OpenCode(cli));
    }
    providers
}

pub(super) struct RunningTurn {
    cancel: Arc<AtomicBool>,
    progress: &'static str,
    stop_focus: FocusHandle,
    pub(super) pending: Option<PendingPrompt>,
}

#[derive(Clone)]
pub(super) struct PendingPrompt {
    text: String,
    attachments: Vec<Attachment>,
}

impl PendingPrompt {
    pub(super) fn prepend_to(self, draft: &mut ConversationDraft) {
        draft.text = if draft.text.is_empty() {
            self.text
        } else {
            format!("{}\n\n{}", self.text, draft.text)
        };
        let mut attachments = self.attachments;
        attachments.append(&mut draft.attachments);
        draft.attachments = attachments;
    }
}

fn check_turn_cancellation(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Acquire) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Message stopped before sending.",
        ));
    }
    Ok(())
}

enum TurnNotice {
    Saved,
    Progress(&'static str),
}

struct TurnWork {
    projects: PathBuf,
    project: Project,
    id: ConversationId,
    provider: InstalledProvider,
    model_selection: ModelSelection,
    prompt: PendingPrompt,
}

struct CompletedTurn {
    history: io::Result<Conversation>,
    tree: io::Result<Vec<ProjectEntry>>,
    versions: io::Result<Vec<Version>>,
    checkpoint_error: Option<io::Error>,
    project: Project,
}

fn run_turn(
    work: TurnWork,
    cancel: &Arc<AtomicBool>,
    notices: async_channel::Sender<TurnNotice>,
) -> io::Result<CompletedTurn> {
    check_turn_cancellation(cancel)?;
    let prepared = work.provider.prepare(cancel)?;
    check_turn_cancellation(cancel)?;
    let account_key = match &prepared {
        PreparedProvider::Codex(codex) => codex.key(),
        PreparedProvider::OpenCode(_) => None,
    };
    snapshot_project(&work.projects, &work.project)?;
    check_turn_cancellation(cancel)?;
    let tools = AssistantTools::new(
        work.projects.clone(),
        work.project.clone(),
        work.id.clone(),
        cancel.clone(),
    );
    let engine_instructions = tools.engine_instructions();
    let instructions = tools.instructions();
    let successful_build = tools.build_status();
    let server = AssistantToolServer::start(tools)?;
    check_turn_cancellation(cancel)?;
    let (mut turn, context) = begin_conversation_turn(
        &work.projects,
        &work.project,
        &work.id,
        StartTurn {
            provider: work.provider.kind(),
            prompt: &work.prompt.text,
            attachments: &work.prompt.attachments,
            codex_account: account_key,
            build_instructions: Some(&instructions),
        },
    )?;
    let _ = notices.try_send(TurnNotice::Saved);
    let mut checkpoint_error = None;
    let mut update = |update: CodexUpdate<'_>| match update {
        CodexUpdate::Session(session) => {
            if let Err(error) = turn.checkpoint_session(session) {
                checkpoint_error = Some(error);
            }
        }
        CodexUpdate::Progress(progress) => {
            let _ = notices.try_send(TurnNotice::Progress(progress));
        }
    };
    let mut execute = |prompt: &str, session_id: Option<&str>, images: &[PathBuf]| match &prepared {
        PreparedProvider::Codex(codex) => {
            let command = match codex.command(&context.workspace) {
                Ok(command) => command,
                Err(error) => return (Err(error.to_string()), None),
            };
            match run_codex(
                command,
                CodexTurnRequest {
                    account_connection: account_key.is_some(),
                    session_id,
                    prompt,
                    images,
                    options: CodexThreadOptions {
                        model: Some(work.model_selection.model.as_str()),
                        reasoning_effort: work.model_selection.effort.as_deref(),
                        fast_mode: work.model_selection.fast_mode,
                        developer_instructions: Some(&engine_instructions),
                        tools: Some(CodexToolConnection {
                            url: server.url(),
                            token: server.token(),
                        }),
                    },
                },
                cancel,
                &mut update,
            ) {
                Ok(reply) => (Ok(reply.text), Some(reply.session_id)),
                Err(error) => (Err(error.message), error.session_id),
            }
        }
        PreparedProvider::OpenCode(cli) => {
            match run_opencode(
                cli,
                OpenCodeTurnRequest {
                    cwd: &context.workspace,
                    session_id,
                    prompt,
                    images,
                    model: Some(work.model_selection.model.as_str()),
                    variant: work.model_selection.effort.as_deref(),
                    tools: Some(OpenCodeToolConnection {
                        url: server.url(),
                        token: server.token(),
                    }),
                },
                cancel,
                &mut update,
            ) {
                Ok(reply) => (Ok(reply.text), reply.session_id),
                Err(error) => (Err(error.message), error.session_id),
            }
        }
    };
    let (mut result, mut session) = execute(
        &context.prompt,
        context.session_id.as_deref(),
        &context.images,
    );
    let unverified_claim = |result: &Result<String, String>| {
        !successful_build.load(Ordering::Acquire)
            && result.as_deref().is_ok_and(claims_successful_build)
    };
    if work.provider.mode() == AssistantConnection::CodexCli
        && !cancel.load(Ordering::Acquire)
        && unverified_claim(&result)
    {
        if let Some(reference) = session.as_deref() {
            let (corrected, updated_session) = execute(
                "Refract recorded no successful build for this turn. If the user requested compilation, call the Refract build tool and report its actual result. Otherwise correct the claim without building.",
                Some(reference),
                &[],
            );
            result = corrected;
            if updated_session.is_some() {
                session = updated_session;
            }
        }
        if unverified_claim(&result) {
            result = Ok("I couldn’t verify a successful Refract build for this turn.".into());
        }
    }
    let history = turn.finish(
        result.as_deref().map_err(String::as_str),
        session.as_deref(),
    );
    let project = server.finish().unwrap_or(work.project);
    let tree = project_tree(&work.projects, &project);
    let versions = snapshot_project(&work.projects, &project)
        .and_then(|_| list_versions(&work.projects, &project));
    Ok(CompletedTurn {
        history,
        tree,
        versions,
        checkpoint_error,
        project,
    })
}

impl Library {
    fn cancel_assistant_turn(&mut self, cx: &mut Context<Self>) {
        if let Some(assistant) = self
            .selection
            .as_ref()
            .and_then(|selection| selection.conversation.running.as_ref())
        {
            if !assistant.cancel.swap(true, Ordering::AcqRel) {
                cx.notify();
            }
        }
    }

    pub(super) fn send_assistant_turn(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.project_writes_idle() || self.navigation_after_save.is_some() {
            return;
        }
        let has_edits = self.has_unsaved_changes(&super::super::Navigation::Projects, cx);
        let Some(selection) = &mut self.selection else {
            return;
        };
        let pane = &mut selection.conversation;
        let Some(composer) = pane.composer.as_mut() else {
            return;
        };
        let Some(provider) = pane.provider.and_then(|kind| {
            pane.providers
                .iter()
                .find(|provider| provider.kind() == kind)
        }) else {
            return;
        };
        let Some(model_selection) = pane
            .models
            .get(&provider.kind())
            .and_then(|picker| picker.read(cx).selection())
        else {
            return;
        };
        let prompt = composer.input.read(cx).value().to_string();
        if prompt.trim().is_empty() || pane.load_task.is_some() {
            return;
        }
        if has_edits {
            self.save_active_file(window, cx);
            if let Some(selection) = &mut self.selection {
                selection.conversation.error =
                    Some("Save pending source changes before sending.".into());
                cx.notify();
            }
            return;
        }
        let pending = PendingPrompt {
            text: prompt,
            attachments: std::mem::take(&mut composer.attachments),
        };
        let work = TurnWork {
            projects: self.projects_dir.clone(),
            project: selection.project.clone(),
            id: pane.active.clone(),
            provider: provider.clone(),
            model_selection,
            prompt: pending.clone(),
        };
        let sequence = self.project_open_sequence;
        let cancel = Arc::new(AtomicBool::new(false));
        let (notify, events) = async_channel::bounded(8);
        let background_cancel = cancel.clone();
        pane.error = None;
        pane.running = Some(RunningTurn {
            cancel,
            progress: "Preparing…",
            stop_focus: cx.focus_handle(),
            pending: Some(pending),
        });
        composer
            .input
            .update(cx, |input, cx| input.set_value("", window, cx));
        composer.dirty = true;
        self.flush_chat_draft(cx);
        let run = cx
            .background_executor()
            .spawn(async move { run_turn(work, &background_cancel, notify) });
        self.entry_write_task = Some(cx.spawn_in(window, async move |library, cx| {
            while let Ok(notice) = events.recv().await {
                let _ = library.update_in(cx, |library, _, cx| {
                    if library.project_open_sequence != sequence {
                        return;
                    }
                    let Some(selection) = &mut library.selection else {
                        return;
                    };
                    let pane = &mut selection.conversation;
                    match notice {
                        TurnNotice::Saved => {
                            if let Some(sent) =
                                pane.running.as_mut().and_then(|turn| turn.pending.take())
                            {
                                if let Some(history) = &mut pane.history {
                                    history.messages.push(Message::User {
                                        text: sent.text,
                                        attachments: Some(sent.attachments),
                                    });
                                    pane.scroll
                                        .scroll_to_top_of_item(history.messages.len() - 1);
                                }
                                if let Some(composer) = &mut pane.composer {
                                    composer.dirty = true;
                                }
                                library.flush_chat_draft(cx);
                            }
                        }
                        TurnNotice::Progress(progress) => {
                            if let Some(running) = &mut pane.running {
                                running.progress = progress;
                            }
                        }
                    }
                    cx.notify();
                });
            }
            let result = run.await;
            let _ = library.update_in(cx, |library, window, cx| {
                library.entry_write_task = None;
                if library.project_open_sequence == sequence {
                    library.complete_assistant_turn(result, window, cx);
                }
                library.start_next_save(window, cx);
                if library.snapshot_timer.is_none() {
                    library.start_snapshot(window, cx);
                }
                library.resume_navigation(window, cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn complete_assistant_turn(
        &mut self,
        result: io::Result<CompletedTurn>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(selection) = &mut self.selection else {
            return;
        };
        let pane = &mut selection.conversation;
        let mut running = pane.running.take();
        if running
            .as_ref()
            .is_some_and(|turn| turn.stop_focus.is_focused(window))
            && let Some(composer) = &pane.composer
        {
            composer.focus(window, cx);
        }
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                let stopped = running.as_ref().is_some_and(|turn| {
                    turn.pending.is_some() && turn.cancel.load(Ordering::Acquire)
                });
                if let Some(pending) = running.as_mut().and_then(|turn| turn.pending.take())
                    && let Some(composer) = &mut pane.composer
                {
                    let mut draft = composer.draft(pane.provider, cx);
                    pending.prepend_to(&mut draft);
                    composer
                        .input
                        .update(cx, |input, cx| input.set_value(draft.text, window, cx));
                    composer.attachments = draft.attachments;
                    composer.dirty = true;
                }
                pane.error = Some(if stopped {
                    "Message stopped before sending.".into()
                } else {
                    format!("Couldn’t send message. {error}")
                });
                self.flush_chat_draft(cx);
                return;
            }
        };
        match result.history {
            Ok(history) => pane.show_history(history),
            Err(error) => pane.error = Some(format!("Couldn’t save the conversation. {error}")),
        }
        if let Some(error) = result.checkpoint_error {
            pane.error = Some(format!("Couldn’t save the assistant session. {error}"));
        }
        selection.tree = Some(result.tree);
        selection.versions = Some(result.versions);
        selection.project = result.project;
        selection.build.invalidate();
        for tab in &mut selection.tabs {
            tab.editor = None;
        }
        let file = selection.file.clone();
        let refresh_chats = pane.sidebar_open;
        if let Some(file) = file {
            self.open_file(file, window, cx);
        }
        if refresh_chats {
            self.refresh_conversation_list(cx);
        }
    }

    pub(super) fn assistant_progress(
        &self,
        pane: &ConversationPane,
        palette: Palette,
    ) -> Option<impl IntoElement> {
        pane.running.as_ref().map(|running| {
            div()
                .text_size(px(12.))
                .text_color(rgb(palette.muted))
                .child(if running.cancel.load(Ordering::Acquire) {
                    "Stopping…"
                } else {
                    running.progress
                })
        })
    }

    pub(super) fn assistant_controls(
        &self,
        pane: &ConversationPane,
        can_send: bool,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let provider = pane.provider.and_then(|kind| {
            pane.providers
                .iter()
                .find(|provider| provider.kind() == kind)
        });
        let actions = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(self.model_picker_button(pane, palette, cx));
        if let Some(running) = &pane.running {
            let stopping = running.cancel.load(Ordering::Acquire);
            actions.child(
                chat_button("stop-chat", palette, cx, |library, _, cx| {
                    library.cancel_assistant_turn(cx)
                })
                .track_focus(&running.stop_focus)
                .size(px(30.))
                .flex()
                .items_center()
                .justify_center()
                .when(stopping, |button| button.cursor_default().opacity(0.5))
                .tooltip(move |window, cx| {
                    Tooltip::new(if stopping {
                        "Stopping…"
                    } else {
                        "Stop response"
                    })
                    .build(window, cx)
                })
                .child(div().size(px(10.)).rounded(px(2.)).bg(rgb(palette.muted))),
            )
        } else {
            let model_issue =
                provider.and_then(|provider| match pane.models.get(&provider.kind()) {
                    Some(picker) => picker.read(cx).send_issue(),
                    None => Some(CHECKING_MODELS),
                });
            let enabled = can_send
                && provider.is_some()
                && pane.load_task.is_none()
                && model_issue.is_none()
                && self.project_writes_idle();
            actions.child(
                chat_button("send", palette, cx, move |library, window, cx| {
                    if enabled {
                        library.send_assistant_turn(window, cx);
                    }
                })
                .size(px(36.))
                .flex()
                .items_center()
                .justify_center()
                .tab_stop(enabled)
                .when(!enabled, |button| button.cursor_default().opacity(0.5))
                .tooltip(move |window, cx| {
                    Tooltip::new(model_issue.unwrap_or("Send")).build(window, cx)
                })
                .child(
                    svg()
                        .path("refract/send.svg")
                        .size(px(16.))
                        .text_color(rgb(palette.muted)),
                ),
            )
        }
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use crate::{Root, initialize_app};
    use gpui::{Focusable, TestAppContext};
    use refract_native::{conversation::load_conversation_draft, create_project};

    #[gpui::test]
    fn keyboard_stop_keeps_the_unsent_draft_and_returns_focus(cx: &mut TestAppContext) {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let project = create_project(&projects, "Paper").unwrap();
        cx.update(initialize_app);
        let mut library = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| Library::new(projects.clone(), window, cx));
            library = Some(view.clone());
            Root::new(view, window, cx)
        });
        let library = library.unwrap();
        cx.update(|window, app| {
            library.update(app, |view, cx| {
                view.open_project(project.clone(), window, cx)
            });
        });
        cx.run_until_parked();

        let cancel = Arc::new(AtomicBool::new(false));
        cx.update(|window, app| {
            library.update(app, |view, cx| {
                let pane = &mut view.selection.as_mut().unwrap().conversation;
                let stop_focus = cx.focus_handle();
                pane.running = Some(RunningTurn {
                    cancel: cancel.clone(),
                    progress: "Preparing…",
                    stop_focus,
                    pending: Some(PendingPrompt {
                        text: "First prompt".into(),
                        attachments: Vec::new(),
                    }),
                });
                pane.composer.as_ref().unwrap().focus(window, cx);
                cx.notify();
            });
        });
        cx.simulate_input("Second draft");
        cx.update(|window, app| {
            library.update(app, |view, cx| {
                assert!(view.flush_chat_draft(cx));
                let pane = &view.selection.as_ref().unwrap().conversation;
                assert_eq!(
                    pane.composer
                        .as_ref()
                        .unwrap()
                        .input
                        .read(cx)
                        .value()
                        .as_ref(),
                    "Second draft"
                );
                window.focus(&pane.running.as_ref().unwrap().stop_focus);
            });
        });
        let draft =
            load_conversation_draft(&projects, &project, &ConversationId::default()).unwrap();
        assert_eq!(draft.text, "First prompt\n\nSecond draft");
        cx.simulate_keystrokes("enter");
        assert!(cancel.load(Ordering::Acquire));
        cx.update(|window, app| {
            library.update(app, |view, cx| {
                view.complete_assistant_turn(
                    Err(check_turn_cancellation(&cancel).unwrap_err()),
                    window,
                    cx,
                );
                let pane = &view.selection.as_ref().unwrap().conversation;
                let input = pane.composer.as_ref().unwrap().input.read(cx);
                assert_eq!(input.value().as_ref(), "First prompt\n\nSecond draft");
                assert_eq!(
                    pane.error.as_deref(),
                    Some("Message stopped before sending.")
                );
                assert!(input.focus_handle(cx).is_focused(window));
            });
        });
    }
}

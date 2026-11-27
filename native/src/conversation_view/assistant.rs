use super::{ConversationPane, Library, Palette, chat_button};
use gpui::{Context, Entity, SharedString, Subscription, Window, div, prelude::*, px, rgb, svg};
use gpui_component::{
    Sizable,
    select::{Select, SelectEvent, SelectItem, SelectState},
    tooltip::Tooltip,
};
use refract_native::{
    Project, ProjectEntry,
    assistant_launch::{CodexCli, OpenCodeCli, discover_codex_cli, discover_opencode_cli},
    assistant_preferences::{AssistantConnection, load_assistant_preferences},
    assistant_protocol::CodexThreadOptions,
    assistant_runtime::{
        CodexTurnRequest, CodexUpdate,
        opencode::{OpenCodeTurnRequest, run_opencode},
        run_codex,
    },
    conversation::{
        Attachment, Conversation, ConversationId, Message, Provider, StartTurn,
        begin_conversation_turn,
    },
    project_tree,
    versions::{Version, list_versions, snapshot_project},
};
use std::{
    io,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone)]
pub(super) enum InstalledProvider {
    Codex(CodexCli),
    OpenCode(OpenCodeCli),
}

impl InstalledProvider {
    pub(super) fn kind(&self) -> Provider {
        match self {
            Self::Codex(_) => Provider::Codex,
            Self::OpenCode(_) => Provider::OpenCode,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Codex(_) => "Codex",
            Self::OpenCode(_) => "OpenCode",
        }
    }
}

pub(super) fn installed_providers() -> Vec<InstalledProvider> {
    let mut providers = Vec::new();
    if let Ok(cli) = discover_codex_cli() {
        providers.push(InstalledProvider::Codex(cli));
    }
    if let Ok(cli) = discover_opencode_cli() {
        providers.push(InstalledProvider::OpenCode(cli));
    }
    providers
}

pub(super) struct RunningTurn {
    cancel: Arc<AtomicBool>,
    progress: &'static str,
}

#[derive(Clone)]
struct ProviderOption {
    kind: Provider,
    name: &'static str,
}

impl SelectItem for ProviderOption {
    type Value = Provider;

    fn title(&self) -> SharedString {
        self.name.into()
    }

    fn value(&self) -> &Provider {
        &self.kind
    }
}

pub(super) struct ProviderPicker {
    state: Entity<SelectState<Vec<ProviderOption>>>,
    _selected: Subscription,
}

impl ProviderPicker {
    pub(super) fn new(
        providers: &[InstalledProvider],
        selected: Option<Provider>,
        window: &mut Window,
        cx: &mut Context<Library>,
    ) -> Option<Self> {
        if providers.len() < 2 {
            return None;
        }
        let choices: Vec<_> = providers
            .iter()
            .map(|provider| ProviderOption {
                kind: provider.kind(),
                name: provider.label(),
            })
            .collect();
        let state = cx.new(|cx| {
            let mut state = SelectState::new(choices, None, window, cx);
            if let Some(selected) = selected {
                state.set_selected_value(&selected, window, cx);
            }
            state
        });
        let selected = cx.subscribe(
            &state,
            |library, source, event: &SelectEvent<Vec<ProviderOption>>, cx| {
                let SelectEvent::Confirm(Some(provider)) = event else {
                    return;
                };
                let Some(pane) = library
                    .selection
                    .as_mut()
                    .map(|selection| &mut selection.conversation)
                else {
                    return;
                };
                if pane
                    .picker
                    .as_ref()
                    .is_some_and(|picker| picker.state == source)
                {
                    pane.provider = Some(*provider);
                    library.schedule_chat_draft_save(cx);
                }
                cx.notify();
            },
        );
        Some(Self {
            state,
            _selected: selected,
        })
    }
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
    prompt: String,
    attachments: Vec<Attachment>,
}

struct CompletedTurn {
    history: io::Result<Conversation>,
    tree: io::Result<Vec<ProjectEntry>>,
    versions: io::Result<Vec<Version>>,
    checkpoint_error: Option<io::Error>,
}

fn run_turn(
    work: TurnWork,
    cancel: &AtomicBool,
    notices: async_channel::Sender<TurnNotice>,
) -> io::Result<CompletedTurn> {
    let mode = match work.provider.kind() {
        Provider::Codex => AssistantConnection::CodexCli,
        Provider::OpenCode => AssistantConnection::OpenCode,
    };
    let profile = work.projects.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Couldn’t locate the Refract profile",
        )
    })?;
    let preferences = load_assistant_preferences(profile, mode)?;
    snapshot_project(&work.projects, &work.project)?;
    let (mut turn, context) = begin_conversation_turn(
        &work.projects,
        &work.project,
        &work.id,
        StartTurn {
            provider: work.provider.kind(),
            prompt: &work.prompt,
            attachments: &work.attachments,
            codex_account: None,
            build_instructions: None,
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
    let (result, session) = match work.provider {
        InstalledProvider::Codex(cli) => {
            match run_codex(
                cli.app_server_command(&context.workspace),
                CodexTurnRequest {
                    account_connection: false,
                    session_id: context.session_id.as_deref(),
                    prompt: &context.prompt,
                    images: &context.images,
                    options: CodexThreadOptions {
                        model: preferences.model.as_deref(),
                        reasoning_effort: preferences.reasoning_effort.as_deref(),
                        fast_mode: preferences.fast_mode,
                        ..Default::default()
                    },
                },
                cancel,
                &mut update,
            ) {
                Ok(reply) => (Ok(reply.text), Some(reply.session_id)),
                Err(error) => (Err(error.message), error.session_id),
            }
        }
        InstalledProvider::OpenCode(cli) => {
            match run_opencode(
                &cli,
                OpenCodeTurnRequest {
                    cwd: &context.workspace,
                    session_id: context.session_id.as_deref(),
                    prompt: &context.prompt,
                    images: &context.images,
                    model: preferences.model.as_deref(),
                    variant: preferences.reasoning_effort.as_deref(),
                    tools: None,
                },
                cancel,
                &mut update,
            ) {
                Ok(reply) => (Ok(reply.text), reply.session_id),
                Err(error) => (Err(error.message), error.session_id),
            }
        }
    };
    let history = turn.finish(
        result.as_deref().map_err(String::as_str),
        session.as_deref(),
    );
    let tree = project_tree(&work.projects, &work.project);
    let versions = snapshot_project(&work.projects, &work.project)
        .and_then(|_| list_versions(&work.projects, &work.project));
    Ok(CompletedTurn {
        history,
        tree,
        versions,
        checkpoint_error,
    })
}

impl Library {
    pub(crate) fn cancel_assistant_turn(&mut self) {
        if let Some(assistant) = self
            .selection
            .as_ref()
            .and_then(|selection| selection.conversation.running.as_ref())
        {
            assistant.cancel.store(true, Ordering::Release);
        }
    }

    pub(super) fn send_assistant_turn(&mut self, window: &Window, cx: &mut Context<Self>) {
        if !self.project_writes_idle() || self.navigation_after_save.is_some() {
            return;
        }
        let has_edits = self.has_unsaved_changes(&super::super::Navigation::Projects, cx);
        let Some(selection) = &mut self.selection else {
            return;
        };
        let pane = &mut selection.conversation;
        let Some(composer) = pane.composer.as_ref() else {
            return;
        };
        let Some(provider) = pane.provider.and_then(|kind| {
            pane.providers
                .iter()
                .find(|provider| provider.kind() == kind)
        }) else {
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
        let work = TurnWork {
            projects: self.projects_dir.clone(),
            project: selection.project.clone(),
            id: pane.active.clone(),
            provider: provider.clone(),
            prompt,
            attachments: composer.attachments.clone(),
        };
        let sequence = self.project_open_sequence;
        let cancel = Arc::new(AtomicBool::new(false));
        let (notify, events) = async_channel::bounded(8);
        let background_cancel = cancel.clone();
        let run = cx
            .background_executor()
            .spawn(async move { run_turn(work, &background_cancel, notify) });
        pane.error = None;
        pane.running = Some(RunningTurn {
            cancel,
            progress: "Preparing…",
        });
        self.entry_write_task = Some(cx.spawn_in(window, async move |library, cx| {
            while let Ok(notice) = events.recv().await {
                let _ = library.update_in(cx, |library, window, cx| {
                    if library.project_open_sequence != sequence {
                        return;
                    }
                    let Some(selection) = &mut library.selection else {
                        return;
                    };
                    let pane = &mut selection.conversation;
                    match notice {
                        TurnNotice::Saved => {
                            if let Some(composer) = &mut pane.composer {
                                if let Some(history) = &mut pane.history {
                                    history.messages.push(Message::User {
                                        text: composer.input.read(cx).value().to_string(),
                                        attachments: Some(composer.attachments.clone()),
                                    });
                                    pane.scroll
                                        .scroll_to_top_of_item(history.messages.len() - 1);
                                }
                                composer
                                    .input
                                    .update(cx, |input, cx| input.set_value("", window, cx));
                                composer.attachments.clear();
                                composer.dirty = true;
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
        pane.running = None;
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                pane.error = Some(format!("Couldn’t send message. {error}"));
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
                .child(running.progress)
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
        let mut actions = div().flex().items_center().gap(px(6.));
        if let Some(picker) = &pane.picker {
            actions = actions.child(
                Select::new(&picker.state)
                    .small()
                    .disabled(pane.running.is_some()),
            );
        } else if let Some(provider) = provider {
            actions = actions.child(div().px_2().text_size(px(12.)).child(provider.label()));
        }
        if pane.running.is_some() {
            actions.child(
                chat_button("stop-chat", palette, cx, |library, _, _| {
                    library.cancel_assistant_turn()
                })
                .size(px(30.))
                .flex()
                .items_center()
                .justify_center()
                .tooltip(|window, cx| Tooltip::new("Stop response").build(window, cx))
                .child(div().size(px(10.)).rounded(px(2.)).bg(rgb(palette.muted))),
            )
        } else {
            let enabled = can_send
                && provider.is_some()
                && pane.load_task.is_none()
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
                .tooltip(|window, cx| Tooltip::new("Send").build(window, cx))
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

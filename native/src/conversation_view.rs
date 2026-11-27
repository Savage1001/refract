use super::{Library, Palette, Selection};
use gpui::{
    Context, FocusHandle, Focusable, FontWeight, KeyDownEvent, ScrollHandle, SharedString,
    Subscription, Task, Window, div, prelude::*, px, rgb, svg,
};
use gpui_component::{
    input::{Input, InputEvent, InputState, SelectAll},
    tooltip::Tooltip,
};
use refract_native::conversation::{
    Conversation, ConversationId, ConversationSummary, Message, create_conversation,
    list_conversations, load_conversation, rename_conversation,
};
use std::{collections::HashMap, io};

#[derive(Default)]
pub(super) struct ConversationPane {
    pub(super) sidebar_open: bool,
    active: ConversationId,
    history: Option<Conversation>,
    rows: Option<io::Result<Vec<ConversationRow>>>,
    load_task: Option<Task<()>>,
    list_task: Option<Task<()>>,
    scroll: ScrollHandle,
    welcome_dismissed: bool,
    draft: Option<NameDraft>,
    error: Option<String>,
}

struct ConversationRow {
    summary: ConversationSummary,
    rename_focus: FocusHandle,
}

struct NameDraft {
    id: ConversationId,
    title: String,
    input: gpui::Entity<InputState>,
    rename_focus: FocusHandle,
    _events: Subscription,
}

enum Change {
    Create,
    Rename { id: ConversationId, title: String },
}

impl ConversationPane {
    fn display(&mut self, id: ConversationId, history: Conversation) {
        self.active = id;
        self.scroll = ScrollHandle::new();
        if let Some(index) = history
            .messages
            .iter()
            .rposition(|message| matches!(message, Message::User { .. }))
        {
            self.scroll.scroll_to_top_of_item(index);
        } else {
            self.scroll.scroll_to_bottom();
        }
        self.history = Some(history);
    }

    fn set_rows(
        &mut self,
        result: io::Result<Vec<ConversationSummary>>,
        cx: &mut Context<Library>,
    ) {
        let mut previous: HashMap<_, _> = self
            .rows
            .take()
            .and_then(Result::ok)
            .unwrap_or_default()
            .into_iter()
            .map(|row| (row.summary.id, row.rename_focus))
            .collect();
        self.rows = Some(result.map(|rows| {
            rows.into_iter()
                .map(|summary| {
                    let rename_focus = previous
                        .remove(&summary.id)
                        .unwrap_or_else(|| cx.focus_handle().tab_stop(true).tab_index(1));
                    ConversationRow {
                        summary,
                        rename_focus,
                    }
                })
                .collect()
        }));
    }
}

impl Library {
    pub(super) fn load_project_conversation(&mut self, id: ConversationId, cx: &mut Context<Self>) {
        let Some(selection) = &mut self.selection else {
            return;
        };
        let sequence = self.project_open_sequence;
        let project = selection.project.clone();
        let path = self.projects_dir.clone();
        selection.conversation.error = None;
        let load = cx.background_executor().spawn(async move {
            let history = load_conversation(&path, &project, &id);
            (id, history)
        });
        selection.conversation.load_task = Some(cx.spawn(async move |library, cx| {
            let (id, result) = load.await;
            let _ = library.update(cx, |library, cx| {
                if library.project_open_sequence == sequence
                    && let Some(selection) = &mut library.selection
                {
                    let pane = &mut selection.conversation;
                    pane.load_task = None;
                    match result {
                        Ok(history) => pane.display(id, history),
                        Err(error) => pane.error = Some(format!("Couldn’t open chat. {error}")),
                    }
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    fn refresh_conversation_list(&mut self, cx: &mut Context<Self>) {
        let Some(selection) = &mut self.selection else {
            return;
        };
        let sequence = self.project_open_sequence;
        let project = selection.project.clone();
        let path = self.projects_dir.clone();
        let list = cx
            .background_executor()
            .spawn(async move { list_conversations(&path, &project) });
        selection.conversation.list_task = Some(cx.spawn(async move |library, cx| {
            let result = list.await;
            let _ = library.update(cx, |library, cx| {
                if library.project_open_sequence == sequence
                    && let Some(selection) = &mut library.selection
                {
                    selection.conversation.list_task = None;
                    selection.conversation.set_rows(result, cx);
                    cx.notify();
                }
            });
        }));
    }

    fn switch_project_sidebar(&mut self, chats: bool, cx: &mut Context<Self>) {
        if let Some(selection) = &mut self.selection {
            selection.conversation.sidebar_open = chats;
            if chats {
                self.refresh_conversation_list(cx);
            }
            cx.notify();
        }
    }

    fn change_conversation(&mut self, change: Change, window: &Window, cx: &mut Context<Self>) {
        let writable = self.project_writes_ready() && self.navigation_after_save.is_none();
        let Some(selection) = &mut self.selection else {
            return;
        };
        let pane = &mut selection.conversation;
        if !writable {
            pane.error = Some("Wait for the current project operation to finish.".into());
            cx.notify();
            return;
        }
        let creating = matches!(change, Change::Create);
        if creating && pane.load_task.is_some() {
            return;
        }
        pane.error = None;
        pane.list_task = None;
        let project = selection.project.clone();
        let path = self.projects_dir.clone();
        let sequence = self.project_open_sequence;
        let operation = cx.background_executor().spawn(async move {
            match change {
                Change::Create => create_conversation(&path, &project).map(Some),
                Change::Rename { id, title } => {
                    rename_conversation(&path, &project, &id, &title).map(|()| None)
                }
            }
        });
        self.entry_write_task = Some(cx.spawn_in(window, async move |library, cx| {
            let result = operation.await;
            let _ = library.update_in(cx, |library, window, cx| {
                library.entry_write_task = None;
                if library.project_open_sequence == sequence
                    && let Some(selection) = &mut library.selection
                {
                    match result {
                        Ok(id) => {
                            if let Some(id) = id {
                                selection.conversation.display(id, Conversation::default());
                            }
                            if selection.conversation.sidebar_open {
                                library.refresh_conversation_list(cx);
                            }
                        }
                        Err(error) => {
                            let verb = if creating { "create" } else { "rename" };
                            selection.conversation.error =
                                Some(format!("Couldn’t {verb} chat. {error}"));
                        }
                    }
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

    fn edit_conversation_name(
        &mut self,
        id: ConversationId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pane) = self.selection.as_mut().map(|s| &mut s.conversation) else {
            return;
        };
        let Some(row) = pane
            .rows
            .as_ref()
            .and_then(|rows| rows.as_ref().ok())
            .and_then(|rows| rows.iter().find(|row| row.summary.id == id))
        else {
            return;
        };
        let title = row.summary.title.clone();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(title.clone()));
        let events = cx.subscribe_in(&input, window, |library, input, event, window, cx| {
            if !library
                .selection
                .as_ref()
                .and_then(|s| s.conversation.draft.as_ref())
                .is_some_and(|draft| draft.input == *input)
            {
                return;
            }
            match event {
                InputEvent::PressEnter { .. } => {
                    library.finish_conversation_name(true, true, window, cx)
                }
                InputEvent::Blur => library.finish_conversation_name(true, false, window, cx),
                _ => {}
            }
        });
        pane.draft = Some(NameDraft {
            id,
            title,
            input: input.clone(),
            rename_focus: row.rename_focus.clone(),
            _events: events,
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        let focus = input.read(cx).focus_handle(cx);
        window.defer(cx, move |window, app| {
            if focus.is_focused(window) {
                window.draw(app).clear();
                focus.dispatch_action(&SelectAll, window, app);
            }
        });
        cx.notify();
    }

    fn finish_conversation_name(
        &mut self,
        save: bool,
        restore_focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(draft) = self
            .selection
            .as_mut()
            .and_then(|s| s.conversation.draft.take())
        else {
            return;
        };
        if restore_focus {
            window.focus(&draft.rename_focus);
        }
        let value = draft.input.read(cx).value();
        let title = value.trim();
        if save && !title.is_empty() && title != draft.title {
            self.change_conversation(
                Change::Rename {
                    id: draft.id,
                    title: title.into(),
                },
                window,
                cx,
            );
        }
        cx.notify();
    }

    pub(super) fn project_sidebar_tabs(
        &self,
        selection: &Selection,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let chats_open = selection.conversation.sidebar_open;
        div()
            .flex()
            .items_center()
            .flex_none()
            .gap_1()
            .pb(px(6.))
            .border_b_1()
            .border_color(rgb(palette.border))
            .children(
                [("files-tab", "Files", false), ("chats-tab", "Chats", true)]
                    .into_iter()
                    .map(|(id, title, chats)| {
                        let active = chats == chats_open;
                        chat_button(id, palette, cx, move |library, _, cx| {
                            library.switch_project_sidebar(chats, cx);
                        })
                        .px_2()
                        .py(px(6.))
                        .when(active, |button| {
                            button
                                .bg(rgb(palette.selection))
                                .text_color(rgb(palette.text))
                                .font_weight(FontWeight::SEMIBOLD)
                        })
                        .child(title)
                    }),
            )
            .child(div().flex_1())
            .child(self.add_items_button(palette, cx))
    }

    pub(super) fn conversation_list(
        &self,
        selection: &Selection,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let pane = &selection.conversation;
        let list = div()
            .id("chat-list")
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll();
        match &pane.rows {
            None => list.child("Loading chats…"),
            Some(Err(error)) => list.child(format!("Couldn’t list chats. {error}")),
            Some(Ok(rows)) => list.children(rows.iter().map(|row| {
                let id = &row.summary.id;
                let row_id = SharedString::from(format!("chat-row/{}", id.as_str()));
                let row_div = div().id(row_id).flex().items_center();
                if let Some(draft) = pane.draft.as_ref().filter(|draft| &draft.id == id) {
                    return row_div
                        .capture_key_down(cx.listener(
                            |library, event: &KeyDownEvent, window, cx| {
                                if event.keystroke.key == "escape" {
                                    cx.stop_propagation();
                                    library.finish_conversation_name(false, true, window, cx);
                                }
                            },
                        ))
                        .child(Input::new(&draft.input));
                }
                let open = id.clone();
                let rename = id.clone();
                row_div
                    .child(
                        chat_button("open-chat", palette, cx, move |library, _, cx| {
                            if library.entry_write_task.is_none() {
                                library.load_project_conversation(open.clone(), cx);
                            }
                        })
                        .debug_selector({
                            let id = id.clone();
                            move || format!("open-chat/{}", id.as_str())
                        })
                        .flex_1()
                        .min_w(px(0.))
                        .px_2()
                        .py_1()
                        .when(pane.active == *id, |button| {
                            button
                                .bg(rgb(palette.selection))
                                .text_color(rgb(palette.text))
                        })
                        .child(div().truncate().child(row.summary.title.clone())),
                    )
                    .child(
                        chat_button("rename-chat", palette, cx, move |library, window, cx| {
                            library.edit_conversation_name(rename.clone(), window, cx);
                        })
                        .debug_selector({
                            let active = pane.active == *id;
                            let id = id.clone();
                            move || {
                                if active {
                                    "rename-active-chat".into()
                                } else {
                                    format!("rename-chat/{}", id.as_str())
                                }
                            }
                        })
                        .track_focus(&row.rename_focus)
                        .size(px(26.))
                        .text_size(px(16.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .tooltip(|window, cx| Tooltip::new("Rename chat").build(window, cx))
                        .child("✎"),
                    )
            })),
        }
    }

    pub(super) fn conversation_pane(
        &self,
        selection: &Selection,
        editor_shown: bool,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let pane = &selection.conversation;
        let header = div()
            .h(px(30.))
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .child(
                div()
                    .flex_1()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Conversation"),
            )
            .child(
                chat_button("expand-chat", palette, cx, move |library, _, cx| {
                    library.expand_workspace_chat(editor_shown, cx)
                })
                .px_2()
                .py_1()
                .child(if editor_shown {
                    "Expand chat"
                } else {
                    "Show editor"
                }),
            )
            .child(
                chat_button("new-chat", palette, cx, |library, window, cx| {
                    library.change_conversation(Change::Create, window, cx)
                })
                .size(px(26.))
                .text_size(px(19.))
                .flex()
                .items_center()
                .justify_center()
                .tooltip(|window, cx| Tooltip::new("New chat").build(window, cx))
                .child("+"),
            );
        let history = div()
            .id("chat-messages")
            .relative()
            .flex_1()
            .min_h(px(0.))
            .px_1()
            .pb(px(10.))
            .overflow_y_scroll()
            .track_scroll(&pane.scroll);
        let history = match &pane.history {
            None if pane.error.is_none() => history.child("Loading conversation…"),
            None => history,
            Some(history_data) if history_data.messages.is_empty() => {
                history.when(!pane.welcome_dismissed, |history| {
                    history
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Welcome to Refract!"),
                        )
                        .child(
                            div()
                                .mt_1()
                                .pr_6()
                                .text_size(px(12.))
                                .text_color(rgb(palette.muted))
                                .child("Ask about your LaTeX, research, or writing."),
                        )
                        .child(
                            chat_button("dismiss-welcome", palette, cx, |library, _, cx| {
                                if let Some(selection) = &mut library.selection {
                                    selection.conversation.welcome_dismissed = true;
                                    cx.notify();
                                }
                            })
                            .absolute()
                            .top_0()
                            .right_0()
                            .p_1()
                            .tooltip(|window, cx| Tooltip::new("Dismiss welcome").build(window, cx))
                            .child(
                                svg()
                                    .path("refract/close.svg")
                                    .size(px(16.))
                                    .text_color(rgb(palette.muted)),
                            ),
                        )
                })
            }
            Some(conversation) => history.children(
                conversation
                    .messages
                    .iter()
                    .map(|message| message_content(message, palette)),
            ),
        };
        div()
            .id("conversation")
            .debug_selector(|| "conversation".into())
            .flex()
            .flex_col()
            .mx_4()
            .mb_4()
            .p_3()
            .gap_2()
            .rounded(px(10.))
            .border_1()
            .border_color(rgb(palette.border))
            .bg(rgb(palette.surface))
            .when(editor_shown, |pane| {
                pane.h(px(280.)).max_h(gpui::relative(0.65)).flex_none()
            })
            .when(!editor_shown, |pane| pane.flex_1().min_h(px(0.)).mt_4())
            .child(header)
            .children(
                pane.error
                    .clone()
                    .map(|error| div().text_color(rgb(palette.warning)).child(error)),
            )
            .child(history)
    }
}

fn chat_button(
    id: &'static str,
    palette: Palette,
    cx: &Context<Library>,
    callback: impl Fn(&mut Library, &mut Window, &mut Context<Library>) + Clone + 'static,
) -> gpui::Stateful<gpui::Div> {
    let keyboard = callback.clone();
    div()
        .id(id)
        .debug_selector(move || id.into())
        .tab_index(1)
        .rounded_md()
        .cursor_pointer()
        .text_size(px(12.))
        .text_color(rgb(palette.muted))
        .hover(move |style| style.bg(rgb(palette.selection)))
        .focus(move |style| style.text_color(rgb(palette.focus)))
        .on_click(cx.listener(move |library, _, window, cx| callback(library, window, cx)))
        .on_key_down(
            cx.listener(move |library, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    keyboard(library, window, cx);
                }
            }),
        )
}

fn message_content(message: &Message, palette: Palette) -> impl IntoElement {
    let block = div().my_2().rounded(px(8.)).py(px(10.)).text_size(px(13.));
    match message {
        Message::User { text, attachments } => block
            .ml_6()
            .px_3()
            .bg(rgb(palette.selection))
            .child(text.clone())
            .children(
                attachments
                    .as_ref()
                    .filter(|files| !files.is_empty())
                    .map(|files| {
                        div()
                            .mt_2()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .children(files.iter().map(|file| {
                                div()
                                    .rounded_md()
                                    .px_2()
                                    .py_1()
                                    .bg(rgb(palette.surface))
                                    .text_color(rgb(palette.muted))
                                    .child(file.name.clone())
                            }))
                    }),
            ),
        Message::Assistant { text } => block.pr_3().child(text.clone()),
        Message::Error { text } => block
            .px_3()
            .bg(rgb(palette.warning_surface))
            .text_color(rgb(palette.warning))
            .child(text.clone()),
        Message::Handoff { from, to, status } => {
            let caption = match status.as_str() {
                "pending" => "Context handoff pending",
                "failed" => "Context handoff failed",
                _ => "Context handoff",
            };
            block
                .flex()
                .items_center()
                .gap_2()
                .text_size(px(12.))
                .text_color(rgb(palette.muted))
                .child(caption)
                .child(format!("{} → {}", provider_name(from), provider_name(to)))
                .child(div().h(px(1.)).flex_1().bg(rgb(palette.border)))
        }
        Message::Unsupported => div(),
    }
}

fn provider_name(provider: &str) -> &str {
    match provider {
        "codex" => "Codex",
        "opencode" => "OpenCode",
        _ => provider,
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use crate::{Root, initialize_app};
    use gpui::{Modifiers, TestAppContext};
    use refract_native::create_project;

    #[gpui::test]
    fn opens_existing_chats_and_creates_and_renames_a_conversation(cx: &mut TestAppContext) {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let project = create_project(&projects, "Paper").unwrap();
        let existing = ConversationId::parse("ed02e1d3-c2c1-4bb2-94aa-966d9275ed14").unwrap();
        let project_dir = projects.join(&project.id);
        std::fs::create_dir(project_dir.join(".refract-chats")).unwrap();
        std::fs::write(
            project_dir
                .join(".refract-chats")
                .join(format!("{}.json", existing.as_str())),
            br#"{"sessionId":"example","provider":"codex","messages":[{"role":"user","text":"Check the abstract."},{"role":"assistant","text":"It states the main result."}]}"#,
        )
        .unwrap();
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
                view.open_project(project.clone(), window, cx);
            });
        });
        cx.run_until_parked();
        cx.refresh().unwrap();

        let tabs = cx.debug_bounds("chats-tab").unwrap().center();
        cx.simulate_click(tabs, Modifiers::none());
        cx.run_until_parked();
        cx.refresh().unwrap();
        let row = cx
            .debug_bounds("open-chat/ed02e1d3-c2c1-4bb2-94aa-966d9275ed14")
            .unwrap()
            .center();
        cx.simulate_click(row, Modifiers::none());
        cx.run_until_parked();
        cx.update(|_, app| {
            let chat = &library.read(app).selection.as_ref().unwrap().conversation;
            assert_eq!(chat.active, existing);
            assert_eq!(chat.history.as_ref().unwrap().messages.len(), 2);
        });

        cx.refresh().unwrap();
        let create = cx.debug_bounds("new-chat").unwrap().center();
        cx.simulate_click(create, Modifiers::none());
        cx.run_until_parked();
        cx.refresh().unwrap();
        let created = cx.update(|_, app| {
            let chat = &library.read(app).selection.as_ref().unwrap().conversation;
            assert_eq!(chat.rows.as_ref().unwrap().as_ref().unwrap().len(), 3);
            assert_ne!(chat.active, existing);
            assert_ne!(chat.active, ConversationId::default());
            chat.active.clone()
        });
        let rename = cx.debug_bounds("rename-active-chat").unwrap().center();
        cx.simulate_click(rename, Modifiers::none());
        cx.refresh().unwrap();
        cx.simulate_input("Literature review");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert_eq!(
            load_conversation(&projects, &project, &created)
                .unwrap()
                .title
                .as_deref(),
            Some("Literature review")
        );
        cx.update(|window, app| {
            let chat = &library.read(app).selection.as_ref().unwrap().conversation;
            assert!(chat.draft.is_none());
            assert!(
                chat.rows
                    .as_ref()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .iter()
                    .any(|row| row.summary.id == created && row.rename_focus.is_focused(window))
            );
        });
    }
}

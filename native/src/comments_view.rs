use super::{Library, Palette, Selection};
use chrono::{DateTime, Local};
use gpui::{
    App, Context, Entity, EntityInputHandler, FontWeight, KeyBinding, KeyDownEvent, SharedString,
    Subscription, Window, actions, div, prelude::*, px, rgb,
};
use gpui_component::input::{Input, InputEvent, InputState};
use refract_native::comments::{Comment, CommentSelection, CommentUpdate, update_comments};
use std::collections::HashMap;

actions!(comment_draft, [NextControl, PreviousControl]);

pub(super) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", NextControl, Some("RefractCommentDraft > Input")),
        KeyBinding::new(
            "shift-tab",
            PreviousControl,
            Some("RefractCommentDraft > Input"),
        ),
    ]);
}

pub(super) struct CommentDrafts {
    search: Entity<InputState>,
    new_comment: Entity<InputState>,
    replies: HashMap<String, ReplyDraft>,
    _watch_search: Subscription,
}

struct ReplyDraft {
    input: Entity<InputState>,
    expanded: bool,
}

impl CommentDrafts {
    fn new(window: &mut Window, cx: &mut Context<Library>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search comments"));
        let watch_search = cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        let new_comment = cx.new(|cx| {
            InputState::new(window, cx)
                .auto_grow(2, 5)
                .placeholder("Add a comment")
        });
        Self {
            search,
            new_comment,
            replies: HashMap::new(),
            _watch_search: watch_search,
        }
    }
}

impl Library {
    pub(super) fn load_workspace_comments(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(selection) = &self.selection else {
            return;
        };
        self.comment_drafts
            .entry(selection.project.id.clone())
            .or_insert_with(|| CommentDrafts::new(window, cx));
        self.load_comments(cx);
        cx.notify();
    }

    fn toggle_reply(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(drafts) = self
            .selection
            .as_ref()
            .and_then(|selection| self.comment_drafts.get_mut(&selection.project.id))
        else {
            return;
        };
        match drafts.replies.get_mut(id) {
            Some(reply) => reply.expanded = !reply.expanded,
            None => {
                let input = cx.new(|cx| {
                    InputState::new(window, cx)
                        .auto_grow(2, 5)
                        .placeholder("Write a reply")
                });
                drafts.replies.insert(
                    id.to_owned(),
                    ReplyDraft {
                        input,
                        expanded: true,
                    },
                );
            }
        }
        cx.notify();
    }

    fn submit_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(selection) = &self.selection else {
            return;
        };
        let Some(file) = selection.file.clone() else {
            return;
        };
        let Some(drafts) = self.comment_drafts.get(&selection.project.id) else {
            return;
        };
        let input = drafts.new_comment.clone();
        let text = input.read(cx).value();
        if text.trim().is_empty() || !self.project_writes_idle() {
            return;
        }
        let selected = selection
            .tabs
            .iter()
            .find(|tab| tab.path == file)
            .and_then(|tab| tab.editor.as_ref()?.as_ref().ok())
            .and_then(|document| {
                document
                    .state
                    .update(cx, |editor, cx| source_selection(editor, window, cx))
            });
        self.write_comment(
            CommentUpdate::Create {
                file,
                text: text.to_string(),
                selection: selected,
            },
            Some((input, text)),
            window,
            cx,
        );
    }

    fn submit_reply(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(reply) = self.selection.as_ref().and_then(|selection| {
            self.comment_drafts
                .get(&selection.project.id)?
                .replies
                .get(id)
        }) else {
            return;
        };
        let input = reply.input.clone();
        let text = input.read(cx).value();
        if text.trim().is_empty() {
            return;
        }
        self.write_comment(
            CommentUpdate::Reply {
                id: id.to_owned(),
                text: text.to_string(),
            },
            Some((input, text)),
            window,
            cx,
        );
    }

    fn write_comment(
        &mut self,
        update: CommentUpdate,
        draft: Option<(Entity<InputState>, SharedString)>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if !self.project_writes_idle() {
            return;
        }
        let Some(selection) = &mut self.selection else {
            return;
        };
        selection.comment_error = None;
        let submit_focus = window.focused(cx);
        self.comments_task = None;
        let project = selection.project.clone();
        let path = self.projects_dir.clone();
        let save = cx
            .background_executor()
            .spawn(async move { update_comments(&path, &project, update) });
        self.comment_write_task = Some(cx.spawn_in(window, async move |library, cx| {
            let result = save.await;
            let _ = library.update_in(cx, |library, window, cx| {
                library.comment_write_task = None;
                if let Some(selection) = &mut library.selection {
                    match result {
                        Ok(comments) => {
                            selection.comments = Some(Ok(comments));
                            if let Some((input, submitted)) = draft
                                && input.read(cx).value() == submitted
                            {
                                let restore_focus = selection.tool
                                    == Some(crate::WorkspaceTool::Comments)
                                    && submit_focus
                                        .as_ref()
                                        .is_some_and(|focus| focus.is_focused(window));
                                input.update(cx, |state, cx| {
                                    state.set_value("", window, cx);
                                    if restore_focus {
                                        state.focus(window, cx);
                                    }
                                });
                            }
                        }
                        Err(error) => selection.comment_error = Some(error.to_string()),
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

    pub(super) fn comments_pane(
        &self,
        selection: &Selection,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let panel = div()
            .id("comments-pane")
            .tab_index(3)
            .tab_stop(false)
            .tab_group()
            .size_full()
            .flex()
            .flex_col()
            .p_4()
            .gap_2()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("All Comments"),
            );
        let Some(drafts) = self.comment_drafts.get(&selection.project.id) else {
            return panel;
        };
        let query = drafts.search.read(cx).value().to_lowercase();
        let ready = self.project_writes_idle();
        let list = div()
            .id("comment-list")
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll();
        let list = match &selection.comments {
            None => list.child("Loading comments…"),
            Some(Err(error)) => list.child(format!("Couldn’t load comments: {error}")),
            Some(Ok(comments)) => {
                let mut matches = comments
                    .iter()
                    .filter(|comment| {
                        query.is_empty()
                            || format!("{} {} {}", comment.text, comment.quote, comment.file)
                                .to_lowercase()
                                .contains(&query)
                    })
                    .peekable();
                if matches.peek().is_none() {
                    list.child(if query.is_empty() {
                        "No comments yet. Add one to start a discussion."
                    } else {
                        "No comments match your search."
                    })
                } else {
                    list.children(matches.map(|comment| {
                        comment_card(comment, drafts.replies.get(&comment.id), ready, palette, cx)
                    }))
                }
            }
        };
        panel
            .child(Input::new(&drafts.search))
            .child(draft_input(&drafts.new_comment))
            .child(comment_control(
                "comment-submit",
                "Comment",
                ready && selection.file.is_some(),
                palette,
                cx,
                |library, window, cx| library.submit_comment(window, cx),
            ))
            .children(
                selection
                    .comment_error
                    .as_ref()
                    .map(|error| format!("Couldn’t update comments. {error}")),
            )
            .child(list)
    }
}

fn source_selection(
    editor: &mut InputState,
    window: &mut Window,
    cx: &mut Context<InputState>,
) -> Option<CommentSelection> {
    let selection = editor.selected_text_range(false, window, cx)?.range;
    if selection.is_empty() {
        return None;
    }
    let prefix = editor.text_for_range(0..selection.start, &mut None, window, cx)?;
    let quote = editor.text_for_range(selection, &mut None, window, cx)?;
    let start = prefix.replace("\r\n", "\n").encode_utf16().count();
    let quote = quote.replace("\r\n", "\n").replace('\r', "\n");
    Some(CommentSelection {
        range: start..start + quote.encode_utf16().count(),
        quote,
    })
}

fn comment_card(
    comment: &Comment,
    reply: Option<&ReplyDraft>,
    ready: bool,
    palette: Palette,
    cx: &Context<Library>,
) -> impl IntoElement {
    let reference = if comment.quote.is_empty() {
        comment.file.clone()
    } else {
        format!("{}: {}", comment.file, comment.quote)
    };
    let open_id = comment.id.clone();
    let status_id = comment.id.clone();
    let reply_id = comment.id.clone();
    let resolve = !comment.resolved;
    let date = comment
        .created
        .as_deref()
        .and_then(|created| DateTime::parse_from_rfc3339(created).ok())
        .map(|date| {
            date.with_timezone(&Local)
                .format("%-m/%-d/%Y, %-I:%M %p")
                .to_string()
        });
    div()
        .id(SharedString::from(comment.id.clone()))
        .border_b_1()
        .border_color(rgb(palette.border))
        .py_2()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .gap_2()
                .text_size(px(12.))
                .text_color(rgb(palette.muted))
                .child(if comment.resolved { "Resolved" } else { "Open" })
                .children(date),
        )
        .child(
            comment_control(
                "reference",
                reference,
                true,
                palette,
                cx,
                move |library, window, cx| library.open_comment(&open_id, window, cx),
            )
            .debug_selector(|| format!("comment-reference-{}", comment.id))
            .text_color(rgb(palette.focus)),
        )
        .when(comment.selection_unavailable(), |card| {
            card.child("Selection unavailable")
        })
        .child(div().child(comment.text.clone()))
        .children(comment.replies.iter().map(|reply| {
            div()
                .pl_2()
                .border_l_1()
                .border_color(rgb(palette.border))
                .child(reply.text.clone())
        }))
        .child(
            div()
                .flex()
                .gap_2()
                .child(comment_control(
                    "reply-toggle",
                    "Reply",
                    true,
                    palette,
                    cx,
                    move |library, window, cx| library.toggle_reply(&reply_id, window, cx),
                ))
                .child(comment_control(
                    "resolve",
                    if resolve { "Resolve" } else { "Reopen" },
                    ready,
                    palette,
                    cx,
                    move |library, window, cx| {
                        library.write_comment(
                            CommentUpdate::SetResolved {
                                id: status_id.clone(),
                                resolved: resolve,
                            },
                            None,
                            window,
                            cx,
                        )
                    },
                )),
        )
        .when_some(reply.filter(|reply| reply.expanded), |card, reply| {
            let comment_id = comment.id.clone();
            card.child(draft_input(&reply.input)).child(comment_control(
                "reply-submit",
                "Reply",
                ready,
                palette,
                cx,
                move |library, window, cx| library.submit_reply(&comment_id, window, cx),
            ))
        })
}

fn draft_input(input: &Entity<InputState>) -> impl IntoElement {
    // Comment text areas use Tab to move focus.
    div()
        .key_context("RefractCommentDraft")
        .on_action(|_: &NextControl, window, _| window.focus_next())
        .on_action(|_: &PreviousControl, window, _| window.focus_prev())
        .child(Input::new(input))
}

fn comment_control(
    id: &'static str,
    label: impl Into<SharedString>,
    enabled: bool,
    palette: Palette,
    cx: &Context<Library>,
    activate: impl Fn(&mut Library, &mut Window, &mut Context<Library>) + Clone + 'static,
) -> impl IntoElement + Styled + InteractiveElement {
    let by_key = activate.clone();
    div()
        .id(id)
        .px_2()
        .py_1()
        .rounded_md()
        .truncate()
        .child(label.into())
        .when(!enabled, |button| button.text_color(rgb(palette.muted)))
        .when(enabled, |button| {
            button
                .tab_index(0)
                .cursor_pointer()
                .hover(move |style| style.bg(rgb(palette.selection)))
                .focus(move |style| style.bg(rgb(palette.selection)))
                .on_click(cx.listener(move |library, _, window, cx| activate(library, window, cx)))
                .on_key_down(
                    cx.listener(move |library, event: &KeyDownEvent, window, cx| {
                        if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                            by_key(library, window, cx);
                        }
                    }),
                )
        })
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use crate::initialize_app;
    use gpui::TestAppContext;
    use gpui_component::{Root, input::Position};
    use refract_native::list_projects;
    use std::fs;

    #[gpui::test]
    fn authors_a_comment_from_the_source_selection_and_updates_its_thread(cx: &mut TestAppContext) {
        let projects = tempfile::tempdir().unwrap();
        let id = "b3b9ffb5-13c0-4d22-b427-54520347684a";
        let folder = projects.path().join(id);
        fs::create_dir(&folder).unwrap();
        fs::write(
            folder.join(".refract.json"),
            serde_json::json!({"id":id,"name":"Paper"}).to_string(),
        )
        .unwrap();
        fs::write(folder.join("main.tex"), "Result\n").unwrap();
        let project = list_projects(projects.path()).unwrap().pop().unwrap();

        cx.update(initialize_app);
        let mut library = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| Library::new(projects.path().to_path_buf(), window, cx));
            library = Some(view.clone());
            Root::new(view, window, cx)
        });
        let library = library.unwrap();
        cx.update(|window, app| {
            library.update(app, |library, cx| library.open_project(project, window, cx));
        });
        cx.run_until_parked();
        cx.update(|window, app| {
            let view = library.read(app);
            let editor = view.selection.as_ref().unwrap().tabs[0]
                .editor
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap()
                .state
                .clone();
            editor.update(app, |editor, cx| {
                editor.set_cursor_position(Position::new(0, 0), window, cx)
            });
        });
        cx.simulate_keystrokes(
            "shift-right shift-right shift-right shift-right shift-right shift-right",
        );

        cx.update(|window, app| {
            library.update(app, |view, cx| {
                view.select_workspace_tool(crate::WorkspaceTool::Comments, window, cx);
                view.comment_drafts[id].new_comment.update(cx, |input, cx| {
                    input.set_value("Clarify this result.", window, cx);
                    input.focus(window, cx);
                });
            });
        });
        cx.refresh().unwrap();
        cx.simulate_keystrokes("tab enter");
        cx.run_until_parked();
        let comment_id = cx.update(|_, app| {
            let view = library.read(app);
            let comments = view
                .selection
                .as_ref()
                .unwrap()
                .comments
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap();
            assert_eq!(comments.len(), 1);
            assert_eq!(comments[0].text, "Clarify this result.");
            assert_eq!(comments[0].quote, "Result");
            assert!(
                view.comment_drafts[id]
                    .new_comment
                    .read(app)
                    .value()
                    .is_empty()
            );
            comments[0].id.clone()
        });
        cx.update(|window, app| {
            library.update(app, |view, cx| {
                view.toggle_reply(&comment_id, window, cx);
                view.comment_drafts[id].replies[&comment_id]
                    .input
                    .update(cx, |input, cx| {
                        input.set_value("Added the assumptions.", window, cx);
                    });
                view.submit_reply(&comment_id, window, cx);
            });
        });
        cx.run_until_parked();
        for resolved in [true, false] {
            cx.update(|window, app| {
                library.update(app, |view, cx| {
                    view.write_comment(
                        CommentUpdate::SetResolved {
                            id: comment_id.clone(),
                            resolved,
                        },
                        None,
                        window,
                        cx,
                    );
                });
            });
            cx.run_until_parked();
            cx.update(|_, app| {
                let view = library.read(app);
                let comments = view
                    .selection
                    .as_ref()
                    .unwrap()
                    .comments
                    .as_ref()
                    .unwrap()
                    .as_ref()
                    .unwrap();
                assert_eq!(comments[0].resolved, resolved);
                assert_eq!(comments[0].replies[0].text, "Added the assumptions.");
            });
        }
    }
}

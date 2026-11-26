use super::{Library, Palette, workspace_tools::tool_button};
use gpui::{
    App, Context, Div, Entity, EntityInputHandler, FocusHandle, Focusable, KeyBinding,
    KeyDownEvent, Subscription, Window, actions, div, prelude::*, px, rgb,
};
use gpui_component::{
    actions::SelectRight,
    input::{Escape, Input, InputEvent, InputState, RopeExt, Search, SelectAll},
};
use refract_native::editor_find::{FindDirection, FindQuery, FindResults};
use std::ops::Range;

actions!(
    refract_find,
    [OpenFind, FindNext, FindPrevious, ConfirmFind, CloseFind]
);

pub(super) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-f", OpenFind, None),
        KeyBinding::new("f3", FindNext, None),
        KeyBinding::new("shift-f3", FindPrevious, None),
        KeyBinding::new("enter", ConfirmFind, Some("RefractFind > Input")),
        KeyBinding::new("shift-enter", FindPrevious, Some("RefractFind > Input")),
        KeyBinding::new("escape", CloseFind, Some("RefractFind")),
    ]);
}

pub(super) struct EditorFind {
    query: Entity<InputState>,
    replacement: Entity<InputState>,
    pub(super) visible: bool,
    match_case: bool,
    whole_word: bool,
    wrap: bool,
    anchor: usize,
    pattern: Option<FindQuery>,
    previous_selection: Option<Range<usize>>,
    status: String,
    _query_changed: Subscription,
}

impl EditorFind {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Library>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Find"));
        let replacement = cx.new(|cx| InputState::new(window, cx).placeholder("Replace"));
        let query_changed = cx.subscribe_in(&query, window, |library, _, event, window, cx| {
            if matches!(event, InputEvent::Change) && library.editor_find.visible {
                library.update_find_query(window, cx);
            }
        });
        Self {
            query,
            replacement,
            visible: false,
            match_case: true,
            whole_word: false,
            wrap: true,
            anchor: 0,
            pattern: None,
            previous_selection: None,
            status: String::new(),
            _query_changed: query_changed,
        }
    }
}

impl Library {
    fn find_editor(&self) -> Option<Entity<InputState>> {
        let selection = self.selection.as_ref()?;
        let file = selection.file.as_ref()?;
        let tab = selection.tabs.iter().find(|tab| tab.path == *file)?;
        Some(tab.editor.as_ref()?.as_ref().ok()?.state.clone())
    }

    fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.find_editor() else {
            return;
        };
        let range = selected_bytes(&editor, window, cx);
        self.editor_find.visible = true;
        self.editor_find.anchor = range.start;
        let input = self.editor_find.query.clone();
        if editor.read(cx).focus_handle(cx).is_focused(window) && !range.is_empty() {
            let selected = editor.read(cx).text().slice(range).to_string();
            if !selected.contains('\n') {
                input.update(cx, |input, cx| input.set_value(selected, window, cx));
            }
        }
        input.update(cx, |input, cx| input.focus(window, cx));
        self.update_find_query(window, cx);
        cx.notify();
        let focus = input.read(cx).focus_handle(cx);
        window.defer(cx, move |window, app| {
            if focus.is_focused(window) {
                window.draw(app).clear();
                focus.dispatch_action(&SelectAll, window, app);
            }
        });
    }

    fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor_find.visible {
            self.editor_find.visible = false;
            if let Some(editor) = self.find_editor() {
                editor.update(cx, |editor, cx| editor.focus(window, cx));
            }
            cx.notify();
        }
    }

    fn update_find_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let find = &mut self.editor_find;
        let query = find.query.read(cx).value();
        find.pattern = FindQuery::new(query.as_ref(), find.match_case, find.whole_word);
        let anchor = find.anchor;
        self.seek_find(FindDirection::AtOrAfter(anchor), window, cx);
    }

    pub(super) fn refresh_find_for_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor_find.visible
            && let Some(editor) = self.find_editor()
        {
            self.editor_find.anchor = selected_bytes(&editor, window, cx).start;
            self.seek_find(
                FindDirection::AtOrAfter(self.editor_find.anchor),
                window,
                cx,
            );
        }
    }

    pub(super) fn sync_find_selection(
        &mut self,
        editor: &Entity<InputState>,
        changed_text: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.editor_find.visible || self.find_editor().as_ref() != Some(editor) {
            return;
        }
        let selection = selected_bytes(editor, window, cx);
        if !changed_text && self.editor_find.previous_selection.as_ref() == Some(&selection) {
            return;
        }
        self.editor_find.previous_selection = Some(selection.clone());
        let content = editor.read(cx).value();
        let results = self.find_in(content.as_ref(), FindDirection::Selection(selection));
        self.update_find_status(&results, None, cx);
    }

    fn find_in(&self, content: &str, direction: FindDirection) -> FindResults {
        self.editor_find.pattern.as_ref().map_or(
            FindResults {
                count: 0,
                current: None,
            },
            |query| query.find(content, direction, self.editor_find.wrap),
        )
    }

    fn navigate_find(&mut self, backward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.find_editor() {
            let range = selected_bytes(&editor, window, cx);
            let direction = if backward {
                FindDirection::Before(range.start)
            } else {
                FindDirection::AtOrAfter(range.end)
            };
            self.seek_find(direction, window, cx);
        }
    }

    fn seek_find(&mut self, direction: FindDirection, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editor_find.visible {
            return;
        }
        let Some(editor) = self.find_editor() else {
            return;
        };
        let edge = if matches!(direction, FindDirection::Before(_)) {
            "Start of file"
        } else {
            "End of file"
        };
        let content = editor.read(cx).value();
        let results = self.find_in(content.as_ref(), direction);
        if let Some(found) = &results.current {
            self.editor_find.previous_selection = Some(found.bytes.clone());
            select_editor_bytes(
                &editor,
                found.bytes.clone(),
                content.as_ref(),
                self.focus_handle.clone(),
                window,
                cx,
            );
        }
        self.update_find_status(&results, Some(edge), cx);
    }

    fn update_find_status(
        &mut self,
        results: &FindResults,
        edge: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let status = if let Some(found) = &results.current {
            format!("{} / {}", found.index, results.count)
        } else if results.count > 0 {
            edge.map_or_else(|| format!("– / {}", results.count), str::to_owned)
        } else if self.editor_find.query.read(cx).value().is_empty() {
            "Type to search".into()
        } else {
            "No matches".into()
        };
        if self.editor_find.status != status {
            self.editor_find.status = status;
            cx.notify();
        }
    }

    fn replace_find(&mut self, all: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.restore_task.is_some() || self.entry_write_task.is_some() {
            return;
        }
        let Some(editor) = self.find_editor() else {
            return;
        };
        let Some(query) = self.editor_find.pattern.as_ref() else {
            return;
        };
        let content = editor.read(cx).value();
        let replacement = self.editor_find.replacement.read(cx).value();
        if all {
            if let Some(value) = query.replace_all(content.as_ref(), replacement.as_ref()) {
                replace_editor_bytes(&editor, 0..content.len(), &value, window, cx);
            }
        } else {
            let selection = selected_bytes(&editor, window, cx);
            let results = query.find(
                content.as_ref(),
                FindDirection::Selection(selection.clone()),
                false,
            );
            if results.current.is_some() {
                replace_editor_bytes(&editor, selection, replacement.as_ref(), window, cx);
            }
            self.navigate_find(false, window, cx);
        }
    }

    pub(super) fn source_find_button(
        &self,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        tool_button(
            "find-source",
            "Find",
            self.editor_find.visible,
            palette,
            cx,
            Self::open_find,
        )
    }

    pub(super) fn source_find_panel(
        &self,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let find = &self.editor_find;
        let can_replace = self.restore_task.is_none() && self.entry_write_task.is_none();
        div()
            .id("editor-find")
            .key_context("RefractFind")
            .capture_action(cx.listener(|view, _: &Escape, window, cx| {
                view.close_find(window, cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|view, _: &CloseFind, window, cx| view.close_find(window, cx)))
            .on_action(cx.listener(|view, _: &ConfirmFind, window, cx| {
                if view
                    .editor_find
                    .replacement
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
                {
                    view.replace_find(false, window, cx);
                } else {
                    view.navigate_find(false, window, cx);
                }
            }))
            .flex()
            .flex_none()
            .flex_wrap()
            .items_center()
            .gap_1()
            .px_4()
            .py_2()
            .bg(rgb(palette.surface))
            .border_b_1()
            .border_color(rgb(palette.border))
            .text_size(px(12.))
            .child(Input::new(&find.query).tab_index(3).w(px(140.)).h(px(28.)))
            .child(
                Input::new(&find.replacement)
                    .tab_index(3)
                    .w(px(140.))
                    .h(px(28.)),
            )
            .child(find_toggle(
                "match-case",
                "Match case",
                find.match_case,
                palette,
                cx,
                |view, window, cx| {
                    view.editor_find.match_case = !view.editor_find.match_case;
                    view.update_find_query(window, cx);
                },
            ))
            .child(find_toggle(
                "whole-word",
                "Whole word",
                find.whole_word,
                palette,
                cx,
                |view, window, cx| {
                    view.editor_find.whole_word = !view.editor_find.whole_word;
                    view.update_find_query(window, cx);
                },
            ))
            .child(find_toggle(
                "wrap-search",
                "Wrap",
                find.wrap,
                palette,
                cx,
                |view, _, cx| {
                    view.editor_find.wrap = !view.editor_find.wrap;
                    cx.notify();
                },
            ))
            .child(tool_button(
                "previous-match",
                "↑",
                false,
                palette,
                cx,
                |view, window, cx| view.navigate_find(true, window, cx),
            ))
            .child(tool_button(
                "next-match",
                "↓",
                false,
                palette,
                cx,
                |view, window, cx| view.navigate_find(false, window, cx),
            ))
            .child(if can_replace {
                tool_button(
                    "replace-match",
                    "Replace",
                    false,
                    palette,
                    cx,
                    |view, window, cx| view.replace_find(false, window, cx),
                )
                .into_any_element()
            } else {
                div()
                    .text_color(rgb(palette.muted))
                    .child("Replace")
                    .into_any_element()
            })
            .child(if can_replace {
                tool_button(
                    "replace-all",
                    "All",
                    false,
                    palette,
                    cx,
                    |view, window, cx| view.replace_find(true, window, cx),
                )
                .into_any_element()
            } else {
                div()
                    .text_color(rgb(palette.muted))
                    .child("All")
                    .into_any_element()
            })
            .child(
                div()
                    .text_color(rgb(palette.muted))
                    .child(find.status.clone()),
            )
            .child(tool_button(
                "close-find",
                "×",
                false,
                palette,
                cx,
                Self::close_find,
            ))
    }

    pub(super) fn find_shortcuts(cx: &Context<Self>) -> Div {
        div()
            .on_action(cx.listener(|view, _: &OpenFind, window, cx| view.open_find(window, cx)))
            .on_action(
                cx.listener(|view, _: &FindNext, window, cx| view.navigate_find(false, window, cx)),
            )
            .on_action(cx.listener(|view, _: &FindPrevious, window, cx| {
                view.navigate_find(true, window, cx)
            }))
    }

    pub(super) fn capture_editor_find(
        cx: &Context<Self>,
    ) -> impl Fn(&Search, &mut Window, &mut App) + 'static {
        cx.listener(|view, _, window, cx| {
            view.open_find(window, cx);
            cx.stop_propagation();
        })
    }

    pub(super) fn capture_editor_find_escape(
        cx: &Context<Self>,
    ) -> impl Fn(&Escape, &mut Window, &mut App) + 'static {
        cx.listener(|view, _, window, cx| {
            if view.editor_find.visible {
                view.close_find(window, cx);
                cx.stop_propagation();
            }
        })
    }
}

fn find_toggle<F>(
    id: &'static str,
    label: &'static str,
    checked: bool,
    palette: Palette,
    cx: &Context<Library>,
    toggle: F,
) -> impl IntoElement
where
    F: Fn(&mut Library, &mut Window, &mut Context<Library>) + Clone + 'static,
{
    let keyboard = toggle.clone();
    div()
        .id(id)
        .debug_selector(|| id.to_owned())
        .tab_index(3)
        .flex()
        .items_center()
        .gap_1()
        .rounded_sm()
        .px_1()
        .cursor_pointer()
        .focus(move |style| style.bg(rgb(palette.selection)))
        .child(
            div()
                .size(px(13.))
                .text_size(px(11.))
                .line_height(px(11.))
                .text_center()
                .rounded_sm()
                .border_1()
                .border_color(rgb(palette.input_border))
                .when(checked, |check| {
                    check
                        .bg(rgb(palette.focus))
                        .text_color(rgb(palette.surface))
                        .child("✓")
                }),
        )
        .child(label)
        .on_click(cx.listener(move |view, _, window, cx| toggle(view, window, cx)))
        .on_key_down(cx.listener(move |view, event: &KeyDownEvent, window, cx| {
            if event.keystroke.key == "space" {
                keyboard(view, window, cx);
            }
        }))
}

fn selected_bytes(editor: &Entity<InputState>, window: &mut Window, app: &mut App) -> Range<usize> {
    editor.update(app, |editor, cx| {
        let range = editor
            .selected_text_range(false, window, cx)
            .expect("text selection")
            .range;
        editor.text().offset_utf16_to_offset(range.start)
            ..editor.text().offset_utf16_to_offset(range.end)
    })
}

fn replace_editor_bytes(
    editor: &Entity<InputState>,
    bytes: Range<usize>,
    replacement: &str,
    window: &mut Window,
    app: &mut App,
) {
    editor.update(app, |editor, cx| {
        let text = editor.text();
        let range =
            text.offset_to_offset_utf16(bytes.start)..text.offset_to_offset_utf16(bytes.end);
        editor.replace_text_in_range(Some(range), replacement, window, cx);
    });
}

fn select_editor_bytes(
    editor: &Entity<InputState>,
    bytes: Range<usize>,
    content: &str,
    root: FocusHandle,
    window: &mut Window,
    cx: &mut Context<Library>,
) {
    let focused = window.focused(cx);
    let editor_focus = editor.read(cx).focus_handle(cx);
    let start = bytes.start;
    let position = editor.read(cx).text().offset_to_position(start);
    let characters = content[bytes].chars().count();
    editor.update(cx, |editor, cx| {
        editor.set_cursor_position(position, window, cx)
    });
    if let Some(focused) = &focused {
        window.focus(focused);
    }
    let editor = editor.downgrade();
    window.defer(cx, move |window, app| {
        if editor
            .upgrade()
            .is_none_or(|editor| editor.read(app).cursor() != start)
        {
            return;
        }
        if !root.contains(&editor_focus, window) {
            window.draw(app).clear();
        }
        for _ in 0..characters {
            editor_focus.dispatch_action(&SelectRight, window, app);
        }
    });
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use crate::{Root, initialize_app};
    use gpui::{Modifiers, TestAppContext};
    use refract_native::create_project;
    use std::fs;

    #[gpui::test]
    fn navigates_and_replaces_without_leaving_the_find_field(cx: &mut TestAppContext) {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let project = create_project(&projects, "Paper").unwrap();
        fs::write(
            projects.join(&project.id).join("main.tex"),
            "needle one\n🙂 needle two\n",
        )
        .unwrap();
        cx.update(initialize_app);
        let mut library = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| Library::new(projects, window, cx));
            library = Some(view.clone());
            Root::new(view, window, cx)
        });
        let library = library.unwrap();
        cx.update(|window, app| {
            library.update(app, |view, cx| view.open_project(project, window, cx));
        });
        cx.run_until_parked();
        cx.update(|window, app| {
            let state = library.read(app).find_editor().unwrap();
            state.update(app, |editor, cx| editor.focus(window, cx));
        });
        cx.simulate_keystrokes(if cfg!(target_os = "macos") {
            "cmd-f"
        } else {
            "ctrl-f"
        });
        cx.simulate_input("needle");
        cx.update(|window, app| {
            let view = library.read(app);
            assert!(view.editor_find.visible);
            assert_eq!(view.editor_find.status, "1 / 2");
            assert!(
                view.editor_find
                    .query
                    .read(app)
                    .focus_handle(app)
                    .is_focused(window)
            );
        });
        cx.simulate_keystrokes("enter");
        cx.update(|_, app| assert_eq!(library.read(app).editor_find.status, "2 / 2"));
        let wrap = cx.debug_bounds("wrap-search").unwrap();
        cx.simulate_click(wrap.center(), Modifiers::none());
        cx.simulate_keystrokes("f3");
        cx.update(|window, app| {
            let find = &library.read(app).editor_find;
            assert_eq!(find.status, "End of file");
            let replacement = find.replacement.clone();
            replacement.update(app, |input, cx| {
                input.set_value("$1", window, cx);
                input.focus(window, cx);
            });
        });
        cx.simulate_keystrokes("enter");
        cx.update(|window, app| {
            let view = library.read(app);
            assert_eq!(
                view.find_editor().unwrap().read(app).value().as_ref(),
                "needle one\n🙂 $1 two\n"
            );
            assert!(
                view.editor_find
                    .replacement
                    .read(app)
                    .focus_handle(app)
                    .is_focused(window)
            );
        });
        cx.simulate_keystrokes("escape");
        cx.simulate_keystrokes(if cfg!(target_os = "macos") {
            "cmd-z"
        } else {
            "ctrl-z"
        });
        cx.update(|_, app| {
            let view = library.read(app);
            assert!(!view.editor_find.visible);
            assert_eq!(
                view.find_editor().unwrap().read(app).value().as_ref(),
                "needle one\n🙂 needle two\n"
            );
        });
    }
}

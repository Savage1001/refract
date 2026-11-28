use super::{
    EditorDocument, Library, Palette,
    editor_find_view::{replace_editor_bytes, selected_bytes},
    monospace_font,
    workspace_tools::tool_button,
};
use gpui::{
    App, Bounds, Context, Entity, EntityInputHandler, KeyBinding, MouseButton, Pixels, Point,
    ScrollHandle, Window, actions, anchored, canvas, deferred, div, point, prelude::*, px, rgb,
};
use gpui_component::input::{
    Enter, IndentInline, Input, InputEvent, InputState, MoveDown, MoveUp, OutdentInline, RopeExt,
};
use refract_native::latex_commands::{Suggestions, suggest};
use std::{cell::Cell, rc::Rc};

const OPTION_HEIGHT: Pixels = px(34.);
const MENU_WIDTH: Pixels = px(200.);
const MENU_MAX_HEIGHT: Pixels = px(240.);
const MENU_INSET: Pixels = px(4.);

actions!(editor, [CompleteLatex]);

pub(super) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new(
            "secondary-space",
            CompleteLatex,
            Some("RefractSource > Input"),
        ),
        #[cfg(target_os = "macos")]
        KeyBinding::new("ctrl-space", CompleteLatex, Some("RefractSource > Input")),
    ]);
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use crate::{Root, initialize_app};
    use gpui::{Focusable, Modifiers, TestAppContext};
    use gpui_component::input::Position;
    use refract_native::create_project;
    use std::fs;

    #[gpui::test]
    fn completes_commands_at_the_caret_with_keyboard_and_pointer(cx: &mut TestAppContext) {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let project = create_project(&projects, "Paper").unwrap();
        fs::write(projects.join(&project.id).join("main.tex"), r"\te").unwrap();
        cx.update(initialize_app);
        let mut view = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let library = cx.new(|cx| Library::new(projects, window, cx));
            view = Some(library.clone());
            Root::new(library, window, cx)
        });
        let library = view.unwrap();
        cx.update(|window, app| {
            library.update(app, |view, cx| view.open_project(project, window, cx));
        });
        cx.run_until_parked();
        let editor = cx.update(|window, app| {
            let editor = library.read(app).active_editor().unwrap();
            editor.update(app, |state, cx| {
                state.set_cursor_position(Position::new(0, 3), window, cx)
            });
            editor
        });
        cx.simulate_keystrokes("ctrl-space");
        cx.update(|_, app| {
            let menu = library.read(app).commands.open.as_ref().unwrap();
            assert_eq!(menu.suggestions.items.len(), 3);
        });
        cx.simulate_keystrokes("down enter");
        cx.update(|window, app| {
            assert!(library.read(app).commands.open.is_none());
            let state = editor.read(app);
            assert_eq!(state.value().as_ref(), r"\textit{}");
            assert_eq!(state.cursor_position(), Position::new(0, 8));
            assert!(state.focus_handle(app).is_focused(window));
        });
        cx.simulate_keystrokes(if cfg!(target_os = "macos") {
            "cmd-z"
        } else {
            "ctrl-z"
        });
        cx.update(|window, app| {
            assert_eq!(editor.read(app).value().as_ref(), r"\te");
            editor.update(app, |state, cx| {
                state.set_value(r"\section rest", window, cx);
                state.set_cursor_position(Position::new(0, 4), window, cx);
            });
        });
        cx.simulate_keystrokes("ctrl-space");
        cx.refresh().unwrap();
        let suggestion = cx.debug_bounds("latex-suggestion-0").unwrap();
        cx.simulate_click(suggestion.center(), Modifiers::none());
        cx.update(|window, app| {
            let state = editor.read(app);
            assert_eq!(state.value().as_ref(), r"\section{} rest");
            assert!(state.focus_handle(app).is_focused(window));
        });
        cx.update(|window, app| {
            editor.update(app, |state, cx| {
                state.set_value("plain", window, cx);
                state.set_cursor_position(Position::new(0, 5), window, cx);
            });
        });
        cx.simulate_keystrokes("ctrl-space up");
        cx.update(|_, app| {
            let menu = library.read(app).commands.open.as_ref().unwrap();
            assert_eq!(menu.suggestions.items[menu.selected].name, "right");
        });
        cx.simulate_keystrokes("left");
        cx.update(|_, app| {
            assert!(library.read(app).commands.open.is_none());
            assert_eq!(editor.read(app).cursor_position(), Position::new(0, 4));
        });
    }
}

#[derive(Default)]
pub(super) struct Commands {
    open: Option<CommandMenu>,
    pane: Rc<Cell<Bounds<Pixels>>>,
}

struct CommandMenu {
    suggestions: Suggestions,
    selected: usize,
    caret: usize,
    position: Point<Pixels>,
    scroll: ScrollHandle,
}

impl Commands {
    pub(super) fn close(&mut self) -> bool {
        self.open.take().is_some()
    }
}

impl Library {
    fn open_commands(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.restore_task.is_some() || self.entry_write_task.is_some() {
            return;
        }
        if let Some(editor) = self.active_editor() {
            editor.update(cx, |editor, cx| editor.focus(window, cx));
            self.update_commands(&editor, true, window, cx);
        }
    }

    fn update_commands(
        &mut self,
        editor: &Entity<InputState>,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selection = selected_bytes(editor, window, cx);
        let suggestions = selection
            .is_empty()
            .then(|| {
                let editor = editor.read(cx);
                let rope = editor.text();
                let position = rope.offset_to_point(selection.start);
                let line_start = rope.line_start_offset(position.row);
                let mut suggestions = suggest(
                    &rope.slice_line(position.row).to_string(),
                    position.column,
                    open,
                )?;
                suggestions.range.start += line_start;
                suggestions.range.end += line_start;
                Some(suggestions)
            })
            .flatten();
        self.commands.open = suggestions.map(|suggestions| {
            let previous = if open {
                None
            } else {
                self.commands.open.take()
            };
            let selected = previous
                .as_ref()
                .map_or(0, |menu| menu.selected)
                .min(suggestions.items.len() - 1);
            let scroll = previous
                .as_ref()
                .map(|menu| menu.scroll.clone())
                .unwrap_or_else(ScrollHandle::new);
            scroll.scroll_to_item(selected);
            CommandMenu {
                suggestions,
                selected,
                caret: selection.start,
                position: previous.map_or(self.commands.pane.get().origin, |menu| menu.position),
                scroll,
            }
        });
        cx.notify();
    }

    pub(super) fn update_commands_for_input(
        &mut self,
        editor: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.commands.open.is_none() || self.active_editor().as_ref() != Some(editor) {
            return;
        }
        match event {
            InputEvent::Change => self.update_commands(editor, false, window, cx),
            InputEvent::Blur => self.dismiss_commands(cx),
            _ => {}
        }
    }

    pub(super) fn sync_commands_selection(
        &mut self,
        editor: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(menu) = &self.commands.open
            && self.active_editor().as_ref() == Some(editor)
            && selected_bytes(editor, window, cx) != (menu.caret..menu.caret)
        {
            self.dismiss_commands(cx);
        }
    }

    fn dismiss_commands(&mut self, cx: &mut Context<Self>) {
        if self.commands.close() {
            cx.notify();
        }
    }

    fn move_command(&mut self, backward: bool, cx: &mut Context<Self>) {
        if let Some(menu) = &mut self.commands.open {
            let count = menu.suggestions.items.len();
            menu.selected = (menu.selected + if backward { count - 1 } else { 1 }) % count;
            menu.scroll.scroll_to_item(menu.selected);
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn accept_selected_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = &self.commands.open {
            self.accept_command(menu.selected, window, cx);
            cx.stop_propagation();
        }
    }

    fn accept_command(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = self.commands.open.take() else {
            return;
        };
        cx.notify();
        if self.restore_task.is_some() || self.entry_write_task.is_some() {
            return;
        }
        if let Some(editor) = self.active_editor()
            && let Some(command) = menu.suggestions.items.get(index)
        {
            let start = menu.suggestions.range.start;
            replace_editor_bytes(&editor, menu.suggestions.range, command.snippet, window, cx);
            editor.update(cx, |editor, cx| {
                let position = editor.text().offset_to_position(start + command.caret);
                editor.set_cursor_position(position, window, cx);
            });
        }
    }

    fn position_commands(
        &mut self,
        editor_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.active_editor() else {
            return;
        };
        let Some(menu) = self.commands.open.as_mut() else {
            return;
        };
        let caret = editor.update(cx, |editor, cx| {
            let caret = editor.text().offset_to_offset_utf16(menu.caret);
            editor.bounds_for_range(caret..caret, editor_bounds, window, cx)
        });
        let anchor = caret.map_or(editor_bounds.origin, |bounds| {
            point(bounds.left(), bounds.bottom())
        });
        let pane = self.commands.pane.get();
        let height =
            (OPTION_HEIGHT * menu.suggestions.items.len() as f32 + px(12.)).min(MENU_MAX_HEIGHT);
        let position = point(
            anchor.x.clamp(
                pane.left() + MENU_INSET,
                (pane.right() - MENU_WIDTH - MENU_INSET).max(pane.left() + MENU_INSET),
            ),
            anchor.y.clamp(
                pane.top() + MENU_INSET,
                (pane.bottom() - height - MENU_INSET).max(pane.top() + MENU_INSET),
            ),
        );
        if menu.position != position {
            menu.position = position;
            // Measure after the editor prepaints so scrolling and wrapping update the popup.
            let view = cx.weak_entity();
            window.defer(cx, move |_, app| {
                let _ = view.update(app, |_, cx| cx.notify());
            });
        }
    }

    pub(super) fn commands_button(&self, palette: Palette, cx: &Context<Self>) -> impl IntoElement {
        if self.restore_task.is_some() || self.entry_write_task.is_some() {
            return div()
                .px_1()
                .py_1()
                .text_color(rgb(palette.muted))
                .child("Commands")
                .into_any_element();
        }
        tool_button(
            "latex-commands",
            "Commands",
            self.commands.open.is_some(),
            palette,
            cx,
            Self::open_commands,
        )
        .into_any_element()
    }

    pub(super) fn commands_popup(
        &self,
        palette: Palette,
        cx: &Context<Self>,
    ) -> Option<impl IntoElement> {
        let menu = self.commands.open.as_ref()?;
        let bounds = self.commands.pane.clone();
        Some(
            div()
                .absolute()
                .left_0()
                .top_0()
                .size_full()
                .child(
                    canvas(
                        move |new_bounds, _, _| bounds.set(new_bounds),
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                )
                .child(deferred(
                    anchored().position(menu.position).snap_to_window().child(
                        div()
                            .id("latex-completions")
                            .debug_selector(|| "latex-completions".into())
                            .occlude()
                            .w(MENU_WIDTH)
                            .max_h(MENU_MAX_HEIGHT)
                            .track_scroll(&menu.scroll)
                            .overflow_y_scroll()
                            .p(px(5.))
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(palette.input_border))
                            .bg(rgb(palette.surface))
                            .shadow_lg()
                            .font_family(monospace_font())
                            .text_size(px(12.))
                            .children(menu.suggestions.items.iter().enumerate().map(
                                |(index, command)| {
                                    div()
                                        .id(("latex-suggestion", index))
                                        .debug_selector(|| format!("latex-suggestion-{index}"))
                                        .h(OPTION_HEIGHT)
                                        .px_2()
                                        .flex()
                                        .items_center()
                                        .rounded_md()
                                        .cursor_pointer()
                                        .when(index == menu.selected, |option| {
                                            option.bg(rgb(palette.selection))
                                        })
                                        .hover(move |style| style.bg(rgb(palette.selection)))
                                        .child(format!("\\{}", command.name))
                                        .on_click(cx.listener(move |view, _, window, cx| {
                                            view.accept_command(index, window, cx)
                                        }))
                                },
                            )),
                    ),
                )),
        )
    }

    pub(super) fn source_editor(
        &self,
        editor: &EditorDocument,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let view = cx.weak_entity();
        div()
            .on_children_prepainted(move |child_bounds, window, app| {
                if let Some(bounds) = child_bounds.first().copied() {
                    let _ = view.update(app, |view, cx| view.position_commands(bounds, window, cx));
                }
            })
            .id("document-editor")
            .debug_selector(|| "document-editor".into())
            .key_context("RefractSource")
            .on_action(
                cx.listener(|view, _: &CompleteLatex, window, cx| view.open_commands(window, cx)),
            )
            .capture_action(cx.listener(|view, _: &MoveUp, _, cx| view.move_command(true, cx)))
            .capture_action(cx.listener(|view, _: &MoveDown, _, cx| view.move_command(false, cx)))
            .capture_action(
                cx.listener(|view, _: &Enter, window, cx| view.accept_selected_command(window, cx)),
            )
            .capture_action(cx.listener(|view, _: &IndentInline, window, cx| {
                view.accept_selected_command(window, cx)
            }))
            .capture_action(cx.listener(|view, _: &OutdentInline, window, cx| {
                view.accept_selected_command(window, cx)
            }))
            .capture_action(Self::capture_editor_find(cx))
            .capture_action(Self::capture_editor_find_escape(cx))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| view.dismiss_commands(cx)),
            )
            .tab_index(4)
            .tab_stop(false)
            .tab_group()
            .flex_1()
            .min_h(px(0.))
            .child(
                Input::new(&editor.state)
                    .disabled(self.restore_task.is_some() || self.entry_write_task.is_some())
                    .h_full()
                    .appearance(false)
                    .px(px(4.))
                    .py(px(2.))
                    .font_family(monospace_font())
                    .text_size(px(14.))
                    .line_height(px(23.)),
            )
    }
}

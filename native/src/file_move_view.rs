use super::{Library, Palette, ProjectEntry, Selection};
use gpui::{
    Context, Div, Entity, FocusHandle, KeyDownEvent, Keystroke, MouseButton, Pixels, Point,
    Stateful, Subscription, Window, anchored, canvas, deferred, div, point, prelude::*, px, rgb,
};
use gpui_component::{
    WindowExt,
    dialog::DialogButtonProps,
    input::{Input, InputEvent, InputState},
};
use refract_native::{
    Project, ProjectEntryKind, ProjectEntryMove, move_project_entry, project_tree,
    versions::{Version, list_versions},
};
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    io,
    rc::Rc,
};

pub(super) struct EntryMoveDraft {
    source: String,
    kind: ProjectEntryKind,
    field: Entity<InputState>,
    error: Rc<RefCell<Option<String>>>,
    busy: Rc<Cell<bool>>,
    _watch_input: Subscription,
}

pub(super) struct EntryMenu {
    path: String,
    position: Point<Pixels>,
    focus: FocusHandle,
    origin: Option<FocusHandle>,
    actions: Vec<(EntryAction, FocusHandle)>,
    _focus_out: Subscription,
}

#[derive(Clone, Copy)]
enum EntryAction {
    AddItems,
    MoveFolder,
    RenameFile,
    DownloadFile,
    Delete(ProjectEntryKind),
}

impl EntryAction {
    fn label(self) -> &'static str {
        match self {
            Self::AddItems => "Add items",
            Self::MoveFolder => "Move or rename folder",
            Self::RenameFile => "Rename file",
            Self::DownloadFile => "Download file",
            Self::Delete(ProjectEntryKind::File) => "Delete file",
            Self::Delete(ProjectEntryKind::Folder) => "Delete folder",
        }
    }
}

struct MovedEntry {
    project: Project,
    tree: io::Result<Vec<ProjectEntry>>,
    versions: io::Result<Vec<Version>>,
}

impl Library {
    pub(super) fn entry_menu(
        row: Stateful<Div>,
        path: String,
        is_folder: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let keyboard_position = Rc::new(Cell::new(Point::default()));
        let row_position = keyboard_position.clone();
        let mouse_path = path.clone();
        let library = cx.weak_entity();
        row.relative()
            .child(
                canvas(
                    move |bounds, _, _| {
                        row_position.set(point(bounds.left(), bounds.bottom()));
                    },
                    |_, (), _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .on_mouse_down(MouseButton::Right, move |event, window, app| {
                let path = mouse_path.clone();
                let position = event.position;
                let library = library.clone();
                // Let the row receive default pointer focus before recording the menu's origin.
                window.defer(app, move |window, app| {
                    let _ = library.update(app, |library, cx| {
                        library.open_entry_menu(&path, is_folder, position, window, cx);
                    });
                });
            })
            .on_key_down(
                cx.listener(move |library, event: &KeyDownEvent, window, cx| {
                    let key = &event.keystroke;
                    if key.key == "menu" || (key.key == "f10" && key.modifiers.shift) {
                        cx.stop_propagation();
                        window.prevent_default();
                        library.open_entry_menu(
                            &path,
                            is_folder,
                            keyboard_position.get(),
                            window,
                            cx,
                        );
                    }
                }),
            )
    }

    fn open_entry_menu(
        &mut self,
        path: &str,
        is_folder: bool,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selection.is_none() || window.has_active_dialog(cx) {
            return;
        }
        let items: &[EntryAction] = if is_folder {
            &[
                EntryAction::AddItems,
                EntryAction::MoveFolder,
                EntryAction::Delete(ProjectEntryKind::Folder),
            ]
        } else {
            &[
                EntryAction::RenameFile,
                EntryAction::DownloadFile,
                EntryAction::Delete(ProjectEntryKind::File),
            ]
        };
        let actions: Vec<_> = items
            .iter()
            .map(|&action| (action, cx.focus_handle().tab_stop(true).tab_index(1)))
            .collect();
        let focus = cx.focus_handle();
        let origin = window.focused(cx);
        let library = cx.weak_entity();
        let focus_out = window.on_focus_out(&focus, cx, move |_, window, app| {
            let _ = library.update(app, |library, cx| {
                library.close_entry_menu(false, window, cx)
            });
        });
        window.focus(&actions[0].1);
        self.entry_menu = Some(EntryMenu {
            path: path.to_owned(),
            position,
            focus,
            origin,
            actions,
            _focus_out: focus_out,
        });
        cx.notify();
    }

    fn close_entry_menu(
        &mut self,
        restore_focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(menu) = self.entry_menu.take() {
            if restore_focus && let Some(origin) = menu.origin {
                window.focus(&origin);
            }
            cx.notify();
        }
    }

    fn handle_entry_menu_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let Some(menu) = &self.entry_menu else {
            return;
        };
        let current = menu
            .actions
            .iter()
            .position(|(_, focus)| focus.is_focused(window));
        match key {
            "escape" => self.close_entry_menu(true, window, cx),
            "enter" | "space" => {
                if let Some(index) = current {
                    self.activate_entry_menu_action(index, window, cx);
                }
            }
            "up" | "down" => {
                let count = menu.actions.len();
                let next = match (current, key) {
                    (Some(index), "up") => (index + count - 1) % count,
                    (Some(index), _) => (index + 1) % count,
                    (None, _) => 0,
                };
                window.focus(&menu.actions[next].1);
            }
            _ => return,
        }
        window.prevent_default();
        cx.stop_propagation();
    }

    fn activate_entry_menu_action(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(menu) = self.entry_menu.as_ref() else {
            return;
        };
        let path = menu.path.clone();
        let action = menu.actions[index].0;
        self.close_entry_menu(true, window, cx);
        match action {
            EntryAction::AddItems => self.open_create_entry_dialog(&path, window, cx),
            EntryAction::MoveFolder => {
                self.open_entry_move_dialog(&path, ProjectEntryKind::Folder, window, cx)
            }
            EntryAction::RenameFile => {
                self.open_entry_move_dialog(&path, ProjectEntryKind::File, window, cx)
            }
            EntryAction::DownloadFile => self.download_file(path, window, cx),
            EntryAction::Delete(kind) => self.confirm_entry_delete(path, kind, window, cx),
        }
    }

    pub(super) fn entry_menu_layer(
        &self,
        palette: Palette,
        cx: &Context<Self>,
    ) -> Option<impl IntoElement> {
        let menu = self.entry_menu.as_ref()?;
        let items = menu
            .actions
            .iter()
            .enumerate()
            .map(|(index, (action, focus))| {
                div()
                    .id(action.label())
                    .track_focus(focus)
                    .w_full()
                    .px(px(10.))
                    .py(px(8.))
                    .rounded_md()
                    .cursor_pointer()
                    .hover(move |item| item.bg(rgb(palette.selection)))
                    .focus(move |item| item.bg(rgb(palette.selection)))
                    .child(action.label())
                    .on_click(cx.listener(move |library, _, window, cx| {
                        cx.stop_propagation();
                        library.activate_entry_menu_action(index, window, cx);
                    }))
            });
        let popup = div()
            .id("entry-menu")
            .debug_selector(|| "entry-menu".to_owned())
            .track_focus(&menu.focus)
            .occlude()
            .min_w(px(180.))
            .p(px(5.))
            .rounded(px(8.))
            .border_1()
            .border_color(rgb(palette.input_border))
            .bg(rgb(palette.surface))
            .shadow_lg()
            .text_size(px(12.))
            .on_key_down(cx.listener(Self::handle_entry_menu_key))
            .on_mouse_down_out(
                cx.listener(|library, _, window, cx| library.close_entry_menu(true, window, cx)),
            )
            .children(items);
        Some(
            deferred(
                anchored()
                    .position(menu.position)
                    .snap_to_window_with_margin(px(8.))
                    .child(popup),
            )
            .with_priority(1),
        )
    }

    fn open_entry_move_dialog(
        &mut self,
        source: &str,
        kind: ProjectEntryKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selection.is_none() || !self.project_writes_idle() || window.has_active_dialog(cx) {
            return;
        }
        let (initial, title, label, action) = match kind {
            ProjectEntryKind::File => (
                source.rsplit('/').next().unwrap_or(source),
                "Rename file",
                "File name",
                "Rename",
            ),
            ProjectEntryKind::Folder => {
                (source, "Move or rename folder", "New project path", "Move")
            }
        };
        let field = cx.new(|cx| InputState::new(window, cx).default_value(initial.to_owned()));
        let error = Rc::new(RefCell::new(None::<String>));
        let clear_error = error.clone();
        let watch_input = cx.subscribe(&field, move |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) && clear_error.borrow_mut().take().is_some() {
                cx.notify();
            }
        });
        let busy = Rc::new(Cell::new(false));
        self.move_draft = Some(EntryMoveDraft {
            source: source.to_owned(),
            kind,
            field: field.clone(),
            error: error.clone(),
            busy: busy.clone(),
            _watch_input: watch_input,
        });
        let library = cx.weak_entity();
        let focus = field.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let submit = library.clone();
            let close = library.clone();
            let cancel = busy.clone();
            dialog
                .title(title)
                .confirm()
                .keyboard(!busy.get())
                .button_props(DialogButtonProps::default().ok_text(action))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(label)
                        .child(Input::new(&field).disabled(busy.get()))
                        .children(error.borrow().clone()),
                )
                .on_ok(move |_, window, app| {
                    submit
                        .update(app, |library, cx| library.submit_entry_move(window, cx))
                        .unwrap_or(true)
                })
                .on_cancel(move |_, _, _| !cancel.get())
                .on_close(move |_, _, app| {
                    let _ = close.update(app, |library, cx| {
                        library.move_draft = None;
                        cx.notify();
                    });
                })
        });
        window.defer(cx, move |window, app| {
            focus.update(app, |field, cx| field.focus(window, cx));
            let shortcut = if cfg!(target_os = "macos") {
                "cmd-a"
            } else {
                "ctrl-a"
            };
            window.dispatch_keystroke(
                Keystroke::parse(shortcut).expect("valid select-all shortcut"),
                app,
            );
        });
        cx.notify();
    }

    fn submit_entry_move(&mut self, window: &Window, cx: &mut Context<Self>) -> bool {
        let (Some(draft), Some(selection)) = (&self.move_draft, &self.selection) else {
            return true;
        };
        if draft.busy.get() {
            return false;
        }
        let fail = |message: String, cx: &mut Context<Self>| {
            *draft.error.borrow_mut() = Some(message);
            cx.notify();
            false
        };
        let value = draft.field.read(cx).value();
        let name = value.trim();
        let destination = match draft.kind {
            ProjectEntryKind::File => {
                if name.is_empty() {
                    return fail("Enter a file name.".into(), cx);
                }
                if name.contains(['/', '\\']) {
                    return fail("Enter a file name without a folder path.".into(), cx);
                }
                if name.starts_with('.') {
                    return fail(
                        "Choose a name that begins with a character other than a dot.".into(),
                        cx,
                    );
                }
                draft
                    .source
                    .rsplit_once('/')
                    .map_or_else(|| name.to_owned(), |(parent, _)| format!("{parent}/{name}"))
            }
            ProjectEntryKind::Folder => {
                if name.is_empty() {
                    return fail("Enter a new project path.".into(), cx);
                }
                name.replace('\\', "/")
            }
        };
        if destination == draft.source {
            return true;
        }
        if !self.project_writes_ready() {
            return fail(
                "Wait for the current project operation to finish.".into(),
                cx,
            );
        }
        let entry = match ProjectEntryMove::new(&draft.source, &destination, draft.kind) {
            Ok(entry) => entry,
            Err(error) => return fail(error.to_string(), cx),
        };
        let remapped: HashSet<String> = selection
            .tabs
            .iter()
            .filter_map(|tab| entry.remap_path(&tab.path))
            .collect();
        let conflicting = selection.tabs.iter().any(|tab| {
            (entry.remap_path(&tab.path).is_some() || remapped.contains(&tab.path))
                && tab
                    .editor
                    .as_ref()
                    .and_then(|editor| editor.as_ref().ok())
                    .is_some_and(|editor| editor.saved != editor.state.read(cx).value().as_ref())
        });
        if conflicting {
            return fail("Save pending changes before moving this item.".into(), cx);
        }
        draft.busy.set(true);
        draft.error.borrow_mut().take();
        let project = selection.project.clone();
        let dir = self.projects_dir.clone();
        let move_entry = entry.clone();
        let operation = cx.background_executor().spawn(async move {
            let project = move_project_entry(&dir, &project, &move_entry)?;
            Ok::<_, io::Error>(MovedEntry {
                tree: project_tree(&dir, &project),
                versions: list_versions(&dir, &project),
                project,
            })
        });
        self.entry_write_task = Some(cx.spawn_in(window, async move |library, cx| {
            let moved = operation.await;
            let _ = library.update_in(cx, |library, window, cx| {
                library.entry_write_task = None;
                match moved {
                    Ok(moved) => library.finish_entry_move(moved, &entry, remapped, window, cx),
                    Err(error) => {
                        if let Some(draft) = &library.move_draft {
                            draft.busy.set(false);
                            *draft.error.borrow_mut() = Some(error.to_string());
                            draft.field.update(cx, |field, cx| field.focus(window, cx));
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
        false
    }

    fn finish_entry_move(
        &mut self,
        moved: MovedEntry,
        entry: &ProjectEntryMove,
        remapped: HashSet<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_draft = None;
        window.close_dialog(cx);
        if let Some(Ok(projects)) = &mut self.projects
            && let Some(stored) = projects
                .iter_mut()
                .find(|project| project.id == moved.project.id)
        {
            *stored = moved.project.clone();
        }
        let Some(selection) = &mut self.selection else {
            return;
        };
        if selection.project.id != moved.project.id {
            return;
        }
        selection.project = moved.project;
        selection.tree = Some(moved.tree);
        selection.build.invalidate();
        selection.versions = Some(moved.versions);
        selection.comparison = None;
        self.comparison_task = None;
        self.tree_task = None;
        let source = selection
            .file
            .as_deref()
            .and_then(|path| entry.remap_path(path));
        remap_open_tabs(selection, entry, &remapped, window, cx);
        selection.expanded_folders = selection
            .expanded_folders
            .drain()
            .map(|path| entry.remap_path(&path).unwrap_or(path))
            .collect();
        let target = source.as_deref().unwrap_or(entry.destination());
        selection.expanded_folders.extend(
            target
                .match_indices('/')
                .map(|(index, _)| target[..index].to_owned()),
        );
        let comments = selection.tool == Some(crate::WorkspaceTool::Comments);
        if let Some(path) = source {
            self.open_file(path, window, cx);
        }
        if comments {
            self.load_comments(cx);
        }
    }
}

fn remap_open_tabs(
    selection: &mut Selection,
    entry: &ProjectEntryMove,
    remapped: &HashSet<String>,
    window: &Window,
    cx: &mut Context<Library>,
) {
    selection
        .tabs
        .retain(|tab| !remapped.contains(&tab.path) || entry.remap_path(&tab.path).is_some());
    for tab in &mut selection.tabs {
        if let Some(path) = entry.remap_path(&tab.path) {
            tab.path = path;
            if tab
                .editor
                .as_ref()
                .and_then(|editor| editor.as_ref().ok())
                .is_some_and(|editor| {
                    !refract_native::file_preview::is_text_asset(&tab.path, editor.saved.as_bytes())
                })
            {
                tab.editor = None;
            } else if let Some(Ok(editor)) = &mut tab.editor {
                editor._autosave = Library::watch_source(&editor.state, &tab.path, window, cx);
            }
        }
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use crate::initialize_app;
    use gpui::{Modifiers, TestAppContext};
    use gpui_component::{Root, input::Position};
    use refract_native::{create_project, list_projects};
    use std::fs;

    #[gpui::test]
    fn tree_context_menu_restores_focus_and_dispatches_actions(cx: &mut TestAppContext) {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let project = create_project(&projects, "Paper").unwrap();
        fs::create_dir(projects.join(&project.id).join("sections")).unwrap();
        cx.update(initialize_app);
        let mut library = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| Library::new(projects.clone(), window, cx));
            library = Some(view.clone());
            Root::new(view, window, cx)
        });
        let library = library.unwrap();
        cx.update(|window, app| {
            library.update(app, |view, cx| view.open_project(project, window, cx));
        });
        cx.run_until_parked();

        let folder_row = cx.debug_bounds("tree/sections").unwrap();
        cx.simulate_click(folder_row.center(), Modifiers::none());
        let folder_focus = cx.update(|window, app| window.focused(app).unwrap());
        cx.simulate_keystrokes("shift-f10");
        cx.update(|window, app| {
            let menu = library.read(app).entry_menu.as_ref().unwrap();
            assert_eq!(menu.path, "sections");
            assert_eq!(menu.position, point(folder_row.left(), folder_row.bottom()));
            assert!(menu.actions[0].1.is_focused(window));
        });
        assert!(cx.debug_bounds("entry-menu").is_some());
        cx.simulate_keystrokes("up");
        cx.update(|window, app| {
            let menu = library.read(app).entry_menu.as_ref().unwrap();
            assert!(menu.actions.last().unwrap().1.is_focused(window));
        });
        cx.simulate_keystrokes("down escape");
        cx.update(|window, app| {
            assert!(library.read(app).entry_menu.is_none());
            assert!(folder_focus.is_focused(window));
        });
        cx.simulate_keystrokes("menu down enter");
        cx.update(|window, app| {
            let view = library.read(app);
            assert!(view.entry_menu.is_none());
            let draft = view.move_draft.as_ref().unwrap();
            assert_eq!(draft.source, "sections");
            assert!(window.has_active_dialog(app));
        });
        cx.simulate_keystrokes("escape");

        let file_row = cx.debug_bounds("tree/main.tex").unwrap();
        cx.simulate_mouse_down(file_row.center(), MouseButton::Right, Modifiers::none());
        cx.simulate_mouse_up(file_row.center(), MouseButton::Right, Modifiers::none());
        cx.update(|window, app| {
            let menu = library.read(app).entry_menu.as_ref().unwrap();
            assert_eq!(menu.path, "main.tex");
            assert_eq!(menu.position, file_row.center());
            assert!(menu.actions[0].1.is_focused(window));
        });
        cx.simulate_keystrokes("escape shift-f10 enter");
        cx.update(|window, app| {
            let view = library.read(app);
            assert!(view.entry_menu.is_none());
            assert_eq!(view.move_draft.as_ref().unwrap().source, "main.tex");
            assert!(window.has_active_dialog(app));
        });
    }

    #[gpui::test]
    fn renames_and_moves_open_files_without_losing_editor_state(cx: &mut TestAppContext) {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let project = create_project(&projects, "Paper").unwrap();
        let folder = projects.join(&project.id);
        fs::create_dir(folder.join("sections")).unwrap();
        fs::write(folder.join("sections/intro.tex"), "Introduction\n").unwrap();
        cx.update(initialize_app);
        let mut library = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| Library::new(projects.clone(), window, cx));
            library = Some(view.clone());
            Root::new(view, window, cx)
        });
        let library = library.unwrap();
        cx.update(|window, app| {
            library.update(app, |view, cx| view.open_project(project, window, cx))
        });
        cx.run_until_parked();
        let main_editor = cx.update(|window, app| {
            let editor = library.read(app).selection.as_ref().unwrap().tabs[0]
                .editor
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap()
                .state
                .clone();
            library.update(app, |view, cx| {
                view.open_entry_move_dialog("main.tex", ProjectEntryKind::File, window, cx)
            });
            editor
        });
        cx.simulate_input("sections/invalid.tex");
        cx.simulate_keystrokes("enter");
        cx.update(|window, app| {
            assert!(window.has_active_dialog(app));
            let draft = library.read(app).move_draft.as_ref().unwrap();
            assert_eq!(
                draft.error.borrow().as_deref(),
                Some("Enter a file name without a folder path.")
            );
        });
        cx.simulate_keystrokes(if cfg!(target_os = "macos") {
            "cmd-a"
        } else {
            "ctrl-a"
        });
        cx.simulate_input("paper.tex");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        cx.update(|window, app| {
            assert!(!window.has_active_dialog(app));
            let selection = library.read(app).selection.as_ref().unwrap();
            assert_eq!(selection.project.root_file(), "paper.tex");
            assert_eq!(selection.file.as_deref(), Some("paper.tex"));
            assert_eq!(
                selection.tabs[0]
                    .editor
                    .as_ref()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .state
                    .entity_id(),
                main_editor.entity_id()
            );
            library.update(app, |view, cx| {
                view.open_file("sections/intro.tex".into(), window, cx)
            });
        });
        assert!(!folder.join("main.tex").exists());
        cx.run_until_parked();
        let intro_editor = cx.update(|window, app| {
            let editor = library.read(app).selection.as_ref().unwrap().tabs[1]
                .editor
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap()
                .state
                .clone();
            library.update(app, |view, cx| {
                view.open_entry_move_dialog("sections", ProjectEntryKind::Folder, window, cx)
            });
            editor
        });
        cx.simulate_input("chapters");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        cx.update(|window, app| {
            assert!(!window.has_active_dialog(app));
            let selection = library.read(app).selection.as_ref().unwrap();
            assert_eq!(selection.file.as_deref(), Some("chapters/intro.tex"));
            assert!(selection.expanded_folders.contains("chapters"));
            assert_eq!(
                selection.tabs[1]
                    .editor
                    .as_ref()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .state
                    .entity_id(),
                intro_editor.entity_id()
            );
            intro_editor.update(app, |editor, cx| {
                editor.set_cursor_position(Position::new(0, 0), window, cx)
            });
        });
        cx.simulate_input("Updated ");
        cx.run_until_parked();
        assert_eq!(
            fs::read_to_string(folder.join("chapters/intro.tex")).unwrap(),
            "Updated Introduction\n"
        );
        assert!(!folder.join("sections").exists());
        assert_eq!(
            list_projects(&projects).unwrap()[0].root_file(),
            "paper.tex"
        );
    }
}

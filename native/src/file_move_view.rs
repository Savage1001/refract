use super::{Library, ProjectEntry, Selection};
use gpui::{Context, Entity, Keystroke, Subscription, Window, div, prelude::*};
use gpui_component::{
    WindowExt,
    dialog::DialogButtonProps,
    input::{Input, InputEvent, InputState},
    menu::{ContextMenuExt, PopupMenuItem},
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

struct MovedEntry {
    project: Project,
    tree: io::Result<Vec<ProjectEntry>>,
    versions: io::Result<Vec<Version>>,
}

impl Library {
    pub(super) fn entry_menu<E>(
        row: E,
        path: String,
        is_folder: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement
    where
        E: ParentElement + Styled + IntoElement + 'static,
    {
        let library = cx.weak_entity();
        row.context_menu(move |menu, _, _| {
            let menu = if is_folder {
                let path = path.clone();
                let library = library.clone();
                menu.item(
                    PopupMenuItem::new("Add items").on_click(move |_, window, app| {
                        let _ = library.update(app, |library, cx| {
                            library.open_create_entry_dialog(&path, window, cx)
                        });
                    }),
                )
            } else {
                menu
            };
            let path = path.clone();
            let library = library.clone();
            let (kind, label) = if is_folder {
                (ProjectEntryKind::Folder, "Move or rename folder")
            } else {
                (ProjectEntryKind::File, "Rename file")
            };
            menu.item(PopupMenuItem::new(label).on_click(move |_, window, app| {
                let _ = library.update(app, |library, cx| {
                    library.open_entry_move_dialog(&path, kind, window, cx)
                });
            }))
        })
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
        let comments = selection.show_comments;
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
            if let Some(Ok(editor)) = &mut tab.editor {
                editor._autosave = Library::watch_source(&editor.state, &tab.path, window, cx);
            }
        }
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use crate::initialize_app;
    use gpui::TestAppContext;
    use gpui_component::{Root, input::Position};
    use refract_native::{create_project, list_projects};
    use std::fs;

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

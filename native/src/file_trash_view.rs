use super::{Library, Palette, ProjectEntry};
use crate::deleted_projects_view::recovery_button;
use chrono::Local;
use gpui::{
    App, Context, FocusHandle, KeyBinding, KeyDownEvent, PromptLevel, SharedString, WeakEntity,
    Window, actions, div, prelude::*, px, rgb,
};
use gpui_component::WindowExt;
use refract_native::{
    DeletedEntry, ProjectEntryKind, list_deleted_entries, project_tree, restore_deleted_entry,
    trash_project_entry,
};
use std::{cell::RefCell, io, rc::Rc};

actions!(
    deleted_files,
    [NextDeletedFileControl, PreviousDeletedFileControl]
);

pub(super) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", NextDeletedFileControl, Some("DeletedFilesDialog")),
        KeyBinding::new(
            "shift-tab",
            PreviousDeletedFileControl,
            Some("DeletedFilesDialog"),
        ),
    ]);
}

pub(super) struct DeletedFilesDialog {
    rows: Vec<(DeletedEntry, FocusHandle)>,
    close_focus: FocusHandle,
    busy: bool,
    error: Option<String>,
    notice: Option<String>,
}

struct RestoredEntry {
    entry: DeletedEntry,
    tree: io::Result<Vec<ProjectEntry>>,
    trash: io::Result<Vec<DeletedEntry>>,
}

impl DeletedFilesDialog {
    fn focus_row(&self, index: usize, window: &mut Window) {
        let focus = self
            .rows
            .get(index.min(self.rows.len().saturating_sub(1)))
            .map(|(_, focus)| focus)
            .unwrap_or(&self.close_focus);
        window.focus(focus);
    }

    fn replace_items(&mut self, items: Vec<DeletedEntry>, cx: &Context<Library>) {
        self.rows = items
            .into_iter()
            .map(|item| (item, cx.focus_handle().tab_stop(true)))
            .collect();
    }

    fn cycle_focus(&self, backward: bool, window: &mut Window) {
        let count = self.rows.len() + 1;
        let current = self
            .rows
            .iter()
            .position(|(_, focus)| focus.is_focused(window))
            .unwrap_or(count - 1);
        let next = if backward {
            (current + count - 1) % count
        } else {
            (current + 1) % count
        };
        let focus = self
            .rows
            .get(next)
            .map(|(_, focus)| focus)
            .unwrap_or(&self.close_focus);
        window.focus(focus);
    }
}

impl Library {
    pub(super) fn confirm_entry_delete(
        &mut self,
        path: String,
        kind: ProjectEntryKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(selection) = &self.selection else {
            return;
        };
        if !self.project_writes_idle() || window.has_active_dialog(cx) {
            return;
        }
        let project_id = selection.project.id.clone();
        let (message, confirm) = match kind {
            ProjectEntryKind::File => (format!("Delete {path}?"), "Delete file"),
            ProjectEntryKind::Folder => (
                format!("Move {path} and its contents to Deleted files?"),
                "Delete folder",
            ),
        };
        let answer = window.prompt(
            PromptLevel::Warning,
            &message,
            None,
            &["Cancel", confirm],
            cx,
        );
        self.entry_delete_prompt = Some(cx.spawn_in(window, async move |library, cx| {
            let confirmed = matches!(answer.await, Ok(1));
            let _ = library.update_in(cx, |library, window, cx| {
                library.entry_delete_prompt = None;
                if confirmed {
                    library.delete_entry(&project_id, path, kind, window, cx);
                }
                cx.notify();
            });
        }));
    }

    fn delete_entry(
        &mut self,
        project_id: &str,
        path: String,
        kind: ProjectEntryKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ready = self.project_writes_idle();
        let Some(selection) = self
            .selection
            .as_mut()
            .filter(|selection| selection.project.id == project_id)
        else {
            return;
        };
        let has_edits = selection.tabs.iter().any(|tab| {
            entry_contains(&path, kind, &tab.path)
                && tab
                    .editor
                    .as_ref()
                    .and_then(|editor| editor.as_ref().ok())
                    .is_some_and(|editor| editor.saved != editor.state.read(cx).value().as_ref())
        });
        if !ready || has_edits {
            selection.file_action_error =
                Some("Save pending changes before deleting this item.".into());
            return;
        }
        selection.file_action_error = None;
        let project = selection.project.clone();
        let dir = self.projects_dir.clone();
        let operation = cx.background_executor().spawn(async move {
            let result = trash_project_entry(&dir, &project, &path, kind)
                .map(|record| (record, project_tree(&dir, &project)));
            (project.id, result)
        });
        self.entry_write_task = Some(cx.spawn_in(window, async move |library, cx| {
            let (id, result) = operation.await;
            let _ = library.update_in(cx, |library, window, cx| {
                library.entry_write_task = None;
                library.finish_entry_delete(&id, result, window, cx);
                library.resume_after_trash_write(window, cx);
            });
        }));
    }

    fn finish_entry_delete(
        &mut self,
        project_id: &str,
        result: io::Result<(DeletedEntry, io::Result<Vec<ProjectEntry>>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(selection) = self
            .selection
            .as_mut()
            .filter(|selection| selection.project.id == project_id)
        else {
            return;
        };
        let (entry, tree) = match result {
            Ok(moved) => moved,
            Err(error) => {
                selection.file_action_error = Some(format!("Couldn’t delete item. {error}"));
                return;
            }
        };
        let removed = |path: &str| entry_contains(&entry.path, entry.kind(), path);
        self.tree_task = None;
        selection.tree = Some(tree);
        selection.build.invalidate();
        selection.expanded_folders.retain(|path| !removed(path));
        selection.tabs.retain(|tab| !removed(&tab.path));
        if selection.file.as_deref().is_some_and(removed) {
            self.preview_task = None;
            selection.file = None;
            selection.pending_comment = None;
            selection.comment_status = None;
            if let Some(tab) = selection.tabs.last() {
                let path = tab.path.clone();
                self.open_file(path, window, cx);
            } else {
                self.outline.clear(window);
                self.commands.close();
            }
        }
        window.focus(&self.deleted_files_focus);
    }

    pub(super) fn deleted_files_button(
        &self,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        div()
            .id("deleted-files")
            .track_focus(&self.deleted_files_focus)
            .rounded_md()
            .px_3()
            .py_2()
            .cursor_pointer()
            .hover(move |style| style.bg(rgb(palette.selection)))
            .focus(move |style| style.bg(rgb(palette.selection)))
            .child("Deleted files")
            .on_click(cx.listener(|library, _, window, cx| library.open_deleted_files(window, cx)))
            .on_key_down(cx.listener(|library, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                    library.open_deleted_files(window, cx);
                }
            }))
    }

    fn open_deleted_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(selection) = &self.selection else {
            return;
        };
        if self.deleted_files_dialog.is_some() || window.has_active_dialog(cx) {
            return;
        }
        let project = selection.project.clone();
        let state = Rc::new(RefCell::new(DeletedFilesDialog {
            rows: Vec::new(),
            close_focus: cx.focus_handle().tab_stop(true),
            busy: true,
            error: None,
            notice: None,
        }));
        self.deleted_files_dialog = Some(state.clone());
        let library = cx.weak_entity();
        let content = state.clone();
        window.open_dialog(cx, move |dialog, window, _| {
            let close = library.clone();
            dialog
                .title("Deleted files")
                .close_button(false)
                .overlay_closable(false)
                .keyboard(!content.borrow().busy)
                .child(deleted_files_contents(
                    &content,
                    &library,
                    Palette::for_window(window),
                ))
                .on_close(move |_, window, app| {
                    let _ = close.update(app, |library, cx| {
                        library.deleted_files_dialog = None;
                        window.focus(&library.deleted_files_focus);
                        cx.notify();
                    });
                })
        });
        window.focus(&state.borrow().close_focus);
        let dir = self.projects_dir.clone();
        let load = cx
            .background_executor()
            .spawn(async move { list_deleted_entries(&dir, &project) });
        cx.spawn_in(window, async move |library, cx| {
            let items = load.await;
            let _ = library.update_in(cx, |library, window, cx| {
                if !library
                    .deleted_files_dialog
                    .as_ref()
                    .is_some_and(|active| Rc::ptr_eq(active, &state))
                {
                    return;
                }
                let mut state = state.borrow_mut();
                state.busy = false;
                match items {
                    Ok(items) => state.replace_items(items, cx),
                    Err(error) => {
                        state.error = Some(format!("Couldn’t load deleted files. {error}"))
                    }
                }
                state.focus_row(0, window);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn restore_trashed_entry(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.deleted_files_dialog.clone() else {
            return;
        };
        let Some(selection) = &self.selection else {
            return;
        };
        let mut dialog = state.borrow_mut();
        if dialog.busy {
            return;
        }
        if !self.project_writes_ready() {
            dialog.error = Some("Wait for the current project operation to finish.".into());
            cx.notify();
            return;
        }
        let Some((record, _)) = dialog.rows.get(index) else {
            return;
        };
        let id = record.id.clone();
        dialog.busy = true;
        dialog.error = None;
        dialog.notice = None;
        drop(dialog);
        let project = selection.project.clone();
        let dir = self.projects_dir.clone();
        let operation = cx.background_executor().spawn(async move {
            let result = restore_deleted_entry(&dir, &project, &id).map(|entry| RestoredEntry {
                entry,
                tree: project_tree(&dir, &project),
                trash: list_deleted_entries(&dir, &project),
            });
            (project.id, result)
        });
        self.entry_write_task = Some(cx.spawn_in(window, async move |library, cx| {
            let (project_id, result) = operation.await;
            let _ = library.update_in(cx, |library, window, cx| {
                library.entry_write_task = None;
                let mut dialog = state.borrow_mut();
                dialog.busy = false;
                match result {
                    Ok(restored) => {
                        dialog.notice = Some(format!("Restored {}", restored.entry.path));
                        match restored.trash {
                            Ok(items) => dialog.replace_items(items, cx),
                            Err(error) => {
                                dialog.rows.remove(index);
                                dialog.error =
                                    Some(format!("Couldn’t refresh deleted files. {error}"));
                            }
                        }
                        library.finish_entry_restore(
                            &project_id,
                            restored.entry,
                            restored.tree,
                            window,
                            cx,
                        );
                    }
                    Err(error) => dialog.error = Some(format!("Couldn’t restore item. {error}")),
                }
                if library
                    .deleted_files_dialog
                    .as_ref()
                    .is_some_and(|active| Rc::ptr_eq(active, &state))
                {
                    dialog.focus_row(index, window);
                }
                library.resume_after_trash_write(window, cx);
            });
        }));
        cx.notify();
    }

    fn finish_entry_restore(
        &mut self,
        project_id: &str,
        entry: DeletedEntry,
        tree: io::Result<Vec<ProjectEntry>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(selection) = self
            .selection
            .as_mut()
            .filter(|selection| selection.project.id == project_id)
        else {
            return;
        };
        self.tree_task = None;
        selection.tree = Some(tree);
        selection.build.invalidate();
        let root = selection.project.root_file().replace('\\', "/");
        let open = match entry.kind() {
            ProjectEntryKind::File => Some(entry.path),
            ProjectEntryKind::Folder
                if selection.file.is_none() && entry_contains(&entry.path, entry.kind(), &root) =>
            {
                Some(root)
            }
            ProjectEntryKind::Folder => None,
        };
        if let Some(open) = open {
            selection.expanded_folders.extend(
                open.match_indices('/')
                    .map(|(index, _)| open[..index].to_owned()),
            );
            self.open_file(open, window, cx);
        }
    }

    fn resume_after_trash_write(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.start_next_save(window, cx);
        if self.snapshot_timer.is_none() {
            self.start_snapshot(window, cx);
        }
        self.resume_navigation(window, cx);
        cx.notify();
    }
}

fn entry_contains(path: &str, kind: ProjectEntryKind, candidate: &str) -> bool {
    match kind {
        ProjectEntryKind::File => path == candidate,
        ProjectEntryKind::Folder => {
            let path = path.to_lowercase();
            let candidate = candidate.to_lowercase();
            candidate
                .strip_prefix(&path)
                .is_some_and(|suffix| suffix.is_empty() || suffix.starts_with('/'))
        }
    }
}

fn deleted_files_contents(
    state: &Rc<RefCell<DeletedFilesDialog>>,
    library: &WeakEntity<Library>,
    palette: Palette,
) -> impl IntoElement {
    let forward = state.clone();
    let backward = state.clone();
    let close = library.clone();
    let state = state.borrow();
    let busy = state.busy;
    let list = div()
        .id("deleted-file-list")
        .max_h(px(320.))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap_2();
    let list =
        if state.rows.is_empty() && state.error.is_none() {
            if busy {
                list.child("Loading deleted files…")
            } else {
                list.child("No deleted files")
                    .child(div().text_color(rgb(palette.muted)).child(
                        "Files and folders deleted from this project appear here for restoration.",
                    ))
            }
        } else {
            list.children(
                state
                    .rows
                    .iter()
                    .enumerate()
                    .map(|(index, (record, focus))| {
                        let library = library.clone();
                        let (name, kind) = match record.kind() {
                            ProjectEntryKind::File => (record.path.clone(), "File"),
                            ProjectEntryKind::Folder => (format!("{}/", record.path), "Folder"),
                        };
                        let detail = match record.deleted_at() {
                            Some(date) => format!(
                                "{kind} · Deleted {}",
                                date.with_timezone(&Local).format("%x %X")
                            ),
                            None => kind.to_owned(),
                        };
                        div()
                            .id(SharedString::from(record.id.clone()))
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .border_b_1()
                            .border_color(rgb(palette.border))
                            .p_2()
                            .child(div().min_w(px(0.)).child(name).child(
                                div().text_xs().text_color(rgb(palette.muted)).child(detail),
                            ))
                            .child(recovery_button(
                                "restore",
                                "Restore",
                                focus,
                                palette,
                                !busy,
                                move |window, app| {
                                    let _ = library.update(app, |library, cx| {
                                        library.restore_trashed_entry(index, window, cx)
                                    });
                                },
                            ))
                    }),
            )
        };
    div()
        .id("deleted-files-dialog")
        .key_context("DeletedFilesDialog")
        .flex()
        .flex_col()
        .gap_3()
        .on_action(move |_: &NextDeletedFileControl, window, _| {
            forward.borrow().cycle_focus(false, window)
        })
        .on_action(move |_: &PreviousDeletedFileControl, window, _| {
            backward.borrow().cycle_focus(true, window)
        })
        .child(list)
        .children(
            state
                .error
                .clone()
                .map(|error| div().text_color(rgb(palette.warning)).child(error)),
        )
        .children(state.notice.clone())
        .child(div().flex().justify_end().child(recovery_button(
            "close",
            "Close",
            &state.close_focus,
            palette,
            !busy,
            move |window, app| {
                let _ = close.update(app, |library, cx| {
                    library.deleted_files_dialog = None;
                    window.close_dialog(cx);
                    window.focus(&library.deleted_files_focus);
                    cx.notify();
                });
            },
        )))
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use crate::initialize_app;
    use gpui::{Modifiers, TestAppContext};
    use gpui_component::Root;
    use refract_native::create_project;
    use std::fs;

    #[gpui::test]
    fn deletes_a_folder_from_the_tree_and_restores_it_from_deleted_files(cx: &mut TestAppContext) {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let project = create_project(&projects, "Paper").unwrap();
        let dir = projects.join(&project.id);
        fs::create_dir(dir.join("sections")).unwrap();
        let source = dir.join("sections/intro.tex");
        fs::write(&source, "Introduction\n").unwrap();
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
        let folder = cx.debug_bounds("tree/sections").unwrap();
        cx.simulate_click(folder.center(), Modifiers::none());
        let file = cx.debug_bounds("tree/sections/intro.tex").unwrap();
        cx.simulate_click(file.center(), Modifiers::none());
        cx.update(|_, app| {
            let selection = library.read(app).selection.as_ref().unwrap();
            assert_eq!(selection.file.as_deref(), Some("sections/intro.tex"));
            assert_eq!(selection.tabs.len(), 2);
        });

        // Focus the folder without toggling its contents while opening the menu.
        cx.simulate_mouse_down(folder.center(), gpui::MouseButton::Right, Modifiers::none());
        cx.simulate_mouse_up(folder.center(), gpui::MouseButton::Right, Modifiers::none());
        cx.simulate_keystrokes("up enter");
        assert!(cx.has_pending_prompt());
        cx.simulate_prompt_answer("Cancel");
        cx.run_until_parked();
        assert!(source.exists());
        cx.simulate_keystrokes("shift-f10 up enter");
        assert!(cx.has_pending_prompt());
        cx.simulate_prompt_answer("Delete folder");
        cx.run_until_parked();
        assert!(!source.exists());
        cx.update(|window, app| {
            let view = library.read(app);
            let selection = view.selection.as_ref().unwrap();
            assert_eq!(selection.file.as_deref(), Some("main.tex"));
            assert_eq!(selection.tabs.len(), 1);
            assert!(view.deleted_files_focus.is_focused(window));
        });

        cx.simulate_keystrokes("enter");
        cx.update(|window, app| {
            let dialog = library
                .read(app)
                .deleted_files_dialog
                .as_ref()
                .unwrap()
                .borrow();
            assert_eq!(dialog.rows[0].0.path, "sections");
            assert!(dialog.rows[0].1.is_focused(window));
        });
        cx.simulate_keystrokes("tab");
        cx.update(|window, app| {
            let view = library.read(app);
            let dialog = view.deleted_files_dialog.as_ref().unwrap().borrow();
            assert!(dialog.close_focus.is_focused(window));
        });
        cx.simulate_keystrokes("shift-tab");
        cx.update(|window, app| {
            let view = library.read(app);
            let dialog = view.deleted_files_dialog.as_ref().unwrap().borrow();
            assert!(dialog.rows[0].1.is_focused(window));
        });
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        cx.update(|window, app| {
            let dialog = library
                .read(app)
                .deleted_files_dialog
                .as_ref()
                .unwrap()
                .borrow();
            assert!(dialog.error.is_none(), "restore failed: {:?}", dialog.error);
            assert_eq!(dialog.notice.as_deref(), Some("Restored sections"));
            assert!(dialog.rows.is_empty());
            assert!(dialog.close_focus.is_focused(window));
        });
        assert_eq!(fs::read_to_string(source).unwrap(), "Introduction\n");
        cx.simulate_keystrokes("enter");
        cx.update(|window, app| {
            let view = library.read(app);
            assert!(view.deleted_files_dialog.is_none());
            assert!(view.deleted_files_focus.is_focused(window));
        });
    }
}

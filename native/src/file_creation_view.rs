use super::{Library, Palette};
use gpui::{
    App, Context, Entity, KeyBinding, KeyDownEvent, PathPromptOptions, SharedString, Subscription,
    WeakEntity, Window, actions, div, prelude::*, px, rgb, svg,
};
use gpui_component::{
    WindowExt,
    dialog::DialogButtonProps,
    input::{Input, InputEvent, InputState},
    select::{Select, SelectItem, SelectState},
    tooltip::Tooltip,
};
use refract_native::{
    Project, ProjectEntry, ProjectEntryKind, create_project_entry, project_tree,
    upload_project_files, upload_project_folder,
};
use std::{
    cell::{Cell, RefCell},
    io,
    path::{Path, PathBuf},
    rc::Rc,
};

actions!(workspace_creation, [ActivateAlternative]);

pub(super) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new(
            "enter",
            ActivateAlternative,
            Some("FileCreationAlternative"),
        ),
        KeyBinding::new(
            "space",
            ActivateAlternative,
            Some("FileCreationAlternative"),
        ),
    ]);
}

#[derive(Clone)]
struct FolderChoice(String);

impl SelectItem for FolderChoice {
    type Value = String;

    fn title(&self) -> SharedString {
        if self.0.is_empty() {
            "Project root".into()
        } else {
            self.0.clone().into()
        }
    }

    fn value(&self) -> &String {
        &self.0
    }
}

pub(super) struct EntryDraft {
    name: Entity<InputState>,
    destination: Entity<SelectState<Vec<FolderChoice>>>,
    error: Rc<RefCell<Option<String>>>,
    _watch_input: Subscription,
}

impl EntryDraft {
    fn selected_folder(&self, cx: &App) -> String {
        self.destination
            .read(cx)
            .selected_value()
            .cloned()
            .unwrap_or_default()
    }
}

#[derive(Clone, Copy)]
enum Upload {
    Files,
    Folder,
}

impl Upload {
    fn label(self) -> &'static str {
        match self {
            Self::Files => "Upload file",
            Self::Folder => "Upload folder",
        }
    }

    fn apply(
        self,
        projects: &Path,
        project: &Project,
        folder: &str,
        paths: &[PathBuf],
    ) -> io::Result<()> {
        match self {
            Self::Files => upload_project_files(projects, project, folder, paths).map(|_| ()),
            Self::Folder => match paths.first() {
                Some(path) => upload_project_folder(projects, project, folder, path).map(|_| ()),
                None => Ok(()),
            },
        }
    }
}

impl Library {
    pub(super) fn add_items_button(
        &self,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        div()
            .id("add-items")
            .tab_index(1)
            .cursor_pointer()
            .rounded_md()
            .w(px(28.))
            .h(px(32.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .hover(move |style| style.bg(rgb(palette.selection)))
            .focus(move |style| style.bg(rgb(palette.selection)))
            .tooltip(|window, cx| Tooltip::new("Add file or folder").build(window, cx))
            .child(
                svg()
                    .path("refract/plus.svg")
                    .size(px(18.))
                    .text_color(rgb(palette.muted)),
            )
            .on_click(cx.listener(|library, _, window, cx| {
                library.open_create_entry_dialog("", window, cx)
            }))
            .on_key_down(cx.listener(|library, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                    library.open_create_entry_dialog("", window, cx);
                }
            }))
    }

    pub(super) fn open_create_entry_dialog(
        &mut self,
        folder: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.project_writes_idle() || window.has_active_dialog(cx) {
            return;
        }
        let Some(selection) = &self.selection else {
            return;
        };
        let Some(Ok(tree)) = &selection.tree else {
            return;
        };
        let mut choices = vec![FolderChoice(String::new())];
        collect_folders(tree, &mut choices);
        let folder = if choices.iter().any(|choice| choice.0 == folder) {
            folder.to_owned()
        } else {
            String::new()
        };
        let destination = cx.new(|cx| {
            let mut select = SelectState::new(choices, None, window, cx);
            select.set_selected_value(&folder, window, cx);
            select
        });
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("e.g. references.bib"));
        let error = Rc::new(RefCell::new(None::<String>));
        let clear_error = error.clone();
        let watch_input = cx.subscribe(&name, move |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) && clear_error.borrow_mut().take().is_some() {
                cx.notify();
            }
        });
        self.entry_draft = Some(EntryDraft {
            name: name.clone(),
            destination: destination.clone(),
            error: error.clone(),
            _watch_input: watch_input,
        });

        let library = cx.weak_entity();
        let field = name.clone();
        let alternatives = Rc::new(Cell::new(false));
        window.open_dialog(cx, move |dialog, window, _| {
            let submit = library.clone();
            let close = library.clone();
            dialog
                .title("New file")
                .confirm()
                .button_props(DialogButtonProps::default().ok_text("Create file"))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child("Folder")
                        .child(Select::new(&destination))
                        .child("File name")
                        .child(Input::new(&field))
                        .children(error.borrow().clone())
                        .child(file_alternatives(
                            &alternatives,
                            &library,
                            Palette::for_window(window),
                        )),
                )
                .on_ok(move |_, window, app| {
                    submit
                        .update(app, |library, cx| {
                            library.submit_create_entry(ProjectEntryKind::File, window, cx)
                        })
                        .unwrap_or(true)
                })
                .on_close(move |_, _, app| {
                    let _ = close.update(app, |library, cx| {
                        library.entry_draft = None;
                        cx.notify();
                    });
                })
        });
        window.defer(cx, move |window, app| {
            name.update(app, |state, cx| state.focus(window, cx))
        });
        cx.notify();
    }

    fn submit_create_entry(
        &mut self,
        kind: ProjectEntryKind,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let (Some(draft), Some(selection)) = (&self.entry_draft, &mut self.selection) else {
            return true;
        };
        let value = draft.name.read(cx).value();
        let name = value.trim();
        if name.is_empty() {
            *draft.error.borrow_mut() = Some("Enter a file or folder name.".into());
            cx.notify();
            return false;
        }
        let destination = draft.selected_folder(cx);
        let path = if destination.is_empty() {
            name.to_owned()
        } else {
            format!("{destination}/{name}")
        };
        let project = selection.project.clone();
        if !self.project_writes_ready() {
            return false;
        }
        if let Some(selection) = &mut self.selection {
            selection.file_action_error = None;
        }
        let id = project.id.clone();
        let projects = self.projects_dir.clone();
        let create = cx.background_executor().spawn(async move {
            let result = create_project_entry(&projects, &project, &path, kind);
            (result, project_tree(&projects, &project))
        });
        self.entry_write_task = Some(cx.spawn_in(window, async move |library, cx| {
            let (result, tree) = create.await;
            let _ = library.update_in(cx, |library, window, cx| {
                library.entry_write_task = None;
                if let Some(selection) = &mut library.selection
                    && selection.project.id == id
                {
                    selection.tree = Some(tree);
                    match result {
                        Ok(path) => {
                            selection.build.invalidate();
                            selection.expanded_folders.extend(
                                path.match_indices('/')
                                    .map(|(index, _)| path[..index].to_owned()),
                            );
                            if kind == ProjectEntryKind::File {
                                library.open_file(path, window, cx);
                            }
                        }
                        Err(error) => {
                            let item = if kind == ProjectEntryKind::File {
                                "file"
                            } else {
                                "folder"
                            };
                            selection.file_action_error =
                                Some(format!("Couldn’t create {item}. {error}"));
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
        true
    }

    fn pick_upload(&mut self, upload: Upload, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(draft), Some(selection)) = (&self.entry_draft, &self.selection) else {
            return;
        };
        if !self.project_writes_ready() {
            *draft.error.borrow_mut() =
                Some("Wait for the current project operation to finish.".into());
            cx.notify();
            return;
        }
        let folder = draft.selected_folder(cx);
        let project = selection.project.clone();
        let project_id = project.id.clone();
        let projects = self.projects_dir.clone();
        self.entry_draft = None;
        window.close_dialog(cx);
        let select_files = matches!(upload, Upload::Files);
        let pick = cx.prompt_for_paths(PathPromptOptions {
            files: select_files,
            directories: !select_files,
            multiple: select_files,
            prompt: Some(upload.label().into()),
        });
        let executor = cx.background_executor().clone();
        self.entry_write_task = Some(cx.spawn_in(window, async move |library, cx| {
            let (error, tree) = match pick.await {
                Ok(Ok(Some(paths))) if !paths.is_empty() => {
                    executor
                        .spawn(async move {
                            let error = upload
                                .apply(&projects, &project, &folder, &paths)
                                .err()
                                .map(|error| error.to_string());
                            (error, Some(project_tree(&projects, &project)))
                        })
                        .await
                }
                Ok(Err(error)) => (Some(error.to_string()), None),
                _ => (None, None),
            };
            let _ = library.update_in(cx, |library, window, cx| {
                library.entry_write_task = None;
                if let Some(selection) = &mut library.selection
                    && selection.project.id == project_id
                {
                    if let Some(tree) = tree {
                        library.tree_task = None;
                        selection.tree = Some(tree);
                        selection.file_action_error = None;
                        selection.build.invalidate();
                    }
                    if let Some(error) = error {
                        let action = if select_files {
                            "upload files"
                        } else {
                            "upload folder"
                        };
                        selection.file_action_error = Some(format!("Couldn’t {action}. {error}"));
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
}

fn collect_folders(entries: &[ProjectEntry], folders: &mut Vec<FolderChoice>) {
    for entry in entries {
        if let Some(children) = &entry.children {
            folders.push(FolderChoice(entry.path.clone()));
            collect_folders(children, folders);
        }
    }
}

fn file_alternatives(
    alternatives: &Rc<Cell<bool>>,
    library: &WeakEntity<Library>,
    palette: Palette,
) -> impl IntoElement {
    let toggle = library.clone();
    let action = library.clone();
    let shown = alternatives.get();
    let alternatives = alternatives.clone();
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(alternative_button(
            "file-alternatives",
            "More file options",
            palette,
            move |_, app| {
                alternatives.set(!alternatives.get());
                let _ = toggle.update(app, |_, cx| cx.notify());
            },
        ))
        .when(shown, |container| {
            container
                .child(alternative_button(
                    "create-folder",
                    "Create folder",
                    palette,
                    move |window, app| {
                        let _ = action.update(app, |library, cx| {
                            if library.submit_create_entry(ProjectEntryKind::Folder, window, cx) {
                                library.entry_draft = None;
                                window.close_dialog(cx);
                                cx.notify();
                            }
                        });
                    },
                ))
                .children(
                    [
                        ("upload-files", Upload::Files),
                        ("upload-folder", Upload::Folder),
                    ]
                    .into_iter()
                    .map(|(id, upload)| {
                        let library = library.clone();
                        alternative_button(id, upload.label(), palette, move |window, app| {
                            let _ = library
                                .update(app, |library, cx| library.pick_upload(upload, window, cx));
                        })
                    }),
                )
        })
}

fn alternative_button(
    id: &'static str,
    label: &'static str,
    palette: Palette,
    activate: impl Fn(&mut Window, &mut App) + Clone + 'static,
) -> impl IntoElement {
    let keyboard = activate.clone();
    div()
        .id(id)
        .debug_selector(|| id.to_owned())
        .tab_index(0)
        .key_context("FileCreationAlternative")
        .cursor_pointer()
        .rounded_md()
        .px_2()
        .py_1()
        .hover(move |style| style.bg(rgb(palette.selection)))
        .focus(move |style| style.bg(rgb(palette.selection)))
        .child(label)
        .on_click(move |_, window, app| activate(window, app))
        .on_action(move |_: &ActivateAlternative, window, app| keyboard(window, app))
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use crate::initialize_app;
    use gpui::TestAppContext;
    use gpui_component::Root;
    use refract_native::create_project;
    use std::fs;

    #[gpui::test]
    fn creates_a_folder_and_opens_a_new_file_in_the_chosen_folder(cx: &mut TestAppContext) {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let project = create_project(&projects, "Paper").unwrap();
        let folder = projects.join(&project.id);
        cx.update(initialize_app);
        let mut library = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| Library::new(projects, window, cx));
            library = Some(view.clone());
            Root::new(view, window, cx)
        });
        let library = library.unwrap();
        cx.update(|window, app| {
            library.update(app, |view, cx| view.open_project(project, window, cx))
        });
        cx.run_until_parked();
        // From the project name, Tab reaches Files, Chats, then Add.
        cx.simulate_keystrokes("tab tab tab enter");
        cx.update(|window, app| assert!(window.has_active_dialog(app)));
        cx.simulate_keystrokes("enter");
        cx.update(|window, app| {
            assert!(window.has_active_dialog(app));
            let draft = library.read(app).entry_draft.as_ref().unwrap();
            assert_eq!(
                draft.error.borrow().as_deref(),
                Some("Enter a file or folder name.")
            );
        });
        cx.simulate_input("sections");
        cx.simulate_keystrokes("tab enter");
        assert!(cx.debug_bounds("upload-files").is_some());
        assert!(cx.debug_bounds("upload-folder").is_some());
        cx.simulate_keystrokes("tab enter");
        cx.run_until_parked();
        assert!(folder.join("sections").is_dir());
        cx.update(|window, app| {
            assert!(!window.has_active_dialog(app));
            assert_eq!(
                library
                    .read(app)
                    .selection
                    .as_ref()
                    .unwrap()
                    .file
                    .as_deref(),
                Some("main.tex")
            );
            library.update(app, |view, cx| {
                view.open_create_entry_dialog("sections", window, cx)
            });
        });
        cx.update(|_, app| {
            let draft = library.read(app).entry_draft.as_ref().unwrap();
            assert_eq!(
                draft
                    .destination
                    .read(app)
                    .selected_value()
                    .map(String::as_str),
                Some("sections")
            );
        });
        cx.simulate_input("references.bib");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert!(
            fs::read(folder.join("sections/references.bib"))
                .unwrap()
                .is_empty()
        );
        cx.update(|window, app| {
            assert!(!window.has_active_dialog(app));
            let selection = library.read(app).selection.as_ref().unwrap();
            assert_eq!(selection.file.as_deref(), Some("sections/references.bib"));
            assert!(selection.expanded_folders.contains("sections"));
            assert_eq!(selection.tabs.len(), 2);
        });
    }
}

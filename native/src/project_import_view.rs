use super::{Library, Navigation, Palette};
use gpui::{
    Context, Entity, KeyDownEvent, PathPromptOptions, SharedString, Window, div, prelude::*, px,
    rgb, svg,
};
use gpui_component::{
    WindowExt,
    dialog::DialogButtonProps,
    select::{Select, SelectItem, SelectState},
};
use refract_native::{ProjectArchive, list_projects};

#[derive(Clone, PartialEq)]
enum RootChoice {
    Document(String),
    All,
}

impl SelectItem for RootChoice {
    type Value = Self;

    fn title(&self) -> SharedString {
        match self {
            Self::Document(path) => path.clone().into(),
            Self::All => "Import all".into(),
        }
    }

    fn value(&self) -> &Self {
        self
    }
}

pub(super) struct ImportDraft {
    archive: ProjectArchive,
    root: Entity<SelectState<Vec<RootChoice>>>,
    sequence: u64,
}

impl Library {
    pub(super) fn project_library_actions(
        &self,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(self.settings_button(palette, cx))
            .child(self.import_project_button(palette, cx))
            .child(self.create_project_button("new-project", "New project", palette, cx))
    }

    fn import_project_button(&self, palette: Palette, cx: &Context<Self>) -> impl IntoElement {
        let enabled = self.can_create_project();
        div()
            .id("import-zip")
            .track_focus(&self.import_focus)
            .rounded_md()
            .flex()
            .items_center()
            .justify_center()
            .size(px(36.))
            .text_color(rgb(palette.text))
            .tooltip(|window, cx| gpui_component::tooltip::Tooltip::new("Import").build(window, cx))
            .child(
                svg()
                    .path("refract/import.svg")
                    .size(px(20.))
                    .text_color(rgb(palette.text)),
            )
            .when(!enabled, |button| button.opacity(0.5))
            .when(enabled, |button| {
                button
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.selection)))
                    .focus(move |style| style.bg(rgb(palette.selection)))
                    .on_click(cx.listener(|library, _, window, cx| {
                        library.pick_project_archive(window, cx)
                    }))
                    .on_key_down(cx.listener(|library, event: &KeyDownEvent, window, cx| {
                        if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                            library.pick_project_archive(window, cx);
                        }
                    }))
            })
    }

    fn pick_project_archive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_create_project() || window.has_active_dialog(cx) {
            return;
        }
        self.dismiss_project_menu(cx);
        self.project_action_error = None;
        let sequence = self.project_open_sequence;
        let pick = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Import project ZIP".into()),
        });
        let executor = cx.background_executor().clone();
        self.project_library_task = Some(cx.spawn_in(window, async move |library, cx| {
            let path = match pick.await {
                Ok(Ok(Some(paths))) => Ok(paths.into_iter().next()),
                Ok(Err(error)) => Err(error.to_string()),
                Ok(Ok(None)) | Err(_) => Ok(None),
            };
            let inspected = match path {
                Ok(Some(path)) => {
                    executor
                        .spawn(async move {
                            ProjectArchive::open(&path)
                                .map(Some)
                                .map_err(|error| error.to_string())
                        })
                        .await
                }
                Ok(None) => Ok(None),
                Err(error) => Err(error),
            };
            let _ = library.update_in(cx, |library, window, cx| {
                library.project_library_task = None;
                match inspected {
                    Ok(Some(archive))
                        if !matches!(
                            library.navigation_after_save,
                            Some(Navigation::CloseWindow)
                        ) =>
                    {
                        library.import_inspected_archive(archive, sequence, window, cx);
                    }
                    Ok(_) => {}
                    Err(error) => {
                        library.project_action_error =
                            Some(format!("Couldn’t import project. {error}"))
                    }
                }
                library.resume_navigation(window, cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn import_inspected_archive(
        &mut self,
        archive: ProjectArchive,
        sequence: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut choices: Vec<_> = archive
            .roots()
            .iter()
            .cloned()
            .map(RootChoice::Document)
            .collect();
        if choices.len() == 1 {
            let roots = archive.roots().to_vec();
            self.write_imported_projects(archive, roots, sequence, window, cx);
            return;
        }
        let first = choices[0].clone();
        choices.push(RootChoice::All);
        let root = cx.new(|cx| {
            let mut state = SelectState::new(choices, None, window, cx);
            state.set_selected_value(&first, window, cx);
            state
        });
        self.import_draft = Some(ImportDraft {
            archive,
            root: root.clone(),
            sequence,
        });
        let library = cx.weak_entity();
        let focus = root.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let submit = library.clone();
            let close = library.clone();
            dialog
                .title(crate::dialog_title("Select the main LaTeX document"))
                .confirm()
                .button_props(DialogButtonProps::default().ok_text("Import"))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            "The selected document will be used to compile the imported project.",
                        )
                        .child(Select::new(&root)),
                )
                .on_ok(move |_, window, app| {
                    submit
                        .update(app, |library, cx| library.submit_project_import(window, cx))
                        .unwrap_or(true)
                })
                .on_close(move |_, _, app| {
                    let _ = close.update(app, |library, cx| {
                        library.import_draft = None;
                        cx.notify();
                    });
                })
        });
        window.defer(cx, move |window, app| {
            focus.update(app, |root, cx| root.focus(window, cx));
        });
        cx.notify();
    }

    fn submit_project_import(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(draft) = self.import_draft.as_ref() else {
            return true;
        };
        let roots = match draft.root.read(cx).selected_value() {
            Some(RootChoice::Document(path)) => vec![path.clone()],
            Some(RootChoice::All) => draft.archive.roots().to_vec(),
            None => return false,
        };
        let draft = self
            .import_draft
            .take()
            .expect("import choice is still active");
        self.write_imported_projects(draft.archive, roots, draft.sequence, window, cx);
        true
    }

    fn write_imported_projects(
        &mut self,
        archive: ProjectArchive,
        roots: Vec<String>,
        sequence: u64,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let projects = self.projects_dir.clone();
        let import = cx.background_executor().spawn(async move {
            let imported = archive.import(&projects, &roots);
            (imported, list_projects(&projects))
        });
        self.project_library_task = Some(cx.spawn_in(window, async move |library, cx| {
            let (imported, projects) = import.await;
            let _ = library.update_in(cx, |library, window, cx| {
                library.project_library_task = None;
                library.set_projects(projects, cx);
                match imported {
                    Ok(project)
                        if library.selection.is_none()
                            && library.project_open_sequence == sequence
                            && !matches!(
                                library.navigation_after_save,
                                Some(Navigation::CloseWindow)
                            ) =>
                    {
                        library.open_project(project, window, cx);
                    }
                    Ok(_) => {}
                    Err(error) => {
                        library.project_action_error =
                            Some(format!("Couldn’t import project. {error}"))
                    }
                }
                library.resume_navigation(window, cx);
                cx.notify();
            });
        }));
        cx.notify();
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use crate::initialize_app;
    use gpui::TestAppContext;
    use gpui_component::Root;
    use std::{fs::File, io::Write};
    use zip::{ZipWriter, write::SimpleFileOptions};

    #[gpui::test]
    fn selects_an_archive_root_and_keeps_canceled_projects_out_of_the_library(
        cx: &mut TestAppContext,
    ) {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let path = profile.path().join("papers.zip");
        let mut zip = ZipWriter::new(File::create(&path).unwrap());
        for name in ["first.tex", "second.tex"] {
            zip.start_file(name, SimpleFileOptions::default()).unwrap();
            zip.write_all(br"\documentclass{article}").unwrap();
        }
        drop(zip.finish().unwrap());
        cx.update(initialize_app);
        let mut library = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| Library::new(projects.clone(), window, cx));
            library = Some(view.clone());
            Root::new(view, window, cx)
        });
        let library = library.unwrap();
        cx.run_until_parked();

        let inspect = |view: &mut Library, window: &mut Window, cx: &mut Context<Library>| {
            // The test platform has no open-file picker. Pass the inspected fixture to the dialog.
            let archive = ProjectArchive::open(&path).unwrap();
            view.import_inspected_archive(archive, view.project_open_sequence, window, cx);
        };
        cx.update(|window, app| library.update(app, |view, cx| inspect(view, window, cx)));
        cx.simulate_keystrokes("escape");
        cx.update(|window, app| {
            assert!(!window.has_active_dialog(app));
            assert!(library.read(app).import_draft.is_none());
        });
        assert!(!projects.exists());

        cx.update(|window, app| library.update(app, |view, cx| inspect(view, window, cx)));
        cx.update(|window, app| {
            let draft = library.read(app).import_draft.as_ref().unwrap();
            let root = draft.root.clone();
            root.update(app, |root, cx| {
                root.set_selected_value(&RootChoice::Document("second.tex".to_owned()), window, cx);
            });
        });
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        cx.update(|window, app| {
            assert!(!window.has_active_dialog(app));
            let view = library.read(app);
            assert!(view.import_draft.is_none());
            let selection = view
                .selection
                .as_ref()
                .expect("import should open the selected document");
            assert_eq!(selection.project.root_file(), "second.tex");
            assert_eq!(selection.file.as_deref(), Some("second.tex"));
        });
        assert_eq!(list_projects(&projects).unwrap().len(), 1);
    }
}

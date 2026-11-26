use super::{Library, Navigation, Palette};
use gpui::{Context, KeyDownEvent, Window, div, prelude::*, rgb};
use gpui_component::WindowExt;
use refract_native::{Project, download_project_file, export_project_zip, tex::save_compiled_pdf};
use std::{
    io,
    path::{Path, PathBuf},
};

enum Transfer {
    File(String),
    Zip,
    Pdf(PathBuf),
}

impl Transfer {
    fn suggested_name(&self) -> &str {
        match self {
            Self::File(path) => path.rsplit('/').next().unwrap_or(path),
            Self::Zip => "project.zip",
            Self::Pdf(_) => "main.pdf",
        }
    }

    fn operation(&self) -> &'static str {
        match self {
            Self::File(_) => "download file",
            Self::Zip => "export project",
            Self::Pdf(_) => "download PDF",
        }
    }

    fn write(self, projects: &Path, project: &Project, destination: &Path) -> io::Result<()> {
        match self {
            Self::File(path) => download_project_file(projects, project, &path, destination),
            Self::Zip => export_project_zip(projects, project, destination),
            Self::Pdf(path) => save_compiled_pdf(&path, destination),
        }
    }
}

impl Library {
    pub(super) fn export_project_button(
        &self,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        div()
            .id("export-project")
            .debug_selector(|| "export-project".to_owned())
            .tab_index(5)
            .cursor_pointer()
            .rounded_md()
            .px_2()
            .py_1()
            .focus(move |style| style.bg(rgb(palette.selection)))
            .hover(move |style| style.bg(rgb(palette.selection)))
            .child("Export project ZIP")
            .on_click(cx.listener(|library, _, window, cx| {
                library.start_file_transfer(Transfer::Zip, window, cx)
            }))
            .on_key_down(cx.listener(|library, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                    library.start_file_transfer(Transfer::Zip, window, cx);
                }
            }))
    }

    pub(super) fn download_file(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_file_transfer(Transfer::File(path), window, cx);
    }

    pub(super) fn download_built_pdf(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self
            .selection
            .as_ref()
            .and_then(|selection| selection.build.pdf())
            .map(Path::to_path_buf)
        {
            self.start_file_transfer(Transfer::Pdf(path), window, cx);
        }
    }

    fn start_file_transfer(
        &mut self,
        transfer: Transfer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if window.has_active_dialog(cx) {
            return;
        }
        let ready = self.project_writes_idle();
        let scope = match &transfer {
            Transfer::File(path) => Navigation::CloseFile(path.clone()),
            Transfer::Zip | Transfer::Pdf(_) => Navigation::Projects,
        };
        let has_edits = self.has_unsaved_changes(&scope, cx);
        let Some(selection) = &mut self.selection else {
            return;
        };
        let action = transfer.operation();
        if !ready || has_edits {
            selection.file_action_error = Some(format!(
                "Couldn’t {action}. Save pending changes and try again."
            ));
            cx.notify();
            return;
        }
        selection.file_action_error = None;
        let project = selection.project.clone();
        let id = project.id.clone();
        let projects = self.projects_dir.clone();
        let directory = if cfg!(windows) {
            PathBuf::new()
        } else {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default()
        };
        let destination = cx.prompt_for_new_path(&directory, Some(transfer.suggested_name()));
        let executor = cx.background_executor().clone();
        self.entry_write_task = Some(cx.spawn_in(window, async move |library, cx| {
            let result = match destination.await {
                Ok(Ok(Some(path))) => {
                    executor
                        .spawn(async move {
                            transfer
                                .write(&projects, &project, &path)
                                .map_err(|error| error.to_string())
                        })
                        .await
                }
                Ok(Err(error)) => Err(error.to_string()),
                Ok(Ok(None)) | Err(_) => Ok(()),
            };
            let _ = library.update_in(cx, |library, window, cx| {
                library.entry_write_task = None;
                if let Err(error) = result
                    && let Some(selection) = &mut library.selection
                    && selection.project.id == id
                {
                    selection.file_action_error = Some(format!("Couldn’t {action}. {error}"));
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

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use crate::initialize_app;
    use gpui::{Modifiers, MouseButton, TestAppContext};
    use gpui_component::Root;
    use refract_native::create_project;
    use std::fs;
    use zip::ZipArchive;

    #[gpui::test]
    fn downloads_binary_files_and_exports_the_zip_through_the_save_picker(cx: &mut TestAppContext) {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let project = create_project(&projects, "Paper").unwrap();
        fs::write(projects.join(&project.id).join("attachment.bin"), [0, 255]).unwrap();
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

        let row = cx.debug_bounds("tree/attachment.bin").unwrap();
        cx.simulate_mouse_down(row.center(), MouseButton::Right, Modifiers::none());
        cx.simulate_mouse_up(row.center(), MouseButton::Right, Modifiers::none());
        cx.simulate_keystrokes("down enter");
        let download = profile.path().join("download.bin");
        cx.simulate_new_path_selection(|_| Some(download.clone()));
        cx.run_until_parked();
        assert_eq!(fs::read(download).unwrap(), [0, 255]);

        let button = cx.debug_bounds("export-project").unwrap();
        cx.simulate_click(button.center(), Modifiers::none());
        cx.simulate_new_path_selection(|_| None);
        cx.run_until_parked();
        cx.update(|_, app| {
            let view = library.read(app);
            assert!(view.entry_write_task.is_none());
            assert!(view.selection.as_ref().unwrap().file_action_error.is_none());
        });
        cx.simulate_keystrokes("enter");
        let zip = profile.path().join("paper.zip");
        cx.simulate_new_path_selection(|_| Some(zip.clone()));
        cx.run_until_parked();
        let mut archive = ZipArchive::new(fs::File::open(zip).unwrap()).unwrap();
        assert!(archive.by_name("attachment.bin").is_ok());
        assert!(archive.by_name("main.tex").is_ok());
    }
}

use super::pdf_view::{PDF_PANE_WIDTH, PdfViewport};
use super::{Library, Navigation, Palette};
use gpui::{Context, FontWeight, KeyDownEvent, Task, Window, div, prelude::*, px, rgb};
use gpui_component::WindowExt;
#[cfg(not(all(test, feature = "ui-tests")))]
use refract_native::tex::{TexToolchain, compile_project};
use refract_native::{Project, tex::BuildResult};
use std::{
    io,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub(super) type Compiler = fn(&Path, &Project, &AtomicBool) -> io::Result<BuildResult>;

pub(super) struct RunningBuild {
    cancel: Arc<AtomicBool>,
    _task: Task<()>,
}

pub(super) struct WorkspaceBuild {
    pub(super) queued: bool,
    outcome: Option<Result<BuildResult, String>>,
    pub(super) sync_error: Option<String>,
    pub(super) preview: PdfViewport,
}

impl WorkspaceBuild {
    pub(super) fn new() -> Self {
        Self {
            queued: true,
            outcome: None,
            sync_error: None,
            preview: PdfViewport::default(),
        }
    }

    pub(super) fn invalidate(&mut self) {
        self.queued = true;
        self.outcome = None;
        self.sync_error = None;
        self.preview.clear_sync();
    }

    pub(super) fn pdf(&self) -> Option<&Path> {
        self.outcome.as_ref()?.as_ref().ok()?.pdf.as_deref()
    }

    pub(super) fn log(&self) -> Option<&str> {
        match self.outcome.as_ref()? {
            Ok(result) => Some(result.log.trim()),
            Err(error) => Some(error),
        }
    }
}

impl Library {
    pub(super) fn start_queued_build(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pdf_task.is_some() || !self.project_writes_idle() || window.has_active_dialog(cx) {
            return;
        }
        // GPUI tests use a fixture compiler so opening a project never launches TeX.
        #[cfg(all(test, feature = "ui-tests"))]
        let Some(compiler) = self.test_compiler else {
            return;
        };
        #[cfg(not(all(test, feature = "ui-tests")))]
        let compiler: Compiler = |path, project, cancel| {
            let toolchain = TexToolchain::discover()?;
            compile_project(path, project, &toolchain, cancel)
        };
        let has_edits = self.has_unsaved_changes(&Navigation::Projects, cx);
        let Some(selection) = self
            .selection
            .as_mut()
            .filter(|selection| selection.build.queued)
        else {
            return;
        };
        selection.build.queued = false;
        if has_edits {
            selection.build.outcome = Some(Err("Save pending changes before compiling.".into()));
            cx.notify();
            return;
        }

        let project = selection.project.clone();
        let id = project.id.clone();
        let sequence = self.project_open_sequence;
        let projects = self.projects_dir.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let run_cancel = cancel.clone();
        let compile = cx.background_executor().spawn(async move {
            compiler(&projects, &project, &run_cancel).map_err(|error| error.to_string())
        });
        let task = cx.spawn_in(window, async move |library, cx| {
            let result = compile.await;
            let _ = library.update_in(cx, |library, window, cx| {
                library.build_task = None;
                if let Some(selection) = &mut library.selection
                    && selection.project.id == id
                    && library.project_open_sequence == sequence
                    && !selection.build.queued
                {
                    selection.build.outcome = Some(result);
                    library.load_built_pdf(window, cx);
                }
                library.start_next_save(window, cx);
                if library.snapshot_timer.is_none() {
                    library.start_snapshot(window, cx);
                }
                library.resume_navigation(window, cx);
                cx.notify();
            });
        });
        self.build_task = Some(RunningBuild {
            cancel,
            _task: task,
        });
        cx.notify();
    }

    pub(super) fn stop_build(&mut self) {
        if let Some(build) = &self.build_task {
            build.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(sync) = &self.sync_task {
            sync.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(selection) = &mut self.selection {
            selection.build.queued = false;
        }
    }

    fn request_build(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.build_task.is_none()
            && let Some(selection) = &mut self.selection
        {
            selection.build.invalidate();
            self.start_queued_build(window, cx);
            cx.notify();
        }
    }

    pub(super) fn compile_pane(
        &self,
        build: &WorkspaceBuild,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let compiling = self.build_task.is_some();
        let compile_label = if compiling { "Compiling…" } else { "Compile" };
        let status = if compiling {
            "Compiling…"
        } else if build.queued {
            "Changes pending compilation"
        } else if build.pdf().is_some() {
            "PDF ready"
        } else if build.outcome.is_some() {
            "Compilation failed"
        } else {
            "PDF preview"
        };
        let body = if build.preview.has_content() {
            let status = if compiling || build.queued {
                Some("Showing the last successful build while changes compile.")
            } else if build.pdf().is_none() {
                Some(status)
            } else {
                None
            };
            build
                .preview
                .content(
                    status,
                    palette,
                    build.pdf().is_some() && self.sync_task.is_none(),
                    cx,
                )
                .into_any_element()
        } else {
            div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .child(div().font_weight(FontWeight::SEMIBOLD).child(status))
                .when(build.pdf().is_none() && build.outcome.is_some(), |pane| {
                    pane.child("Open Logs in Tools for details.")
                })
                .into_any_element()
        };
        div()
            .id("preview-pane")
            .w(px(PDF_PANE_WIDTH))
            .flex_none()
            .flex()
            .flex_col()
            .bg(rgb(palette.surface))
            .border_l_1()
            .border_color(rgb(palette.border))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .p_2()
                    .border_b_1()
                    .border_color(rgb(palette.border))
                    .child(preview_button(
                        "compile",
                        compile_label,
                        !compiling,
                        palette,
                        cx,
                        Self::request_build,
                    ))
                    .child(preview_button(
                        "download-pdf",
                        "Download PDF",
                        build.pdf().is_some() && self.entry_write_task.is_none(),
                        palette,
                        cx,
                        Self::download_built_pdf,
                    )),
            )
            .child(self.pdf_controls(&build.preview, build.pdf().is_some(), palette, cx))
            .children(build.sync_error.clone().map(|error| {
                div()
                    .px_3()
                    .py_2()
                    .text_color(rgb(palette.warning))
                    .child(error)
            }))
            .child(div().flex_1().min_h(px(0.)).child(body))
    }
}

fn preview_button(
    id: &'static str,
    label: &'static str,
    enabled: bool,
    palette: Palette,
    cx: &Context<Library>,
    activate: fn(&mut Library, &mut Window, &mut Context<Library>),
) -> impl IntoElement {
    div()
        .id(id)
        .debug_selector(|| id.to_owned())
        .rounded_md()
        .px_2()
        .py_1()
        .child(label)
        .when(!enabled, |button| button.text_color(rgb(palette.muted)))
        .when(enabled, |button| {
            button
                .tab_index(5)
                .cursor_pointer()
                .hover(move |style| style.bg(rgb(palette.selection)))
                .focus(move |style| style.bg(rgb(palette.selection)))
                .on_click(cx.listener(move |library, _, window, cx| activate(library, window, cx)))
                .on_key_down(
                    cx.listener(move |library, event: &KeyDownEvent, window, cx| {
                        if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                            activate(library, window, cx);
                        }
                    }),
                )
        })
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use crate::initialize_app;
    use gpui::{Modifiers, TestAppContext};
    use gpui_component::Root;
    use refract_native::create_project;
    use std::fs;

    fn fixture_compile(
        projects: &Path,
        project: &Project,
        cancel: &AtomicBool,
    ) -> io::Result<BuildResult> {
        assert!(!cancel.load(Ordering::Relaxed));
        let root = projects.join(&project.id);
        let source = fs::read_to_string(root.join("main.tex"))?;
        let pdf = root.join(".refract-build/main.pdf");
        fs::create_dir_all(pdf.parent().unwrap())?;
        fs::write(&pdf, format!("%PDF-fixture\n{source}"))?;
        Ok(BuildResult {
            arguments: Vec::new(),
            exit_code: Some(0),
            log: "Fixture build output".into(),
            pdf: Some(pdf),
        })
    }

    #[gpui::test]
    fn recompiles_saved_changes_and_downloads_the_latest_pdf(cx: &mut TestAppContext) {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let project = create_project(&projects, "Paper").unwrap();
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
                view.test_compiler = Some(fixture_compile);
                view.open_project(project, window, cx);
            });
        });
        cx.run_until_parked();
        cx.update(|window, app| {
            let selection = library.read(app).selection.as_ref().unwrap();
            assert!(selection.build.pdf().is_some());
            let editor = selection.tabs[0]
                .editor
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap()
                .state
                .clone();
            editor.update(app, |input, cx| input.focus(window, cx));
        });
        cx.simulate_input("New draft");
        cx.run_until_parked();
        cx.update(|_, app| {
            let selection = library.read(app).selection.as_ref().unwrap();
            let pdf = selection
                .build
                .pdf()
                .expect("updated build should be ready");
            assert!(fs::read_to_string(pdf).unwrap().contains("New draft"));
        });
        let button = cx.debug_bounds("compile").unwrap();
        cx.simulate_click(button.center(), Modifiers::none());
        let download = cx.debug_bounds("download-pdf").unwrap();
        cx.simulate_click(download.center(), Modifiers::none());
        let output = profile.path().join("saved.pdf");
        cx.simulate_new_path_selection(|_| Some(output.clone()));
        cx.run_until_parked();
        assert!(fs::read_to_string(output).unwrap().contains("New draft"));
    }
}

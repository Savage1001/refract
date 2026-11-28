use super::pdf_view::PdfViewport;
use super::{
    Library, Navigation, Palette, Selection, build_log_view::tree_has_file, monospace_font,
    workspace_tools::tool_button,
};
use gpui::{
    ClickEvent, Context, FontWeight, KeyDownEvent, Task, Window, div, prelude::*, px, rgb, svg,
};
use gpui_component::{WindowExt, tooltip::Tooltip};
use refract_native::tex::BuildResult;
use refract_native::tex::diagnostics::diagnostic_source;
#[cfg(not(all(test, feature = "ui-tests")))]
use refract_native::tex::{TexToolchain, compile_project_with_preview};
#[cfg(all(test, feature = "ui-tests"))]
use refract_native::{Project, tex::retain_compiled_pdf};
use std::{
    io,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[cfg(all(test, feature = "ui-tests"))]
pub(super) type Compiler = fn(&Path, &Project, &AtomicBool, bool) -> io::Result<BuildResult>;

pub(super) struct RunningBuild {
    cancel: Arc<AtomicBool>,
    _task: Task<()>,
}

pub(super) struct WorkspaceBuild {
    pub(super) queued: bool,
    force_next: bool,
    outcome: Option<Result<BuildResult, String>>,
    show_error_log: bool,
    pub(super) sync_error: Option<String>,
    pub(super) preview: PdfViewport,
}

impl WorkspaceBuild {
    pub(super) fn new() -> Self {
        Self {
            queued: true,
            force_next: false,
            outcome: None,
            show_error_log: false,
            sync_error: None,
            preview: PdfViewport::default(),
        }
    }

    pub(super) fn invalidate(&mut self) {
        self.queued = true;
        self.outcome = None;
        self.show_error_log = false;
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
        if !self.project_writes_idle() || window.has_active_dialog(cx) {
            return;
        }
        // GPUI tests use a fixture compiler so opening a project never launches TeX.
        #[cfg(all(test, feature = "ui-tests"))]
        let Some(compiler) = self.test_compiler else {
            return;
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
        let force = std::mem::take(&mut selection.build.force_next);
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
            #[cfg(all(test, feature = "ui-tests"))]
            let result = compiler(&projects, &project, &run_cancel, force).map(|build| {
                let preview = build
                    .pdf
                    .as_ref()
                    .map(|_| retain_compiled_pdf(&projects, &project, &run_cancel));
                (build, preview)
            });
            #[cfg(not(all(test, feature = "ui-tests")))]
            let result = TexToolchain::discover().and_then(|toolchain| {
                compile_project_with_preview(&projects, &project, &toolchain, &run_cancel, force)
            });
            result.map_err(|error| error.to_string())
        });
        let task = cx.spawn_in(window, async move |library, cx| {
            let (result, preview) = match compile.await {
                Ok((build, preview)) => (Ok(build), preview),
                Err(error) => (Err(error), None),
            };
            let _ = library.update_in(cx, |library, window, cx| {
                library.build_task = None;
                if let Some(selection) = &mut library.selection
                    && selection.project.id == id
                    && library.project_open_sequence == sequence
                    && !selection.build.queued
                {
                    selection.build.outcome = Some(result);
                    if let Some(preview) = preview.filter(|preview| {
                        !matches!(preview, Err(error) if error.kind() == io::ErrorKind::Interrupted)
                    }) {
                        library.load_built_pdf(preview, window, cx);
                    }
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
            selection.build.force_next = false;
        }
    }

    fn request_build(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.build_task.is_none()
            && let Some(selection) = &mut self.selection
        {
            selection.build.invalidate();
            selection.build.force_next = true;
            self.start_queued_build(window, cx);
            cx.notify();
        }
    }

    pub(super) fn compile_pane(
        &self,
        selection: &Selection,
        pane_width: f32,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let build = &selection.build;
        let compiling = self.build_task.is_some();
        let retained = build.preview.has_content();
        let failure =
            !compiling && !build.queued && build.outcome.is_some() && build.pdf().is_none();
        let compile_label = if compiling {
            "Compiling…"
        } else if build.pdf().is_some() {
            "Compiled"
        } else if build.outcome.is_some() {
            "Errors"
        } else {
            "Compile"
        };
        let status = if compiling {
            "Compiling…"
        } else if build.queued {
            "Changes pending compilation"
        } else if build.pdf().is_some() {
            "PDF ready"
        } else {
            "PDF preview"
        };
        let body = if retained {
            let status = if compiling || build.queued {
                Some("Showing the last successful build while changes compile.")
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
        } else if !failure {
            div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .child(div().font_weight(FontWeight::SEMIBOLD).child(status))
                .when(build.queued, |pane| {
                    pane.child("Your preview will update after the document is compiled.")
                })
                .into_any_element()
        } else {
            div().into_any_element()
        };
        div()
            .id("preview-pane")
            .flex_1()
            .min_w(px(0.))
            .flex()
            .flex_col()
            .bg(rgb(palette.selection))
            .border_l_1()
            .border_color(rgb(palette.border))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .flex_none()
                    .items_center()
                    .gap(px(6.))
                    .px(px(if pane_width <= 651. { 8. } else { 16. }))
                    .py(px(12.))
                    .bg(rgb(palette.canvas))
                    .border_b_1()
                    .border_color(rgb(palette.border))
                    .child(
                        preview_button("compile", !compiling, palette, cx, Self::request_build)
                            .px(px(10.))
                            .gap(px(7.))
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .tooltip(|window, cx| {
                                Tooltip::new("Compile document").build(window, cx)
                            })
                            .child(
                                svg()
                                    .path("refract/compile.svg")
                                    .size(px(16.))
                                    .text_color(rgb(palette.muted)),
                            )
                            .child(compile_label),
                    )
                    .children(self.pdf_controls(&build.preview, pane_width - 1., palette, cx))
                    .child(
                        preview_button(
                            "download-pdf",
                            build.pdf().is_some() && self.entry_write_task.is_none(),
                            palette,
                            cx,
                            Self::download_built_pdf,
                        )
                        .ml_auto()
                        .w(px(30.))
                        .tooltip(|window, cx| Tooltip::new("Download PDF").build(window, cx))
                        .child(
                            svg()
                                .path("refract/download.svg")
                                .size(px(18.))
                                .text_color(rgb(palette.muted)),
                        ),
                    ),
            )
            .children(build.sync_error.clone().map(|error| {
                div()
                    .px_3()
                    .py_2()
                    .text_color(rgb(palette.warning))
                    .child(error)
            }))
            .when(failure, |pane| {
                pane.child(self.preview_error(selection, retained, palette, cx))
            })
            .child(div().flex_1().min_h(px(0.)).child(body))
    }

    fn preview_error(
        &self,
        selection: &Selection,
        retained: bool,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let build = &selection.build;
        let log = build.log().unwrap_or_default();
        let diagnostic = log
            .lines()
            .find_map(|line| Some((diagnostic_source(line)?, line)));
        let cause = diagnostic
            .as_ref()
            .and_then(|(_, line)| line.splitn(3, ':').nth(2).map(str::trim))
            .or_else(|| log.lines().find(|line| line.starts_with("! ")))
            .or_else(|| log.lines().find(|line| !line.trim().is_empty()))
            .unwrap_or("Check the build output for details, then compile again.");
        let location = log.lines().find(|line| {
            line.strip_prefix("l.")
                .is_some_and(|line| line.starts_with(|c: char| c.is_ascii_digit()))
        });
        let summary = location.map_or_else(|| cause.to_owned(), |line| format!("{cause} - {line}"));
        let position = diagnostic.map(|(position, _)| position).filter(|position| {
            selection
                .tree
                .as_ref()
                .and_then(|tree| tree.as_ref().ok())
                .is_some_and(|entries| tree_has_file(entries, &position.file))
        });
        let (error, surface) = if palette.canvas == 0x0a0a0a {
            (0xff6467, 0x301214)
        } else {
            (0xc10007, 0xfcebec)
        };
        div()
            .id("preview-error")
            .debug_selector(|| "preview-error".into())
            .flex()
            .flex_col()
            .flex_none()
            .items_start()
            .gap(px(8.))
            .mx(px(12.))
            .mt(px(8.))
            .p(px(20.))
            .max_h(if retained {
                gpui::relative(0.35)
            } else {
                gpui::relative(0.8)
            })
            .overflow_y_scroll()
            .rounded(px(8.))
            .border_1()
            .border_color(rgb(error))
            .bg(rgb(surface))
            .text_size(px(12.))
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(error))
                    .child("Could not compile"),
            )
            .child(summary)
            .child(if retained {
                "Showing the last successful PDF. Fix the issue and compile again."
            } else {
                "Resolve the issue, then compile again."
            })
            .when_some(position, |pane, position| {
                let label = format!("Go to line {} in {}", position.line, position.file);
                pane.child(tool_button(
                    "preview-error-source",
                    label,
                    false,
                    palette,
                    cx,
                    move |view, window, cx| {
                        view.jump_to_source_position(position.clone(), window, cx)
                    },
                ))
            })
            .child(tool_button(
                "preview-error-log",
                "View build output",
                build.show_error_log,
                palette,
                cx,
                |view, _, cx| {
                    if let Some(selection) = &mut view.selection {
                        selection.build.show_error_log = !selection.build.show_error_log;
                        cx.notify();
                    }
                },
            ))
            .when(build.show_error_log, |pane| {
                pane.child(
                    div()
                        .debug_selector(|| "preview-error-output".into())
                        .w_full()
                        .p(px(14.))
                        .rounded(px(6.))
                        .border_1()
                        .border_color(rgb(palette.border))
                        .bg(rgb(palette.surface))
                        .font_family(monospace_font())
                        .line_height(px(20.))
                        .child(log.to_owned()),
                )
            })
    }
}

fn preview_button(
    id: &'static str,
    enabled: bool,
    palette: Palette,
    cx: &Context<Library>,
    activate: fn(&mut Library, &mut Window, &mut Context<Library>),
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .debug_selector(|| id.to_owned())
        .h(px(30.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .tab_index(5)
        .tab_stop(enabled || id == "compile")
        .when(!enabled && id != "compile", |button| button.opacity(0.45))
        .when(enabled, |button| {
            button
                .cursor_pointer()
                .hover(move |style| style.bg(rgb(palette.selection)))
                .focus(move |style| style.bg(rgb(palette.selection)))
                .on_click(cx.listener(move |library, event: &ClickEvent, window, cx| {
                    if matches!(event, ClickEvent::Mouse(_)) {
                        activate(library, window, cx);
                    }
                }))
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
        force: bool,
    ) -> io::Result<BuildResult> {
        assert!(!cancel.load(Ordering::Relaxed));
        let root = projects.join(&project.id);
        let source = fs::read_to_string(root.join("main.tex"))?;
        let pdf = root.join(".refract-build/main.pdf");
        fs::create_dir_all(pdf.parent().unwrap())?;
        if force {
            fs::write(pdf.with_file_name("forced.marker"), b"forced")?;
        }
        fs::write(&pdf, format!("%PDF-fixture\n{source}"))?;
        Ok(BuildResult {
            cached: false,
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
        cx.run_until_parked();
        cx.update(|_, app| {
            let build = &library.read(app).selection.as_ref().unwrap().build;
            assert!(
                build
                    .pdf()
                    .unwrap()
                    .with_file_name("forced.marker")
                    .is_file()
            );
        });
        let download = cx.debug_bounds("download-pdf").unwrap();
        cx.simulate_click(download.center(), Modifiers::none());
        let output = profile.path().join("saved.pdf");
        cx.simulate_new_path_selection(|_| Some(output.clone()));
        cx.run_until_parked();
        assert!(fs::read_to_string(output).unwrap().contains("New draft"));
    }
}

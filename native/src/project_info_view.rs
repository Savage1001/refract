use super::{Library, Palette, Selection};
use chrono::{DateTime, Local};
use gpui::{
    Context, Entity, FontWeight, KeyDownEvent, SharedString, Subscription, Window, div, prelude::*,
    px, rgb,
};
use gpui_component::{
    checkbox::Checkbox,
    select::{Select, SelectEvent, SelectItem, SelectState},
};
use refract_native::{ProjectSetting, tex::LatexEngine, update_project_setting};
use std::collections::VecDeque;

#[derive(Clone)]
struct EngineChoice(LatexEngine);

impl SelectItem for EngineChoice {
    type Value = LatexEngine;

    fn title(&self) -> SharedString {
        match self.0 {
            LatexEngine::PdfLatex => "pdfLaTeX",
            LatexEngine::XeLatex => "XeLaTeX",
            LatexEngine::LuaLatex => "LuaLaTeX",
        }
        .into()
    }

    fn value(&self) -> &LatexEngine {
        &self.0
    }
}

pub(super) struct ProjectInfoState {
    engine: Entity<SelectState<Vec<EngineChoice>>>,
    pending: VecDeque<ProjectSetting>,
    writing: Option<ProjectSetting>,
    error: Option<String>,
    _engine_change: Subscription,
}

impl ProjectInfoState {
    fn draft_mode(&self, saved: bool) -> bool {
        self.pending
            .iter()
            .rev()
            .copied()
            .chain(self.writing)
            .find_map(|setting| match setting {
                ProjectSetting::DraftMode(enabled) => Some(enabled),
                ProjectSetting::Engine(_) => None,
            })
            .unwrap_or(saved)
    }
}

impl Library {
    pub(super) fn load_project_info(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(selection) = &mut self.selection else {
            return;
        };
        if selection.project_info.is_none() {
            let current = selection.project.latex_engine().ok();
            let choices = [
                LatexEngine::PdfLatex,
                LatexEngine::XeLatex,
                LatexEngine::LuaLatex,
            ]
            .into_iter()
            .map(EngineChoice)
            .collect::<Vec<_>>();
            let engine = cx.new(|cx| {
                let mut state = SelectState::new(choices, None, window, cx);
                if let Some(engine) = current {
                    state.set_selected_value(&engine, window, cx);
                }
                state
            });
            let engine_change = cx.subscribe_in(
                &engine,
                window,
                |library, _, event: &SelectEvent<Vec<EngineChoice>>, window, cx| {
                    if let SelectEvent::Confirm(Some(engine)) = event {
                        library.queue_project_setting(ProjectSetting::Engine(*engine), window, cx);
                    }
                },
            );
            selection.project_info = Some(ProjectInfoState {
                engine,
                pending: VecDeque::new(),
                writing: None,
                error: None,
                _engine_change: engine_change,
            });
        }
        cx.notify();
    }

    fn queue_project_setting(
        &mut self,
        setting: ProjectSetting,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(selection) = &mut self.selection else {
            return;
        };
        let Some(info) = &mut selection.project_info else {
            return;
        };
        let saved = match setting {
            ProjectSetting::Engine(engine) => selection.project.latex_engine().ok() == Some(engine),
            ProjectSetting::DraftMode(enabled) => selection.project.draft_mode() == enabled,
        };
        if saved && info.writing.is_none() && info.pending.is_empty() {
            return;
        }
        info.pending
            .retain(|queued| std::mem::discriminant(queued) != std::mem::discriminant(&setting));
        info.pending.push_back(setting);
        info.error = None;
        selection.build.invalidate();
        self.start_pending_project_setting(window, cx);
        cx.notify();
    }

    pub(super) fn start_pending_project_setting(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.project_writes_idle() {
            return;
        }
        let Some(selection) = &mut self.selection else {
            return;
        };
        let Some(info) = &mut selection.project_info else {
            return;
        };
        let Some(setting) = info.pending.pop_front() else {
            return;
        };
        info.writing = Some(setting);
        let project = selection.project.clone();
        let id = project.id.clone();
        let projects = self.projects_dir.clone();
        let write = cx
            .background_executor()
            .spawn(async move { update_project_setting(&projects, &project, setting) });
        self.entry_write_task = Some(cx.spawn_in(window, async move |library, cx| {
            let updated = write.await;
            let _ = library.update_in(cx, |library, window, cx| {
                library.entry_write_task = None;
                if let Some(selection) = &mut library.selection
                    && selection.project.id == id
                    && let Some(info) = &mut selection.project_info
                {
                    info.writing = None;
                    match updated {
                        Ok(project) => {
                            if let Some(Ok(projects)) = &mut library.projects
                                && let Some(stored) =
                                    projects.iter_mut().find(|project| project.id == id)
                            {
                                *stored = project.clone();
                            }
                            selection.project = project;
                        }
                        Err(error) => {
                            if matches!(setting, ProjectSetting::Engine(_))
                                && !info
                                    .pending
                                    .iter()
                                    .any(|setting| matches!(setting, ProjectSetting::Engine(_)))
                                && let Ok(engine) = selection.project.latex_engine()
                            {
                                info.engine.update(cx, |state, cx| {
                                    state.set_selected_value(&engine, window, cx)
                                });
                            }
                            info.error = Some(format!("Couldn’t update project settings. {error}"));
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
    }

    fn reveal_root_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(selection) = &self.selection else {
            return;
        };
        let root = selection.project.root_file().replace('\\', "/");
        self.reveal_workspace_pane(refract_native::workspace_layout::Pane::Editor, window, cx);
        self.open_file(root.clone(), window, cx);
        let Some(selection) = &mut self.selection else {
            return;
        };
        if let Some(Ok(editor)) = selection
            .tabs
            .iter()
            .find(|tab| tab.path == root)
            .and_then(|tab| tab.editor.as_ref())
        {
            let editor = editor.state.clone();
            window.defer(cx, move |window, cx| {
                editor.update(cx, |input, cx| input.focus(window, cx))
            });
        } else {
            selection.focus_source_on_load = true;
        }
    }

    pub(super) fn project_info_pane(
        &self,
        selection: &Selection,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let created = selection
            .project
            .created
            .as_deref()
            .and_then(|date| DateTime::parse_from_rfc3339(date).ok())
            .map(|date| date.with_timezone(&Local).format("%-m/%-d/%Y").to_string())
            .unwrap_or_else(|| "Invalid Date".into());
        let project = &selection.project;
        div()
            .id("project-info")
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Project Info"),
            )
            .child(fact(
                "Name",
                div().truncate().child(project.name.clone()),
                palette,
            ))
            .child(fact(
                "Root file",
                div()
                    .id("root-file")
                    .tab_index(5)
                    .cursor_pointer()
                    .text_color(rgb(palette.focus))
                    .focus(move |style| style.bg(rgb(palette.selection)))
                    .child(project.root_file().to_owned())
                    .on_click(cx.listener(|library, _, window, cx| {
                        library.reveal_root_document(window, cx)
                    }))
                    .on_key_down(cx.listener(|library, event: &KeyDownEvent, window, cx| {
                        if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                            library.reveal_root_document(window, cx);
                        }
                    })),
                palette,
            ))
            .child(fact("Created", div().child(created), palette))
            .children(selection.project_info.as_ref().map(|info| {
                let library = cx.weak_entity();
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .border_t_1()
                    .border_color(rgb(palette.border))
                    .pt_3()
                    .child("LaTeX engine")
                    .child(Select::new(&info.engine))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(palette.muted))
                            .child("Choose the compiler used to build this project."),
                    )
                    .child(
                        Checkbox::new("draft-mode")
                            .checked(info.draft_mode(project.draft_mode()))
                            .label("Draft mode (image placeholders for faster compilation)")
                            .on_click(move |enabled, window, cx| {
                                let _ = library.update(cx, |library, cx| {
                                    library.queue_project_setting(
                                        ProjectSetting::DraftMode(*enabled),
                                        window,
                                        cx,
                                    )
                                });
                            }),
                    )
                    .children(
                        info.error
                            .clone()
                            .map(|error| div().text_color(rgb(palette.warning)).child(error)),
                    )
            }))
    }
}

fn fact(label: &'static str, value: impl IntoElement, palette: Palette) -> impl IntoElement {
    div()
        .flex()
        .justify_between()
        .items_center()
        .gap_2()
        .child(div().text_color(rgb(palette.muted)).child(label))
        .child(value)
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use crate::initialize_app;
    use gpui::{Modifiers, TestAppContext};
    use gpui_component::Root;
    use refract_native::{
        Project, create_project,
        tex::{BuildResult, build_settings},
    };
    use std::{fs, io, path::Path, sync::atomic::AtomicBool};

    fn fixture_compile(path: &Path, project: &Project, _: &AtomicBool) -> io::Result<BuildResult> {
        let config = build_settings(path, project)?;
        let pdf = path.join(&project.id).join(".refract-build/main.pdf");
        fs::create_dir_all(pdf.parent().unwrap())?;
        fs::write(
            &pdf,
            format!(
                "%PDF-fixture\n{} draft={}",
                config.engine.name(),
                config.draft_mode
            ),
        )?;
        Ok(BuildResult {
            arguments: Vec::new(),
            exit_code: Some(0),
            log: String::new(),
            pdf: Some(pdf),
        })
    }

    #[gpui::test]
    fn project_info_applies_saved_engine_and_draft_mode_to_the_next_build(cx: &mut TestAppContext) {
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
        let tools = cx.debug_bounds("project-tools").unwrap();
        cx.simulate_click(tools.center(), Modifiers::none());
        let info = cx.debug_bounds("tool-info").unwrap();
        cx.simulate_click(info.center(), Modifiers::none());
        cx.update(|window, app| {
            let view = library.read(app);
            let selection = view.selection.as_ref().unwrap();
            assert_eq!(selection.tool, Some(crate::WorkspaceTool::ProjectInfo));
            let engine = selection.project_info.as_ref().unwrap().engine.clone();
            engine.update(app, |state, cx| {
                state.set_selected_value(&LatexEngine::XeLatex, window, cx);
                cx.emit(SelectEvent::Confirm(Some(LatexEngine::XeLatex)));
            });
            library.update(app, |view, cx| {
                view.queue_project_setting(ProjectSetting::DraftMode(true), window, cx)
            });
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            let selection = library.read(app).selection.as_ref().unwrap();
            assert_eq!(
                selection.project.latex_engine().unwrap(),
                LatexEngine::XeLatex
            );
            assert!(selection.project.draft_mode());
            let pdf = selection
                .build
                .pdf()
                .expect("new settings should trigger a build");
            assert!(
                fs::read_to_string(pdf)
                    .unwrap()
                    .contains("xelatex draft=true")
            );
        });
    }
}

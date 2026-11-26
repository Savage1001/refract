use super::{Library, Navigation, Palette};
use gpui::{
    Context, Entity, FocusHandle, FontWeight, KeyDownEvent, Keystroke, PromptLevel, Window, div,
    prelude::*, px, rgb,
};
use gpui_component::{
    WindowExt,
    dialog::DialogButtonProps,
    input::{Input, InputState},
};
use refract_native::{
    Project, create_project, duplicate_project, list_projects, rename_project, trash_project,
};
use std::{cell::Cell, io, rc::Rc};

pub(super) struct RenameDraft {
    input: Entity<InputState>,
    invalid: Rc<Cell<bool>>,
}

pub(super) struct ProjectFocus {
    pub(super) open: FocusHandle,
    actions: FocusHandle,
}

#[derive(Clone, Copy)]
enum ProjectAction {
    Duplicate,
    Delete,
}

impl ProjectAction {
    fn label(self) -> &'static str {
        match self {
            Self::Duplicate => "Duplicate",
            Self::Delete => "Delete",
        }
    }
}

impl Library {
    pub(super) fn set_projects(
        &mut self,
        projects: io::Result<Vec<Project>>,
        cx: &mut Context<Self>,
    ) {
        if let Ok(projects) = &projects {
            let mut previous = std::mem::take(&mut self.project_focus);
            self.project_focus = projects
                .iter()
                .map(|project| {
                    let focus = previous
                        .remove(&project.id)
                        .unwrap_or_else(|| ProjectFocus {
                            open: cx.focus_handle().tab_stop(true).tab_index(2),
                            actions: cx.focus_handle().tab_stop(true).tab_index(2),
                        });
                    (project.id.clone(), focus)
                })
                .collect();
        }
        self.projects = Some(projects);
    }

    pub(super) fn can_create_project(&self) -> bool {
        self.projects.is_some()
            && self.selection.is_none()
            && self.project_library_task.is_none()
            && self.project_rename_task.is_none()
            && self.project_action_prompt.is_none()
            && self.deleted_projects_dialog.is_none()
    }

    fn can_rename_project(&self) -> bool {
        self.selection.is_some()
            && self.project_rename_task.is_none()
            && self.project_library_task.is_none()
            && self.restore_task.is_none()
    }

    fn create_new_project(&mut self, window: &Window, cx: &mut Context<Self>) {
        if !self.can_create_project() {
            return;
        }
        self.start_library_operation(None, window, cx);
    }

    fn start_library_operation(
        &mut self,
        source: Option<(Project, ProjectAction)>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.project_action_error = None;
        self.project_menu = None;
        self.project_action_source = source.as_ref().map(|(project, _)| project.id.clone());
        let action = match &source {
            Some((_, ProjectAction::Duplicate)) => "duplicate",
            Some((_, ProjectAction::Delete)) => "delete",
            None => "create",
        };
        let sequence = self.project_open_sequence;
        let path = self.projects_dir.clone();
        let create = cx.background_executor().spawn(async move {
            let project = match source {
                Some((project, ProjectAction::Duplicate)) => {
                    Some(duplicate_project(&path, &project)?)
                }
                Some((project, ProjectAction::Delete)) => {
                    trash_project(&path, &project)?;
                    None
                }
                None => Some(create_project(&path, "New Project")?),
            };
            Ok::<_, io::Error>((project, list_projects(&path)))
        });
        self.project_library_task = Some(cx.spawn_in(window, async move |library, cx| {
            let result = create.await;
            let _ = library.update_in(cx, |library, window, cx| {
                library.project_library_task = None;
                let source = library.project_action_source.take();
                match result {
                    Ok((project, projects)) => {
                        library.set_projects(projects, cx);
                        if library.selection.is_none()
                            && library.project_open_sequence == sequence
                            && !matches!(
                                library.navigation_after_save,
                                Some(Navigation::CloseWindow)
                            )
                        {
                            if let Some(project) = project {
                                library.open_project(project, window, cx);
                            } else {
                                let next = library
                                    .projects
                                    .as_ref()
                                    .and_then(|projects| projects.as_ref().ok())
                                    .and_then(|projects| projects.first())
                                    .and_then(|project| library.project_focus.get(&project.id));
                                window.focus(
                                    next.map_or(&library.deleted_projects_focus, |focus| {
                                        &focus.open
                                    }),
                                );
                            }
                        }
                    }
                    Err(error) => {
                        library.project_action_error =
                            Some(format!("Couldn’t {action} project. {error}"));
                        if library.selection.is_none()
                            && library.project_open_sequence == sequence
                            && let Some(focus) =
                                source.and_then(|id| library.project_focus.get(&id))
                        {
                            window.focus(&focus.actions);
                        }
                    }
                }
                library.resume_navigation(window, cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn dismiss_project_menu(&mut self, cx: &mut Context<Self>) {
        if self.project_menu.take().is_some() {
            cx.notify();
        }
    }

    fn toggle_project_menu(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.project_menu.as_deref() == Some(id) {
            self.dismiss_project_menu(cx);
        } else {
            self.project_menu = Some(id.to_owned());
            cx.notify();
        }
    }

    fn confirm_project_action(
        &mut self,
        project: Project,
        action: ProjectAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_create_project() || self.project_menu.as_deref() != Some(project.id.as_str()) {
            return;
        }
        let (level, title, confirm) = match action {
            ProjectAction::Duplicate => (
                PromptLevel::Info,
                format!("Duplicate {}?", project.name),
                "Duplicate",
            ),
            ProjectAction::Delete => (
                PromptLevel::Warning,
                format!("Move {} to Deleted projects?", project.name),
                "Delete project",
            ),
        };
        let answer = window.prompt(level, &title, None, &["Cancel", confirm], cx);
        self.project_action_prompt = Some(cx.spawn_in(window, async move |library, cx| {
            let confirmed = matches!(answer.await, Ok(1));
            let _ = library.update_in(cx, |library, window, cx| {
                library.project_action_prompt = None;
                if confirmed
                    && library.can_create_project()
                    && library.project_menu.as_deref() == Some(project.id.as_str())
                {
                    library.start_library_operation(Some((project, action)), window, cx);
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn project_actions(
        &self,
        project: &Project,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let id = project.id.clone();
        let click_id = id.clone();
        let key_id = id.clone();
        let open = self.project_menu.as_deref() == Some(id.as_str());
        div()
            .id("project-actions")
            .relative()
            .flex_none()
            .when(self.grid_view, |actions| {
                actions.absolute().right(px(12.)).top(px(12.))
            })
            .on_click(|_, _, cx| cx.stop_propagation())
            .on_key_down(
                cx.listener(move |library, event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape"
                        && library.project_menu.as_deref() == Some(id.as_str())
                    {
                        cx.stop_propagation();
                        library.dismiss_project_menu(cx);
                        if let Some(focus) = library.project_focus.get(&id) {
                            window.focus(&focus.actions);
                        }
                    }
                }),
            )
            .child(
                div()
                    .id("project-actions-toggle")
                    .track_focus(&self.project_focus[&project.id].actions)
                    .rounded_md()
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.selection)))
                    .focus(move |style| style.bg(rgb(palette.selection)))
                    .child("⋯")
                    .on_click(cx.listener(move |library, _, _, cx| {
                        cx.stop_propagation();
                        library.toggle_project_menu(&click_id, cx);
                    }))
                    .on_key_down(cx.listener(move |library, event: &KeyDownEvent, _, cx| {
                        if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                            cx.stop_propagation();
                            library.toggle_project_menu(&key_id, cx);
                        }
                    })),
            )
            .when(open, |actions| {
                actions.child(
                    div()
                        .id("project-actions-menu")
                        .absolute()
                        .top(px(32.))
                        .right_0()
                        .min_w(px(140.))
                        .occlude()
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(palette.border))
                        .bg(rgb(palette.surface))
                        .shadow_sm()
                        .children(
                            [ProjectAction::Duplicate, ProjectAction::Delete]
                                .into_iter()
                                .map(|action| {
                                    let click_project = project.clone();
                                    let key_project = project.clone();
                                    div()
                                        .id(action.label())
                                        .tab_index(2)
                                        .px_3()
                                        .py_2()
                                        .rounded_md()
                                        .when(self.can_create_project(), |button| {
                                            button.cursor_pointer()
                                        })
                                        .hover(move |style| style.bg(rgb(palette.selection)))
                                        .focus(move |style| style.bg(rgb(palette.selection)))
                                        .child(action.label())
                                        .on_click(cx.listener(move |library, _, window, cx| {
                                            cx.stop_propagation();
                                            library.confirm_project_action(
                                                click_project.clone(),
                                                action,
                                                window,
                                                cx,
                                            );
                                        }))
                                        .on_key_down(cx.listener(
                                            move |library, event: &KeyDownEvent, window, cx| {
                                                if event.keystroke.key == "enter"
                                                    || event.keystroke.key == "space"
                                                {
                                                    cx.stop_propagation();
                                                    library.confirm_project_action(
                                                        key_project.clone(),
                                                        action,
                                                        window,
                                                        cx,
                                                    );
                                                }
                                            },
                                        ))
                                }),
                        ),
                )
            })
    }

    pub(super) fn create_project_button(
        &self,
        id: &'static str,
        label: &'static str,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let enabled = self.can_create_project();
        div()
            .id(id)
            .rounded_md()
            .px_3()
            .py_2()
            .font_weight(FontWeight::SEMIBOLD)
            .bg(rgb(palette.focus))
            .text_color(rgb(0xffffff))
            .child(label)
            .when(!enabled, |button| button.opacity(0.5))
            .when(enabled, |button| {
                button
                    .tab_index(1)
                    .cursor_pointer()
                    .hover(|style| style.opacity(0.9))
                    .focus(move |style| style.border_1().border_color(rgb(palette.input_border)))
                    .on_click(
                        cx.listener(|library, _, window, cx| {
                            library.create_new_project(window, cx)
                        }),
                    )
                    .on_key_down(cx.listener(|library, event: &KeyDownEvent, window, cx| {
                        if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                            library.create_new_project(window, cx);
                        }
                    }))
            })
    }

    pub(super) fn project_name_control(
        &self,
        project: &Project,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        div()
            .id("project-name")
            .track_focus(&self.project_name_focus)
            .truncate()
            .rounded_md()
            .px_2()
            .py_1()
            .font_weight(FontWeight::SEMIBOLD)
            .child(project.name.clone())
            .when(self.can_rename_project(), |button| {
                button
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.selection)))
                    .focus(move |style| style.bg(rgb(palette.selection)))
                    .on_click(
                        cx.listener(|library, _, window, cx| {
                            library.open_rename_dialog(window, cx)
                        }),
                    )
                    .on_key_down(cx.listener(|library, event: &KeyDownEvent, window, cx| {
                        if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                            library.open_rename_dialog(window, cx);
                        }
                    }))
            })
    }

    fn open_rename_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.rename_draft.is_some() || !self.can_rename_project() || window.has_active_dialog(cx)
        {
            return;
        }
        let Some(selection) = &self.selection else {
            return;
        };
        let name = selection.project.name.clone();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name));
        let invalid = Rc::new(Cell::new(false));
        self.rename_draft = Some(RenameDraft {
            input: input.clone(),
            invalid: invalid.clone(),
        });
        let library = cx.weak_entity();
        let field = input.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let submit = library.clone();
            let close = library.clone();
            dialog
                .title("Rename project")
                .confirm()
                .button_props(DialogButtonProps::default().ok_text("Rename"))
                .child("Project name")
                .child(Input::new(&field))
                .when(invalid.get(), |dialog| {
                    dialog.child("Project name is required")
                })
                .on_ok(move |_, window, app| {
                    submit
                        .update(app, |library, cx| library.submit_project_name(window, cx))
                        .unwrap_or(true)
                })
                .on_close(move |_, _, app| {
                    let _ = close.update(app, |library, cx| {
                        library.rename_draft = None;
                        cx.notify();
                    });
                })
        });
        window.defer(cx, move |window, app| {
            input.update(app, |state, cx| state.focus(window, cx));
            let select_all = if cfg!(target_os = "macos") {
                "cmd-a"
            } else {
                "ctrl-a"
            };
            window.dispatch_keystroke(
                Keystroke::parse(select_all).expect("valid select-all shortcut"),
                app,
            );
        });
        cx.notify();
    }

    fn submit_project_name(&mut self, window: &Window, cx: &mut Context<Self>) -> bool {
        if !self.can_rename_project() {
            return false;
        }
        let (Some(draft), Some(selection)) = (&self.rename_draft, &self.selection) else {
            return true;
        };
        let value = draft.input.read(cx).value();
        let name = value.trim();
        if name.is_empty() {
            draft.invalid.set(true);
            cx.notify();
            return false;
        }
        let name = name.to_owned();
        let project = selection.project.clone();
        self.project_action_error = None;
        let path = self.projects_dir.clone();
        let rename = cx.background_executor().spawn(async move {
            let project = rename_project(&path, &project, &name)?;
            Ok::<_, io::Error>((project, list_projects(&path)))
        });
        self.project_rename_task = Some(cx.spawn_in(window, async move |library, cx| {
            let result = rename.await;
            let _ = library.update_in(cx, |library, window, cx| {
                library.project_rename_task = None;
                match result {
                    Ok((project, projects)) => {
                        if let Some(selection) = &mut library.selection
                            && selection.project.id == project.id
                        {
                            selection.project = project;
                        }
                        library.set_projects(projects, cx);
                    }
                    Err(error) => {
                        library.project_action_error =
                            Some(format!("Couldn’t rename project. {error}"))
                    }
                }
                library.resume_navigation(window, cx);
                cx.notify();
            });
        }));
        cx.notify();
        true
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use crate::initialize_app;
    use gpui::TestAppContext;
    use gpui_component::Root;

    #[gpui::test]
    fn moves_a_project_to_deleted_projects_and_restores_it_by_keyboard(cx: &mut TestAppContext) {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let original = create_project(&projects, "Paper").unwrap();
        cx.update(initialize_app);
        let mut library = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| Library::new(projects.clone(), window, cx));
            library = Some(view.clone());
            Root::new(view, window, cx)
        });
        let library = library.unwrap();
        cx.run_until_parked();
        cx.update(|window, app| {
            window.focus(&library.read(app).project_focus[&original.id].actions)
        });
        cx.simulate_keystrokes("enter tab tab enter");
        assert!(cx.has_pending_prompt());
        cx.simulate_prompt_answer("Delete project");
        cx.run_until_parked();
        assert!(list_projects(&projects).unwrap().is_empty());
        assert_eq!(
            refract_native::list_deleted_projects(&projects).unwrap()[0].project_id,
            original.id
        );
        cx.update(|window, app| {
            assert!(library.read(app).deleted_projects_focus.is_focused(window))
        });

        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        cx.update(|window, app| assert!(window.has_active_dialog(app)));
        cx.simulate_keystrokes("tab shift-tab enter");
        cx.run_until_parked();
        assert_eq!(list_projects(&projects).unwrap()[0].id, original.id);
        assert!(
            refract_native::list_deleted_projects(&projects)
                .unwrap()
                .is_empty()
        );
        cx.simulate_keystrokes("enter");
        cx.update(|window, app| {
            assert!(!window.has_active_dialog(app));
            let view = library.read(app);
            assert!(view.deleted_projects_dialog.is_none());
            assert!(view.deleted_projects_focus.is_focused(window));
            assert!(view.project_focus.contains_key(&original.id));
        });
    }

    #[gpui::test]
    fn opens_the_project_copy_after_confirmation_and_keeps_the_original(cx: &mut TestAppContext) {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let original = create_project(&projects, "Paper").unwrap();
        create_project(&projects, "Other project").unwrap();
        cx.update(initialize_app);
        let mut library = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| Library::new(projects.clone(), window, cx));
            library = Some(view.clone());
            Root::new(view, window, cx)
        });
        let library = library.unwrap();
        cx.run_until_parked();
        cx.update(|window, app| {
            window.focus(&library.read(app).project_focus[&original.id].actions)
        });
        cx.simulate_keystrokes("enter");
        cx.update(|_, app| {
            let view = library.read(app);
            assert_eq!(view.project_menu.as_deref(), Some(original.id.as_str()));
            assert!(view.selection.is_none());
        });
        cx.simulate_keystrokes("tab enter");
        assert!(cx.has_pending_prompt());
        cx.simulate_prompt_answer("Cancel");
        cx.run_until_parked();
        assert_eq!(list_projects(&projects).unwrap().len(), 2);
        cx.simulate_keystrokes("escape");
        cx.update(|window, app| {
            let view = library.read(app);
            assert!(view.project_menu.is_none());
            assert!(view.project_focus[&original.id].actions.is_focused(window));
        });

        cx.update(|_, app| {
            library.update(app, |view, cx| {
                view.grid_view = true;
                cx.notify();
            })
        });
        cx.simulate_keystrokes("enter tab enter");
        assert!(cx.has_pending_prompt());
        cx.simulate_prompt_answer("Duplicate");
        cx.run_until_parked();
        cx.update(|window, app| {
            let view = library.read(app);
            let selection = view.selection.as_ref().unwrap();
            assert_eq!(selection.project.name, "Paper (copy)");
            assert_ne!(selection.project.id, original.id);
            assert_eq!(selection.file.as_deref(), Some("main.tex"));
            assert!(view.project_name_focus.is_focused(window));
        });
        let projects = list_projects(&projects).unwrap();
        assert_eq!(projects.len(), 3);
        assert!(
            projects
                .iter()
                .any(|project| project.name == "Paper" && project.id == original.id)
        );
    }

    #[gpui::test]
    fn creates_once_and_renames_from_the_workspace_keyboard(cx: &mut TestAppContext) {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        cx.update(initialize_app);
        let mut library = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| Library::new(projects.clone(), window, cx));
            library = Some(view.clone());
            Root::new(view, window, cx)
        });
        let library = library.unwrap();
        cx.run_until_parked();
        cx.update(|window, app| {
            library.update(app, |library, cx| {
                library.create_new_project(window, cx);
                library.create_new_project(window, cx);
            });
        });
        cx.run_until_parked();
        let records = list_projects(&projects).unwrap();
        assert_eq!(records.len(), 1);
        let id = records[0].id.clone();
        cx.update(|window, app| {
            let view = library.read(app);
            let selection = view.selection.as_ref().unwrap();
            assert_eq!(selection.project.id, id);
            assert_eq!(selection.project.name, "New Project");
            assert_eq!(selection.file.as_deref(), Some("main.tex"));
            assert!(view.project_name_focus.is_focused(window));
        });

        cx.simulate_keystrokes("enter");
        cx.update(|window, app| assert!(window.has_active_dialog(app)));
        cx.simulate_keystrokes("escape");
        cx.update(|window, app| {
            assert!(!window.has_active_dialog(app));
            assert!(library.read(app).project_name_focus.is_focused(window));
        });
        cx.simulate_keystrokes("enter");
        cx.simulate_input("  Spectral methods  ");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        let records = list_projects(&projects).unwrap();
        assert_eq!(records[0].name, "Spectral methods");
        cx.update(|window, app| {
            assert!(!window.has_active_dialog(app));
            let view = library.read(app);
            assert_eq!(
                view.selection.as_ref().unwrap().project.name,
                "Spectral methods"
            );
            assert!(view.project_name_focus.is_focused(window));
            window.focus(&view.home_focus);
        });
        cx.simulate_keystrokes("enter");
        cx.update(|window, app| {
            let view = library.read(app);
            assert!(view.selection.is_none());
            assert_eq!(
                view.projects.as_ref().unwrap().as_ref().unwrap()[0].name,
                "Spectral methods"
            );
            assert!(view.project_focus[&id].open.is_focused(window));
        });
    }
}

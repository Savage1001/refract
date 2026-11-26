use super::{Library, Navigation, Palette};
use gpui::{
    Context, Entity, FontWeight, KeyDownEvent, Keystroke, Window, div, prelude::*, px, rgb,
};
use gpui_component::{
    WindowExt,
    dialog::DialogButtonProps,
    input::{Input, InputState},
};
use refract_native::{Project, create_project, list_projects, rename_project};
use std::{cell::Cell, io, rc::Rc};

pub(super) struct RenameDraft {
    input: Entity<InputState>,
    invalid: Rc<Cell<bool>>,
}

impl Library {
    pub(super) fn set_projects(
        &mut self,
        projects: io::Result<Vec<Project>>,
        cx: &mut Context<Self>,
    ) {
        if let Ok(projects) = &projects {
            for project in projects {
                self.project_focus
                    .entry(project.id.clone())
                    .or_insert_with(|| cx.focus_handle().tab_stop(true).tab_index(2));
            }
        }
        self.projects = Some(projects);
    }

    fn can_create_project(&self) -> bool {
        self.projects.is_some()
            && self.selection.is_none()
            && self.project_create_task.is_none()
            && self.project_rename_task.is_none()
    }

    fn can_rename_project(&self) -> bool {
        self.selection.is_some()
            && self.project_rename_task.is_none()
            && self.project_create_task.is_none()
            && self.restore_task.is_none()
    }

    fn create_new_project(&mut self, window: &Window, cx: &mut Context<Self>) {
        if !self.can_create_project() {
            return;
        }
        self.project_action_error = None;
        let sequence = self.project_open_sequence;
        let path = self.projects_dir.clone();
        let create = cx.background_executor().spawn(async move {
            let project = create_project(&path, "New Project")?;
            Ok::<_, io::Error>((project, list_projects(&path)))
        });
        self.project_create_task = Some(cx.spawn_in(window, async move |library, cx| {
            let result = create.await;
            let _ = library.update_in(cx, |library, window, cx| {
                library.project_create_task = None;
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
                            library.open_project(project, window, cx);
                        }
                    }
                    Err(error) => {
                        library.project_action_error =
                            Some(format!("Couldn’t create project. {error}"))
                    }
                }
                library.resume_navigation(window, cx);
                cx.notify();
            });
        }));
        cx.notify();
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
        if self.rename_draft.is_some() || !self.can_rename_project() {
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
            assert!(view.project_focus[&id].is_focused(window));
        });
    }
}

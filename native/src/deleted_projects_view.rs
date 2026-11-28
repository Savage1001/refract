use super::{Library, Palette};
use chrono::{Local, Utc};
use gpui::{
    App, Context, FocusHandle, FontWeight, KeyBinding, KeyDownEvent, WeakEntity, Window, actions,
    div, prelude::*, px, rgb, svg,
};
use gpui_component::{WindowExt, tooltip::Tooltip};
use refract_native::{
    DeletedProject, list_deleted_projects, list_projects, restore_deleted_project,
};
use std::{cell::RefCell, collections::HashMap, io, rc::Rc};

actions!(deleted_projects, [Activate, NextControl, PreviousControl]);

pub(super) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", Activate, Some("RecoveryControl")),
        KeyBinding::new("space", Activate, Some("RecoveryControl")),
        KeyBinding::new("tab", NextControl, Some("DeletedProjectsDialog")),
        KeyBinding::new("shift-tab", PreviousControl, Some("DeletedProjectsDialog")),
    ]);
}

pub(super) struct DeletedProjectsDialog {
    items: Option<io::Result<Vec<DeletedProject>>>,
    restore_focus: HashMap<String, FocusHandle>,
    close_focus: FocusHandle,
    busy: bool,
    notice: Option<String>,
}

impl DeletedProjectsDialog {
    fn records(&self) -> &[DeletedProject] {
        self.items
            .as_ref()
            .and_then(|items| items.as_ref().ok())
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    fn set_items(&mut self, items: io::Result<Vec<DeletedProject>>, cx: &mut Context<Library>) {
        let mut old = std::mem::take(&mut self.restore_focus);
        if let Ok(records) = &items {
            self.restore_focus = records
                .iter()
                .map(|record| {
                    let focus = old
                        .remove(&record.id)
                        .unwrap_or_else(|| cx.focus_handle().tab_stop(true));
                    (record.id.clone(), focus)
                })
                .collect();
        }
        self.items = Some(items);
    }

    fn focus_row(&self, index: usize, window: &mut Window) {
        let records = self.records();
        let next = records
            .get(index.min(records.len().saturating_sub(1)))
            .and_then(|record| self.restore_focus.get(&record.id));
        window.focus(next.unwrap_or(&self.close_focus));
    }

    fn move_focus(&self, window: &mut Window, backward: bool) {
        let controls: Vec<&FocusHandle> = self
            .records()
            .iter()
            .filter_map(|record| self.restore_focus.get(&record.id))
            .chain(std::iter::once(&self.close_focus))
            .collect();
        let position = controls.iter().position(|focus| focus.is_focused(window));
        let next = match position {
            Some(index) if backward => (index + controls.len() - 1) % controls.len(),
            Some(index) => (index + 1) % controls.len(),
            None if backward => controls.len() - 1,
            None => 0,
        };
        window.focus(controls[next]);
    }
}

impl Library {
    pub(super) fn deleted_projects_button(
        &self,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        div()
            .id("deleted-projects")
            .track_focus(&self.deleted_projects_focus)
            .rounded_md()
            .size(px(32.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(move |style| style.bg(rgb(palette.selection)))
            .focus(move |style| style.bg(rgb(palette.selection)))
            .child(
                svg()
                    .path("refract/trash.svg")
                    .size(px(18.))
                    .text_color(rgb(palette.muted)),
            )
            .tooltip(|window, cx| Tooltip::new("Deleted projects").build(window, cx))
            .on_click(
                cx.listener(|library, _, window, cx| library.open_deleted_projects(window, cx)),
            )
            .on_key_down(cx.listener(|library, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                    library.open_deleted_projects(window, cx);
                }
            }))
    }

    fn open_deleted_projects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_create_project() {
            return;
        }
        self.dismiss_project_menu(cx);
        let close_focus = cx.focus_handle().tab_stop(true);
        let state = Rc::new(RefCell::new(DeletedProjectsDialog {
            items: None,
            restore_focus: HashMap::new(),
            close_focus: close_focus.clone(),
            busy: true,
            notice: None,
        }));
        self.deleted_projects_dialog = Some(state.clone());
        let library = cx.weak_entity();
        window.open_dialog(cx, move |dialog, window, _| {
            let close = library.clone();
            dialog
                .title(crate::dialog_title("Deleted projects"))
                .close_button(false)
                .overlay_closable(false)
                .child(deleted_projects_contents(
                    &state,
                    library.clone(),
                    Palette::for_window(window),
                ))
                .on_close(move |_, _, app| {
                    let _ = close.update(app, |library, cx| {
                        library.deleted_projects_dialog = None;
                        cx.notify();
                    });
                })
        });
        window.focus(&close_focus);
        self.update_deleted_projects(None, window, cx);
    }

    fn restore_deleted(&mut self, token: &str, window: &Window, cx: &mut Context<Self>) {
        if self.project_library_task.is_some() {
            return;
        }
        let Some(state) = &self.deleted_projects_dialog else {
            return;
        };
        let item = {
            let mut view = state.borrow_mut();
            let Some((index, record)) = view
                .records()
                .iter()
                .enumerate()
                .find(|(_, record)| record.id == token)
            else {
                return;
            };
            let item = (index, record.clone());
            view.busy = true;
            view.notice = None;
            item
        };
        self.update_deleted_projects(Some(item), window, cx);
    }

    fn update_deleted_projects(
        &mut self,
        restore: Option<(usize, DeletedProject)>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog) = self.deleted_projects_dialog.as_ref().cloned() else {
            return;
        };
        let path = self.projects_dir.clone();
        let operation = cx.background_executor().spawn(async move {
            let restored = restore.map(|(index, record)| {
                let result = restore_deleted_project(&path, &record.id);
                (index, record, result, list_projects(&path))
            });
            (restored, list_deleted_projects(&path))
        });
        self.project_library_task = Some(cx.spawn_in(window, async move |library, cx| {
            let (restored, items) = operation.await;
            let _ = library.update_in(cx, |library, window, cx| {
                library.project_library_task = None;
                let focus = restored.map(|(index, record, result, projects)| {
                    library.set_projects(projects, cx);
                    (index, record, result)
                });
                if library
                    .deleted_projects_dialog
                    .as_ref()
                    .is_some_and(|active| Rc::ptr_eq(active, &dialog))
                {
                    let mut state = dialog.borrow_mut();
                    state.busy = false;
                    let mut focus_row = None;
                    if let Some((index, record, result)) = focus {
                        if state
                            .restore_focus
                            .get(&record.id)
                            .is_some_and(|focus| focus.is_focused(window))
                        {
                            focus_row = Some(index);
                        }
                        state.notice = Some(match result {
                            Ok(_) => format!("Restored {}.", record.name),
                            Err(error) => format!("Couldn’t restore project. {error}"),
                        });
                    } else if state.close_focus.is_focused(window) {
                        focus_row = Some(0);
                    }
                    state.set_items(items, cx);
                    if let Some(index) = focus_row {
                        state.focus_row(index, window);
                    }
                }
                library.resume_navigation(window, cx);
                cx.notify();
            });
        }));
        cx.notify();
    }
}

fn deleted_projects_contents(
    state: &Rc<RefCell<DeletedProjectsDialog>>,
    library: WeakEntity<Library>,
    palette: Palette,
) -> impl IntoElement {
    let data = state.borrow();
    let next = state.clone();
    let previous = state.clone();
    let contents = div()
        .id("deleted-projects-list")
        .max_h(px(440.))
        .overflow_y_scroll();
    let contents = match &data.items {
        None => contents.child("Loading deleted projects…"),
        Some(Err(error)) => contents.child(format!("Couldn’t load deleted projects. {error}")),
        Some(Ok(items)) if items.is_empty() => contents
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("No deleted projects"),
            )
            .child(
                div()
                    .text_color(rgb(palette.muted))
                    .child("Projects you delete appear here for up to 30 days."),
            ),
        Some(Ok(items)) => {
            let now = Utc::now();
            contents.children(items.iter().map(|record| {
                let date = record.deleted_at().map_or_else(
                    || "Deletion date unavailable".into(),
                    |date| {
                        format!(
                            "Deleted {}",
                            date.with_timezone(&Local).format("%-m/%-d/%Y, %-I:%M %p")
                        )
                    },
                );
                let remaining = record
                    .days_remaining(now)
                    .map(|days| format!("{days} days remaining"));
                let restore = library.clone();
                let token = record.id.clone();
                div()
                    .id(gpui::SharedString::from(record.id.clone()))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .border_b_1()
                    .border_color(rgb(palette.border))
                    .py_2()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(record.name.clone())
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(palette.muted))
                                    .child(date),
                            )
                            .children(remaining.map(|label| {
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(palette.muted))
                                    .child(label)
                            })),
                    )
                    .child(recovery_button(
                        "restore",
                        "Restore",
                        &data.restore_focus[&record.id],
                        palette,
                        !data.busy,
                        move |window, app| {
                            let _ = restore.update(app, |library, cx| {
                                library.restore_deleted(&token, window, cx)
                            });
                        },
                    ))
            }))
        }
    };
    div()
        .key_context("DeletedProjectsDialog")
        .flex()
        .flex_col()
        .gap_3()
        .on_action(move |_: &NextControl, window, _| next.borrow().move_focus(window, false))
        .on_action(move |_: &PreviousControl, window, _| previous.borrow().move_focus(window, true))
        .children(data.notice.clone())
        .child(contents)
        .child(div().flex().justify_end().child(recovery_button(
            "close",
            "Close",
            &data.close_focus,
            palette,
            true,
            move |window, app| {
                let _ = library.update(app, |library, cx| {
                    library.deleted_projects_dialog = None;
                    window.close_dialog(cx);
                    cx.notify();
                });
            },
        )))
}

pub(super) fn recovery_button(
    id: &'static str,
    label: &'static str,
    focus: &FocusHandle,
    palette: Palette,
    enabled: bool,
    activate: impl Fn(&mut Window, &mut App) + Clone + 'static,
) -> impl IntoElement + Styled {
    let keyboard = activate.clone();
    div()
        .id(id)
        .track_focus(focus)
        .key_context("RecoveryControl")
        .rounded_md()
        .border_1()
        .border_color(rgb(palette.border))
        .px_3()
        .py_1()
        .child(label)
        .focus(move |style| style.bg(rgb(palette.selection)))
        .when(!enabled, |button| button.text_color(rgb(palette.muted)))
        .on_click(move |_, window, app| {
            if enabled {
                activate(window, app);
            }
        })
        .on_action(move |_: &Activate, window, app| {
            if enabled {
                keyboard(window, app);
            }
        })
}

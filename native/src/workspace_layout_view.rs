use super::{Library, Palette, Selection};
use gpui::{
    ClickEvent, Context, DragMoveEvent, KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Render,
    Window, div, prelude::*, px, rgb, svg,
};
use gpui_component::tooltip::Tooltip;
use refract_native::workspace_layout::{Divider, Pane, PaneVisibility, WorkspaceLayout};

pub(super) struct DividerDrag {
    divider: Divider,
    origin: Pixels,
    length: f32,
}

struct DividerGhost;

impl Render for DividerGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size(px(0.))
    }
}

impl Library {
    pub(super) fn displayed_panes(&self, selection: &Selection, width: f32) -> PaneVisibility {
        let mut visible = self.workspace_layout.visible(width);
        if selection.tool.is_some() {
            visible.preview = false;
            if WorkspaceLayout::compact(width) || selection.expanded_tools() {
                visible.editor = false;
                visible.agent = false;
            }
        }
        visible
    }

    pub(super) fn pdf_is_visible(&self, window: &Window) -> bool {
        self.selection.as_ref().is_some_and(|selection| {
            self.displayed_panes(selection, f32::from(window.viewport_size().width))
                .preview
        })
    }

    fn save_panes(&self, previous: PaneVisibility) {
        let current = self.workspace_layout.panes;
        if current != previous
            && let Err(error) = current.save(
                &self
                    .projects_dir
                    .with_file_name(".refract-native-workspace.json"),
            )
        {
            eprintln!("Couldn’t save the visible workspace panes. {error}");
        }
    }

    fn toggle_workspace_pane(&mut self, pane: Pane, window: &Window, cx: &mut Context<Self>) {
        let Some(selection) = &self.selection else {
            return;
        };
        let width = f32::from(window.viewport_size().width);
        let displayed = self.displayed_panes(selection, width);
        let previous = self.workspace_layout.panes;
        self.close_workspace_tools(cx);
        self.workspace_layout.toggle(pane, width, displayed);
        self.save_panes(previous);
        cx.notify();
    }

    pub(super) fn reveal_workspace_pane(
        &mut self,
        pane: Pane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(selection) = &self.selection else {
            return;
        };
        let width = f32::from(window.viewport_size().width);
        let displayed = self.displayed_panes(selection, width);
        if !displayed.contains(pane) {
            let previous = self.workspace_layout.panes;
            self.close_workspace_tools(cx);
            self.workspace_layout.reveal(pane, width);
            self.save_panes(previous);
            if pane == Pane::Preview && WorkspaceLayout::compact(width) {
                window.focus(&self.preview_pane_focus);
            }
            cx.notify();
        }
    }

    pub(super) fn expand_workspace_chat(&mut self, expanded: bool, cx: &mut Context<Self>) {
        let previous = self.workspace_layout.panes;
        self.close_workspace_tools(cx);
        self.workspace_layout.expand_chat(expanded);
        self.save_panes(previous);
        cx.notify();
    }

    pub(super) fn workspace_pane_buttons(
        &self,
        visible: PaneVisibility,
        compact: bool,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        div()
            .flex()
            .flex_none()
            .gap(px(2.))
            .p(px(1.))
            .rounded(px(5.))
            .border_1()
            .border_color(rgb(palette.input_border))
            .children(
                [
                    (
                        Pane::Agent,
                        "pane-agent",
                        "Agent",
                        "refract/agent.svg",
                        visible.agent,
                    ),
                    (
                        Pane::Editor,
                        "pane-editor",
                        "Editor",
                        "refract/editor.svg",
                        visible.editor,
                    ),
                    (
                        Pane::Preview,
                        "pane-preview",
                        "Preview",
                        "refract/preview.svg",
                        visible.preview,
                    ),
                ]
                .into_iter()
                .map(|(pane, id, label, icon, active)| {
                    let tip = match (pane, active, compact) {
                        (Pane::Agent, true, _) => "Hide agent",
                        (Pane::Agent, false, _) => "Show agent",
                        (Pane::Editor, true, _) => "Hide editor",
                        (Pane::Editor, false, _) => "Show editor",
                        (Pane::Preview, true, true) => "Return to writing",
                        (Pane::Preview, true, false) => "Hide preview",
                        (Pane::Preview, false, _) => "Show preview",
                    };
                    div()
                        .id(id)
                        .debug_selector(|| id.to_owned())
                        .tab_index(3)
                        .when(pane == Pane::Preview, |button| {
                            button.track_focus(&self.preview_pane_focus)
                        })
                        .h(px(26.))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap_1()
                        .rounded(px(4.))
                        .cursor_pointer()
                        .text_size(px(11.))
                        .when(compact, |button| button.w(px(28.)))
                        .child(
                            svg()
                                .path(icon)
                                .size(px(16.))
                                .text_color(rgb(palette.muted)),
                        )
                        .when(!compact, |button| button.px(px(6.)).child(label))
                        .when(active, |button| button.bg(rgb(palette.selection)))
                        .hover(move |style| style.bg(rgb(palette.selection)))
                        .focus(move |style| style.border_1().border_color(rgb(palette.focus)))
                        .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
                        .on_click(cx.listener(move |view, event: &ClickEvent, window, cx| {
                            if matches!(event, ClickEvent::Mouse(_)) {
                                view.toggle_workspace_pane(pane, window, cx);
                            }
                        }))
                        .on_key_down(cx.listener(move |view, event: &KeyDownEvent, window, cx| {
                            if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                                view.toggle_workspace_pane(pane, window, cx);
                            }
                        }))
                }),
            )
    }

    pub(super) fn sidebar_collapse_button(
        &self,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let label = if self.workspace_layout.sidebar_collapsed {
            "Expand sidebar"
        } else {
            "Collapse sidebar"
        };
        div()
            .id("collapse-sidebar")
            .debug_selector(|| "collapse-sidebar".to_owned())
            .track_focus(&self.sidebar_collapse_focus)
            .size(px(28.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded_sm()
            .cursor_pointer()
            .child(
                svg()
                    .path("refract/menu.svg")
                    .size(px(18.))
                    .text_color(rgb(palette.muted)),
            )
            .hover(move |style| style.bg(rgb(palette.selection)))
            .focus(move |style| style.border_1().border_color(rgb(palette.focus)))
            .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
            .on_click(cx.listener(|view, event: &ClickEvent, _, cx| {
                if matches!(event, ClickEvent::Mouse(_)) {
                    view.workspace_layout.sidebar_collapsed =
                        !view.workspace_layout.sidebar_collapsed;
                    cx.notify();
                }
            }))
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                    view.workspace_layout.sidebar_collapsed =
                        !view.workspace_layout.sidebar_collapsed;
                    cx.notify();
                }
            }))
    }

    pub(super) fn workspace_divider(
        &self,
        divider: Divider,
        length: f32,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let (id, label) = match divider {
            Divider::Sidebar => ("sidebar-resize", "Resize sidebar"),
            Divider::Editor => ("workspace-resize", "Resize editor and PDF"),
            Divider::Chat => ("chat-resize", "Resize chat"),
        };
        let horizontal = divider == Divider::Chat;
        div()
            .id(id)
            .debug_selector(|| id.to_owned())
            .tab_index(4)
            .flex_none()
            .when(horizontal, |handle| {
                handle
                    .absolute()
                    .top(px(-5.))
                    .left(gpui::relative(0.25))
                    .right(gpui::relative(0.25))
                    .h(px(10.))
                    .cursor_row_resize()
                    .child(
                        div()
                            .absolute()
                            .top(px(4.))
                            .left(gpui::relative(0.4))
                            .right(gpui::relative(0.4))
                            .h(px(2.))
                            .rounded(px(2.))
                            .bg(rgb(palette.input_border)),
                    )
            })
            .when(!horizontal, |handle| {
                handle.w(px(6.)).h_full().mx(px(-3.)).cursor_col_resize()
            })
            .hover(move |style| style.bg(rgb(palette.selection)))
            .focus(move |style| style.bg(rgb(palette.focus)))
            .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, event: &MouseDownEvent, _, _| {
                    view.divider_drag = Some(DividerDrag {
                        divider,
                        origin: if horizontal {
                            event.position.y
                        } else {
                            event.position.x
                        },
                        length,
                    });
                }),
            )
            .on_drag(DividerGhost, |_, _, _, cx| cx.new(|_| DividerGhost))
            .on_drag_move(cx.listener(
                move |view, event: &DragMoveEvent<DividerGhost>, window, cx| {
                    if let Some(drag) = &view.divider_drag
                        && drag.divider == divider
                    {
                        let viewport = window.viewport_size();
                        let (delta, extent) = if horizontal {
                            (drag.origin - event.event.position.y, viewport.height)
                        } else {
                            (event.event.position.x - drag.origin, viewport.width)
                        };
                        let target = drag.length + f32::from(delta);
                        view.workspace_layout
                            .resize(divider, target, f32::from(extent));
                        cx.notify();
                    }
                },
            ))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, _, _, _| view.divider_drag = None),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|view, _, _, _| view.divider_drag = None),
            )
            .on_key_down(cx.listener(move |view, event: &KeyDownEvent, window, cx| {
                let delta = match (horizontal, event.keystroke.key.as_str()) {
                    (false, "left") | (true, "down") => -10.,
                    (false, "right") | (true, "up") => 10.,
                    _ => return,
                };
                let viewport = window.viewport_size();
                let extent = if horizontal {
                    viewport.height
                } else {
                    viewport.width
                };
                view.workspace_layout
                    .resize(divider, length + delta, f32::from(extent));
                cx.stop_propagation();
                cx.notify();
            }))
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use crate::{Root, initialize_app};
    use gpui::{KeyUpEvent, Keystroke, Modifiers, TestAppContext, point};
    use refract_native::create_project;

    #[gpui::test]
    fn resizes_the_split_with_pointer_and_keyboard_and_switches_preview(cx: &mut TestAppContext) {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let project = create_project(&projects, "Paper").unwrap();
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
        cx.refresh().unwrap();
        let divider = cx.debug_bounds("workspace-resize").unwrap().center();
        let across = |offset| point(divider.x + px(offset), divider.y);
        cx.simulate_mouse_down(divider, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(across(15.), MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(across(80.), MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(across(80.), MouseButton::Left, Modifiers::none());
        cx.refresh().unwrap();
        let shifted = cx.debug_bounds("workspace-resize").unwrap().center();
        assert_eq!(shifted.x - divider.x, px(80.));
        cx.simulate_click(shifted, Modifiers::none());
        cx.simulate_keystrokes("left");
        cx.refresh().unwrap();
        assert_eq!(
            cx.debug_bounds("workspace-resize").unwrap().center().x,
            shifted.x - px(10.)
        );

        let preview = cx.debug_bounds("pane-preview").unwrap().center();
        cx.simulate_click(preview, Modifiers::none());
        cx.update(|window, app| assert!(!library.read(app).pdf_is_visible(window)));
        assert!(
            !PaneVisibility::read(&profile.path().join(".refract-native-workspace.json")).preview
        );
        cx.simulate_keystrokes("enter");
        cx.refresh().unwrap();
        cx.simulate_event(KeyUpEvent {
            keystroke: Keystroke::parse("enter").unwrap(),
        });
        cx.update(|window, app| assert!(library.read(app).pdf_is_visible(window)));
    }
}

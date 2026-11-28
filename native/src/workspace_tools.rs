use super::{Library, Palette, Selection};
use gpui::{ClickEvent, Context, KeyDownEvent, Window, div, prelude::*, px, rgb, svg};
use gpui_component::tooltip::Tooltip;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WorkspaceTool {
    ProjectInfo,
    Versions,
    Comments,
    Logs,
}

impl WorkspaceTool {
    fn label(self) -> &'static str {
        match self {
            Self::ProjectInfo => "Project Info",
            Self::Versions => "Versions",
            Self::Comments => "Comments",
            Self::Logs => "Logs",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::ProjectInfo => "tool-info",
            Self::Versions => "tool-versions",
            Self::Comments => "tool-comments",
            Self::Logs => "tool-logs",
        }
    }
}

impl Selection {
    pub(super) fn expanded_tools(&self) -> bool {
        self.tools_expanded.unwrap_or_else(|| {
            self.comparison.is_some()
                || self.tool == Some(WorkspaceTool::Logs)
                    && self.build_logs.as_ref().is_some_and(|view| view.expanded)
        })
    }
}

impl Library {
    pub(super) fn select_workspace_tool(
        &mut self,
        tool: WorkspaceTool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(selection) = &mut self.selection else {
            return;
        };
        if selection.tool == Some(tool) {
            return;
        }
        selection.tool = Some(tool);
        self.last_tool = tool;
        if selection.tools_expanded != Some(true) {
            selection.tools_expanded = None;
        }
        if tool != WorkspaceTool::Versions {
            selection.comparison = None;
            self.comparison_task = None;
        }
        match tool {
            WorkspaceTool::ProjectInfo => self.load_project_info(window, cx),
            WorkspaceTool::Comments => self.load_workspace_comments(window, cx),
            WorkspaceTool::Logs => self.load_build_logs(window, cx),
            WorkspaceTool::Versions => {}
        }
        cx.notify();
    }

    pub(super) fn close_workspace_tools(&mut self, cx: &mut Context<Self>) {
        if let Some(selection) = &mut self.selection
            && selection.tool.take().is_some()
        {
            selection.comparison = None;
            self.comparison_task = None;
            selection.tools_expanded = None;
            cx.notify();
        }
    }

    fn toggle_workspace_tools(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .selection
            .as_ref()
            .is_some_and(|selection| selection.tool.is_some())
        {
            self.close_workspace_tools(cx);
        } else {
            self.select_workspace_tool(self.last_tool, window, cx);
        }
    }

    pub(super) fn workspace_tools_button(
        &self,
        selection: &Selection,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        tool_button(
            "project-tools",
            svg()
                .path("refract/wrench.svg")
                .size(px(18.))
                .text_color(rgb(palette.muted)),
            selection.tool.is_some(),
            palette,
            cx,
            Self::toggle_workspace_tools,
        )
        .w(px(26.))
        .h(px(24.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .border_1()
        .border_color(rgb(palette.input_border))
        .tooltip(|window, cx| Tooltip::new("Tools").build(window, cx))
    }

    pub(super) fn workspace_tools_pane(
        &self,
        selection: &Selection,
        tool: WorkspaceTool,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let expanded = selection.expanded_tools();
        let tabs = [
            WorkspaceTool::ProjectInfo,
            WorkspaceTool::Versions,
            WorkspaceTool::Comments,
            WorkspaceTool::Logs,
        ];
        let body = match tool {
            WorkspaceTool::ProjectInfo => self
                .project_info_pane(selection, palette, cx)
                .into_any_element(),
            WorkspaceTool::Versions => self
                .versions_pane(selection, palette, cx)
                .into_any_element(),
            WorkspaceTool::Comments => self
                .comments_pane(selection, palette, cx)
                .into_any_element(),
            WorkspaceTool::Logs => self
                .build_output_pane(selection, palette, cx)
                .into_any_element(),
        };
        div()
            .id("tools-panel")
            .flex_1()
            .min_w(px(0.))
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(rgb(palette.border))
            .bg(rgb(palette.surface))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_1()
                    .p_2()
                    .border_b_1()
                    .border_color(rgb(palette.border))
                    .children(tabs.into_iter().map(|tab| {
                        tool_button(
                            tab.id(),
                            tab.label(),
                            tab == tool,
                            palette,
                            cx,
                            move |library, window, cx| {
                                library.select_workspace_tool(tab, window, cx)
                            },
                        )
                    }))
                    .child(tool_button(
                        "expand-tools",
                        if expanded {
                            "Collapse panel"
                        } else {
                            "Expand panel"
                        },
                        expanded,
                        palette,
                        cx,
                        |library, _, cx| {
                            if let Some(selection) = &mut library.selection {
                                selection.tools_expanded = Some(!selection.expanded_tools());
                                cx.notify();
                            }
                        },
                    )),
            )
            .child(div().flex_1().min_h(px(0.)).child(body))
    }
}

pub(super) fn tool_button<F>(
    id: &'static str,
    label: impl IntoElement,
    selected: bool,
    palette: Palette,
    cx: &Context<Library>,
    activate: F,
) -> gpui::Stateful<gpui::Div>
where
    F: Fn(&mut Library, &mut Window, &mut Context<Library>) + Clone + 'static,
{
    let keyboard = activate.clone();
    div()
        .id(id)
        .debug_selector(|| id.to_owned())
        .tab_index(3)
        .rounded_md()
        .px_1()
        .py_1()
        .cursor_pointer()
        .when(selected, |button| button.bg(rgb(palette.selection)))
        .hover(move |style| style.bg(rgb(palette.selection)))
        .focus(move |style| style.text_color(rgb(palette.focus)))
        .child(label)
        .on_click(cx.listener(move |library, event: &ClickEvent, window, cx| {
            if matches!(event, ClickEvent::Mouse(_)) {
                activate(library, window, cx);
            }
        }))
        .on_key_down(
            cx.listener(move |library, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                    keyboard(library, window, cx);
                }
            }),
        )
}

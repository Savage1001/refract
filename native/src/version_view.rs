use super::{Library, Palette, monospace_font};
use gpui::{
    AnyElement, Context, ElementId, FontWeight, KeyDownEvent, ListAlignment, ListState,
    SharedString, Window, div, list, prelude::*, px, rgb,
};
use refract_native::versions::{
    ComparisonSide, Difference, DifferenceLines, Version, compare_lines, compare_project,
};
use std::{io, rc::Rc};

pub(super) struct Comparison {
    version_id: String,
    pub(super) changes: Option<io::Result<Vec<Difference>>>,
    pub(super) selection: Option<ComparedFile>,
}

pub(super) struct ComparedFile {
    difference: Difference,
    pub(super) lines: Option<io::Result<ComparisonView>>,
    changed_only: bool,
    wrap_lines: bool,
}

pub(super) struct ComparisonView {
    previous: Option<LineList>,
    pub(super) current: Option<LineList>,
    skipped_alignment: bool,
}

pub(super) struct LineList {
    pub(super) content: Rc<ComparisonSide>,
    state: ListState,
    unwrapped_width: f32,
}

impl LineList {
    fn new(side: ComparisonSide) -> Self {
        let unwrapped_width = side
            .lines
            .iter()
            .map(|line| line.text.chars().count() + 3 * line.text.matches('\t').count())
            .max()
            .unwrap_or_default() as f32
            * 8.
            + 16.;
        Self {
            state: ListState::new(side.changed_indices.len(), ListAlignment::Top, px(150.)),
            content: Rc::new(side),
            unwrapped_width,
        }
    }

    fn reset(&self, changed_only: bool) {
        self.state.reset(if changed_only {
            self.content.changed_indices.len()
        } else {
            self.content.lines.len()
        });
    }
}

impl Library {
    fn toggle_comparison(&mut self, version_id: &str, cx: &mut Context<Self>) {
        let Some(selection) = &mut self.selection else {
            return;
        };
        self.comparison_task = None;
        if selection
            .comparison
            .as_ref()
            .is_some_and(|active| active.version_id == version_id)
        {
            selection.comparison = None;
            cx.notify();
            return;
        }
        selection.comparison = Some(Comparison {
            version_id: version_id.into(),
            changes: None,
            selection: None,
        });
        let root = self.projects_dir.clone();
        let project = selection.project.clone();
        let id = version_id.to_owned();
        let load = cx
            .background_executor()
            .spawn(async move { compare_project(&root, &project, &id) });
        self.comparison_task = Some(cx.spawn(async move |library, cx| {
            let changes = load.await;
            let _ = library.update(cx, |library, cx| {
                if let Some(comparison) = library
                    .selection
                    .as_mut()
                    .and_then(|selection| selection.comparison.as_mut())
                {
                    comparison.changes = Some(changes);
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    fn toggle_compared_file(&mut self, difference: &Difference, cx: &mut Context<Self>) {
        let Some(selection) = &mut self.selection else {
            return;
        };
        let Some(comparison) = &mut selection.comparison else {
            return;
        };
        if comparison
            .selection
            .as_ref()
            .is_some_and(|file| file.difference == *difference)
        {
            comparison.selection = None;
            self.comparison_task = None;
            cx.notify();
            return;
        }
        comparison.selection = Some(ComparedFile {
            difference: difference.clone(),
            lines: None,
            changed_only: true,
            wrap_lines: true,
        });
        let root = self.projects_dir.clone();
        let project = selection.project.clone();
        let version_id = comparison.version_id.clone();
        let difference = difference.clone();
        let load = cx
            .background_executor()
            .spawn(async move { compare_lines(&root, &project, &version_id, &difference) });
        self.comparison_task = Some(cx.spawn(async move |library, cx| {
            let content = load.await;
            let _ = library.update(cx, |library, cx| {
                let Some(file) = library
                    .selection
                    .as_mut()
                    .and_then(|selection| selection.comparison.as_mut()?.selection.as_mut())
                else {
                    return;
                };
                file.lines = Some(content.map(
                    |DifferenceLines {
                         previous,
                         current,
                         skipped_alignment,
                     }| ComparisonView {
                        previous: previous.map(LineList::new),
                        current: current.map(LineList::new),
                        skipped_alignment,
                    },
                ));
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn comparison_setting(&mut self, setting: ComparisonSetting, cx: &mut Context<Self>) {
        let Some(file) = self
            .selection
            .as_mut()
            .and_then(|selection| selection.comparison.as_mut()?.selection.as_mut())
        else {
            return;
        };
        match setting {
            ComparisonSetting::ChangedLines => file.changed_only = !file.changed_only,
            ComparisonSetting::Wrap => file.wrap_lines = !file.wrap_lines,
        }
        if let Some(Ok(lines)) = &file.lines {
            for side in [&lines.previous, &lines.current].into_iter().flatten() {
                side.reset(file.changed_only);
            }
        }
        cx.notify();
    }

    pub(super) fn version_row(
        &self,
        version: &Version,
        number: usize,
        comparison: Option<&Comparison>,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let timestamp = chrono::DateTime::parse_from_rfc3339(&version.created)
            .map(|date| {
                date.with_timezone(&chrono::Local)
                    .format("%-m/%-d/%Y, %-I:%M %p")
                    .to_string()
            })
            .unwrap_or_else(|_| version.created.clone());
        let files = version.files.len();
        let unit = if files == 1 { "file" } else { "files" };
        let version_id = version.id.clone();
        let comparison = comparison.filter(|active| active.version_id == version_id);
        div()
            .id(SharedString::from(format!("version/{}", version.id)))
            .border_b_1()
            .border_color(rgb(palette.border))
            .py_2()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("v{number} · {timestamp} · {files} {unit}")),
            )
            .child(comparison_button(
                "compare",
                "Compare with current",
                comparison.is_some(),
                palette,
                cx,
                move |library, _, cx| {
                    library.toggle_comparison(&version_id, cx);
                },
            ))
            .when_some(comparison, |row, comparison| {
                row.child(self.comparison_entries(comparison, palette, cx))
            })
    }

    fn comparison_entries(
        &self,
        comparison: &Comparison,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let rows = div().flex().flex_col().gap_1().py_1();
        match &comparison.changes {
            None => rows.child("Loading changes…"),
            Some(Err(error)) => rows.child(format!("Couldn’t compare: {error}")),
            Some(Ok(changes)) if changes.is_empty() => rows.child("No file changes"),
            Some(Ok(changes)) => {
                let description = if changes.len() == 1 {
                    "1 file differs from the current project.".into()
                } else {
                    format!("{} files differ from the current project.", changes.len())
                };
                rows.child(div().text_color(rgb(palette.muted)).child(description))
                    .children(changes.iter().enumerate().map(|(index, difference)| {
                        let file = comparison
                            .selection
                            .as_ref()
                            .filter(|file| file.difference == *difference);
                        let label = format!("{}: {}", difference.kind.label(), difference.name);
                        let difference = difference.clone();
                        div()
                            .id(("difference", index))
                            .child(comparison_button(
                                "select-file",
                                label,
                                file.is_some(),
                                palette,
                                cx,
                                move |library, _, cx| {
                                    library.toggle_compared_file(&difference, cx);
                                },
                            ))
                            .when_some(file, |row, file| {
                                row.child(self.compared_lines(file, palette, cx))
                            })
                    }))
            }
        }
    }

    fn compared_lines(
        &self,
        file: &ComparedFile,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let pane = div().p_2().flex().flex_col().gap_2();
        match &file.lines {
            None => pane.child("Loading file…"),
            Some(Err(error)) => pane.child(format!("Couldn’t open version text: {error}")),
            Some(Ok(lines)) => {
                let hint = if lines.skipped_alignment {
                    "File is too large for line alignment; all lines are highlighted."
                } else {
                    "Highlighted lines show changes between the version and the current file."
                };
                pane.child(div().text_color(rgb(palette.muted)).child(hint))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(comparison_button(
                                "filter",
                                if file.changed_only {
                                    "Show all lines"
                                } else {
                                    "Show only differing lines"
                                },
                                false,
                                palette,
                                cx,
                                |library, _, cx| {
                                    library.comparison_setting(ComparisonSetting::ChangedLines, cx);
                                },
                            ))
                            .child(comparison_button(
                                "wrap",
                                if file.wrap_lines {
                                    "Unwrap lines"
                                } else {
                                    "Wrap lines"
                                },
                                false,
                                palette,
                                cx,
                                |library, _, cx| {
                                    library.comparison_setting(ComparisonSetting::Wrap, cx);
                                },
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(line_column(
                                "Version",
                                lines.previous.as_ref(),
                                file,
                                palette,
                            ))
                            .child(line_column(
                                "Current",
                                lines.current.as_ref(),
                                file,
                                palette,
                            )),
                    )
            }
        }
    }
}

enum ComparisonSetting {
    ChangedLines,
    Wrap,
}

fn comparison_button<F>(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    active: bool,
    palette: Palette,
    cx: &Context<Library>,
    action: F,
) -> impl IntoElement
where
    F: Fn(&mut Library, &mut Window, &mut Context<Library>) + Clone + 'static,
{
    let by_key = action.clone();
    div()
        .id(id)
        .tab_index(3)
        .cursor_pointer()
        .rounded_md()
        .px_2()
        .py_1()
        .when(active, |button| button.bg(rgb(palette.selection)))
        .hover(move |style| style.bg(rgb(palette.selection)))
        .focus(move |style| style.text_color(rgb(palette.focus)))
        .child(label.into())
        .on_click(cx.listener(move |library, _, window, cx| action(library, window, cx)))
        .on_key_down(
            cx.listener(move |library, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                    by_key(library, window, cx);
                }
            }),
        )
}

fn line_column(
    label: &'static str,
    side: Option<&LineList>,
    file: &ComparedFile,
    palette: Palette,
) -> impl IntoElement {
    let column = div()
        .flex_1()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .gap_1()
        .child(div().font_weight(FontWeight::SEMIBOLD).child(label));
    let content: AnyElement = match side {
        None => div()
            .child("Not present, binary, or too large to display")
            .into_any_element(),
        Some(side) if file.changed_only && side.content.changed_indices.is_empty() => {
            div().child("No differing lines").into_any_element()
        }
        Some(side) => {
            let rows = side.content.clone();
            let changed_only = file.changed_only;
            let wrap_lines = file.wrap_lines;
            let body = list(side.state.clone(), move |index, _, _| {
                let line = &rows.lines[if changed_only {
                    rows.changed_indices[index]
                } else {
                    index
                }];
                let text = line
                    .text
                    .strip_suffix('\r')
                    .unwrap_or(&line.text)
                    .replace('\t', "    ");
                div()
                    .px_1()
                    .text_size(px(12.))
                    .min_h(px(20.))
                    .font_family(monospace_font())
                    .when(!wrap_lines, |row| row.whitespace_nowrap())
                    .when(line.changed, |row| {
                        row.bg(rgb(palette.warning_surface))
                            .border_l_2()
                            .border_color(rgb(palette.warning))
                    })
                    .child(if text.is_empty() { " ".into() } else { text })
                    .into_any_element()
            })
            .h(px(170.))
            .w_full()
            .when(!wrap_lines, |list| list.min_w(px(side.unwrapped_width)));
            div()
                .id(label)
                .w_full()
                .overflow_x_scroll()
                .child(body)
                .into_any_element()
        }
    };
    column.child(content)
}

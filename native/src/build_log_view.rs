use super::{
    Library, Palette, ProjectEntry, Selection, monospace_font, workspace_tools::tool_button,
};
use gpui::{
    Context, Entity, HighlightStyle, SharedString, StyledText, Subscription, Window, div,
    prelude::*, px, rgb,
};
use gpui_component::input::{Input, InputEvent, InputState};
use refract_native::tex::diagnostics::{DiagnosticLevel, diagnostic_level, diagnostic_source};

pub(super) struct BuildLogView {
    query: Entity<InputState>,
    pub(super) expanded: bool,
    issues_only: bool,
    wrap: bool,
    _query_change: Subscription,
}

impl Library {
    pub(super) fn load_build_logs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(selection) = &mut self.selection else {
            return;
        };
        if selection.build_logs.is_none() {
            let query = cx.new(|cx| InputState::new(window, cx).placeholder("Filter output lines"));
            let query_change = cx.subscribe(&query, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            });
            selection.build_logs = Some(BuildLogView {
                query,
                expanded: false,
                issues_only: false,
                wrap: true,
                _query_change: query_change,
            });
        }
    }

    pub(super) fn build_output_pane(
        &self,
        selection: &Selection,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let fallback = if self.build_task.is_some() {
            "Compiling…"
        } else if selection.build.queued {
            "Changes pending compilation"
        } else {
            ""
        };
        let text = selection
            .build
            .log()
            .filter(|log| !log.is_empty())
            .unwrap_or(fallback);
        let mut errors = 0;
        let mut warnings = 0;
        let mut diagnostics = Vec::new();
        let mut repeated = false;
        for line in text.lines() {
            let Some(level) = diagnostic_level(line) else {
                continue;
            };
            match level {
                DiagnosticLevel::Error => errors += 1,
                DiagnosticLevel::Warning => warnings += 1,
            }
            if errors + warnings <= 20 {
                if diagnostics.iter().any(|(seen, _)| line == *seen) {
                    repeated = true;
                } else {
                    diagnostics.push((line, level));
                }
            }
        }
        let summary = if text.is_empty() {
            "No build output yet.".to_owned()
        } else {
            let error_label = if errors == 1 {
                "error line"
            } else {
                "error lines"
            };
            let warning_label = if warnings == 1 {
                "warning line"
            } else {
                "warning lines"
            };
            format!("{errors} detected {error_label} · {warnings} detected {warning_label}")
        };
        let diagnostics = div()
            .id("log-diagnostics")
            .max_h(px(190.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_1()
            .when(repeated, |list| {
                list.child(
                    div()
                        .text_color(rgb(palette.muted))
                        .child("Repeated diagnostics remain in the full output."),
                )
            })
            .children(
                diagnostics
                    .into_iter()
                    .enumerate()
                    .map(|(index, (line, level))| {
                        let source = diagnostic_source(line).filter(|source| {
                            selection
                                .tree
                                .as_ref()
                                .and_then(|tree| tree.as_ref().ok())
                                .is_some_and(|tree| {
                                    tree_has_file(tree, &selection.expanded_folders, &source.file)
                                })
                        });
                        div()
                            .id(SharedString::from(format!("diagnostic-{index}")))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_color(rgb(diagnostic_color(level, palette)))
                                    .child(line.to_owned()),
                            )
                            .children(source.map(|source| {
                                let label =
                                    format!("Go to line {} in {}", source.line, source.file);
                                tool_button(
                                    "diagnostic-source",
                                    label,
                                    false,
                                    palette,
                                    cx,
                                    move |library, window, cx| {
                                        library.jump_to_source_position(source.clone(), window, cx);
                                    },
                                )
                            }))
                    }),
            );
        div()
            .id("build-output")
            .size_full()
            .flex()
            .flex_col()
            .p_4()
            .gap_2()
            .child(crate::tool_heading("Build output"))
            .child(div().text_color(rgb(palette.muted)).child(summary))
            .child(diagnostics)
            .children(
                selection
                    .build_logs
                    .as_ref()
                    .map(|view| raw_output(view, text, palette, cx)),
            )
    }
}

fn raw_output(
    view: &BuildLogView,
    text: &str,
    palette: Palette,
    cx: &Context<Library>,
) -> impl IntoElement {
    let disclosure = tool_button(
        "full-build-output",
        "Full build output",
        view.expanded,
        palette,
        cx,
        |library, _, cx| {
            if let Some(view) = library
                .selection
                .as_mut()
                .and_then(|selection| selection.build_logs.as_mut())
            {
                view.expanded = !view.expanded;
                cx.notify();
            }
        },
    );
    div()
        .flex()
        .flex_1()
        .min_h(px(0.))
        .flex_col()
        .gap_2()
        .child(disclosure)
        .when(view.expanded, |pane| {
            let query = view.query.read(cx).value().trim().to_lowercase();
            let (output, count) = matching_lines(text, &query, view.issues_only, palette);
            let filtered = !query.is_empty() || view.issues_only;
            let count_label = if count == 1 {
                "1 matching line".to_owned()
            } else {
                format!("{count} matching lines")
            };
            pane.child(Input::new(&view.query))
                .child(
                    div()
                        .flex()
                        .gap_1()
                        .items_center()
                        .child(tool_button(
                            "issues-only",
                            "Issues only",
                            view.issues_only,
                            palette,
                            cx,
                            |library, _, cx| {
                                if let Some(view) = library
                                    .selection
                                    .as_mut()
                                    .and_then(|selection| selection.build_logs.as_mut())
                                {
                                    view.issues_only = !view.issues_only;
                                    cx.notify();
                                }
                            },
                        ))
                        .child(tool_button(
                            "wrap-build-output",
                            if view.wrap {
                                "Disable line wrap"
                            } else {
                                "Enable line wrap"
                            },
                            view.wrap,
                            palette,
                            cx,
                            |library, _, cx| {
                                if let Some(view) = library
                                    .selection
                                    .as_mut()
                                    .and_then(|selection| selection.build_logs.as_mut())
                                {
                                    view.wrap = !view.wrap;
                                    cx.notify();
                                }
                            },
                        ))
                        .when(filtered, |controls| {
                            controls.child(div().text_color(rgb(palette.muted)).child(count_label))
                        }),
                )
                .when(filtered && count == 0, |pane| {
                    pane.child("No output lines match these filters.")
                })
                .when(!filtered || count > 0, |pane| {
                    pane.child(
                        div()
                            .id("compile-log")
                            .flex_1()
                            .min_h(px(0.))
                            .overflow_scroll()
                            .p_3()
                            .bg(rgb(palette.canvas))
                            .font_family(monospace_font())
                            .text_size(px(12.))
                            .text_color(rgb(palette.text))
                            .when(!view.wrap, |log| log.whitespace_nowrap())
                            .child(output),
                    )
                })
        })
}

fn matching_lines(
    text: &str,
    query: &str,
    issues_only: bool,
    palette: Palette,
) -> (StyledText, usize) {
    let mut displayed = String::new();
    let mut highlights = Vec::new();
    let mut count = 0;
    for line in text.lines() {
        if !query.is_empty() && !line.to_lowercase().contains(query) {
            continue;
        }
        let level = diagnostic_level(line);
        if issues_only && level.is_none() {
            continue;
        }
        if count != 0 {
            displayed.push('\n');
        }
        let start = displayed.len();
        displayed.push_str(line);
        count += 1;
        if let Some(level) = level {
            highlights.push((
                start..displayed.len(),
                HighlightStyle {
                    color: Some(rgb(diagnostic_color(level, palette)).into()),
                    ..Default::default()
                },
            ));
        }
    }
    (
        StyledText::new(displayed).with_highlights(highlights),
        count,
    )
}

fn diagnostic_color(level: DiagnosticLevel, palette: Palette) -> u32 {
    match level {
        DiagnosticLevel::Error => {
            if palette.canvas == 0x0a0a0a {
                0xfca5a5
            } else {
                0xb91c1c
            }
        }
        DiagnosticLevel::Warning => palette.warning,
    }
}

fn tree_has_file(
    tree: &[ProjectEntry],
    expanded: &std::collections::HashSet<String>,
    file: &str,
) -> bool {
    tree.iter().any(|entry| match &entry.children {
        Some(children) => expanded.contains(&entry.path) && tree_has_file(children, expanded, file),
        None => entry.path == file,
    })
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use crate::{WorkspaceTool, initialize_app};
    use gpui::{Modifiers, TestAppContext};
    use gpui_component::Root;
    use refract_native::{Project, create_project, tex::BuildResult};
    use std::{fs, io, path::Path, sync::atomic::AtomicBool};

    const BUILD_LOG: &str = "./chapter.tex:2: Undefined control sequence.\nPackage sample Warning: Retry.\nTranscript written.";

    fn fixture_compile(_: &Path, _: &Project, _: &AtomicBool) -> io::Result<BuildResult> {
        Ok(BuildResult {
            arguments: Vec::new(),
            exit_code: Some(1),
            log: BUILD_LOG.into(),
            pdf: None,
        })
    }

    #[gpui::test]
    fn filters_logs_and_jumps_to_the_reported_source_line(cx: &mut TestAppContext) {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let project = create_project(&projects, "Paper").unwrap();
        fs::write(
            projects.join(&project.id).join("chapter.tex"),
            "First line\nSecond line\n",
        )
        .unwrap();
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
            })
        });
        cx.run_until_parked();
        let tools = cx.debug_bounds("project-tools").unwrap();
        cx.simulate_click(tools.center(), Modifiers::none());
        let disclosure = cx.debug_bounds("full-build-output").unwrap();
        cx.simulate_click(disclosure.center(), Modifiers::none());
        let issues = cx.debug_bounds("issues-only").unwrap();
        cx.simulate_click(issues.center(), Modifiers::none());
        cx.update(|window, app| {
            let selection = library.read(app).selection.as_ref().unwrap();
            assert_eq!(selection.tool, Some(WorkspaceTool::Logs));
            let view = selection.build_logs.as_ref().unwrap();
            assert!(view.expanded && view.issues_only);
            let palette = Palette::for_window(window);
            assert_eq!(matching_lines(BUILD_LOG, "", true, palette).1, 2);
            assert_eq!(matching_lines(BUILD_LOG, "retry", true, palette).1, 1);
        });
        let source = cx.debug_bounds("diagnostic-source").unwrap();
        cx.simulate_click(source.center(), Modifiers::none());
        cx.run_until_parked();
        cx.update(|_, app| {
            let selection = library.read(app).selection.as_ref().unwrap();
            assert_eq!(selection.file.as_deref(), Some("chapter.tex"));
            let tab = selection
                .tabs
                .iter()
                .find(|tab| tab.path == "chapter.tex")
                .unwrap();
            assert_eq!(
                tab.editor
                    .as_ref()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .state
                    .read(app)
                    .cursor_position()
                    .line,
                1
            );
        });
    }
}

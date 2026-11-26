use super::{Library, Palette, Selection};
use gpui::{
    AssetSource, Context, Entity, Focusable, FontWeight, HighlightStyle, KeyDownEvent,
    SharedString, StyledText, Subscription, Task, Window, div, prelude::*, px, rgb, svg,
};
use gpui_component::{
    actions::SelectRight,
    input::{Input, InputEvent, InputState, Position},
};
use refract_native::project_search::{SearchHit, search_project};
use std::{
    borrow::Cow,
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

const SEARCH_DELAY: Duration = Duration::from_millis(200);

pub(super) struct RefractAssets;

impl AssetSource for RefractAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let bytes: &[u8] = match path {
            "refract/search.svg" => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M21 21l-5-5M17 10a7 7 0 1 1-14 0 7 7 0 0 1 14 0"/></svg>"#,
            "refract/close.svg" => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M6 6l12 12M18 6 6 18"/></svg>"#,
            _ => return Ok(None),
        };
        Ok(Some(Cow::Borrowed(bytes)))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(if path.trim_end_matches('/') == "refract" {
            vec!["refract/search.svg".into(), "refract/close.svg".into()]
        } else {
            Vec::new()
        })
    }
}

pub(super) struct FileSearch {
    query: Entity<InputState>,
    results: Option<io::Result<Vec<SearchHit>>>,
    request: Option<SearchRequest>,
    pub(super) pending_hit: Option<SearchHit>,
    _query_changed: Subscription,
}

struct SearchRequest {
    cancelled: Arc<AtomicBool>,
    _task: Task<()>,
}

impl Drop for SearchRequest {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

impl FileSearch {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Library>) -> Self {
        let query = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search files")
                .clean_on_escape()
        });
        let query_changed = cx.subscribe_in(
            &query,
            window,
            |library, query, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    library.run_file_search(query.read(cx).value(), window, cx);
                }
            },
        );
        Self {
            query,
            results: None,
            request: None,
            pending_hit: None,
            _query_changed: query_changed,
        }
    }
}

impl Library {
    fn run_file_search(&mut self, query: SharedString, window: &Window, cx: &mut Context<Self>) {
        let Some(selection) = &mut self.selection else {
            return;
        };
        let view = &mut selection.file_search;
        view.request = None;
        if query.trim().is_empty() {
            view.results = None;
            cx.notify();
            return;
        }
        let executor = cx.background_executor().clone();
        let delay = executor.timer(SEARCH_DELAY);
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker = cancelled.clone();
        let project = selection.project.clone();
        let path = self.projects_dir.clone();
        let task = cx.spawn_in(window, async move |library, cx| {
            delay.await;
            let results = executor
                .spawn(async move { search_project(&path, &project, &query, &worker) })
                .await;
            let _ = library.update_in(cx, |library, _, cx| {
                if let Some(selection) = &mut library.selection {
                    selection.file_search.results = Some(results);
                    cx.notify();
                }
            });
        });
        view.request = Some(SearchRequest {
            cancelled,
            _task: task,
        });
        cx.notify();
    }

    fn open_search_hit(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let hit = self
            .selection
            .as_ref()
            .and_then(|selection| selection.file_search.results.as_ref())
            .and_then(|results| results.as_ref().ok())
            .and_then(|results| results.get(index));
        let Some(hit) = hit.cloned() else {
            return;
        };
        self.open_file(hit.file.clone(), window, cx);
        let Some(selection) = &mut self.selection else {
            return;
        };
        let editor = selection
            .tabs
            .iter()
            .find(|tab| tab.path == hit.file)
            .and_then(|tab| tab.editor.as_ref())
            .and_then(|editor| editor.as_ref().ok());
        if let Some(editor) = editor {
            Self::select_search_hit(&editor.state, &hit, window, cx);
        } else {
            selection.file_search.pending_hit = Some(hit);
        }
    }

    pub(super) fn select_search_hit(
        editor: &Entity<InputState>,
        hit: &SearchHit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = editor.read(cx).value();
        let line_index = hit.line - 1;
        let Some(line) = text.split('\n').nth(line_index as usize) else {
            return;
        };
        let line = line.strip_suffix('\r').unwrap_or(line);
        // GPUI's editor positions count Unicode scalars, while the search coordinates count UTF-16 units.
        let start = character_column(line, hit.column);
        let end = character_column(line, hit.column + hit.length);
        let focus = editor.read(cx).focus_handle(cx);
        editor.update(cx, |editor, cx| {
            editor.set_cursor_position(Position::new(line_index, start), window, cx);
        });
        cx.notify();
        window.defer(cx, move |window, app| {
            if !focus.is_focused(window) {
                return;
            }
            // A tab opened by this result needs its input handler installed before dispatch.
            window.draw(app).clear();
            for _ in start..end {
                focus.dispatch_action(&SelectRight, window, app);
            }
        });
    }

    pub(super) fn file_search_panel(
        &self,
        selection: &Selection,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let view = &selection.file_search;
        let query = view.query.clone();
        let input = Input::new(&query)
            .appearance(false)
            .h(px(36.))
            .tab_index(1)
            .prefix(
                svg()
                    .path("refract/search.svg")
                    .size(px(18.))
                    .text_color(rgb(palette.muted)),
            )
            .when(!query.read(cx).value().is_empty(), |input| {
                input.suffix(
                    div()
                        .id("clear-file-search")
                        .debug_selector(|| "clear-file-search".into())
                        .tab_index(1)
                        .cursor_pointer()
                        .rounded_md()
                        .p_1()
                        .hover(move |style| style.bg(rgb(palette.selection)))
                        .child(
                            svg()
                                .path("refract/close.svg")
                                .size(px(16.))
                                .text_color(rgb(palette.muted)),
                        )
                        .on_click(cx.listener(|library, _, window, cx| {
                            library.clear_file_search(window, cx)
                        }))
                        .on_key_down(cx.listener(|library, event: &KeyDownEvent, window, cx| {
                            if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                                library.clear_file_search(window, cx);
                            }
                        })),
                )
            });
        let results = div()
            .id("file-search-results")
            .max_h(px(220.))
            .overflow_y_scroll()
            .text_size(px(12.))
            .text_color(rgb(palette.muted));
        let results = match &view.results {
            None => results,
            Some(Err(error)) => results.child(format!("Couldn’t search project files: {error}")),
            Some(Ok(hits)) => results
                .child(match hits.len() {
                    1 => "1 result".to_owned(),
                    count => format!("{count} results"),
                })
                .when(hits.is_empty(), |results| {
                    results.child(div().child("No matches. Try another search."))
                })
                .children(hits.iter().enumerate().map(|(index, hit)| {
                    let highlight = hit.highlight.clone().map(|range| {
                        (
                            range,
                            HighlightStyle {
                                color: Some(rgb(palette.warning).into()),
                                background_color: Some(rgb(palette.warning_surface).into()),
                                ..Default::default()
                            },
                        )
                    });
                    div()
                        .id(("file-search-result", index))
                        .debug_selector(|| format!("file-search-result-{index}"))
                        .tab_index(1)
                        .min_w(px(0.))
                        .mt(px(5.))
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(palette.border))
                        .bg(rgb(palette.surface))
                        .px_2()
                        .py_1()
                        .cursor_pointer()
                        .hover(move |style| style.bg(rgb(palette.selection)))
                        .focus(move |style| style.border_color(rgb(palette.focus)))
                        .child(
                            div()
                                .truncate()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.text))
                                .child(format!("{}:{}", hit.file, hit.line)),
                        )
                        .child(
                            div().truncate().child(
                                StyledText::new(hit.excerpt.clone()).with_highlights(highlight),
                            ),
                        )
                        .on_click(cx.listener(move |library, _, window, cx| {
                            library.open_search_hit(index, window, cx)
                        }))
                        .on_key_down(cx.listener(
                            move |library, event: &KeyDownEvent, window, cx| {
                                if event.keystroke.key == "enter" || event.keystroke.key == "space"
                                {
                                    library.open_search_hit(index, window, cx);
                                }
                            },
                        ))
                })),
        };
        div()
            .id("file-search")
            .debug_selector(|| "file-search".into())
            .flex_none()
            .child(input)
            .child(results)
    }

    fn clear_file_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(selection) = &self.selection {
            selection.file_search.query.update(cx, |query, cx| {
                query.set_value("", window, cx);
                query.focus(window, cx);
            });
        }
    }
}

fn character_column(line: &str, utf16_column: usize) -> u32 {
    let mut offset = 0;
    line.chars()
        .take_while(|ch| {
            offset += ch.len_utf16();
            offset <= utf16_column
        })
        .count() as u32
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use crate::{Root, initialize_app};
    use gpui::{EntityInputHandler, Modifiers, TestAppContext};
    use refract_native::create_project;
    use std::fs;

    #[gpui::test]
    fn searches_saved_files_and_selects_the_match_in_a_new_tab(cx: &mut TestAppContext) {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let project = create_project(&projects, "Paper").unwrap();
        fs::write(
            projects.join(&project.id).join("chapter.tex"),
            "Unrelated line\r\n𐐀 A.b axb\r\n",
        )
        .unwrap();
        cx.update(initialize_app);
        let mut library = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| Library::new(projects, window, cx));
            library = Some(view.clone());
            Root::new(view, window, cx)
        });
        let library = library.unwrap();
        cx.update(|window, app| {
            library.update(app, |view, cx| view.open_project(project, window, cx));
        });
        cx.run_until_parked();
        let focus_query = |window: &mut Window, app: &mut gpui::App| {
            let query = library
                .read(app)
                .selection
                .as_ref()
                .unwrap()
                .file_search
                .query
                .clone();
            query.update(app, |query, cx| query.focus(window, cx));
        };
        cx.update(focus_query);
        cx.update(|window, app| {
            let query = library
                .read(app)
                .selection
                .as_ref()
                .unwrap()
                .file_search
                .query
                .clone();
            query.update(app, |query, cx| query.set_value("a.b", window, cx));
        });
        cx.executor().advance_clock(SEARCH_DELAY);
        cx.run_until_parked();
        cx.refresh().unwrap();
        let hit = cx.debug_bounds("file-search-result-0").unwrap();
        cx.simulate_click(hit.center(), Modifiers::none());
        cx.run_until_parked();
        cx.update(|window, app| {
            let selection = library.read(app).selection.as_ref().unwrap();
            assert_eq!(selection.file.as_deref(), Some("chapter.tex"));
            let tab = selection
                .tabs
                .iter()
                .find(|tab| tab.path == "chapter.tex")
                .unwrap();
            let editor = tab.editor.as_ref().unwrap().as_ref().unwrap().state.clone();
            editor.update(app, |editor, cx| {
                let selection = editor.selected_text_range(false, window, cx).unwrap();
                assert_eq!(
                    editor
                        .text_for_range(selection.range, &mut None, window, cx)
                        .as_deref(),
                    Some("A.b")
                );
            });
        });
        cx.update(focus_query);
        cx.simulate_keystrokes("escape");
        cx.update(|_, app| {
            let search = &library.read(app).selection.as_ref().unwrap().file_search;
            assert!(search.query.read(app).value().is_empty());
            assert!(search.results.is_none());
        });
    }
}

use super::{Library, Palette, Selection, editor_find_view::extend_editor_selection};
use gpui::{
    AssetSource, Context, Entity, Focusable, FontWeight, HighlightStyle, KeyDownEvent,
    SharedString, StyledText, Subscription, Task, Window, div, prelude::*, px, rgb, svg,
};
use gpui_component::input::{Input, InputEvent, InputState, Position};
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
            "refract/close.svg" | "icons/close.svg" => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M6 6l12 12M18 6 6 18"/></svg>"#,
            "refract/chevron-right.svg" | "icons/chevron-right.svg" => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="m9 6 6 6-6 6"/></svg>"#,
            "refract/chevron-left.svg" | "icons/chevron-left.svg" => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="m15 6-6 6 6 6"/></svg>"#,
            "refract/chevron-down.svg" | "icons/chevron-down.svg" => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>"#,
            "icons/check.svg" => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="m5 12 4 4L19 6"/></svg>"#,
            "refract/openai.svg" => include_bytes!("../../resources/provider-logos/openai-black.svg"),
            "refract/opencode.svg" => include_bytes!("../../resources/provider-logos/opencode-light.svg"),
            "refract/favorite-star.svg" => include_bytes!("../../resources/provider-logos/favorite-star.svg"),
            "refract/folder.svg" => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.5"><path d="M3 6.5h6l2 2h10v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></svg>"#,
            "refract/import.svg" => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M3 11V6h6l2 2h9a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1v-2M1 14h16m-3-3 3 3-3 3"/></svg>"#,
            "refract/settings.svg" => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M9.7 3h4.6l.6 2.4 2 .9 2.2-.8 2.3 4-1.7 1.7v2.3l1.7 1.7-2.3 4-2.2-.8-2 .9-.6 2.4H9.7l-.6-2.4-2-.9-2.2.8-2.3-4 1.7-1.7v-2.3L2.6 9.5l2.3-4 2.2.8 2-.9z"/><circle cx="12" cy="12" r="3"/></svg>"#,
            "refract/create-project.svg" => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M13 3H5v18h14v-7 M9 16l1-4 8-8 3 3-8 8z"/></svg>"#,
            "refract/trash.svg" => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7M14 10v7"/></svg>"#,
            "refract/ellipsis.svg" => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12h.1M12 12h.1M19 12h.1"/></svg>"#,
            "refract/menu.svg" => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 6h16M4 12h16M4 18h16"/></svg>"#,
            "refract/agent.svg" => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M15 14H7l-4 3V6a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2Z M10 14v2a2 2 0 0 0 2 2h5l4 3v-8a2 2 0 0 0-2-2h-2"/></svg>"#,
            "refract/attach.svg" => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M8 11V7a4 4 0 0 1 8 0v10a6 6 0 0 1-12 0V8m8 0v9a2 2 0 0 1-4 0v-6"/></svg>"#,
            "refract/send.svg" => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12 12 4.5 19 12M12 4.5v15"/></svg>"#,
            "refract/editor.svg" => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="m7 8-4 4 4 4m10-8 4 4-4 4M14 5l-4 14"/></svg>"#,
            "refract/preview.svg" => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H5v20h14V7Z M14 2v5h5M8 11h8M8 15h8M8 18h5"/></svg>"#,
            "refract/compile.svg" => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M19 9a7.2 7.2 0 1 0 .2 5m-3.2-5 3.5.1L20 5.5"/></svg>"#,
            "refract/download.svg" => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3v12m-5-5 5 5 5-5M5 16v4h14v-4"/></svg>"#,
            "refract/minus.svg" => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12h14"/></svg>"#,
            "refract/plus.svg" => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 5v14M5 12h14"/></svg>"#,
            "refract/export.svg" => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M14 3h7v7m0-7L10 14M10 5H5a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-5"/></svg>"#,
            "refract/wrench.svg" => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.8-3.8a6 6 0 0 1-7.9 7.9l-7 7a2.1 2.1 0 0 1-3-3l7-7a6 6 0 0 1 7.9-7.9z"/></svg>"#,
            _ => return Ok(None),
        };
        Ok(Some(Cow::Borrowed(bytes)))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(if path.trim_end_matches('/') == "refract" {
            vec![
                "refract/search.svg".into(),
                "refract/close.svg".into(),
                "refract/chevron-right.svg".into(),
                "refract/chevron-left.svg".into(),
                "refract/chevron-down.svg".into(),
                "refract/openai.svg".into(),
                "refract/opencode.svg".into(),
                "refract/favorite-star.svg".into(),
                "refract/folder.svg".into(),
                "refract/import.svg".into(),
                "refract/settings.svg".into(),
                "refract/create-project.svg".into(),
                "refract/trash.svg".into(),
                "refract/ellipsis.svg".into(),
                "refract/menu.svg".into(),
                "refract/agent.svg".into(),
                "refract/attach.svg".into(),
                "refract/send.svg".into(),
                "refract/editor.svg".into(),
                "refract/preview.svg".into(),
                "refract/compile.svg".into(),
                "refract/download.svg".into(),
                "refract/minus.svg".into(),
                "refract/plus.svg".into(),
                "refract/export.svg".into(),
                "refract/wrench.svg".into(),
            ]
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
        self.reveal_workspace_pane(refract_native::workspace_layout::Pane::Editor, window, cx);
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
            extend_editor_selection(&focus, (end - start) as usize, window, app);
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
            .font_weight(FontWeight::MEDIUM)
            .text_size(px(13.))
            .line_height(px(19.5))
            .h(px(36.))
            .rounded(px(8.))
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
            .py(px(6.))
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

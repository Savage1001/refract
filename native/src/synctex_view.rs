use super::{Library, Palette, Selection};
use gpui::{Context, Entity, Focusable, KeyDownEvent, Task, Window, div, prelude::*, rgb};
use gpui_component::input::{InputState, Position, SelectToEndOfLine};
use refract_native::tex::{
    TexToolchain,
    synctex::{PdfPosition, SourcePosition, forward, reverse},
};
use std::sync::{Arc, atomic::AtomicBool};

pub(super) struct RunningSync {
    pub(super) cancel: Arc<AtomicBool>,
    _task: Task<()>,
}

enum SyncRequest {
    Forward { file: String, line: u32 },
    Reverse(PdfPosition),
}

enum SyncResult {
    Pdf(PdfPosition),
    Source(SourcePosition),
}

impl Library {
    pub(super) fn show_in_pdf_button(
        &self,
        selection: &Selection,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let enabled = cfg!(windows) && selection.build.pdf().is_some() && self.sync_task.is_none();
        div()
            .id("show-in-pdf")
            .rounded_md()
            .px_2()
            .py_1()
            .child("Show in PDF")
            .when(!enabled, |button| button.text_color(rgb(palette.muted)))
            .when(enabled, |button| {
                button
                    .tab_index(5)
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.selection)))
                    .focus(move |style| style.bg(rgb(palette.selection)))
                    .on_click(
                        cx.listener(|library, _, window, cx| {
                            library.show_source_in_pdf(window, cx)
                        }),
                    )
                    .on_key_down(cx.listener(|library, event: &KeyDownEvent, window, cx| {
                        if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                            library.show_source_in_pdf(window, cx);
                        }
                    }))
            })
    }

    fn show_source_in_pdf(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(selection) = &self.selection else {
            return;
        };
        let Some(file) = &selection.file else {
            return;
        };
        let Some(Ok(editor)) = selection
            .tabs
            .iter()
            .find(|tab| &tab.path == file)
            .and_then(|tab| tab.editor.as_ref())
        else {
            return;
        };
        let line = editor
            .state
            .read(cx)
            .cursor_position()
            .line
            .saturating_add(1);
        self.start_synctex(
            SyncRequest::Forward {
                file: file.clone(),
                line,
            },
            window,
            cx,
        );
    }

    pub(super) fn reverse_pdf_at(
        &mut self,
        position: PdfPosition,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_synctex(SyncRequest::Reverse(position), window, cx);
    }

    fn start_synctex(&mut self, request: SyncRequest, window: &mut Window, cx: &mut Context<Self>) {
        if self.sync_task.is_some() || self.build_task.is_some() {
            return;
        }
        let Some(selection) = &mut self.selection else {
            return;
        };
        if selection.build.pdf().is_none() {
            return;
        }
        selection.build.sync_error = None;
        let expected_source = match &request {
            SyncRequest::Forward { file, .. } => Some(file.clone()),
            SyncRequest::Reverse(_) => None,
        };
        let project = selection.project.clone();
        let sequence = self.project_open_sequence;
        let projects = self.projects_dir.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let job_cancel = cancel.clone();
        let query = cx.background_executor().spawn(async move {
            let tools = TexToolchain::discover()?;
            match request {
                SyncRequest::Forward { file, line } => {
                    forward(&projects, &project, &tools, &file, line, &job_cancel)
                        .map(SyncResult::Pdf)
                }
                SyncRequest::Reverse(position) => {
                    reverse(&projects, &project, &tools, position, &job_cancel)
                        .map(SyncResult::Source)
                }
            }
        });
        let task = cx.spawn_in(window, async move |library, cx| {
            let result = query.await;
            let _ = library.update_in(cx, |library, window, cx| {
                library.sync_task = None;
                if let Some(selection) = &mut library.selection
                    && library.project_open_sequence == sequence
                    && selection.build.pdf().is_some()
                    && library.navigation_after_save.is_none()
                    && expected_source
                        .as_ref()
                        .is_none_or(|file| selection.file.as_ref() == Some(file))
                {
                    match result {
                        Ok(SyncResult::Pdf(position)) => {
                            selection.show_comments = false;
                            selection.show_versions = false;
                            selection.show_project_info = false;
                            selection.build.show_log = false;
                            selection.build.preview.jump_to(position);
                            library.request_pdf_frame(window, cx);
                        }
                        Ok(SyncResult::Source(position)) => {
                            library.jump_from_pdf(position, window, cx)
                        }
                        Err(error) if error.kind() != std::io::ErrorKind::Interrupted => {
                            let action = if expected_source.is_some() {
                                "find source in PDF"
                            } else {
                                "find PDF source"
                            };
                            selection.build.sync_error =
                                Some(format!("Couldn’t {action}. {error}"));
                        }
                        Err(_) => {}
                    }
                }
                library.resume_navigation(window, cx);
                cx.notify();
            });
        });
        self.sync_task = Some(RunningSync {
            cancel,
            _task: task,
        });
        cx.notify();
    }

    fn jump_from_pdf(
        &mut self,
        position: SourcePosition,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_file(position.file.clone(), window, cx);
        let Some(selection) = &mut self.selection else {
            return;
        };
        if let Some(Ok(editor)) = selection
            .tabs
            .iter()
            .find(|tab| tab.path == position.file)
            .and_then(|tab| tab.editor.as_ref())
        {
            Self::select_source_line(&editor.state, position.line, window, cx);
        } else {
            selection.pending_source_jump = Some(position);
        }
    }

    pub(super) fn select_source_line(
        editor: &Entity<InputState>,
        line: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(line) = line.checked_sub(1) else {
            return;
        };
        if editor
            .read(cx)
            .value()
            .split('\n')
            .nth(line as usize)
            .is_none()
        {
            return;
        }
        let focus = editor.read(cx).focus_handle(cx);
        editor.update(cx, |editor, cx| {
            editor.set_cursor_position(Position::new(line, 0), window, cx)
        });
        window.defer(cx, move |window, app| {
            if focus.is_focused(window) {
                window.dispatch_action(Box::new(SelectToEndOfLine), app);
            }
        });
    }
}

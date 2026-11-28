use crate::EditorDocument;
use gpui::{App, Entity};
use gpui_component::input::InputState;
use std::collections::VecDeque;

const MAX_VIEWS: usize = 32;
const MAX_SOURCE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Default)]
pub(super) struct EditorViews {
    recent: VecDeque<EditorView>,
}

struct EditorView {
    project: String,
    file: String,
    state: Entity<InputState>,
    bytes: usize,
}

impl EditorViews {
    pub(super) fn remember(
        &mut self,
        project: &str,
        file: String,
        editor: EditorDocument,
        cx: &App,
    ) {
        self.recent
            .retain(|view| view.project != project || view.file != file);
        let source = editor.state.read(cx).text();
        let bytes = source.len();
        if bytes > MAX_SOURCE_BYTES || !source.eq(editor.saved.as_str()) {
            return;
        }

        // GPUI keeps the selection direction and both scroll offsets inside InputState.
        // Keep the state without the document's extra source copy or write subscriptions.
        let mut total = bytes + self.recent.iter().map(|view| view.bytes).sum::<usize>();
        while (total > MAX_SOURCE_BYTES || self.recent.len() >= MAX_VIEWS)
            && let Some(oldest) = self.recent.pop_front()
        {
            total -= oldest.bytes;
        }
        self.recent.push_back(EditorView {
            project: project.to_owned(),
            file,
            state: editor.state,
            bytes,
        });
    }

    pub(super) fn restore(
        &mut self,
        project: &str,
        file: &str,
        source: &str,
        cx: &App,
    ) -> Option<Entity<InputState>> {
        let index = self
            .recent
            .iter()
            .position(|view| view.project == project && view.file == file)?;
        let view = self.recent.remove(index)?;
        // Always read the current file first. A restore or external edit may have replaced it.
        view.state.read(cx).text().eq(source).then_some(view.state)
    }
}

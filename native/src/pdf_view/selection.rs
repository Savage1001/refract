use super::{Library, PageSlot, PdfViewport};
use gpui::{
    AnyElement, Bounds, ClipboardItem, Context, MouseDownEvent, MouseMoveEvent, Pixels, Point, div,
    prelude::*, px, rgba,
};
use refract_native::pdf_raster::text::TextBounds;
use std::ops::Range;

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
struct Caret {
    page: u32,
    offset: usize,
}

#[derive(Clone, Copy)]
pub(super) struct TextSelection {
    anchor: Caret,
    end: Caret,
    dragging: bool,
}

impl TextSelection {
    fn ends(self) -> (Caret, Caret) {
        (self.anchor.min(self.end), self.anchor.max(self.end))
    }

    pub(super) fn contains_page(self, index: u32) -> bool {
        let (start, end) = self.ends();
        start.page <= index && index <= end.page
    }

    fn on_page(self, page: u32, length: usize) -> Option<Range<usize>> {
        if !self.contains_page(page) {
            return None;
        }
        let (start, end) = self.ends();
        let first = if start.page == page { start.offset } else { 0 };
        let last = if end.page == page { end.offset } else { length };
        (first < last).then_some(first..last)
    }
}

impl Library {
    pub(super) fn clear_pdf_selection(&mut self, cx: &mut Context<Self>) {
        if let Some(selection) = &mut self.selection
            && selection.build.preview.text_selection.take().is_some()
        {
            cx.notify();
        }
    }

    pub(super) fn begin_pdf_selection(
        &mut self,
        index: u32,
        bounds: Bounds<Pixels>,
        event: &MouseDownEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(selection) = &mut self.selection else {
            return;
        };
        let preview = &mut selection.build.preview;
        let hit = caret_at(preview, index, bounds, event.position, false);
        preview.text_selection = hit.map(|hit| TextSelection {
            anchor: preview
                .text_selection
                .filter(|_| event.modifiers.shift)
                .map_or(hit, |existing| existing.anchor),
            end: hit,
            dragging: true,
        });
        cx.notify();
    }

    pub(super) fn move_pdf_selection(
        &mut self,
        index: u32,
        bounds: Bounds<Pixels>,
        event: &MouseMoveEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(selection) = &mut self.selection else {
            return;
        };
        let preview = &mut selection.build.preview;
        let Some(mut selected) = preview.text_selection.filter(|selected| selected.dragging) else {
            return;
        };
        if let Some(hit) = caret_at(preview, index, bounds, event.position, true)
            && hit != selected.end
        {
            selected.end = hit;
            preview.text_selection = Some(selected);
            cx.notify();
        }
    }

    pub(super) fn finish_pdf_selection(&mut self, cx: &mut Context<Self>) {
        if let Some(selection) = &mut self.selection
            && let Some(text) = &mut selection.build.preview.text_selection
            && text.dragging
        {
            text.dragging = false;
            cx.notify();
        }
    }

    pub(super) fn copy_pdf_selection(&self, cx: &mut Context<Self>) -> bool {
        let Some(preview) = self
            .selection
            .as_ref()
            .map(|selection| &selection.build.preview)
        else {
            return false;
        };
        let Some(selected) = preview.text_selection else {
            return false;
        };
        let (start, end) = selected.ends();
        let mut text = String::new();
        for (&index, page) in preview.text_pages.range(start.page..=end.page) {
            if let Some(range) = selected.on_page(index, page.len()) {
                let chunk = page.text(range);
                if !chunk.is_empty() {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(&chunk);
                }
            }
        }
        if text.is_empty() {
            return false;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        true
    }
}

fn caret_at(
    preview: &PdfViewport,
    index: u32,
    bounds: Bounds<Pixels>,
    position: Point<Pixels>,
    outside: bool,
) -> Option<Caret> {
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    if width <= 0. || height <= 0. {
        return None;
    }
    let x = f32::from(position.x - bounds.origin.x) / width;
    let y = f32::from(position.y - bounds.origin.y) / height;
    let offset = preview.text_pages.get(&index)?.caret(x, y, outside)?;
    Some(Caret {
        page: index,
        offset,
    })
}

pub(super) fn highlights(preview: &PdfViewport, index: u32, slot: &PageSlot) -> Vec<AnyElement> {
    let Some(page) = preview.text_pages.get(&index) else {
        return Vec::new();
    };
    let Some(range) = preview
        .text_selection
        .and_then(|selection| selection.on_page(index, page.len()))
    else {
        return Vec::new();
    };
    let mut rectangles: Vec<TextBounds> = Vec::new();
    for glyph in page.bounds(range) {
        if let Some(line) = rectangles.last_mut()
            && (glyph.top - line.top).abs() * slot.height < 1.
            && (glyph.bottom - line.bottom).abs() * slot.height < 1.
            && glyph.left <= line.right + 4. / slot.width
            && glyph.right >= line.left - 4. / slot.width
        {
            line.left = line.left.min(glyph.left);
            line.right = line.right.max(glyph.right);
        } else {
            rectangles.push(glyph);
        }
    }
    rectangles
        .into_iter()
        .map(|rect| {
            div()
                .absolute()
                .left(px(rect.left * slot.width))
                .top(px(rect.top * slot.height))
                .w(px((rect.right - rect.left) * slot.width))
                .h(px((rect.bottom - rect.top) * slot.height))
                .bg(rgba(0x3970f359))
                .into_any_element()
        })
        .collect()
}

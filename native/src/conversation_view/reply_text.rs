use super::{nodes, safe_link};
use crate::{Palette, monospace_font};
use ::markdown::{ParseOptions, mdast::Node};
use gpui::{
    App, ClipboardItem, DispatchPhase, FocusHandle, FontStyle, FontWeight, KeyDownEvent,
    MouseButton, MouseMoveEvent, MouseUpEvent, Pixels, Point, SharedString, StrikethroughStyle,
    StyledText, TextLayout, TextRun, TextStyle, UnderlineStyle, Window, canvas, div, prelude::*,
    px, rgb,
};
use std::{cell::RefCell, collections::HashMap, ops::Range, rc::Rc};

#[derive(Clone, Default, PartialEq, Eq)]
struct Mark {
    bold: bool,
    italic: bool,
    code: bool,
    strike: bool,
    link: Option<SharedString>,
}

struct Span {
    range: Range<usize>,
    mark: Mark,
}

struct Paragraph {
    range: Range<usize>,
    spans: Vec<Span>,
}

pub(super) struct ReplyText {
    text: SharedString,
    paragraphs: Vec<Paragraph>,
}

#[derive(Default)]
struct ParagraphBuilder {
    text: String,
    spans: Vec<Span>,
    pending_space: Option<Mark>,
}

impl ParagraphBuilder {
    fn append(&mut self, text: &str, mark: &Mark) {
        for character in text.chars() {
            match character {
                ' ' | '\t' | '\n' | '\r' | '\u{000C}' => {
                    if !self.text.is_empty()
                        && !self.text.ends_with('\n')
                        && self.pending_space.is_none()
                    {
                        self.pending_space = Some(mark.clone());
                    }
                }
                character => {
                    if let Some(space) = self.pending_space.take() {
                        self.push(' ', &space);
                    }
                    self.push(character, mark);
                }
            }
        }
    }

    fn push(&mut self, character: char, mark: &Mark) {
        let start = self.text.len();
        self.text.push(character);
        if let Some(last) = self.spans.last_mut()
            && &last.mark == mark
        {
            last.range.end = self.text.len();
        } else {
            self.spans.push(Span {
                range: start..self.text.len(),
                mark: mark.clone(),
            });
        }
    }

    fn node(
        &mut self,
        node: &Node,
        mut mark: Mark,
        references: &HashMap<&str, &str>,
    ) -> Option<()> {
        let children = match node {
            Node::Text(text) => {
                self.append(&text.value, &mark);
                return Some(());
            }
            Node::InlineCode(code) => {
                mark.code = true;
                self.append(&code.value, &mark);
                return Some(());
            }
            Node::Break(_) | Node::Html(_) if super::safe_node(node) => {
                self.pending_space = None;
                self.push('\n', &mark);
                return Some(());
            }
            Node::Strong(strong) => {
                mark.bold = true;
                &strong.children
            }
            Node::Emphasis(emphasis) => {
                mark.italic = true;
                &emphasis.children
            }
            Node::Delete(strike) => {
                mark.strike = true;
                &strike.children
            }
            Node::Link(link) => {
                mark.link = safe_link(&link.url).then(|| link.url.clone().into());
                &link.children
            }
            Node::LinkReference(link) => {
                mark.link = references
                    .get(link.identifier.as_str())
                    .copied()
                    .filter(|url| safe_link(url))
                    .map(SharedString::from);
                &link.children
            }
            _ => return None,
        };
        for child in children {
            self.node(child, mark.clone(), references)?;
        }
        Some(())
    }
}

impl ReplyText {
    pub(super) fn from_markdown(source: &str) -> Option<Self> {
        let document = ::markdown::to_mdast(source, &ParseOptions::gfm()).ok()?;
        let mut references = HashMap::new();
        for node in nodes(&document) {
            if let Node::Definition(definition) = node {
                references
                    .entry(definition.identifier.as_str())
                    .or_insert(definition.url.as_str());
            }
        }
        let mut text = String::new();
        let mut paragraphs = Vec::new();
        for node in document.children()? {
            let paragraph = match node {
                Node::Paragraph(paragraph) => paragraph,
                Node::Definition(_) => continue,
                _ => return None,
            };
            let mut builder = ParagraphBuilder::default();
            for child in &paragraph.children {
                builder.node(child, Mark::default(), &references)?;
            }
            if builder.text.is_empty() {
                continue;
            }
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            let start = text.len();
            text.push_str(&builder.text);
            for span in &mut builder.spans {
                span.range.start += start;
                span.range.end += start;
            }
            paragraphs.push(Paragraph {
                range: start..text.len(),
                spans: builder.spans,
            });
        }
        (!paragraphs.is_empty()).then(|| Self {
            text: text.into(),
            paragraphs,
        })
    }

    fn link_at(&self, index: usize) -> Option<SharedString> {
        let paragraph = self
            .paragraphs
            .iter()
            .find(|paragraph| paragraph.range.contains(&index))?;
        paragraph
            .spans
            .iter()
            .find(|span| span.range.contains(&index))?
            .mark
            .link
            .clone()
    }

    pub(super) fn render(
        self: Rc<Self>,
        controls: Rc<ReplyControls>,
        palette: Palette,
        window: &mut Window,
    ) -> gpui::AnyElement {
        let style = window.text_style();
        let view_id = window.current_view();
        let selection = controls.selection();
        let mut layouts = Vec::with_capacity(self.paragraphs.len());
        let mut body = div()
            .id("formatted-reply")
            .debug_selector(|| "formatted-reply".into())
            .track_focus(&controls.focus)
            .tab_index(1)
            .tab_stop(false)
            .relative()
            .flex()
            .flex_col()
            .gap(px(9.75))
            .text_size(px(13.))
            .line_height(px(21.45));
        for paragraph in &self.paragraphs {
            let runs = paragraph
                .spans
                .iter()
                .flat_map(|span| text_runs(span, &style, &selection, palette))
                .collect();
            let text =
                StyledText::new(self.text[paragraph.range.clone()].to_owned()).with_runs(runs);
            layouts.push((paragraph.range.clone(), text.layout().clone()));
            body = body.child(text);
        }
        let layouts = Rc::new(layouts);
        let press = controls.clone();
        let move_controls = controls.clone();
        let move_layouts = layouts.clone();
        let release = controls.clone();
        let release_layouts = layouts.clone();
        let release_text = self.clone();
        let copy = controls.clone();
        body.on_mouse_down(MouseButton::Left, move |event, window, app| {
            let start = reply_index(&layouts, event.position);
            *press.drag.borrow_mut() = Some(Drag {
                origin: start,
                index: start,
                position: event.position,
                link_start: link_index(&layouts, event.position),
                moved: false,
            });
            window.focus(&press.focus);
            app.notify(view_id);
        })
        .on_mouse_move(move |event, _, app| {
            if event.dragging() && move_controls.update_drag(&move_layouts, event.position) {
                app.notify(view_id);
            }
        })
        .on_mouse_up(MouseButton::Left, move |event, _, app| {
            if let Some(drag) = release.drag.borrow().as_ref()
                && !drag.moved
                && let Some(pressed) = drag
                    .link_start
                    .and_then(|index| release_text.link_at(index))
                && let Some(link_index) = link_index(&release_layouts, event.position)
                && release_text.link_at(link_index).as_ref() == Some(&pressed)
            {
                app.open_url(&pressed);
                app.stop_propagation();
            }
            release.finish_drag();
            app.notify(view_id);
        })
        .on_key_down(move |event: &KeyDownEvent, window, app| {
            let modifiers = event.keystroke.modifiers;
            if event.keystroke.key == "c"
                && (modifiers.control || modifiers.platform)
                && !(modifiers.alt || modifiers.shift)
                && let Some(selection) = copy.selection()
            {
                if let Some(text) = self.text.get(selection) {
                    app.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
                    window.prevent_default();
                    app.stop_propagation();
                }
            }
        })
        .when(controls.drag.borrow().is_some(), |body| {
            let outside = controls.clone();
            let release = controls.clone();
            body.child(
                canvas(
                    |_, _, _| (),
                    move |bounds, (), window, _| {
                        let outside = outside.clone();
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, app| {
                            if phase == DispatchPhase::Bubble
                                && event.dragging()
                                && !bounds.contains(&event.position)
                                && outside.update_drag(&move_layouts, event.position)
                            {
                                app.notify(view_id);
                            }
                        });
                        let release = release.clone();
                        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, app| {
                            if phase == DispatchPhase::Bubble
                                && event.button == MouseButton::Left
                                && !bounds.contains(&event.position)
                            {
                                release.finish_drag();
                                app.notify(view_id);
                            }
                        });
                    },
                )
                .absolute()
                .size_full(),
            )
        })
        .into_any_element()
    }
}

fn text_runs(
    span: &Span,
    style: &TextStyle,
    selection: &Option<Range<usize>>,
    palette: Palette,
) -> Vec<TextRun> {
    let mut run = style.to_run(span.range.len());
    run.color = rgb(palette.text).into();
    if span.mark.bold {
        run.font.weight = FontWeight::BOLD;
    }
    if span.mark.italic {
        run.font.style = FontStyle::Italic;
    }
    if span.mark.code {
        run.font.family = monospace_font().into();
    }
    if span.mark.strike {
        run.strikethrough = Some(StrikethroughStyle {
            thickness: px(1.),
            ..Default::default()
        });
    }
    if span.mark.link.is_some() {
        run.color = rgb(palette.focus).into();
        run.underline = Some(UnderlineStyle {
            thickness: px(1.),
            ..Default::default()
        });
    }
    let Some(selection) = selection else {
        return vec![run];
    };
    let start = span.range.start.max(selection.start);
    let end = span.range.end.min(selection.end);
    if start >= end {
        return vec![run];
    }
    let mut selected = run.clone();
    selected.len = end - start;
    selected.background_color = Some(
        rgb(if palette.canvas == 0x0a0a0a {
            0x253c69
        } else {
            0xdce7fc
        })
        .into(),
    );
    let mut parts = Vec::with_capacity(3);
    if start > span.range.start {
        let mut before = run.clone();
        before.len = start - span.range.start;
        parts.push(before);
    }
    parts.push(selected);
    if end < span.range.end {
        run.len = span.range.end - end;
        parts.push(run);
    }
    parts
}

pub(super) struct ReplyControls {
    focus: FocusHandle,
    drag: RefCell<Option<Drag>>,
    selection: RefCell<Option<Range<usize>>>,
}

struct Drag {
    origin: usize,
    index: usize,
    position: Point<Pixels>,
    link_start: Option<usize>,
    moved: bool,
}

impl ReplyControls {
    pub(super) fn new(app: &App) -> Self {
        Self {
            focus: app.focus_handle(),
            drag: RefCell::new(None),
            selection: RefCell::new(None),
        }
    }

    fn selection(&self) -> Option<Range<usize>> {
        if let Some(drag) = self.drag.borrow().as_ref() {
            let (start, end) = (drag.origin.min(drag.index), drag.origin.max(drag.index));
            (start < end).then_some(start..end)
        } else {
            self.selection.borrow().clone()
        }
    }

    fn update_drag(&self, layouts: &[(Range<usize>, TextLayout)], position: Point<Pixels>) -> bool {
        let mut drag = self.drag.borrow_mut();
        let Some(drag) = drag.as_mut() else {
            return false;
        };
        let index = reply_index(layouts, position);
        let changed = drag.index != index;
        drag.moved |= changed
            || (drag.position.x - position.x).abs() > px(3.)
            || (drag.position.y - position.y).abs() > px(3.);
        drag.index = index;
        changed
    }

    fn finish_drag(&self) {
        if self.drag.borrow().is_some() {
            let selection = self.selection();
            *self.selection.borrow_mut() = selection;
            self.drag.borrow_mut().take();
        }
    }
}

fn reply_index(layouts: &[(Range<usize>, TextLayout)], point: Point<Pixels>) -> usize {
    for (range, layout) in layouts {
        let bounds = layout.bounds();
        if point.y < bounds.top() {
            return range.start;
        }
        if point.y <= bounds.bottom() {
            let index = layout
                .index_for_position(point)
                .unwrap_or_else(|index| index);
            return range.start + index.min(range.len());
        }
    }
    layouts.last().map_or(0, |(range, _)| range.end)
}

fn link_index(layouts: &[(Range<usize>, TextLayout)], point: Point<Pixels>) -> Option<usize> {
    let (range, layout) = layouts
        .iter()
        .find(|(_, layout)| layout.bounds().contains(&point))?;
    layout
        .index_for_position(point)
        .ok()
        .map(|index| range.start + index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_nested_weights_monospace_links_and_paragraph_breaks() {
        let reply = ReplyText::from_markdown("See **[an *italic* `\\section`][paper]**.\n\nNext  paragraph.\n\n[paper]: https://example.org/paper").unwrap();
        assert_eq!(
            reply.text.as_ref(),
            "See an italic \\section.\n\nNext paragraph."
        );
        let spans = &reply.paragraphs[0].spans;
        let italic = spans
            .iter()
            .find(|span| &reply.text[span.range.clone()] == "italic")
            .unwrap();
        assert!(italic.mark.bold && italic.mark.italic && italic.mark.link.is_some());
        let code = spans.iter().find(|span| span.mark.code).unwrap();
        assert_eq!(&reply.text[code.range.clone()], "\\section");
        assert!(code.mark.bold && code.mark.link.is_some());
        assert_eq!(
            reply.link_at(code.range.start).unwrap().as_ref(),
            "https://example.org/paper"
        );
        assert!(reply.link_at(0).is_none());
    }

    #[cfg(feature = "ui-tests")]
    #[gpui::test]
    fn copies_a_selection_across_two_formatted_paragraphs(cx: &mut gpui::TestAppContext) {
        use gpui::{Context, Modifiers, Render, point};

        struct Fixture(Rc<ReplyText>, Rc<ReplyControls>);
        impl Render for Fixture {
            fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().size_full().p(px(10.)).child(self.0.clone().render(
                    self.1.clone(),
                    Palette::for_window(window),
                    window,
                ))
            }
        }

        cx.update(crate::initialize_app);
        let (_, cx) = cx.add_window_view(|_, cx| {
            Fixture(
                Rc::new(
                    ReplyText::from_markdown("**Strong** `code`\n\nAnd *italic* text.").unwrap(),
                ),
                Rc::new(ReplyControls::new(cx)),
            )
        });
        cx.refresh().unwrap();
        let bounds = cx.debug_bounds("formatted-reply").unwrap();
        let start = bounds.origin + point(px(0.5), px(8.));
        let end = point(bounds.right() - px(1.), bounds.bottom() - px(4.));
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(end, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::none());
        cx.simulate_keystrokes(if cfg!(target_os = "macos") {
            "cmd-c"
        } else {
            "ctrl-c"
        });
        cx.update(|_, app| {
            assert_eq!(
                app.read_from_clipboard()
                    .and_then(|item| item.text())
                    .as_deref(),
                Some("Strong code\n\nAnd italic text.")
            );
        });
    }
}

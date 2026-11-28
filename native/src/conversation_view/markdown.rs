use crate::{Palette, monospace_font};
use ::markdown::{ParseOptions, mdast::Node};
use gpui::{
    App, IntoElement, RenderOnce, SharedString, Styled, Window, WindowAppearance, div, px, rems,
    rgb,
};
use gpui_component::{
    ActiveTheme,
    text::{TextView, TextViewStyle},
};
use std::{borrow::Cow, collections::HashMap, ops::Range};
use url::Url;

pub(super) fn assistant_reply(index: usize, source: &str, palette: Palette) -> impl IntoElement {
    AssistantReply {
        index,
        source: source.to_owned().into(),
        palette,
    }
}

#[derive(IntoElement)]
struct AssistantReply {
    index: usize,
    source: SharedString,
    palette: Palette,
}

struct ReplyState {
    source: SharedString,
    markup: SharedString,
}

impl ReplyState {
    fn new(source: SharedString) -> Self {
        let markup = match safe_markdown(&source) {
            Some(Cow::Borrowed(_)) => source.clone(),
            Some(Cow::Owned(safe)) => safe.into(),
            None => literal_markdown(&source).into(),
        };
        Self { source, markup }
    }
}

impl RenderOnce for AssistantReply {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(("assistant-format", self.index), cx, |_, _| {
            ReplyState::new(self.source.clone())
        });
        if state.read(cx).source != self.source {
            state.update(cx, |state, _| *state = ReplyState::new(self.source));
        }
        let markup = state.read(cx).markup.clone();
        let view = TextView::markdown(("assistant-markdown", self.index), markup, window, cx);
        let palette = self.palette;
        let mut code = div()
            .p(px(12.))
            .rounded(px(6.))
            .border_1()
            .border_color(rgb(palette.border))
            .bg(rgb(palette.canvas))
            .font_family(monospace_font())
            .text_size(px(13.));
        let style = TextViewStyle {
            paragraph_gap: rems(px(9.75) / window.rem_size()),
            heading_base_font_size: px(13.),
            highlight_theme: cx.theme().highlight_theme.clone(),
            code_block: Styled::style(&mut code).clone(),
            is_dark: matches!(
                window.appearance(),
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ),
            ..TextViewStyle::default()
        };
        view.style(style)
            .selectable(true)
            .text_size(px(13.))
            .line_height(px(21.45))
    }
}

fn safe_markdown(source: &str) -> Option<Cow<'_, str>> {
    // TextView uses GFM with the same parser. Inspect its syntax tree instead of
    // searching raw text, where links and HTML may appear in literal code spans.
    let options = ParseOptions::gfm();
    let root = ::markdown::to_mdast(source, &options).ok()?;
    let mut definitions = HashMap::new();
    for node in nodes(&root) {
        if let Node::Definition(definition) = node {
            definitions
                .entry(definition.identifier.as_str())
                .or_insert_with(|| safe_link(&definition.url));
        }
    }

    let mut removals = Vec::new();
    for node in nodes(&root) {
        match node {
            Node::Link(link) if !safe_link(&link.url) => {
                remove_link_destination(node, &mut removals)?;
            }
            Node::LinkReference(link)
                if definitions.get(link.identifier.as_str()) == Some(&false) =>
            {
                remove_link_destination(node, &mut removals)?;
            }
            node if !safe_node(node) => removals.push(span(node)?),
            _ => {}
        }
    }
    if removals.is_empty() {
        return Some(Cow::Borrowed(source));
    }

    removals.sort_unstable_by_key(|range| (range.start, range.end));
    let mut cleaned = String::with_capacity(source.len());
    let mut offset = 0;
    for range in removals {
        if range.end <= offset {
            continue;
        }
        if source.get(range.clone()).is_none() {
            return None;
        }
        if range.start >= offset {
            cleaned.push_str(source.get(offset..range.start)?);
        }
        offset = range.end;
    }
    cleaned.push_str(source.get(offset..)?);

    // Removals can join tokens into a new image or link. Reparse the exact text
    // passed to the renderer and fall back to literal text if it is still unsafe.
    let verified = ::markdown::to_mdast(&cleaned, &options).ok()?;
    nodes(&verified)
        .all(safe_node)
        .then_some(Cow::Owned(cleaned))
}

fn safe_node(node: &Node) -> bool {
    match node {
        Node::Image(_) | Node::ImageReference(_) => false,
        Node::Html(html) => ["<br>", "<br/>", "<br />"]
            .into_iter()
            .any(|tag| html.value.trim().eq_ignore_ascii_case(tag)),
        Node::Link(link) => safe_link(&link.url),
        Node::Definition(definition) => safe_link(&definition.url),
        _ => true,
    }
}

fn safe_link(source: &str) -> bool {
    let Some((scheme, rest)) = source.split_once("://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let is_web = scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http");
    is_web
        && !authority.is_empty()
        && !authority.contains('@')
        && !source.contains('\\')
        && !source
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace())
        && Url::parse(source).is_ok_and(|url| {
            url.has_host() && url.username().is_empty() && url.password().is_none()
        })
}

fn nodes(root: &Node) -> impl Iterator<Item = &Node> {
    let mut pending = vec![root];
    std::iter::from_fn(move || {
        let node = pending.pop()?;
        if let Some(children) = node.children() {
            pending.extend(children.iter().rev());
        }
        Some(node)
    })
}

fn span(node: &Node) -> Option<Range<usize>> {
    let location = node.position()?;
    Some(location.start.offset..location.end.offset)
}

fn remove_link_destination(node: &Node, removals: &mut Vec<Range<usize>>) -> Option<()> {
    let link = span(node)?;
    let children = node.children()?;
    let (Some(first), Some(last)) = (children.first(), children.last()) else {
        removals.push(link);
        return Some(());
    };
    let label_start = span(first)?.start;
    let label_end = span(last)?.end;
    if !(link.start <= label_start && label_start <= label_end && label_end <= link.end) {
        return None;
    }
    removals.push(link.start..label_start);
    removals.push(label_end..link.end);
    Some(())
}

fn literal_markdown(source: &str) -> String {
    // A fence longer than any in the source keeps every character literal,
    // including strings that the HTML parser or an autolink could reinterpret.
    let longest = source
        .as_bytes()
        .split(|&byte| byte != b'`')
        .map(<[u8]>::len)
        .max()
        .unwrap_or_default();
    let fence = "`".repeat((longest + 1).max(3));
    format!("{fence}\n{source}\n{fence}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_safe_markdown_including_links_and_literal_code() {
        let source = "# Reply\n\n1. **Bold** and *italic*\n2. [Guide][guide]\n\n\
                      | Column | Value |\n| ------ | ----- |\n| Code | `<img>` |\n\n\
                      ```text\n[example](file:///tmp/demo)\n```\n\n\
                      <https://example.org/paper>\n\n[guide]: https://example.org/docs";
        assert_eq!(safe_markdown(source), Some(Cow::Borrowed(source)));
    }

    #[test]
    fn removes_images_and_unsafe_targets_without_erasing_link_labels() {
        let source = "Résumé [**saved file**](file:///notes.tex) and [paper][paper].\n\n\
                      <img src=\"file:///local.png\">\n\n\
                      ![remote](https://example.org/remote.png) ![reference][image]\n\n\
                      [email][mail] [account](https://name:pass@example.org)\n\n\
                      [paper]: https://example.org/paper\n\
                      [image]: https://example.org/image.png\n\
                      [mail]: mailto:someone@example.org";
        let safe = safe_markdown(source).expect("safe Markdown").into_owned();
        assert!(safe.contains("Résumé **saved file** and [paper][paper]."));
        assert!(safe.contains("email account"));
        assert!(!safe.contains("<img"));
        assert!(!safe.contains("!["));
        assert!(!safe.contains("file:"));
        assert!(!safe.contains("mailto:"));
        assert!(!safe.contains("name:pass"));
    }

    #[test]
    fn refuses_new_images_formed_by_rejoining_text() {
        let source = "![label]<!-- comment -->(https://example.org/image.png)";
        assert!(safe_markdown(source).is_none());
        let literal = format!("{source}\n```\n<img src=\"file:///notes.tex\">");
        let markup = literal_markdown(&literal);
        let root = ::markdown::to_mdast(&markup, &ParseOptions::gfm()).unwrap();
        assert!(matches!(
            root.children().unwrap().as_slice(),
            [Node::Code(code)] if code.value == literal
        ));
    }

    #[test]
    fn validates_only_absolute_web_urls_without_credentials() {
        assert!(safe_link("HTTP://example.org"));
        assert!(safe_link("https://[::1]:8443/?q=a@b"));
        assert!(safe_link("https://例え.jp/資料"));
        for target in [
            "https://user@example.org",
            "https://@example.org",
            "https:///example.org",
            "https://example.org\\resource",
            "https://example.org\n/file",
            "//example.org",
            "relative/path",
            "mailto:test@example.org",
            "file:///etc/passwd",
            "javascript:alert(1)",
        ] {
            assert!(!safe_link(target), "{target}");
        }
    }
}

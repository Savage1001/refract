use regex::Regex;
use std::sync::LazyLock;

static HEADING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\\(?:sub)*section\*?\{").expect("valid heading pattern"));

#[derive(Debug, Eq, PartialEq)]
pub struct Section<'a> {
    pub title: &'a str,
    pub offset: usize,
}

pub fn sections(source: &str) -> Vec<Section<'_>> {
    let mut candidates = HEADING.find_iter(source).peekable();
    let mut openings = Vec::new();
    let mut headings = Vec::new();
    let mut depth = 0usize;
    let mut chars = source.char_indices();
    while let Some((position, ch)) = chars.next() {
        match ch {
            '\\' => {
                chars.next();
            }
            '{' => {
                depth += 1;
                if candidates
                    .peek()
                    .is_some_and(|matched| matched.end() == position + 1)
                {
                    let matched = candidates.next().expect("next heading");
                    openings.push((depth, matched));
                }
            }
            '}' => {
                if openings
                    .last()
                    .is_some_and(|(open_depth, _)| *open_depth == depth)
                {
                    let (_, matched) = openings.pop().expect("open heading");
                    headings.push((matched.start(), matched.end(), position));
                }
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
    }
    // Closed outer titles consume their nested commands. A malformed outer title
    // still lets a later closed heading appear, as in the Electron outline.
    headings.sort_unstable();
    let mut previous_end = 0;
    headings
        .into_iter()
        .filter_map(|(offset, start, end)| {
            if offset < previous_end {
                return None;
            }
            previous_end = end + 1;
            (start < end).then(|| Section {
                title: &source[start..end],
                offset,
            })
        })
        .collect()
}

#[derive(Debug, Eq, PartialEq)]
pub enum TitlePart<'a> {
    Text(&'a str),
    Math { formula: &'a str, source: &'a str },
}

pub fn title_parts(title: &str) -> Vec<TitlePart<'_>> {
    let mut parts = Vec::new();
    let mut text_start = 0;
    let mut index = 0;
    while let Some(ch) = title[index..].chars().next() {
        let rest = &title[index..];
        let delimiter = if rest.starts_with(r"\(") {
            Some((2, r"\)"))
        } else if rest.starts_with("$$") {
            index += 2;
            continue;
        } else if ch == '$' {
            Some((1, "$"))
        } else {
            None
        };
        if let Some((width, closing)) = delimiter
            && let Some(end) = math_end(&rest[width..], closing)
            && end != 0
        {
            if text_start < index {
                parts.push(TitlePart::Text(&title[text_start..index]));
            }
            let total = width + end + closing.len();
            parts.push(TitlePart::Math {
                formula: &rest[width..width + end],
                source: &rest[..total],
            });
            index += total;
            text_start = index;
        } else {
            index += ch.len_utf8();
            if ch == '\\' {
                index += title[index..].chars().next().map_or(0, char::len_utf8);
            }
        }
    }
    if text_start < title.len() {
        parts.push(TitlePart::Text(&title[text_start..]));
    }
    parts
}

fn math_end(source: &str, closing: &str) -> Option<usize> {
    let mut chars = source.char_indices();
    while let Some((index, ch)) = chars.next() {
        if ch == '\n' {
            break;
        }
        if source[index..].starts_with(closing) {
            return Some(index);
        }
        if ch == '\\' && chars.next().is_some_and(|(_, ch)| ch == '\n') {
            break;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_balanced_titles_and_original_byte_offsets() {
        let source = concat!(
            "🙂 \\section{First {part} \\} \\section{nested}}\r\n",
            "\\subsection*{Second}\n\\section{}\n",
            "\\section{Unterminated \\subsubsection{Recovered}",
        );
        let rows = sections(source);
        assert_eq!(
            rows.iter().map(|row| row.title).collect::<Vec<_>>(),
            [r"First {part} \} \section{nested}", "Second", "Recovered"]
        );
        assert_eq!(&source[rows[0].offset..][..8], r"\section");
        assert_eq!(rows[0].offset, "🙂 ".len());
        assert_eq!(rows[2].offset, source.find(r"\subsubsection").unwrap());
    }

    #[test]
    fn identifies_both_inline_math_delimiters_and_preserves_escaped_currency() {
        assert_eq!(
            title_parts(r"Cost \$5, $J_{2}$ Truth, \(\Delta t=1800\) s"),
            [
                TitlePart::Text(r"Cost \$5, "),
                TitlePart::Math {
                    formula: "J_{2}",
                    source: "$J_{2}$"
                },
                TitlePart::Text(" Truth, "),
                TitlePart::Math {
                    formula: r"\Delta t=1800",
                    source: r"\(\Delta t=1800\)"
                },
                TitlePart::Text(" s"),
            ]
        );
        assert_eq!(
            title_parts("$$x$$ and $incomplete"),
            [TitlePart::Text("$$x$$ and $incomplete")]
        );
    }
}

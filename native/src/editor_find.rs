use regex::{Regex, RegexBuilder};
use std::{ops::Range, sync::LazyLock};

static WORD_CHARACTER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\p{L}\p{M}\p{N}\p{Pc}]").expect("valid Unicode word classes"));

pub struct FindQuery {
    expression: Regex,
    whole_word: bool,
}

pub enum FindDirection {
    AtOrAfter(usize),
    Before(usize),
    Selection(Range<usize>),
}

pub struct FindResults {
    pub count: usize,
    pub current: Option<FindMatch>,
}

pub struct FindMatch {
    pub index: usize,
    pub bytes: Range<usize>,
}

impl FindQuery {
    pub fn new(query: &str, match_case: bool, whole_word: bool) -> Option<Self> {
        if query.is_empty() {
            return None;
        }
        let expression = RegexBuilder::new(&regex::escape(query))
            .case_insensitive(!match_case)
            .build()
            .ok()?;
        Some(Self {
            expression,
            whole_word,
        })
    }

    fn matches<'a>(&'a self, text: &'a str) -> impl Iterator<Item = Range<usize>> + 'a {
        self.expression.find_iter(text).filter_map(|found| {
            let range = found.range();
            let is_word = |ch: char| WORD_CHARACTER.is_match(ch.encode_utf8(&mut [0; 4]));
            (!self.whole_word
                || (!text[..range.start].chars().next_back().is_some_and(is_word)
                    && !text[range.end..].chars().next().is_some_and(is_word)))
            .then_some(range)
        })
    }

    pub fn find(&self, text: &str, direction: FindDirection, wrap: bool) -> FindResults {
        let mut count = 0;
        let mut first = None;
        let mut last = None;
        let mut current = None;
        for bytes in self.matches(text) {
            count += 1;
            let use_match = match &direction {
                FindDirection::AtOrAfter(offset) => current.is_none() && bytes.start >= *offset,
                FindDirection::Before(offset) => bytes.start < *offset,
                FindDirection::Selection(selection) => bytes == *selection,
            };
            if use_match {
                current = Some(FindMatch {
                    index: count,
                    bytes: bytes.clone(),
                });
            }
            if wrap && count == 1 {
                first = Some(FindMatch {
                    index: count,
                    bytes: bytes.clone(),
                });
            }
            last = Some(bytes);
        }
        if wrap && current.is_none() {
            current = match direction {
                FindDirection::AtOrAfter(_) => first,
                FindDirection::Before(_) => last.map(|bytes| FindMatch {
                    index: count,
                    bytes,
                }),
                FindDirection::Selection(_) => None,
            };
        }
        FindResults { count, current }
    }

    pub fn replace_all(&self, text: &str, replacement: &str) -> Option<String> {
        let mut matches = self.matches(text).peekable();
        matches.peek()?;
        let mut result = String::with_capacity(text.len());
        let mut previous = 0;
        for range in matches {
            result.push_str(&text[previous..range.start]);
            result.push_str(replacement);
            previous = range.end;
        }
        result.push_str(&text[previous..]);
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_case_folded_whole_words_without_changing_offsets() {
        let query = FindQuery::new("cat", false, true).unwrap();
        let text = "🙂 İ CAT scatter cat\u{301} cat_ catβ cat 😺";
        let first = query.find(text, FindDirection::AtOrAfter(0), false);
        assert_eq!(first.count, 2);
        let first = first.current.unwrap();
        assert_eq!(first.index, 1);
        assert_eq!(first.bytes.start, "🙂 İ ".len());
        let last = query
            .find(text, FindDirection::Before(first.bytes.start), true)
            .current
            .unwrap();
        assert_eq!(last.index, 2);
        assert_eq!(&text[last.bytes.clone()], "cat");
        assert!(
            query
                .find(text, FindDirection::AtOrAfter(last.bytes.end), false)
                .current
                .is_none()
        );
        assert_eq!(
            query
                .find(text, FindDirection::Selection(last.bytes), false)
                .current
                .unwrap()
                .index,
            2
        );
    }

    #[test]
    fn treats_metacharacters_in_both_fields_as_text() {
        let query = FindQuery::new("a.b", true, false).unwrap();
        assert_eq!(
            query.replace_all("a.b axb A.b a.b", r"$1\"),
            Some(r"$1\ axb A.b $1\".into())
        );
        assert!(query.replace_all("axb", "x").is_none());
    }
}

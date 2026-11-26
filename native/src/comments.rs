use crate::{Project, project_directory, read_local_file};
use serde::{Deserialize, Deserializer, de::Error};
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};
use std::{io, ops::Range, path::Path};

const COMMENTS_LIMIT: usize = 4 * 1024 * 1024;
const MAX_MATCHES: usize = 10_000;

#[derive(Clone, Deserialize)]
pub struct Comment {
    pub id: String,
    pub file: String,
    #[serde(default)]
    pub quote: String,
    pub text: String,
    #[serde(default)]
    pub resolved: bool,
    #[serde(default)]
    pub replies: Vec<Reply>,
    anchor: Option<Anchor>,
}

#[derive(Clone, Deserialize)]
pub struct Reply {
    pub text: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Anchor {
    start: usize,
    end: usize,
    #[serde(deserialize_with = "decode_context")]
    prefix: Vec<u16>,
    #[serde(deserialize_with = "decode_context")]
    suffix: Vec<u16>,
    source_hash: String,
    status: String,
}

pub fn read_comments(projects_dir: &Path, project: &Project) -> io::Result<Vec<Comment>> {
    let folder = project_directory(projects_dir, project)?;
    let path = folder.join(".refract-comments.json");
    let metadata = match path.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid comments file",
        ));
    }
    let bytes = read_local_file(&folder, &path, COMMENTS_LIMIT)?;
    Ok(serde_json::from_slice(&bytes)?)
}

impl Comment {
    pub fn source_position(&self, source: &str) -> Option<(u32, u32)> {
        let text = source.replace("\r\n", "\n").replace('\r', "\n");
        let units: Vec<u16> = text.encode_utf16().collect();
        let start = self.locate(&text, &units)?.start;
        // GPUI positions count Unicode scalar values within LF-delimited lines.
        let mut source = source.chars().peekable();
        let (mut offset, mut line, mut column) = (0, 0u32, 0u32);
        while let Some(mut ch) = source.next() {
            if offset == start {
                return Some((line, column));
            }
            if ch == '\r' && source.peek() == Some(&'\n') {
                source.next();
                ch = '\n';
            }
            offset += ch.len_utf16();
            if ch == '\n' {
                line += 1;
                column = 0;
            } else {
                column += 1;
            }
        }
        (offset == start).then_some((line, column))
    }

    fn locate(&self, text: &str, units: &[u16]) -> Option<Range<usize>> {
        if self.quote.is_empty()
            || self
                .anchor
                .as_ref()
                .is_some_and(|anchor| anchor.status == "orphaned")
        {
            return None;
        }
        let quote: Vec<u16> = self.quote.encode_utf16().collect();
        if let Some(anchor) = &self.anchor
            && units.get(anchor.start..anchor.end) == Some(quote.as_slice())
            && format!("{:x}", Sha256::digest(text.as_bytes())) == anchor.source_hash
        {
            return Some(anchor.start..anchor.end);
        }

        let mut found = 0;
        let mut best = None;
        let mut bytes = 0;
        let mut offset = 0;
        while let Some(relative) = text[bytes..].find(&self.quote) {
            let at = bytes + relative;
            offset += text[bytes..at].encode_utf16().count();
            let range = offset..offset + quote.len();
            found += 1;
            if found > MAX_MATCHES {
                return None;
            }
            let first = text[at..].chars().next()?;
            bytes = at + first.len_utf8();
            offset += first.len_utf16();
            let Some(anchor) = &self.anchor else {
                if found > 1 {
                    return None;
                }
                best = Some((range, 0, false));
                continue;
            };
            let prefix = units[..range.start]
                .iter()
                .rev()
                .zip(anchor.prefix.iter().rev())
                .take_while(|(left, right)| left == right)
                .count();
            let suffix = units[range.end..]
                .iter()
                .zip(&anchor.suffix)
                .take_while(|(left, right)| left == right)
                .count();
            let score = prefix + suffix;
            match &mut best {
                Some((_, best_score, tied)) if score == *best_score => *tied = true,
                Some((_, best_score, _)) if score < *best_score => {}
                _ => best = Some((range, score, false)),
            }
        }
        let (range, score, tied) = best?;
        match &self.anchor {
            None if !tied => Some(range),
            Some(anchor) if !tied => {
                let minimum = 8.min(anchor.prefix.len() + anchor.suffix.len());
                (minimum > 0 && score >= minimum).then_some(range)
            }
            _ => None,
        }
    }
}

// Electron stores substring context in UTF-16. A context boundary can bisect a surrogate pair.
fn decode_context<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u16>, D::Error> {
    let raw = Box::<RawValue>::deserialize(deserializer)?;
    let text = raw
        .get()
        .strip_prefix('"')
        .and_then(|text| text.strip_suffix('"'))
        .ok_or_else(|| D::Error::custom("Expected comment anchor context"))?;
    let mut result = Vec::new();
    let mut remainder = text;
    while let Some((chunk, escaped)) = remainder.split_once('\\') {
        result.extend(chunk.encode_utf16());
        let Some(first) = escaped.as_bytes().first() else {
            return Err(D::Error::custom("Incomplete comment anchor escape"));
        };
        if *first == b'u' {
            let value = escaped
                .get(1..5)
                .and_then(|value| u16::from_str_radix(value, 16).ok())
                .ok_or_else(|| D::Error::custom("Invalid comment anchor Unicode escape"))?;
            result.push(value);
            remainder = &escaped[5..];
        } else {
            result.push(match *first {
                b'"' | b'\\' | b'/' => u16::from(*first),
                b'b' => 8,
                b'f' => 12,
                b'n' => 10,
                b'r' => 13,
                b't' => 9,
                _ => return Err(D::Error::custom("Invalid comment anchor escape")),
            });
            remainder = &escaped[1..];
        }
    }
    result.extend(remainder.encode_utf16());
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resolves_legacy_quotes_with_native_columns_and_normalized_newlines() {
        let comment: Comment = serde_json::from_value(json!({
            "id": "one", "file": "main.tex", "quote": "item", "text": "Clarify this",
        }))
        .unwrap();
        assert_eq!(comment.source_position("First\r\n😀item"), Some((1, 1)));
        assert_eq!(comment.source_position("First\r😀item"), Some((0, 7)));
        assert_eq!(comment.source_position("item item"), None);
    }

    #[test]
    fn uses_exact_anchors_and_relocates_only_unambiguous_matches() {
        let text = "Start context: item and rest";
        let comment: Comment = serde_json::from_value(json!({
            "id": "one", "file": "main.tex", "quote": "item", "text": "Clarify this",
            "anchor": {
                "start": 15, "end": 19, "prefix": "Start context: ", "suffix": " and rest",
                "sourceHash": format!("{:x}", Sha256::digest(text.as_bytes())), "status": "attached",
            }
        })).unwrap();
        assert_eq!(comment.source_position(text), Some((0, 15)));
        assert_eq!(
            comment.source_position(&format!("An item elsewhere\n{text}")),
            Some((1, 15))
        );
        assert_eq!(comment.source_position(&format!("{text}\n{text}")), None);
    }

    #[test]
    fn loads_anchor_context_containing_an_unpaired_surrogate() {
        let comment: Comment = serde_json::from_str(r#"{"id":"one","file":"main.tex","quote":"item","text":"Clarify this","anchor":{"start":0,"end":0,"prefix":"\ude00 comments: ","suffix":"\n","sourceHash":"","status":"attached"}}"#).unwrap();
        assert_eq!(comment.anchor.as_ref().unwrap().prefix[0], 0xde00);
        assert_eq!(
            comment.source_position("😀 comments: item\n"),
            Some((0, 12))
        );
    }
}

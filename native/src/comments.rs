use crate::{Project, project_directory, read_local_file};
use serde::{Deserialize, Deserializer, Serialize, de::Error};
use serde_json::value::{RawValue, to_raw_value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
    io,
    ops::Range,
    path::Path,
};

mod update;
pub use update::{CommentSelection, CommentUpdate, update_comments};

const COMMENTS_LIMIT: usize = 4 * 1024 * 1024;
const MAX_MATCHES: usize = 10_000;

#[derive(Clone, Deserialize)]
pub struct Comment {
    pub id: String,
    pub file: String,
    #[serde(default)]
    pub quote: String,
    pub text: String,
    pub created: Option<String>,
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
    match read_comments_file(&folder)? {
        Some(bytes) => Ok(serde_json::from_slice(&bytes)?),
        None => Ok(Vec::new()),
    }
}

pub(crate) fn read_comments_file(folder: &Path) -> io::Result<Option<Vec<u8>>> {
    let path = folder.join(".refract-comments.json");
    let metadata = match path.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid comments file",
        ));
    }
    read_local_file(folder, &path, COMMENTS_LIMIT).map(Some)
}

impl Comment {
    pub fn selection_unavailable(&self) -> bool {
        self.anchor
            .as_ref()
            .is_some_and(|anchor| anchor.status == "orphaned")
    }

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

pub(crate) fn plan_edit(
    metadata: &[u8],
    file: &str,
    before: Option<&str>,
    after: Option<&str>,
) -> io::Result<Option<Vec<u8>>> {
    if before == after {
        return Ok(None);
    }
    let (Some(before), Some(after)) = (before, after) else {
        return update_anchors(metadata, file, |_, _| Ok(None));
    };
    let before = before.replace("\r\n", "\n").replace('\r', "\n");
    let after = after.replace("\r\n", "\n").replace('\r', "\n");
    if before == after {
        return Ok(None);
    }
    let before_units: Vec<u16> = before.encode_utf16().collect();
    let after_units: Vec<u16> = after.encode_utf16().collect();
    let prefix_len = before_units
        .iter()
        .zip(&after_units)
        .take_while(|(a, b)| a == b)
        .count();
    let suffix_len = before_units[prefix_len..]
        .iter()
        .rev()
        .zip(after_units[prefix_len..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let old_end = before_units.len() - suffix_len;
    let delta = after_units.len() as isize - before_units.len() as isize;
    let hash = format!("{:x}", Sha256::digest(after.as_bytes()));

    update_anchors(metadata, file, |comment, anchor| {
        let selection = comment
            .locate(&before, &before_units)
            .and_then(|selection| {
                if selection.end <= prefix_len {
                    Some(selection)
                } else if selection.start >= old_end {
                    Some(
                        selection.start.checked_add_signed(delta)?
                            ..selection.end.checked_add_signed(delta)?,
                    )
                } else if prefix_len >= selection.start && old_end <= selection.end {
                    Some(selection.start..selection.end.checked_add_signed(delta)?)
                } else {
                    relocate_after_separate_edits(comment, anchor, &before, &after, &after_units)
                }
            });

        let updated = selection
            .filter(|range| !range.is_empty())
            .and_then(|range| {
                let quote = String::from_utf16(after_units.get(range.clone())?).ok()?;
                Some((range, quote))
            });
        updated
            .map(|(range, quote)| Ok((make_anchor(&after_units, range, &hash)?, quote)))
            .transpose()
    })
}

pub(crate) fn has_attached_anchors(metadata: &[u8], file: &str) -> io::Result<bool> {
    Ok(attached_files(metadata)?.contains(file))
}

pub(crate) fn remap_files(
    metadata: &[u8],
    mut remap: impl FnMut(&str) -> Option<String>,
) -> io::Result<Option<Vec<u8>>> {
    let mut records: Vec<Box<RawValue>> = serde_json::from_slice(metadata)?;
    let mut changed = false;
    for record in &mut records {
        let mut fields: BTreeMap<String, Box<RawValue>> = serde_json::from_str(record.get())?;
        let next = fields
            .get("file")
            .and_then(|value| serde_json::from_str::<String>(value.get()).ok())
            .and_then(|file| remap(&file));
        if let Some(file) = next {
            fields.insert("file".into(), to_raw_value(&file)?);
            *record = to_raw_value(&fields)?;
            changed = true;
        }
    }
    encode_changed_comments(&records, changed)
}

pub(crate) fn attached_files(metadata: &[u8]) -> io::Result<BTreeSet<String>> {
    let comments: Vec<Comment> = serde_json::from_slice(metadata)?;
    Ok(comments
        .into_iter()
        .filter(|comment| {
            comment
                .anchor
                .as_ref()
                .is_some_and(|anchor| anchor.status != "orphaned")
        })
        .map(|comment| comment.file)
        .collect())
}

fn update_anchors(
    metadata: &[u8],
    file: &str,
    mut relocate: impl FnMut(&Comment, &Anchor) -> serde_json::Result<Option<(Box<RawValue>, String)>>,
) -> io::Result<Option<Vec<u8>>> {
    let mut records: Vec<Box<RawValue>> = serde_json::from_slice(metadata)?;
    let mut changed = false;
    for record in &mut records {
        let comment: Comment = serde_json::from_str(record.get())?;
        let Some(anchor) = &comment.anchor else {
            continue;
        };
        if comment.file != file || anchor.status == "orphaned" {
            continue;
        }
        let mut fields: BTreeMap<String, Box<RawValue>> = serde_json::from_str(record.get())?;
        if let Some((anchor, quote)) = relocate(&comment, anchor)? {
            fields.insert("anchor".into(), anchor);
            fields.insert("quote".into(), to_raw_value(&quote)?);
        } else if let Some(old) = fields.get("anchor") {
            let mut anchor_fields: BTreeMap<String, Box<RawValue>> =
                serde_json::from_str(old.get())?;
            anchor_fields.insert("status".into(), to_raw_value("orphaned")?);
            fields.insert("anchor".into(), to_raw_value(&anchor_fields)?);
        }
        *record = to_raw_value(&fields)?;
        changed = true;
    }

    encode_changed_comments(&records, changed)
}

fn encode_changed_comments(
    records: &[Box<RawValue>],
    changed: bool,
) -> io::Result<Option<Vec<u8>>> {
    if changed {
        let bytes = serde_json::to_vec(records)?;
        if bytes.len() > COMMENTS_LIMIT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Comment data exceeds the save limit",
            ));
        }
        Ok(Some(bytes))
    } else {
        Ok(None)
    }
}

fn relocate_after_separate_edits(
    comment: &Comment,
    anchor: &Anchor,
    before: &str,
    after: &str,
    units: &[u16],
) -> Option<Range<usize>> {
    let before_count = quote_count(before, &comment.quote);
    if before_count == 0
        || before_count > MAX_MATCHES
        || quote_count(after, &comment.quote) != before_count
    {
        return None;
    }
    let range = comment.locate(after, units)?;
    let prefix = &anchor.prefix[anchor.prefix.len().saturating_sub(8)..];
    let suffix = &anchor.suffix[..anchor.suffix.len().min(8)];
    let left = units.get(range.start.saturating_sub(prefix.len())..range.start)?;
    let right = units.get(range.end..range.end + suffix.len())?;
    (left == prefix && right == suffix).then_some(range)
}

fn quote_count(text: &str, quote: &str) -> usize {
    if quote.is_empty() {
        return 0;
    }
    let mut count = 0;
    let mut rest = text;
    while let Some(found) = rest.find(quote) {
        count += 1;
        if count > MAX_MATCHES {
            break;
        }
        let first = rest[found..].chars().next().expect("a non-empty quote");
        rest = &rest[found + first.len_utf8()..];
    }
    count
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SavedAnchor<'a> {
    start: usize,
    end: usize,
    prefix: Box<RawValue>,
    suffix: Box<RawValue>,
    source_hash: &'a str,
    status: &'static str,
}

fn make_anchor(
    units: &[u16],
    range: Range<usize>,
    hash: &str,
) -> serde_json::Result<Box<RawValue>> {
    to_raw_value(&SavedAnchor {
        start: range.start,
        end: range.end,
        prefix: encode_context(&units[range.start.saturating_sub(48)..range.start])?,
        suffix: encode_context(&units[range.end..(range.end + 48).min(units.len())])?,
        source_hash: hash,
        status: "attached",
    })
}

fn encode_context(units: &[u16]) -> serde_json::Result<Box<RawValue>> {
    if let Ok(text) = String::from_utf16(units) {
        return to_raw_value(&text);
    }
    let mut text = String::from('"');
    for unit in units {
        write!(text, "\\u{unit:04x}").expect("writing to a String cannot fail");
    }
    text.push('"');
    RawValue::from_string(text)
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

    #[test]
    fn removing_quoted_text_does_not_move_the_comment_to_a_second_match() {
        let before = "Start context: item and rest\nAnother item";
        let after = "Start context:  and rest\nAnother item";
        let records = serde_json::json!([{
            "id": "one", "file": "main.tex", "quote": "item", "text": "Clarify this",
            "anchor": {
                "start": 15, "end": 19, "prefix": "Start context: ", "suffix": " and rest",
                "sourceHash": "", "status": "attached",
            }
        }]);
        let updated = plan_edit(
            &serde_json::to_vec(&records).unwrap(),
            "main.tex",
            Some(before),
            Some(after),
        )
        .unwrap()
        .unwrap();
        let updated: Vec<Comment> = serde_json::from_slice(&updated).unwrap();
        assert_eq!(updated[0].anchor.as_ref().unwrap().status, "orphaned");
        assert_eq!(updated[0].source_position(after), None);
    }
}

use super::{COMMENTS_LIMIT, Comment, make_anchor, read_comments_file};
use crate::{Project, project_directory, read_source, stage_file, valid_project_component};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::value::{RawValue, to_raw_value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{self, Write},
    ops::Range,
    path::Path,
};
use uuid::Uuid;

const QUOTE_LIMIT: usize = 10_000;

pub struct CommentSelection {
    pub quote: String,
    pub range: Range<usize>,
}

pub enum CommentUpdate {
    Create {
        file: String,
        text: String,
        selection: Option<CommentSelection>,
    },
    Reply {
        id: String,
        text: String,
    },
    SetResolved {
        id: String,
        resolved: bool,
    },
}

pub fn update_comments(
    projects_dir: &Path,
    project: &Project,
    update: CommentUpdate,
) -> io::Result<Vec<Comment>> {
    let folder = project_directory(projects_dir, project)?;
    let original = read_comments_file(&folder)?;
    let mut records: Vec<Box<RawValue>> =
        serde_json::from_slice(original.as_deref().unwrap_or(b"[]"))?;
    match update {
        CommentUpdate::Create {
            file,
            text,
            selection,
        } => {
            records.push(new_comment(projects_dir, project, &file, &text, selection)?);
        }
        CommentUpdate::Reply { id, text } => {
            let text = nonempty_text(&text, "Reply is empty")?;
            edit_record(&mut records, &id, |fields| {
                let mut replies: Vec<Box<RawValue>> = fields
                    .get("replies")
                    .and_then(|raw| serde_json::from_str(raw.get()).ok())
                    .unwrap_or_default();
                replies.push(to_raw_value(&serde_json::json!({
                    "id": Uuid::new_v4().to_string(), "text": text, "created": timestamp(),
                }))?);
                fields.insert("replies".into(), to_raw_value(&replies)?);
                Ok(())
            })?;
        }
        CommentUpdate::SetResolved { id, resolved } => {
            edit_record(&mut records, &id, |fields| {
                fields.insert("resolved".into(), to_raw_value(&resolved)?);
                Ok(())
            })?;
        }
    }
    let bytes = serde_json::to_vec(&records)?;
    if bytes.len() > COMMENTS_LIMIT {
        return Err(invalid_comment("Comment data exceeds the save limit"));
    }
    let comments = serde_json::from_slice(&bytes)?;
    let target = folder.join(".refract-comments.json");
    let staged = if original.is_some() {
        stage_file(&target, &bytes)?
    } else {
        let mut staged = tempfile::Builder::new()
            .prefix(".refract-comments-")
            .tempfile_in(&folder)?;
        staged.write_all(&bytes)?;
        staged.as_file().sync_all()?;
        staged
    };
    if read_comments_file(&folder)? != original {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Comments changed during the update.",
        ));
    }
    let result = if original.is_some() {
        staged.persist(target)
    } else {
        staged.persist_noclobber(target)
    };
    result.map_err(|error| error.error)?;
    Ok(comments)
}

fn new_comment(
    projects_dir: &Path,
    project: &Project,
    file: &str,
    text: &str,
    selection: Option<CommentSelection>,
) -> io::Result<Box<RawValue>> {
    if !file.split(['/', '\\']).all(valid_project_component) {
        return Err(invalid_comment("Invalid project file path"));
    }
    let file = file.replace('\\', "/");
    let text = nonempty_text(text, "Comment is empty")?;
    let (quote, anchor) = match selection {
        Some(selection) => {
            let source = read_source(projects_dir, project, &file)?;
            let source = source.replace("\r\n", "\n").replace('\r', "\n");
            let units: Vec<u16> = source.encode_utf16().collect();
            let quote: Vec<u16> = selection.quote.encode_utf16().collect();
            if quote.is_empty() || quote.len() > QUOTE_LIMIT {
                return Err(invalid_comment("Invalid selection"));
            }
            if units.get(selection.range.clone()) != Some(quote.as_slice()) {
                return Err(invalid_comment(
                    "The selected text has changed. Select it again.",
                ));
            }
            let hash = format!("{:x}", Sha256::digest(source.as_bytes()));
            let anchor = make_anchor(&units, selection.range, &hash)?;
            (selection.quote, Some(anchor))
        }
        None => (String::new(), None),
    };
    Ok(to_raw_value(&NewComment {
        id: Uuid::new_v4().to_string(),
        file,
        quote,
        text,
        created: timestamp(),
        resolved: false,
        anchor,
    })?)
}

type CommentFields = BTreeMap<String, Box<RawValue>>;

fn edit_record(
    records: &mut [Box<RawValue>],
    id: &str,
    edit: impl FnOnce(&mut CommentFields) -> serde_json::Result<()>,
) -> io::Result<()> {
    #[derive(Deserialize)]
    struct CommentId {
        id: String,
    }
    for record in records {
        let entry: CommentId = serde_json::from_str(record.get())?;
        if entry.id == id {
            let mut fields: CommentFields = serde_json::from_str(record.get())?;
            edit(&mut fields)?;
            *record = to_raw_value(&fields)?;
            return Ok(());
        }
    }
    Err(io::Error::new(io::ErrorKind::NotFound, "Comment not found"))
}

#[derive(Serialize)]
struct NewComment<'a> {
    id: String,
    file: String,
    quote: String,
    text: &'a str,
    created: String,
    resolved: bool,
    anchor: Option<Box<RawValue>>,
}

fn timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn nonempty_text<'a>(text: &'a str, error: &'static str) -> io::Result<&'a str> {
    let text =
        text.trim_matches(|ch: char| ch.is_whitespace() && ch != '\u{0085}' || ch == '\u{feff}');
    if text.is_empty() {
        Err(invalid_comment(error))
    } else {
        Ok(text)
    }
}

fn invalid_comment(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{list_projects, save_source};
    use std::fs;

    #[test]
    fn writes_electron_comments_and_replies_with_utf16_anchors() {
        let root = tempfile::tempdir().unwrap();
        let id = "b3b9ffb5-13c0-4d22-b427-54520347684a";
        let folder = root.path().join(id);
        fs::create_dir(&folder).unwrap();
        fs::write(
            folder.join(".refract.json"),
            serde_json::json!({"id":id,"name":"Paper"}).to_string(),
        )
        .unwrap();
        let source = "😀\r\nRepeat\r\nRepeat";
        fs::write(folder.join("main.tex"), source).unwrap();
        let project = list_projects(root.path()).unwrap().pop().unwrap();
        let comments = update_comments(
            root.path(),
            &project,
            CommentUpdate::Create {
                file: "main.tex".into(),
                text: " Clarify this result. ".into(),
                selection: Some(CommentSelection {
                    quote: "Repeat".into(),
                    range: 10..16,
                }),
            },
        )
        .unwrap();
        let comment = &comments[0];
        assert_eq!(comment.text, "Clarify this result.");
        assert_eq!(comment.source_position(source), Some((2, 0)));
        assert!(
            comment
                .created
                .as_deref()
                .is_some_and(|created| created.ends_with('Z'))
        );
        let path = folder.join(".refract-comments.json");
        let mut document: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        document[0]["externalField"] = serde_json::json!({"keep":true});
        fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        let comments = update_comments(
            root.path(),
            &project,
            CommentUpdate::Reply {
                id: comment.id.clone(),
                text: " Added context. ".into(),
            },
        )
        .unwrap();
        assert_eq!(comments[0].replies[0].text, "Added context.");
        for resolved in [true, false] {
            let comments = update_comments(
                root.path(),
                &project,
                CommentUpdate::SetResolved {
                    id: comment.id.clone(),
                    resolved,
                },
            )
            .unwrap();
            assert_eq!(comments[0].resolved, resolved);
        }
        let document: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(document[0]["externalField"]["keep"], true);
        assert_eq!(document[0]["anchor"]["start"], 10);
        assert_eq!(document[0]["anchor"]["end"], 16);
        assert!(document[0]["replies"][0]["id"].as_str().is_some());
        save_source(
            root.path(),
            &project,
            "main.tex",
            source,
            &format!("Draft\r\n{source}"),
        )
        .unwrap();
        let document: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(document[0]["anchor"]["start"], 16);
    }

    #[test]
    fn rejects_a_stale_selection_and_accepts_a_file_level_comment() {
        let root = tempfile::tempdir().unwrap();
        let id = "b3b9ffb5-13c0-4d22-b427-54520347684a";
        let folder = root.path().join(id);
        fs::create_dir(&folder).unwrap();
        fs::write(
            folder.join(".refract.json"),
            serde_json::json!({"id":id,"name":"Paper"}).to_string(),
        )
        .unwrap();
        fs::write(folder.join("main.tex"), "Edited source").unwrap();
        let project = list_projects(root.path()).unwrap().pop().unwrap();
        let error = update_comments(
            root.path(),
            &project,
            CommentUpdate::Create {
                file: "main.tex".into(),
                text: "A note".into(),
                selection: Some(CommentSelection {
                    quote: "Original".into(),
                    range: 0..8,
                }),
            },
        )
        .err()
        .unwrap();
        assert!(error.to_string().contains("selected text has changed"));
        assert!(!folder.join(".refract-comments.json").exists());
        let comments = update_comments(
            root.path(),
            &project,
            CommentUpdate::Create {
                file: "main.tex".into(),
                text: "A note".into(),
                selection: None,
            },
        )
        .unwrap();
        assert_eq!(comments[0].file, "main.tex");
        assert!(comments[0].quote.is_empty());
    }
}

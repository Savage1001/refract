use super::{Attachment, ConversationId, MAX_ATTACHMENTS, private_directory};
use crate::{
    Project, open_local_file,
    project_entries::checked_project_directory,
    project_management::{has_one_link, is_link},
};
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Read, Write},
    path::Path,
};

const DRAFTS_DIRECTORY: &str = ".refract-native-drafts";
const DRAFT_LIMIT: u64 = 6 * 1024 * 1024;

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct ConversationDraft {
    pub text: String,
    pub attachments: Vec<Attachment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

pub fn load_conversation_draft(
    projects_dir: &Path,
    project: &Project,
    id: &ConversationId,
) -> io::Result<ConversationDraft> {
    let root = checked_project_directory(projects_dir, project)?;
    let dir = match private_directory(&root, DRAFTS_DIRECTORY, false) {
        Ok(dir) => dir,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(ConversationDraft::default());
        }
        Err(error) => return Err(error),
    };
    let path = dir.join(format!("{}.json", id.as_str()));
    match path.symlink_metadata() {
        Ok(metadata) if metadata.is_file() && !is_link(&metadata) => {}
        Ok(_) => return Err(invalid_draft()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(ConversationDraft::default());
        }
        Err(error) => return Err(error),
    }
    let file = open_local_file(&dir, &path)?;
    if !has_one_link(&file)? || file.metadata()?.len() > DRAFT_LIMIT {
        return Err(invalid_draft());
    }
    let mut bytes = Vec::new();
    file.take(DRAFT_LIMIT + 1).read_to_end(&mut bytes)?;
    let draft: ConversationDraft = serde_json::from_slice(&bytes)?;
    if bytes.len() as u64 > DRAFT_LIMIT || draft.attachments.len() > MAX_ATTACHMENTS {
        return Err(invalid_draft());
    }
    Ok(draft)
}

pub fn save_conversation_draft(
    projects_dir: &Path,
    project: &Project,
    id: &ConversationId,
    draft: &ConversationDraft,
) -> io::Result<()> {
    let content = serde_json::to_vec(draft)?;
    if content.len() as u64 > DRAFT_LIMIT || draft.attachments.len() > MAX_ATTACHMENTS {
        return Err(invalid_draft());
    }
    let root = checked_project_directory(projects_dir, project)?;
    let dir = private_directory(&root, DRAFTS_DIRECTORY, true)?;
    let path = dir.join(format!("{}.json", id.as_str()));
    let mut staged = tempfile::NamedTempFile::new_in(&dir)?;
    staged.write_all(&content)?;
    private_directory(&root, DRAFTS_DIRECTORY, false)?;
    staged
        .persist(&path)
        .map(|_| ())
        .map_err(|error| error.error)
}

fn invalid_draft() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Invalid or oversized chat draft",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{conversation::create_conversation, create_project};

    #[test]
    fn stores_drafts_per_chat_without_mutating_the_saved_conversations() {
        let root = tempfile::tempdir().unwrap();
        let project = create_project(root.path(), "Notes").unwrap();
        let id = create_conversation(root.path(), &project).unwrap();
        let draft = ConversationDraft {
            text: "Check the introduction".into(),
            attachments: vec![Attachment {
                token: "28a08ee7-40a1-4ca9-9b12-877bc839e95c.png".into(),
                name: "Screenshot.png".into(),
            }],
            provider: Some("opencode".into()),
        };
        save_conversation_draft(root.path(), &project, &id, &draft).unwrap();
        let restored = load_conversation_draft(root.path(), &project, &id).unwrap();
        assert_eq!(restored.text, draft.text);
        assert_eq!(restored.attachments, draft.attachments);
        assert_eq!(restored.provider, draft.provider);
        let default = ConversationId::default();
        assert!(
            load_conversation_draft(root.path(), &project, &default)
                .unwrap()
                .text
                .is_empty()
        );
        assert!(
            !root
                .path()
                .join(project.id)
                .join(".refract-chat.json")
                .exists()
        );
    }
}

use crate::{
    Project, is_project_id, open_local_file,
    project_entries::checked_project_directory,
    project_management::{has_one_link, is_link},
    stage_file,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const MAX_HISTORY_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ConversationId(String);

impl Default for ConversationId {
    fn default() -> Self {
        Self("default".into())
    }
}

impl ConversationId {
    pub fn parse(id: &str) -> io::Result<Self> {
        if id != "default" && !is_project_id(id) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid chat ID",
            ));
        }
        Ok(Self(id.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn is_default(&self) -> bool {
        self.0 == "default"
    }
}

#[derive(Clone, Debug)]
pub struct ConversationSummary {
    pub id: ConversationId,
    pub title: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct Conversation {
    pub title: Option<String>,
    #[serde(default)]
    pub messages: Vec<Message>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum Message {
    User {
        text: String,
        attachments: Option<Vec<Attachment>>,
    },
    Assistant {
        text: String,
    },
    Error {
        text: String,
    },
    Handoff {
        from: String,
        to: String,
        status: String,
    },
    #[serde(other)]
    Unsupported,
}

#[derive(Debug, Deserialize)]
pub struct Attachment {
    pub token: String,
    pub name: String,
}

impl Conversation {
    fn summary(&self, id: ConversationId) -> ConversationSummary {
        let title = self
            .title
            .as_deref()
            .filter(|title| !title.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                if id.is_default() {
                    return "Chat".into();
                }
                if let Some(Message::User { text, .. }) = self
                    .messages
                    .iter()
                    .find(|message| matches!(message, Message::User { .. }))
                {
                    let mut count = 0;
                    let title: String = text
                        .chars()
                        .take_while(|character| {
                            count += character.len_utf16();
                            count <= 50
                        })
                        .collect();
                    if !title.is_empty() {
                        return title;
                    }
                }
                "New chat".into()
            });
        ConversationSummary { id, title }
    }
}

pub fn load_conversation(
    projects_dir: &Path,
    project: &Project,
    id: &ConversationId,
) -> io::Result<Conversation> {
    let root = checked_project_directory(projects_dir, project)?;
    load(&root, id)
}

pub fn list_conversations(
    projects_dir: &Path,
    project: &Project,
) -> io::Result<Vec<ConversationSummary>> {
    let root = checked_project_directory(projects_dir, project)?;
    let default = ConversationId::default();
    let mut conversations = vec![load(&root, &default)?.summary(default)];
    let dir = match conversations_dir(&root, false) {
        Ok(dir) => dir,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(conversations),
        Err(error) => return Err(error),
    };
    let mut named = Vec::new();
    for file in fs::read_dir(dir)? {
        let file = file?;
        let name = file.file_name();
        if let Some(id) = name
            .to_str()
            .and_then(|name| name.strip_suffix(".json"))
            .filter(|id| is_project_id(id))
        {
            named.push(ConversationId(id.to_owned()));
        }
    }
    named.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    for id in named {
        conversations.push(load(&root, &id)?.summary(id));
    }
    Ok(conversations)
}

pub fn create_conversation(projects_dir: &Path, project: &Project) -> io::Result<ConversationId> {
    let root = checked_project_directory(projects_dir, project)?;
    let id = ConversationId(uuid::Uuid::new_v4().to_string());
    let path = conversation_file(&root, &id, true)?;
    let parent = path.parent().expect("chat path has a parent directory");
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    staged.write_all(br#"{"sessionId":null,"messages":[]}"#)?;
    conversations_dir(&root, false)?;
    staged
        .persist_noclobber(path)
        .map_err(|error| error.error)?;
    Ok(id)
}

pub fn rename_conversation(
    projects_dir: &Path,
    project: &Project,
    id: &ConversationId,
    title: &str,
) -> io::Result<()> {
    let title = title.trim();
    if title.is_empty() || title.encode_utf16().count() > 100 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Chat name must be between 1 and 100 characters",
        ));
    }
    let root = checked_project_directory(projects_dir, project)?;
    let path = conversation_file(&root, id, false)?;
    let original = read_conversation(&root, &path, id)?;
    let mut document: Map<String, Value> = match &original {
        Some(bytes) => serde_json::from_slice(bytes)?,
        None => Map::from_iter([
            ("sessionId".into(), Value::Null),
            ("messages".into(), json!([])),
        ]),
    };
    document.insert("title".into(), json!(title));
    let content = serde_json::to_vec(&document)?;
    if original.is_some() {
        let staged = stage_file(&path, &content)?;
        conversation_file(&root, id, false)?;
        if read_conversation(&root, &path, id)? != original {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "The conversation changed during the rename",
            ));
        }
        staged.persist(path).map_err(|error| error.error)?;
    } else {
        let mut staged = tempfile::NamedTempFile::new_in(&root)?;
        staged.write_all(&content)?;
        staged
            .persist_noclobber(path)
            .map_err(|error| error.error)?;
    }
    Ok(())
}

fn conversations_dir(root: &Path, create: bool) -> io::Result<PathBuf> {
    let dir = root.join(".refract-chats");
    if create {
        match fs::create_dir(&dir) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    let metadata = dir.symlink_metadata()?;
    if !metadata.is_dir() || is_link(&metadata) || dir.canonicalize()? != dir {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid private chat directory",
        ));
    }
    Ok(dir)
}

fn conversation_file(root: &Path, id: &ConversationId, create_dir: bool) -> io::Result<PathBuf> {
    if id.is_default() {
        Ok(root.join(".refract-chat.json"))
    } else {
        Ok(conversations_dir(root, create_dir)?.join(format!("{}.json", id.as_str())))
    }
}

fn read_conversation(root: &Path, path: &Path, id: &ConversationId) -> io::Result<Option<Vec<u8>>> {
    let metadata = match path.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) if id.is_default() && error.kind() == io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    if !metadata.is_file() || is_link(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid conversation file",
        ));
    }
    let file = open_local_file(root, path)?;
    if !has_one_link(&file)? || file.metadata()?.len() > MAX_HISTORY_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid conversation file",
        ));
    }
    let mut data = Vec::new();
    file.take(MAX_HISTORY_BYTES + 1).read_to_end(&mut data)?;
    if data.len() as u64 > MAX_HISTORY_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "The conversation file is too large",
        ));
    }
    Ok(Some(data))
}

fn load(root: &Path, id: &ConversationId) -> io::Result<Conversation> {
    let path = conversation_file(root, id, false)?;
    match read_conversation(root, &path, id)? {
        Some(data) => Ok(serde_json::from_slice(&data)?),
        None => Ok(Conversation::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_project;

    #[test]
    fn uses_electron_conversation_files_without_altering_provider_or_messages() {
        let root = tempfile::tempdir().unwrap();
        let project = create_project(root.path(), "Paper").unwrap();
        let default = ConversationId::default();
        let folder = root.path().join(&project.id);
        assert_eq!(
            list_conversations(root.path(), &project).unwrap()[0].title,
            "Chat"
        );
        assert!(!folder.join(".refract-chat.json").exists());
        let id = create_conversation(root.path(), &project).unwrap();
        assert_eq!(
            list_conversations(root.path(), &project).unwrap()[1].title,
            "New chat"
        );
        let path = folder
            .join(".refract-chats")
            .join(format!("{}.json", id.as_str()));
        let record = json!({"sessionId":"session", "provider":"opencode", "handoffPending":true, "messages":[{"role":"user", "text":"Explain", "metadata":7}, {"role":"handoff","from":"codex","to":"opencode","status":"pending"}]});
        fs::write(&path, record.to_string()).unwrap();
        assert_eq!(
            list_conversations(root.path(), &project).unwrap()[1].title,
            "Explain"
        );
        rename_conversation(root.path(), &project, &id, "  Review  ").unwrap();
        let mut saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            saved.as_object_mut().unwrap().remove("title"),
            Some(json!("Review"))
        );
        assert_eq!(saved, record);
        assert!(rename_conversation(root.path(), &project, &id, &"🙂".repeat(51)).is_err());
        rename_conversation(root.path(), &project, &default, "Original").unwrap();
        assert_eq!(
            load_conversation(root.path(), &project, &default)
                .unwrap()
                .title
                .as_deref(),
            Some("Original")
        );
    }

    #[test]
    fn rejects_invalid_ids_and_linked_conversation_records() {
        let root = tempfile::tempdir().unwrap();
        let project = create_project(root.path(), "Paper").unwrap();
        assert!(ConversationId::parse("../main.tex").is_err());
        let id = create_conversation(root.path(), &project).unwrap();
        let folder = root.path().join(&project.id);
        let path = folder
            .join(".refract-chats")
            .join(format!("{}.json", id.as_str()));
        let linked = folder.join("linked.json");
        fs::hard_link(&path, &linked).unwrap();
        assert!(load_conversation(root.path(), &project, &id).is_err());
        assert!(rename_conversation(root.path(), &project, &id, "Changed").is_err());
    }
}

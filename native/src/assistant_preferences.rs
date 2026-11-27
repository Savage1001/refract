use crate::{
    open_local_file,
    project_management::{has_one_link, is_link},
};
use serde::{Deserialize, Deserializer, Serialize};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{self, Read},
    path::Path,
};

const MAX_PREFERENCES_BYTES: u64 = 1024 * 1024;
const MAX_IDENTIFIER_UNITS: usize = 200;
const MAX_FAVORITES: usize = 500;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssistantConnection {
    CodexCli,
    CodexAccount,
    OpenCode,
}

impl AssistantConnection {
    fn file_name(self) -> &'static str {
        match self {
            Self::CodexCli => "codex-preferences.json",
            Self::CodexAccount => "codex-account-preferences.json",
            Self::OpenCode => "opencode-preferences.json",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantPreferences {
    #[serde(deserialize_with = "Deserialize::deserialize")]
    pub model: Option<String>,
    #[serde(deserialize_with = "Deserialize::deserialize")]
    pub reasoning_effort: Option<String>,
    pub favorites: Vec<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_fast_mode",
        skip_serializing_if = "Option::is_none"
    )]
    pub fast_mode: Option<bool>,
}

impl AssistantPreferences {
    pub fn validate(&self, connection: AssistantConnection) -> io::Result<()> {
        let valid_effort = match connection {
            AssistantConnection::OpenCode => valid_opencode_variant,
            AssistantConnection::CodexCli | AssistantConnection::CodexAccount => valid_codex_effort,
        };
        if (connection == AssistantConnection::OpenCode && self.fast_mode.is_some())
            || self
                .model
                .as_deref()
                .is_some_and(|model| !valid_model_id(model))
            || self
                .reasoning_effort
                .as_deref()
                .is_some_and(|effort| !valid_effort(effort))
            || self.favorites.len() > MAX_FAVORITES
        {
            return Err(invalid_preferences());
        }
        let mut seen = HashSet::with_capacity(self.favorites.len());
        if !self
            .favorites
            .iter()
            .all(|model| valid_model_id(model) && seen.insert(model.as_str()))
        {
            return Err(invalid_preferences());
        }
        Ok(())
    }
}

pub fn valid_model_id(id: &str) -> bool {
    valid_identifier_length(id)
        && id.trim_matches(js_whitespace) == id
        && !id.chars().any(|character| character.is_ascii_control())
}

pub fn valid_opencode_variant(variant: &str) -> bool {
    valid_identifier_length(variant)
        && !variant.chars().any(|character| {
            js_whitespace(character)
                || character.is_ascii_control()
                || matches!(character, '#' | '/')
        })
}

pub fn valid_codex_effort(effort: &str) -> bool {
    matches!(
        effort,
        "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra"
    )
}

fn valid_identifier_length(value: &str) -> bool {
    !value.is_empty()
        && value.encode_utf16().take(MAX_IDENTIFIER_UNITS + 1).count() <= MAX_IDENTIFIER_UNITS
}

fn js_whitespace(character: char) -> bool {
    matches!(
        character,
        '\u{0009}'..='\u{000d}'
            | ' '
            | '\u{00a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
            | '\u{feff}'
    )
}

fn deserialize_fast_mode<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<bool>, D::Error> {
    bool::deserialize(deserializer).map(Some)
}

pub fn load_assistant_preferences(
    profile: &Path,
    connection: AssistantConnection,
) -> io::Result<AssistantPreferences> {
    let path = profile.join(connection.file_name());
    let Some(file) = open_preferences(profile, &path)? else {
        return Ok(AssistantPreferences::default());
    };
    if file.metadata()?.len() > MAX_PREFERENCES_BYTES {
        return Err(invalid_preferences());
    }
    let mut content = Vec::new();
    file.take(MAX_PREFERENCES_BYTES + 1)
        .read_to_end(&mut content)?;
    if content.len() as u64 > MAX_PREFERENCES_BYTES {
        return Err(invalid_preferences());
    }
    let value: serde_json::Value = serde_json::from_slice(&content)?;
    if !value.is_object() {
        return Err(invalid_preferences());
    }
    let preferences: AssistantPreferences = serde_json::from_value(value)?;
    preferences.validate(connection)?;
    Ok(preferences)
}

pub fn save_assistant_preferences(
    profile: &Path,
    connection: AssistantConnection,
    preferences: &AssistantPreferences,
) -> io::Result<()> {
    preferences.validate(connection)?;
    fs::create_dir_all(profile)?;
    let path = profile.join(connection.file_name());
    open_preferences(profile, &path)?;
    let mut staged = tempfile::NamedTempFile::new_in(profile)?;
    serde_json::to_writer(staged.as_file_mut(), preferences)?;
    staged.as_file().sync_all()?;
    staged
        .persist(path)
        .map(|_| ())
        .map_err(|error| error.error)
}

fn open_preferences(profile: &Path, path: &Path) -> io::Result<Option<File>> {
    match path.symlink_metadata() {
        Ok(info) if info.is_file() && !is_link(&info) => {
            let file = open_local_file(profile, path)?;
            if !has_one_link(&file)? {
                return Err(invalid_preferences());
            }
            Ok(Some(file))
        }
        Ok(_) => Err(invalid_preferences()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn invalid_preferences() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Invalid assistant preferences")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_electron_profiles_and_keeps_connection_preferences_separate() {
        let temp = tempfile::tempdir().unwrap();
        let profile = temp.path().join("Refract");
        assert_eq!(
            load_assistant_preferences(&profile, AssistantConnection::CodexCli).unwrap(),
            AssistantPreferences::default()
        );
        assert!(!profile.exists());
        fs::create_dir(&profile).unwrap();
        let cli_path = profile.join("codex-preferences.json");
        let cli_json = r#"{"model":"model-a","reasoningEffort":"ultra","favorites":["model-b","model-a"],"fastMode":false}"#;
        fs::write(&cli_path, cli_json).unwrap();
        let mut account =
            load_assistant_preferences(&profile, AssistantConnection::CodexCli).unwrap();
        assert_eq!(account.model.as_deref(), Some("model-a"));
        assert_eq!(account.fast_mode, Some(false));
        assert_eq!(account.favorites, ["model-b", "model-a"]);
        account.reasoning_effort = Some("max".into());
        account.fast_mode = None;
        save_assistant_preferences(&profile, AssistantConnection::CodexAccount, &account).unwrap();
        let account_path = profile.join("codex-account-preferences.json");
        let stored: serde_json::Value =
            serde_json::from_slice(&fs::read(account_path).unwrap()).unwrap();
        assert_eq!(
            stored,
            json!({"model":"model-a", "reasoningEffort":"max", "favorites":["model-b", "model-a"]})
        );
        assert_eq!(fs::read_to_string(cli_path).unwrap(), cli_json);
        let open_code = AssistantPreferences {
            model: Some("vendor/model".into()),
            reasoning_effort: Some("deep".into()),
            ..AssistantPreferences::default()
        };
        save_assistant_preferences(&profile, AssistantConnection::OpenCode, &open_code).unwrap();
        assert!(profile.join("opencode-preferences.json").is_file());
        assert_eq!(
            load_assistant_preferences(&profile, AssistantConnection::OpenCode).unwrap(),
            open_code
        );
    }

    #[test]
    fn rejects_invalid_preferences_without_overwriting_existing_files() {
        let profile = tempfile::tempdir().unwrap();
        let path = profile.path().join("codex-preferences.json");
        for invalid in [
            r#"{}"#,
            r#"{"reasoningEffort":null,"favorites":[]}"#,
            r#"{"model":null,"favorites":[]}"#,
            r#"{"model":null,"reasoningEffort":null,"favorites":[],"fastMode":null}"#,
            r#"{"model":null,"reasoningEffort":"deep","favorites":[]}"#,
            r#"{"model":null,"reasoningEffort":null,"favorites":["same","same"]}"#,
            r#"{"model":null,"reasoningEffort":null,"favorites":[],"extra":0}"#,
            r#"[null,null,[]]"#,
        ] {
            fs::write(&path, invalid).unwrap();
            assert!(
                load_assistant_preferences(profile.path(), AssistantConnection::CodexCli).is_err()
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), invalid);
        }
        let invalid = AssistantPreferences {
            reasoning_effort: Some("deep".into()),
            ..AssistantPreferences::default()
        };
        assert!(
            save_assistant_preferences(profile.path(), AssistantConnection::CodexCli, &invalid)
                .is_err()
        );
        assert_eq!(fs::read_to_string(path).unwrap(), r#"[null,null,[]]"#);
        assert!(
            AssistantPreferences {
                fast_mode: Some(false),
                ..AssistantPreferences::default()
            }
            .validate(AssistantConnection::OpenCode)
            .is_err()
        );
    }

    #[test]
    fn checks_utf16_identifier_lengths_and_reference_whitespace() {
        assert!(valid_model_id(&"📄".repeat(100)));
        assert!(!valid_model_id(&"📄".repeat(101)));
        assert!(valid_model_id("model name"));
        assert!(!valid_model_id("\u{feff}model"));
        assert!(!valid_model_id("model\nname"));
        assert!(valid_opencode_variant("deep-long"));
        assert!(!valid_opencode_variant("deep\u{00a0}long"));
        assert!(!valid_opencode_variant("bad#variant"));
        assert!(!valid_opencode_variant("bad/variant"));
    }
}

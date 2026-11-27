use super::{CANCELLED, Connection, cancelled, with_connection};
use crate::{
    assistant_preferences::valid_model_id,
    assistant_protocol::{CodexMessage, CodexRequest, CodexResult},
};
use serde_json::Value;
use std::{
    collections::HashSet,
    process::Command,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(15);
const INITIALIZE_REQUEST: u64 = 1;
const MODEL_LIST_REQUEST: u64 = 2;
const INVALID_CATALOG: &str = "Invalid Codex model catalog";
const INVALID_PAGINATION: &str = "Invalid Codex model catalog pagination";
const MAX_PAGES: usize = 20;
const MAX_RECORDS: usize = 2000;
const MAX_CURSOR_BYTES: usize = 4096;
const MAX_EFFORTS_PER_MODEL: usize = 64;
const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexCatalogEffort {
    pub effort: String,
    pub description: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexCatalogModel {
    pub id: String,
    pub model: String,
    pub display_name: String,
    pub description: String,
    pub supported_reasoning_efforts: Vec<CodexCatalogEffort>,
    pub default_reasoning_effort: Option<String>,
    pub is_default: bool,
}

/// Query the installed Codex app server from a worker thread.
pub fn discover_codex_models(
    command: Command,
    account_connection: bool,
    cancel: &AtomicBool,
) -> Result<Vec<CodexCatalogModel>, String> {
    with_connection(command, cancel, |connection, _| {
        let deadline = Instant::now() + DISCOVERY_TIMEOUT;
        connection.request(CodexRequest::initialize(
            INITIALIZE_REQUEST,
            account_connection,
        ))?;
        discovery_reply(connection, INITIALIZE_REQUEST, deadline, cancel)?;
        connection.request(CodexRequest::initialized())?;

        let mut catalog = Catalog::default();
        let mut cursor = None;
        for _ in 0..MAX_PAGES {
            if cancelled(cancel) {
                return Err(CANCELLED.into());
            }
            connection.request(CodexRequest::model_list(
                MODEL_LIST_REQUEST,
                cursor.as_deref(),
            ))?;
            let result = discovery_reply(connection, MODEL_LIST_REQUEST, deadline, cancel)?;
            cursor = catalog.add_page(result)?;
            if cursor.is_none() {
                return Ok(catalog.models);
            }
        }
        Err(INVALID_PAGINATION.into())
    })
}

fn discovery_reply(
    connection: &Connection,
    request: u64,
    deadline: Instant,
    cancel: &AtomicBool,
) -> Result<CodexResult, String> {
    while Instant::now() < deadline {
        if cancelled(cancel) {
            return Err(CANCELLED.into());
        }
        if let Some(CodexMessage::Reply { id, result }) = connection.poll(Some(deadline))?
            && id == request
        {
            return result.map_err(|_| "Codex model discovery request failed".into());
        }
    }
    Err("Codex model discovery timed out".into())
}

#[derive(Default)]
struct Catalog {
    models: Vec<CodexCatalogModel>,
    ids: HashSet<String>,
    cursors: HashSet<String>,
    records: usize,
    text_bytes: usize,
}

impl Catalog {
    fn add_page(&mut self, page: CodexResult) -> Result<Option<String>, &'static str> {
        let data = page.data.ok_or(INVALID_CATALOG)?;
        self.records += data.len();
        if self.records > MAX_RECORDS {
            return Err(INVALID_CATALOG);
        }
        for item in data {
            let Some(model) = parse_model(&item)? else {
                continue;
            };
            if self.ids.insert(model.id.clone()) {
                self.text_bytes += model.text_bytes();
                if self.text_bytes > MAX_TEXT_BYTES {
                    return Err(INVALID_CATALOG);
                }
                self.models.push(model);
            }
        }
        let next = page.next_cursor.filter(|cursor| !cursor.is_empty());
        if let Some(cursor) = &next
            && (cursor.len() > MAX_CURSOR_BYTES || !self.cursors.insert(cursor.clone()))
        {
            return Err(INVALID_PAGINATION);
        }
        Ok(next)
    }
}

impl CodexCatalogModel {
    fn text_bytes(&self) -> usize {
        self.id.len()
            + self.model.len()
            + self.display_name.len()
            + self.description.len()
            + self
                .default_reasoning_effort
                .as_ref()
                .map_or(0, String::len)
            + self
                .supported_reasoning_efforts
                .iter()
                .map(|effort| effort.effort.len() + effort.description.len())
                .sum::<usize>()
    }
}

fn parse_model(value: &Value) -> Result<Option<CodexCatalogModel>, &'static str> {
    let id = required_string(value, "id")?;
    let model = required_string(value, "model")?;
    let display_name = required_string(value, "displayName")?;
    let efforts = value
        .get("supportedReasoningEfforts")
        .and_then(Value::as_array)
        .filter(|efforts| efforts.len() <= MAX_EFFORTS_PER_MODEL)
        .ok_or(INVALID_CATALOG)?;
    let supported_reasoning_efforts = efforts
        .iter()
        .map(|option| {
            let effort = option
                .get("reasoningEffort")
                .filter(|effort| !effort.is_null())
                .or_else(|| option.get("effort"))
                .and_then(Value::as_str)
                .ok_or(INVALID_CATALOG)?;
            Ok(CodexCatalogEffort {
                effort: effort.into(),
                description: required_string(option, "description")?.into(),
            })
        })
        .collect::<Result<Vec<_>, &'static str>>()?;
    if !valid_model_id(id) || !valid_model_id(model) {
        return Ok(None);
    }
    Ok(Some(CodexCatalogModel {
        id: id.into(),
        model: model.into(),
        display_name: display_name.into(),
        description: value
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        supported_reasoning_efforts,
        default_reasoning_effort: value
            .get("defaultReasoningEffort")
            .and_then(Value::as_str)
            .map(str::to_owned),
        is_default: value.get("isDefault").and_then(Value::as_bool) == Some(true),
    }))
}

fn required_string<'a>(value: &'a Value, field: &str) -> Result<&'a str, &'static str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or(INVALID_CATALOG)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn model(id: &str, reasoning: Value) -> Value {
        json!({
            "id": id,
            "model": id,
            "displayName": "Model label",
            "description": "Model description",
            "supportedReasoningEfforts": reasoning,
            "defaultReasoningEffort": "high",
            "isDefault": true
        })
    }

    fn page(models: Vec<Value>, cursor: Option<&str>) -> CodexResult {
        CodexResult {
            data: Some(models),
            next_cursor: cursor.map(str::to_owned),
            ..CodexResult::default()
        }
    }

    #[test]
    fn accepts_legacy_and_current_effort_fields_and_filters_unusable_ids() {
        let mut catalog = Catalog::default();
        let efforts = json!([
            {"effort": "high", "description": "Thorough"},
            {"reasoningEffort": "ultra", "effort": "low", "description": "Maximum"},
            {"reasoningEffort": null, "effort": "xhigh", "description": ""}
        ]);
        let result = catalog.add_page(page(
            vec![
                model("--vendor/model with space", efforts),
                model("bad\nmodel", json!([])),
                model("", json!([])),
            ],
            Some(""),
        ));
        assert_eq!(result.unwrap(), None);
        assert_eq!(catalog.models.len(), 1);
        let model = &catalog.models[0];
        assert_eq!(model.id, "--vendor/model with space");
        assert_eq!(model.display_name, "Model label");
        assert_eq!(model.description, "Model description");
        assert_eq!(model.default_reasoning_effort.as_deref(), Some("high"));
        assert!(model.is_default);
        assert_eq!(
            model
                .supported_reasoning_efforts
                .iter()
                .map(|effort| (effort.effort.as_str(), effort.description.as_str()))
                .collect::<Vec<_>>(),
            [("high", "Thorough"), ("ultra", "Maximum"), ("xhigh", "")]
        );
    }

    #[test]
    fn tracks_pages_in_catalog_order_and_rejects_cursor_cycles() {
        let mut catalog = Catalog::default();
        let first = model("first", json!([]));
        let second = model("second", json!([]));
        assert_eq!(
            catalog
                .add_page(page(vec![first.clone()], Some("next")))
                .unwrap()
                .as_deref(),
            Some("next")
        );
        assert_eq!(
            catalog
                .add_page(page(vec![first, second], Some("last")))
                .unwrap()
                .as_deref(),
            Some("last")
        );
        assert_eq!(
            catalog
                .models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert_eq!(
            catalog.add_page(page(Vec::new(), Some("next"))),
            Err(INVALID_PAGINATION)
        );
    }

    #[test]
    fn rejects_malformed_required_fields_and_defaults_invalid_optional_fields() {
        assert!(parse_model(&model("ok", json!([{"effort": "high"}]))).is_err());
        assert!(
            parse_model(&model(
                "ok",
                json!([{"reasoningEffort": false, "effort": "high", "description": ""}])
            ))
            .is_err()
        );
        let mut row = model("ok", json!([]));
        row["description"] = json!(null);
        row["defaultReasoningEffort"] = json!(0);
        row["isDefault"] = json!("true");
        let row = parse_model(&row).unwrap().unwrap();
        assert!(row.description.is_empty());
        assert_eq!(row.default_reasoning_effort, None);
        assert!(!row.is_default);
        assert_eq!(
            Catalog::default().add_page(CodexResult::default()),
            Err(INVALID_CATALOG)
        );
    }
}

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};

use anyhow::Result;
use clashofmeat_core::Env;
use clashofmeat_daemon::config;
use serde_json::{json, Map, Value};
use time::{macros::format_description, OffsetDateTime};

pub fn run() {
    let answer = std::panic::catch_unwind(handle).unwrap_or_else(|_| "{}".to_owned());
    let _ = std::panic::catch_unwind(|| println!("{answer}"));
}

fn handle() -> String {
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return "{}".into();
    }
    let Ok(payload) = serde_json::from_str::<Value>(&input) else {
        return "{}".into();
    };
    let event = payload["hook_event_name"].as_str();
    if let Some(record) = record(&payload) {
        let _ = append(&record);
    }
    if event == Some("beforeSubmitPrompt") {
        json!({"continue":true}).to_string()
    } else {
        "{}".into()
    }
}

fn printable_id(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str).filter(|id| {
        !id.is_empty()
            && id.len() <= 128
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'))
    })
}

fn category(name: &str) -> &'static str {
    let value = name.to_ascii_lowercase();
    if ["web", "browser", "fetch", "search_query"]
        .iter()
        .any(|x| value.contains(x))
    {
        "web"
    } else if ["test", "spec", "verify"].iter().any(|x| value.contains(x)) {
        "test"
    } else if ["write", "edit", "patch", "replace", "create"]
        .iter()
        .any(|x| value.contains(x))
    {
        "write"
    } else if ["read", "find", "grep", "glob", "search", "list", "view"]
        .iter()
        .any(|x| value.contains(x))
    {
        "read"
    } else {
        "other"
    }
}

fn record(payload: &Value) -> Option<Value> {
    let event = payload["hook_event_name"].as_str()?;
    if !matches!(
        event,
        "beforeSubmitPrompt" | "postToolUse" | "afterAgentResponse" | "stop"
    ) {
        return None;
    }
    let conversation = printable_id(payload.get("conversation_id"))?;
    let generation = printable_id(payload.get("generation_id"))?;
    let at = OffsetDateTime::now_utc()
        .format(&format_description!(
            "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
        ))
        .ok()?;
    let mut fields = Map::new();
    fields.insert("event".into(), json!(event));
    fields.insert("conversation".into(), json!(conversation));
    fields.insert("generation".into(), json!(generation));
    fields.insert("at".into(), json!(at));
    if let Some(model) = payload
        .get("model_id")
        .filter(|v| !v.is_null())
        .or_else(|| payload.get("model"))
        .and_then(Value::as_str)
    {
        let clean: String = model
            .to_ascii_lowercase()
            .chars()
            .filter(|c| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | '-')
            })
            .take(64)
            .collect();
        if !clean.is_empty() {
            fields.insert("model".into(), json!(clean));
        }
    }
    if let Some(version) = payload["cursor_version"].as_str().filter(|v| {
        !v.is_empty()
            && v.len() <= 64
            && v.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'+' | b'-'))
    }) {
        fields.insert("version".into(), json!(version));
    }
    if event == "beforeSubmitPrompt" {
        if let Some(prompt) = payload["prompt"].as_str() {
            fields.insert("prompt_chars".into(), json!(prompt.chars().count()));
        }
    }
    if event == "stop" {
        if let Some(status) = payload["status"]
            .as_str()
            .filter(|v| matches!(*v, "completed" | "aborted" | "error"))
        {
            fields.insert("status".into(), json!(status));
        }
    }
    if event == "postToolUse" {
        let name = payload["tool_name"].as_str().unwrap_or("");
        fields.insert("tool".into(), json!(category(name)));
        let lower = name.to_ascii_lowercase();
        if lower == "task" || lower.contains("subagent") {
            fields.insert("subagent".into(), json!(true));
        }
    }
    for key in [
        "input_tokens",
        "output_tokens",
        "cache_read_tokens",
        "cache_write_tokens",
    ] {
        if let Some(value) = payload[key].as_f64().filter(|value| {
            value.is_finite()
                && *value >= 0.0
                && value.fract() == 0.0
                && *value <= 9_007_199_254_740_991.0
        }) {
            fields.insert(key.into(), json!(value as u64));
        }
    }
    Some(Value::Object(fields))
}

fn append(record: &Value) -> Result<()> {
    let env = Env::from_process();
    let dir = config::home(&env).join("spool");
    fs::create_dir_all(&dir)?;
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    // Windows keeps a user's profile private already; Unix needs the modes spelled out.
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        fs::set_permissions(config::home(&env), fs::Permissions::from_mode(0o700))?;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        options.mode(0o600);
    }
    let mut file = options.open(dir.join("cursor.jsonl"))?;
    serde_json::to_writer(&mut file, record)?;
    file.write_all(b"\n")?;
    Ok(())
}

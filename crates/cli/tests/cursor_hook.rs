use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};

use clashofmeat_core::{Cx, Provider, Record, Signal};
use clashofmeat_providers::cursor::Cursor;
use serde_json::{json, Map, Value};

fn invoke(home: &std::path::Path, payload: &str) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_clashofmeat"))
        .arg("cursor-hook")
        .env("CLASHOFMEAT_HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn hook_allowlist_and_cursor_provider() {
    let home = tempfile::tempdir().unwrap();
    let base = json!({"conversation_id":"conv-1","generation_id":"gen-1"});
    let payloads = [
        (
            json!({"hook_event_name":"beforeSubmitPrompt","prompt":"SECRET PROMPT"}),
            json!({"continue":true}),
        ),
        (
            json!({"hook_event_name":"postToolUse","tool_name":"Task","tool_input":{"path":"/private/secret"}}),
            json!({}),
        ),
        (
            json!({"hook_event_name":"afterAgentResponse","input_tokens":35,"output_tokens":12,"cache_read_tokens":5,"model_id":"Claude/Opus 4.1","cursor_version":"1.2.3"}),
            json!({}),
        ),
        (
            json!({"hook_event_name":"stop","status":"completed","cache_write_tokens":3}),
            json!({}),
        ),
    ];
    for (fields, expected) in payloads {
        let mut payload = base.clone();
        payload
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        assert_eq!(invoke(home.path(), &payload.to_string()), expected);
    }
    let path = home.path().join("spool/cursor.jsonl");
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(!raw.contains("SECRET PROMPT"));
    assert!(!raw.contains("/private/secret"));
    let records: Vec<Value> = raw
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records.len(), 4);
    assert_eq!(records[1]["tool"], "other");
    assert_eq!(records[1]["subagent"], true);
    assert_eq!(records[2]["model"], "claudeopus4.1");
    assert_eq!(records[2]["version"], "1.2.3");
    assert_eq!(records[3]["status"], "completed");
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let mut state = Map::new();
    let signals = Cursor.parse(
        &Record {
            source: 0,
            file_key: "cursor.jsonl".into(),
            offset: 0,
            value: records[2].clone(),
        },
        &mut Cx {
            file_state: &mut state,
            now: time::OffsetDateTime::now_utc(),
        },
    );
    assert!(signals.iter().any(|signal| matches!(signal, Signal::Event(event) if event.key == "input_tokens" && event.value == 30)));
    assert!(signals.iter().any(|signal| matches!(signal, Signal::Event(event) if event.key == "output_tokens" && event.value == 12)));
}

#[test]
fn invalid_and_unknown_hooks_are_ignored() {
    let home = tempfile::tempdir().unwrap();
    assert_eq!(invoke(home.path(), "not json"), json!({}));
    assert_eq!(
        invoke(
            home.path(),
            &json!({"hook_event_name":"unknown","conversation_id":"conv","generation_id":"gen"})
                .to_string()
        ),
        json!({})
    );
    assert_eq!(invoke(home.path(), &json!({"hook_event_name":"stop","conversation_id":"bad id","generation_id":"gen","response":"SECRET"}).to_string()), json!({}));
    assert!(!home.path().join("spool/cursor.jsonl").exists());
}

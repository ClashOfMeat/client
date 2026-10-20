//! Claude desktop Cowork audit records, mapped to Claude transcript events.

use clashofmeat_core::{
    ids, Cx, Detect, Manifest, Observation, Provider, Record, Signal, Source, SourceFormat,
    TaskRef, TurnPhase,
};
use serde_json::{Map, Value};

use crate::claude_code::ClaudeCode;

pub struct ClaudeCowork;

static MANIFEST: Manifest = Manifest {
    id: "claude-cowork",
    name: "Claude Cowork",
    detect: Detect {
        commands: &[],
        apps: &["Claude"],
        paths: &["~/Library/Application Support/Claude/local-agent-mode-sessions"],
    },
    sources: &[Source {
        root: "~/Library/Application Support/Claude/local-agent-mode-sessions",
        glob: "**/audit.jsonl",
        format: SourceFormat::Jsonl,
    }],
    wiring: &[],
};

const SESSION_KEY: &str = "sessionId";

impl Provider for ClaudeCowork {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn parse(&self, record: &Record, cx: &mut Cx) -> Vec<Signal> {
        let value = &record.value;
        // The prompt's Cowork session ID precedes the Agent SDK's different ID.
        // The first valid ID belongs to the entire audit file.
        if !cx.file_state.contains_key(SESSION_KEY) {
            if let Some(session_id) = value
                .get("session_id")
                .and_then(Value::as_str)
                .filter(|id| ids::is_uuid(id))
            {
                cx.file_state
                    .insert(SESSION_KEY.into(), Value::from(session_id));
            }
        }
        let Some(session_id) = cx
            .file_state
            .get(SESSION_KEY)
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return Vec::new();
        };

        let record_type = value.get("type").and_then(Value::as_str);
        if record_type == Some("system")
            && value.get("subtype").and_then(Value::as_str) == Some("init")
        {
            if let Some(version) = value
                .get("claude_code_version")
                .and_then(Value::as_str)
                .filter(|v| {
                    !v.is_empty()
                        && v.len() <= 64
                        && v.bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"_.+-".contains(&b))
                })
            {
                cx.file_state
                    .insert("observerVersion".into(), Value::from(version));
                return vec![Signal::Observer(Observation::ClientVersion {
                    task: TaskRef::Active(session_id),
                    client_version: version.into(),
                })];
            }
        }
        if !matches!(record_type, Some("user" | "assistant" | "result")) {
            return Vec::new();
        }

        let mut mapped = Map::new();
        mapped.insert(SESSION_KEY.into(), Value::from(session_id.as_str()));
        if let Some(timestamp) = value.get("_audit_timestamp") {
            mapped.insert("timestamp".into(), timestamp.clone());
        }
        if record_type == Some("result") {
            // A result has no event-bearing transcript equivalent. Feed a bare
            // terminating assistant turn through the same timestamp gate.
            mapped.insert("type".into(), Value::from("assistant"));
            mapped.insert(
                "message".into(),
                serde_json::json!({"stop_reason": "end_turn"}),
            );
        } else {
            mapped.insert("type".into(), Value::from(record_type.unwrap_or_default()));
            if let Some(uuid) = value.get("uuid") {
                mapped.insert("uuid".into(), uuid.clone());
            }
            if let Some(message) = value.get("message") {
                mapped.insert("message".into(), message.clone());
            }
            if let Some(version) = cx.file_state.get("observerVersion") {
                mapped.insert("version".into(), version.clone());
            }
            mapped.insert(
                "isSidechain".into(),
                Value::Bool(
                    value
                        .get("parent_tool_use_id")
                        .is_some_and(|v| !v.is_null()),
                ),
            );
        }

        let mapped_record = Record {
            source: record.source,
            file_key: record.file_key.clone(),
            offset: record.offset,
            value: Value::Object(mapped),
        };
        let mut signals = ClaudeCode.parse(&mapped_record, cx);
        if record_type == Some("result") {
            if let Some(at) = signals.iter().find_map(|signal| match signal {
                Signal::Turn(turn) => Some(turn.at.clone()),
                _ => None,
            }) {
                signals.push(Signal::Observer(Observation::Finish {
                    task: TaskRef::Active(session_id.clone()),
                    at,
                    status: if value.get("is_error").and_then(Value::as_bool) == Some(true) {
                        "failed"
                    } else {
                        "completed"
                    }
                    .into(),
                    active_guard: true,
                }));
            }
        }
        let source_key = record.file_key.split('/').next().unwrap_or("collector");
        for signal in &mut signals {
            if let Signal::Observer(Observation::Start {
                source_key: key, ..
            }) = signal
            {
                *key = if ids::is_uuid(source_key) {
                    source_key
                } else {
                    "collector"
                }
                .into();
            }
        }
        // Cowork's explicit lifecycle uses prompts, assistant messages, and
        // results. Tool-result user records carry no additional Turn phase.
        if record_type == Some("user") {
            signals.retain(
                |signal| !matches!(signal, Signal::Turn(turn) if turn.phase == TurnPhase::Step),
            );
        }
        for signal in &mut signals {
            if let Signal::Turn(turn) = signal {
                turn.provider = "claude-cowork".into();
            }
        }
        signals
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use time::macros::datetime;

    fn parse(value: Value, state: &mut Map<String, Value>) -> Vec<Signal> {
        let record = Record {
            source: 0,
            file_key: "audit.jsonl".into(),
            offset: 0,
            value,
        };
        let mut cx = Cx {
            file_state: state,
            now: datetime!(2026-10-03 00:00:00 UTC),
        };
        ClaudeCowork.parse(&record, &mut cx)
    }

    fn phases(signals: &[Signal]) -> Vec<TurnPhase> {
        signals
            .iter()
            .filter_map(|signal| match signal {
                Signal::Turn(turn) => {
                    assert_eq!(turn.provider, "claude-cowork");
                    assert_eq!(turn.session_id, "33333333-3333-4333-8333-333333333333");
                    assert_eq!(turn.at, "2026-10-01T12:00:00.000Z");
                    Some(turn.phase.clone())
                }
                Signal::Event(_) | Signal::Observer(_) => None,
            })
            .collect()
    }

    fn audit(record_type: &str, session_id: &str, message: Value) -> Value {
        json!({
            "type": record_type,
            "session_id": session_id,
            "_audit_timestamp": "2026-10-01T12:00:00Z",
            "uuid": "prompt-1",
            "message": message,
        })
    }

    #[test]
    fn prompt_assistant_and_result_turns() {
        let mut state = Map::new();
        let session = "33333333-3333-4333-8333-333333333333";
        assert_eq!(
            phases(&parse(
                audit("user", session, json!({"content": "hi"})),
                &mut state
            )),
            [TurnPhase::Start]
        );
        assert_eq!(
            phases(&parse(
                audit("assistant", session, json!({"content": []})),
                &mut state
            )),
            [TurnPhase::Step]
        );
        assert_eq!(
            phases(&parse(
                audit("assistant", session, json!({"stop_reason": "end_turn"})),
                &mut state
            )),
            [TurnPhase::Stop]
        );
        assert_eq!(
            phases(&parse(
                audit(
                    "assistant",
                    session,
                    json!({"stop_reason": "stop_sequence"})
                ),
                &mut state
            )),
            [TurnPhase::Stop]
        );
        assert_eq!(
            phases(&parse(audit("result", session, Value::Null), &mut state)),
            [TurnPhase::Stop]
        );
        assert!(phases(&parse(
            audit(
                "user",
                session,
                json!({"content": [{"type": "tool_result"}]})
            ),
            &mut state,
        ))
        .is_empty());
    }

    #[test]
    fn first_valid_session_wins_and_sidechain_prompt_is_suppressed() {
        let mut state = Map::new();
        assert!(parse(
            audit("user", "invalid", json!({"content": "hi"})),
            &mut state
        )
        .is_empty());
        let session = "33333333-3333-4333-8333-333333333333";
        parse(audit("system", session, Value::Null), &mut state);
        let mut sidechain = audit(
            "user",
            "55555555-5555-4555-8555-555555555555",
            json!({"content": "hi"}),
        );
        sidechain["parent_tool_use_id"] = json!("tool-1");
        assert!(parse(sidechain, &mut state).is_empty());
        assert_eq!(state[SESSION_KEY], session);
    }

    #[test]
    fn future_result_has_no_turn() {
        let mut state = Map::new();
        let mut result = audit(
            "result",
            "33333333-3333-4333-8333-333333333333",
            Value::Null,
        );
        result["_audit_timestamp"] = json!("2026-10-03T00:05:00.001Z");
        assert!(parse(result, &mut state).is_empty());
    }
}

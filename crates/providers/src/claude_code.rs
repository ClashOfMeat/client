//! Claude Code transcript records.

use clashofmeat_core::{
    ids, Cx, Detect, Event, EventKind, Manifest, Provider, Record, Signal, Source, SourceFormat,
    Turn, TurnPhase,
};
use serde_json::Value;
use time::{
    format_description::well_known::Rfc3339, macros::format_description, Date, OffsetDateTime,
    UtcOffset,
};

pub const MAX_DEPTH: usize = 4;

pub struct ClaudeCode;

static MANIFEST: Manifest = Manifest {
    id: "claude-code",
    name: "Claude Code",
    detect: Detect {
        commands: &["claude"],
        apps: &[],
        paths: &["$CLAUDE_CONFIG_DIR|~/.claude"],
    },
    sources: &[Source {
        root: "$CLAUDE_CONFIG_DIR/projects|~/.claude/projects",
        glob: "**/*.jsonl",
        format: SourceFormat::Jsonl,
    }],
    wiring: &[],
};

const TOKEN_KEYS: [(&str, &str); 4] = [
    ("input_tokens", "input_tokens"),
    ("output_tokens", "output_tokens"),
    ("cache_read_input_tokens", "cache_read_tokens"),
    ("cache_creation_input_tokens", "cache_write_tokens"),
];

fn truthy_string(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null | Value::Bool(false)) => false,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|number| number != 0.0),
        Some(Value::String(string)) => !string.is_empty(),
        Some(Value::Bool(true) | Value::Array(_) | Value::Object(_)) => true,
    }
}

fn js_number(value: Option<&Value>) -> u64 {
    const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
    value
        .and_then(Value::as_f64)
        .filter(|number| {
            number.is_finite()
                && *number >= 0.0
                && number.fract() == 0.0
                && *number <= MAX_SAFE_INTEGER
        })
        .map_or(0, |number| number as u64)
}

fn timestamp(value: &Value) -> Option<OffsetDateTime> {
    match value {
        Value::String(raw) => OffsetDateTime::parse(raw, &Rfc3339).ok().or_else(|| {
            Date::parse(raw, &format_description!("[year]-[month]-[day]"))
                .ok()
                .map(|date| date.midnight().assume_utc())
        }),
        Value::Number(number) => {
            let milliseconds = number.as_f64()?;
            if !milliseconds.is_finite() || milliseconds.abs() > 8_640_000_000_000_000.0 {
                return None;
            }
            OffsetDateTime::from_unix_timestamp_nanos(milliseconds.trunc() as i128 * 1_000_000).ok()
        }
        Value::Null => OffsetDateTime::from_unix_timestamp(0).ok(),
        Value::Bool(boolean) => {
            OffsetDateTime::from_unix_timestamp_nanos(i128::from(*boolean) * 1_000_000).ok()
        }
        Value::Array(_) | Value::Object(_) => None,
    }
}

fn iso_millis(date: OffsetDateTime) -> Option<String> {
    date.to_offset(UtcOffset::UTC)
        .format(&format_description!(
            "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
        ))
        .ok()
}

fn has_tool_result(content: &Value) -> bool {
    content.as_array().is_some_and(|blocks| {
        blocks
            .iter()
            .any(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
    })
}

impl Provider for ClaudeCode {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn parse(&self, record: &Record, cx: &mut Cx) -> Vec<Signal> {
        let value = &record.value;
        let Some(session_id) = value
            .get("sessionId")
            .and_then(Value::as_str)
            .filter(|id| ids::is_uuid(id))
        else {
            return Vec::new();
        };
        let Some(date) = value.get("timestamp").and_then(timestamp) else {
            return Vec::new();
        };
        if date.unix_timestamp_nanos() > cx.now.unix_timestamp_nanos() + 300_000_000_000 {
            return Vec::new();
        }
        let Some(at) = iso_millis(date) else {
            return Vec::new();
        };
        let mut signals = Vec::new();
        let message = value.get("message").unwrap_or(&Value::Null);
        let content = message.get("content").unwrap_or(&Value::Null);
        let record_type = value.get("type").and_then(Value::as_str);
        let uuid = truthy_string(value.get("uuid"));
        let human = record_type == Some("user")
            && !truthy(value.get("isSidechain"))
            && !truthy(value.get("isMeta"))
            && !truthy(value.get("isSynthetic"))
            && uuid.is_some()
            && (content.is_string()
                || content
                    .as_array()
                    .is_some_and(|blocks| !blocks.is_empty() && !has_tool_result(content)));

        let mut put = |identity: String, key: &str, amount: u64, model: Option<&str>| {
            signals.push(Signal::Event(Event {
                identity,
                key: key.to_owned(),
                value: amount,
                kind: EventKind::Counter,
                session_id: session_id.to_owned(),
                at: at.clone(),
                resets_at: None,
                model: model.map(str::to_owned),
                provider: Some("claude".to_owned()),
            }));
        };

        if human {
            let prompt = format!("prompt:{}", uuid.expect("human prompt has uuid"));
            put(format!("session:{session_id}"), "session_count", 1, None);
            put(prompt.clone(), "prompt_count", 1, None);
            let chars = if let Some(text) = content.as_str() {
                text.encode_utf16().count() as u64
            } else {
                content.as_array().map_or(0, |blocks| {
                    blocks
                        .iter()
                        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                        .filter_map(|block| block.get("text").and_then(Value::as_str))
                        .map(|text| text.encode_utf16().count() as u64)
                        .sum()
                })
            };
            put(prompt, "prompt_chars", chars, None);
        }
        if record_type == Some("assistant") {
            if let (Some(id), Some(usage)) = (
                truthy_string(message.get("id")),
                message.get("usage").filter(|usage| truthy(Some(usage))),
            ) {
                let identity = format!("request:{id}");
                put(identity.clone(), "model_step_count", 1, None);
                let model = ids::model_family(message.get("model").and_then(Value::as_str));
                for (field, key) in TOKEN_KEYS {
                    put(
                        identity.clone(),
                        key,
                        js_number(usage.get(field)),
                        Some(model),
                    );
                }
            }
            if let Some(blocks) = content.as_array() {
                for block in blocks {
                    if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                        if let Some(id) = truthy_string(block.get("id")) {
                            put(format!("tool:{id}"), "tool_call_count", 1, None);
                            if matches!(
                                block.get("name").and_then(Value::as_str),
                                Some("Agent" | "Task")
                            ) {
                                put(format!("agent:{id}"), "subagent_count", 1, None);
                            }
                        }
                    }
                }
            }
        }
        if record_type == Some("system")
            && value.get("subtype").and_then(Value::as_str) == Some("compact_boundary")
        {
            if let Some(uuid) = uuid {
                put(format!("compact:{uuid}"), "compaction_count", 1, None);
            }
        }

        let phase = if human {
            Some(TurnPhase::Start)
        } else if record_type == Some("assistant") {
            Some(
                if matches!(
                    message.get("stop_reason").and_then(Value::as_str),
                    Some("end_turn" | "stop_sequence")
                ) {
                    TurnPhase::Stop
                } else {
                    TurnPhase::Step
                },
            )
        } else if record_type == Some("user") && has_tool_result(content) {
            Some(TurnPhase::Step)
        } else {
            None
        };
        if let Some(phase) = phase {
            signals.push(Signal::Turn(Turn {
                provider: "claude-code".to_owned(),
                session_id: session_id.to_owned(),
                phase,
                at,
            }));
        }
        signals
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Map};
    use time::macros::datetime;

    fn signals(value: Value) -> Vec<Signal> {
        let record = Record {
            source: 0,
            file_key: "test.jsonl".into(),
            offset: 0,
            value,
        };
        let mut state = Map::new();
        let mut cx = Cx {
            file_state: &mut state,
            now: datetime!(2026-10-03 00:00:00 UTC),
        };
        ClaudeCode.parse(&record, &mut cx)
    }

    fn phase(signals: &[Signal]) -> Option<TurnPhase> {
        signals.iter().find_map(|signal| match signal {
            Signal::Turn(turn) => {
                assert_eq!(turn.provider, "claude-code");
                assert_eq!(turn.session_id, "11111111-1111-4111-8111-111111111111");
                assert_eq!(turn.at, "2026-10-02T12:00:00.000Z");
                Some(turn.phase.clone())
            }
            Signal::Event(_) => None,
        })
    }

    fn base(record_type: &str, message: Value) -> Value {
        json!({
            "type": record_type,
            "sessionId": "11111111-1111-4111-8111-111111111111",
            "timestamp": "2026-10-02T12:00:00Z",
            "uuid": "prompt-1",
            "message": message,
        })
    }

    #[test]
    fn human_prompt_starts_turn() {
        assert_eq!(
            phase(&signals(base("user", json!({"content": "hello"})))),
            Some(TurnPhase::Start)
        );
    }

    #[test]
    fn assistant_tool_use_steps_turn() {
        assert_eq!(
            phase(&signals(base(
                "assistant",
                json!({"content": [{"type":"tool_use", "id":"tool-1"}]})
            ))),
            Some(TurnPhase::Step)
        );
    }

    #[test]
    fn user_tool_result_steps_turn() {
        assert_eq!(
            phase(&signals(base(
                "user",
                json!({"content": [{"type":"tool_result", "tool_use_id":"tool-1"}]})
            ))),
            Some(TurnPhase::Step)
        );
    }

    #[test]
    fn assistant_end_turn_stops_turn() {
        assert_eq!(
            phase(&signals(base(
                "assistant",
                json!({"stop_reason":"end_turn"})
            ))),
            Some(TurnPhase::Stop)
        );
        assert_eq!(
            phase(&signals(base(
                "assistant",
                json!({"stop_reason":"stop_sequence"})
            ))),
            Some(TurnPhase::Stop)
        );
    }

    #[test]
    fn invalid_session_has_no_signals() {
        let mut value = base("user", json!({"content": "hello"}));
        value["sessionId"] = json!("invalid");
        assert!(signals(value).is_empty());
    }

    #[test]
    fn future_timestamp_has_no_signals() {
        let mut value = base("user", json!({"content": "hello"}));
        value["timestamp"] = json!("2026-10-03T00:05:00.001Z");
        assert!(signals(value).is_empty());
    }

    #[test]
    fn prompt_chars_count_utf16_units() {
        let signals = signals(base("user", json!({"content": "a😀"})));
        assert!(signals.iter().any(|signal| matches!(signal, Signal::Event(event) if event.key == "prompt_chars" && event.value == 3)));
    }
}

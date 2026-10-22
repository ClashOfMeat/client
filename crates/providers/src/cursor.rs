//! Content-free Cursor hook spool records.

use clashofmeat_core::{
    ids, observer, Cx, Detect, Event, EventKind, Manifest, Observation, Provider, Record, Signal,
    Source, SourceFormat, TaskRef, Turn, TurnPhase, Usage,
};
use serde_json::Value;
use time::{
    format_description::well_known::Rfc3339, macros::format_description, Date, OffsetDateTime,
    UtcOffset,
};

pub struct Cursor;

static MANIFEST: Manifest = Manifest {
    id: "cursor",
    name: "Cursor",
    detect: Detect {
        commands: &[],
        apps: &["Cursor"],
        paths: &["~/.cursor"],
    },
    sources: &[Source {
        root: "$CLASHOFMEAT_HOME/spool|~/.clashofmeat/spool",
        glob: "cursor.jsonl",
        format: SourceFormat::Jsonl,
    }],
    wiring: &["~/.cursor/hooks.json"],
};

fn printable_id(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str).filter(|text| {
        !text.is_empty()
            && text.len() <= 128
            && text.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':' | b'-')
            })
    })
}

// JavaScript's Number.isSafeInteger(value) && value >= 0 ? value : 0.
fn count(value: Option<&Value>) -> u64 {
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

// task-observer.mjs interprets numeric values below 1e12 as Unix seconds.
fn timestamp(value: &Value) -> Option<OffsetDateTime> {
    match value {
        Value::String(raw) => OffsetDateTime::parse(raw, &Rfc3339).ok().or_else(|| {
            Date::parse(raw, &format_description!("[year]-[month]-[day]"))
                .ok()
                .map(|date| date.midnight().assume_utc())
        }),
        Value::Number(number) => {
            let value = number.as_f64()?;
            let milliseconds = if value < 1e12 { value * 1000.0 } else { value };
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

fn iso(value: &Value) -> Option<String> {
    timestamp(value)?
        .to_offset(UtcOffset::UTC)
        .format(&format_description!(
            "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
        ))
        .ok()
}

fn event(identity: &str, session_id: &str, at: &str, key: &str, value: u64) -> Signal {
    Signal::Event(Event {
        identity: format!("cursor:{identity}"),
        key: key.to_owned(),
        value,
        kind: EventKind::Counter,
        session_id: session_id.to_owned(),
        at: at.to_owned(),
        resets_at: None,
        model: None,
        provider: Some("cursor".to_owned()),
    })
}

fn turn(session_id: &str, at: &str, phase: TurnPhase) -> Signal {
    Signal::Turn(Turn {
        provider: "cursor".to_owned(),
        session_id: session_id.to_owned(),
        phase,
        at: at.to_owned(),
    })
}

impl Provider for Cursor {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn parse(&self, record: &Record, _cx: &mut Cx) -> Vec<Signal> {
        let value = &record.value;
        let (Some(at), Some(conversation), Some(generation)) = (
            value.get("at").and_then(iso),
            printable_id(value.get("conversation")),
            printable_id(value.get("generation")),
        ) else {
            return Vec::new();
        };
        let session_id = ids::session_uuid("cursor", conversation);
        let mut signals = Vec::new();
        let task = TaskRef::Key(generation.into());
        let event_type = value.get("event").and_then(Value::as_str);
        if event_type == Some("beforeSubmitPrompt") {
            signals.push(Signal::Observer(Observation::Start {
                context: conversation.into(),
                task_key: generation.into(),
                source_key: "collector".into(),
                session_id: conversation.into(),
                at: at.clone(),
                client_version: value
                    .get("version")
                    .and_then(Value::as_str)
                    .map(observer::client_label)
                    .unwrap_or("unknown")
                    .into(),
                close_previous_completed: false,
            }));
        }
        if event_type == Some("postToolUse") {
            let name = value.get("tool").and_then(Value::as_str).unwrap_or("");
            signals.push(Signal::Observer(Observation::Tool {
                task: task.clone(),
                tool_id: format!("{generation}:{}", record.offset)
                    .chars()
                    .take(128)
                    .collect(),
                category: observer::category(name).into(),
                subagent: value.get("subagent").and_then(Value::as_bool) == Some(true),
                at: at.clone(),
            }));
        }
        if !matches!(event_type, Some("beforeSubmitPrompt" | "postToolUse")) {
            let cache_read = count(value.get("cache_read_tokens"));
            let cache_write = count(value.get("cache_write_tokens"));
            if [
                "input_tokens",
                "output_tokens",
                "cache_read_tokens",
                "cache_write_tokens",
            ]
            .iter()
            .any(|key| value.get(*key).is_some())
            {
                signals.push(Signal::Observer(Observation::Request {
                    task: task.clone(),
                    response_id: generation.into(),
                    usage: Usage {
                        input: count(value.get("input_tokens"))
                            .saturating_sub(cache_read)
                            .saturating_sub(cache_write),
                        output: count(value.get("output_tokens")),
                        cache_read,
                        cache_write,
                        reasoning: 0,
                    },
                    model: value
                        .get("model")
                        .and_then(Value::as_str)
                        .map(|model| observer::model_label(model, "cursor")),
                    at: at.clone(),
                }));
            }
            if let Some(model) = value.get("model").and_then(Value::as_str) {
                signals.push(Signal::Observer(Observation::Model {
                    task: task.clone(),
                    model: observer::model_label(model, "cursor"),
                }));
            }
            if event_type == Some("stop") {
                let status = match value.get("status").and_then(Value::as_str) {
                    Some("aborted") => "interrupted",
                    Some("error") => "failed",
                    _ => "completed",
                };
                signals.push(Signal::Observer(Observation::Finish {
                    task,
                    at: at.clone(),
                    status: status.into(),
                    active_guard: false,
                }));
            }
        }

        if !matches!(
            value.get("event").and_then(Value::as_str),
            Some("beforeSubmitPrompt" | "postToolUse")
        ) {
            let cache_read = count(value.get("cache_read_tokens"));
            let cache_write = count(value.get("cache_write_tokens"));
            let counts = [
                (
                    "input_tokens",
                    value.get("input_tokens").map(|_| {
                        count(value.get("input_tokens"))
                            .saturating_sub(cache_read)
                            .saturating_sub(cache_write)
                    }),
                ),
                (
                    "output_tokens",
                    value.get("output_tokens").map(|v| count(Some(v))),
                ),
                (
                    "cache_read_tokens",
                    value.get("cache_read_tokens").map(|_| cache_read),
                ),
                (
                    "cache_write_tokens",
                    value.get("cache_write_tokens").map(|_| cache_write),
                ),
            ];
            for (key, amount) in counts {
                if let Some(amount) = amount.filter(|amount| *amount <= ids::MAX_OBSERVATION_VALUE)
                {
                    signals.push(event(generation, &session_id, &at, key, amount));
                }
            }
        }

        // Only the stop hook ends the turn; the prompt, tool and response hooks keep it busy.
        signals.push(turn(&session_id, &at, TurnPhase::Step));
        if event_type == Some("stop") {
            signals.push(turn(&session_id, &at, TurnPhase::Stop));
        }
        signals
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Map};
    use time::macros::datetime;

    fn parse(value: Value) -> Vec<Signal> {
        Cursor.parse(
            &Record {
                source: 0,
                file_key: "cursor.jsonl".into(),
                offset: 0,
                value,
            },
            &mut Cx {
                file_state: &mut Map::new(),
                now: datetime!(2026-10-03 00:00:00 UTC),
            },
        )
    }

    fn base(event: &str) -> Value {
        json!({
            "event": event,
            "at": "2026-10-02T12:00:00Z",
            "conversation": "conversation-1",
            "generation": "generation-1",
        })
    }

    #[test]
    fn valid_spool_record_steps_and_stops_session() {
        let signals = parse(base("stop"));
        let turns: Vec<_> = signals
            .iter()
            .filter_map(|signal| match signal {
                Signal::Turn(turn) => Some(turn),
                Signal::Event(_) | Signal::Observer(_) => None,
            })
            .collect();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].phase, TurnPhase::Step);
        assert_eq!(turns[1].phase, TurnPhase::Stop);
        for turn in turns {
            assert_eq!(turn.provider, "cursor");
            assert_eq!(
                turn.session_id,
                ids::session_uuid("cursor", "conversation-1")
            );
            assert_eq!(turn.at, "2026-10-02T12:00:00.000Z");
        }
    }

    #[test]
    fn hooks_before_stop_keep_the_session_busy() {
        for hook in ["beforeSubmitPrompt", "postToolUse", "afterAgentResponse"] {
            let phases: Vec<_> = parse(base(hook))
                .into_iter()
                .filter_map(|signal| match signal {
                    Signal::Turn(turn) => Some(turn.phase),
                    Signal::Event(_) | Signal::Observer(_) => None,
                })
                .collect();
            assert_eq!(phases, vec![TurnPhase::Step], "{hook}");
        }
    }

    #[test]
    fn invalid_timestamp_or_ids_emit_nothing() {
        let mut record = base("stop");
        record["at"] = json!("invalid");
        assert!(parse(record).is_empty());
        let mut record = base("stop");
        record["conversation"] = json!("bad id");
        assert!(parse(record).is_empty());
        let mut record = base("stop");
        record["generation"] = json!("é");
        assert!(parse(record).is_empty());
    }

    #[test]
    fn prompt_and_tool_hooks_have_no_token_events() {
        for hook in ["beforeSubmitPrompt", "postToolUse"] {
            let mut record = base(hook);
            record["input_tokens"] = json!(10);
            assert!(parse(record)
                .iter()
                .all(|signal| !matches!(signal, Signal::Event(_))));
        }
    }

    #[test]
    fn present_invalid_counts_become_zero_and_oversized_counts_are_skipped() {
        let mut record = base("stop");
        record["input_tokens"] = json!(40);
        record["cache_read_tokens"] = json!(30);
        record["cache_write_tokens"] = json!(20);
        record["output_tokens"] = json!(1_000_000_001_u64);
        let events: Vec<_> = parse(record)
            .into_iter()
            .filter_map(|signal| match signal {
                Signal::Event(event) => Some(event),
                Signal::Turn(_) | Signal::Observer(_) => None,
            })
            .collect();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].key, "input_tokens");
        assert_eq!(events[0].value, 0);
        assert_eq!(events[0].identity, "cursor:generation-1");
        assert_eq!(events[0].provider.as_deref(), Some("cursor"));
    }
}

//! Gemini CLI chat recordings, including legacy single-document sessions.

use clashofmeat_core::{
    ids, observer, Cx, Detect, Event, EventKind, Manifest, Observation, Provider, Record, Signal,
    Source, SourceFormat, TaskRef, Turn, TurnPhase, Usage,
};
use serde_json::Value;
use time::{
    format_description::well_known::Rfc3339, macros::format_description, Date, OffsetDateTime,
    UtcOffset,
};

pub struct Gemini;

static MANIFEST: Manifest = Manifest {
    id: "gemini",
    name: "Gemini CLI",
    detect: Detect {
        commands: &["gemini"],
        apps: &[],
        paths: &["$GEMINI_CLI_HOME/.gemini|~/.gemini"],
    },
    sources: &[
        Source {
            root: "$GEMINI_CLI_HOME/.gemini/tmp|~/.gemini/tmp",
            glob: "**/chats/**/*.jsonl",
            format: SourceFormat::Jsonl,
        },
        Source {
            root: "$GEMINI_CLI_HOME/.gemini/tmp|~/.gemini/tmp",
            glob: "**/chats/**/*.json",
            format: SourceFormat::Json,
        },
    ],
    wiring: &[],
};

fn printable_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.:-".contains(&byte))
}

fn count(value: Option<&Value>) -> u64 {
    const MAX_SAFE: f64 = 9_007_199_254_740_991.0;
    value
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite() && *n >= 0.0 && n.fract() == 0.0 && *n <= MAX_SAFE)
        .map_or(0, |n| n as u64)
}

fn timestamp(value: &Value) -> Option<OffsetDateTime> {
    match value {
        Value::String(raw) => OffsetDateTime::parse(raw, &Rfc3339).ok().or_else(|| {
            Date::parse(raw, &format_description!("[year]-[month]-[day]"))
                .ok()
                .map(|date| date.midnight().assume_utc())
        }),
        Value::Number(number) => {
            let number = number.as_f64()?;
            let millis = if number < 1e12 {
                number * 1000.0
            } else {
                number
            };
            if !millis.is_finite() || millis.abs() > 8_640_000_000_000_000.0 {
                return None;
            }
            OffsetDateTime::from_unix_timestamp_nanos(millis.trunc() as i128 * 1_000_000).ok()
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

fn begin(record: &Record, cx: &mut Cx) {
    if !cx.file_state.contains_key("subagent") {
        let parent = record.file_key.rsplit('/').nth(1);
        cx.file_state.insert(
            "subagent".into(),
            Value::Bool(parent.is_some_and(|parent| parent != "chats")),
        );
    }
    if cx.file_state.get("sessionId").is_none()
        && cx.file_state.get("subagent").and_then(Value::as_bool) == Some(true)
    {
        let filename = record.file_key.rsplit('/').next().unwrap_or_default();
        let session_id = filename
            .strip_suffix(".jsonl")
            .or_else(|| filename.strip_suffix(".json"))
            .unwrap_or(filename);
        cx.file_state
            .insert("sessionId".into(), Value::String(session_id.to_owned()));
    }
}

fn observe(message: &Value, cx: &Cx, record: &Record) -> Vec<Signal> {
    let Some(id) = message
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| printable_id(id))
    else {
        return Vec::new();
    };
    let Some(at) = message.get("timestamp").and_then(iso) else {
        return Vec::new();
    };
    let Some(session_id) = cx.file_state.get("sessionId").and_then(Value::as_str) else {
        return Vec::new();
    };
    let session_uuid = ids::session_uuid("gemini", session_id);
    let message_type = message.get("type").and_then(Value::as_str);
    let phase = match message_type {
        Some("user") => Some(TurnPhase::Start),
        Some("gemini") => Some(TurnPhase::Step),
        _ => None,
    };
    let mut signals = Vec::new();
    let task_key: String = format!("{session_id}:{id}").chars().take(128).collect();
    let context = ids::stable_id(&format!(
        "gemini-file:{}:{}",
        record.source, record.file_key
    ));
    let task = TaskRef::Active(context.clone());
    let subagent = cx.file_state.get("subagent").and_then(Value::as_bool) == Some(true);
    if message_type == Some("user") && !subagent {
        signals.push(Signal::Observer(Observation::Start {
            context,
            task_key: task_key.clone(),
            source_key: "collector".into(),
            session_id: session_id.into(),
            at: at.clone(),
            client_version: "unknown".into(),
            close_previous_completed: true,
        }));
    }
    if let Some(phase) = phase {
        signals.push(Signal::Turn(Turn {
            provider: "gemini".into(),
            session_id: session_uuid.clone(),
            phase,
            at: at.clone(),
        }));
    }
    if message_type != Some("gemini") {
        return signals;
    }
    if !subagent {
        if let Some(tokens) = message
            .get("tokens")
            .filter(|tokens| tokens.is_object() || tokens.is_array())
        {
            let input = count(tokens.get("input"));
            let cached = count(tokens.get("cached"));
            let thoughts = count(tokens.get("thoughts"));
            signals.push(Signal::Observer(Observation::Request {
                task: task.clone(),
                response_id: id.into(),
                usage: Usage {
                    input: input.saturating_sub(cached) + count(tokens.get("tool")),
                    output: count(tokens.get("output")) + thoughts,
                    cache_read: cached,
                    cache_write: 0,
                    reasoning: thoughts,
                },
                model: message
                    .get("model")
                    .and_then(Value::as_str)
                    .map(|model| observer::model_label(model, "gemini")),
                at: at.clone(),
            }));
        }
        if let Some(calls) = message.get("toolCalls").and_then(Value::as_array) {
            for call in calls {
                if let Some(id) = call.get("id").and_then(Value::as_str) {
                    let name = call.get("name").and_then(Value::as_str).unwrap_or("");
                    signals.push(Signal::Observer(Observation::Tool {
                        task: task.clone(),
                        tool_id: id.into(),
                        category: observer::category(name).into(),
                        subagent: ["subagent", "delegate"]
                            .iter()
                            .any(|part| name.to_ascii_lowercase().contains(part)),
                        at: at.clone(),
                    }));
                }
            }
        }
    }
    let Some(tokens) = message
        .get("tokens")
        .filter(|tokens| tokens.is_object() || tokens.is_array())
    else {
        return signals;
    };
    let input = count(tokens.get("input"));
    let cached = count(tokens.get("cached"));
    let counts = [
        (
            "input_tokens",
            input.saturating_sub(cached) + count(tokens.get("tool")),
        ),
        (
            "output_tokens",
            count(tokens.get("output")) + count(tokens.get("thoughts")),
        ),
        ("cache_read_tokens", cached),
        ("cache_write_tokens", 0),
    ];
    for (key, value) in counts {
        if value > ids::MAX_OBSERVATION_VALUE {
            continue;
        }
        signals.push(Signal::Event(Event {
            identity: format!("gemini:{task_key}"),
            key: key.into(),
            value,
            kind: EventKind::Counter,
            session_id: session_uuid.clone(),
            at: at.clone(),
            resets_at: None,
            model: None,
            provider: Some("gemini".into()),
        }));
    }
    signals
}

impl Provider for Gemini {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn parse(&self, record: &Record, cx: &mut Cx) -> Vec<Signal> {
        begin(record, cx);
        match MANIFEST
            .sources
            .get(record.source)
            .map(|source| source.format)
        {
            Some(SourceFormat::Jsonl) => {
                let value = &record.value;
                if ["$set", "$patch", "$rewindTo"]
                    .iter()
                    .any(|key| value.get(*key).is_some_and(truthy))
                {
                    return Vec::new();
                }
                if let Some(session_id) = value.get("sessionId").and_then(Value::as_str) {
                    if value.get("id").is_none() {
                        if printable_id(session_id) {
                            cx.file_state
                                .insert("sessionId".into(), Value::String(session_id.into()));
                        }
                        return Vec::new();
                    }
                }
                observe(value, cx, record)
            }
            Some(SourceFormat::Json) => {
                if let Some(session_id) = record
                    .value
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .filter(|id| printable_id(id))
                {
                    cx.file_state
                        .insert("sessionId".into(), Value::String(session_id.into()));
                }
                let messages = record.value.get("messages").and_then(Value::as_array);
                let seen = cx
                    .file_state
                    .get("seen")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let start = usize::try_from(seen.saturating_sub(1)).unwrap_or(usize::MAX);
                let signals = messages
                    .into_iter()
                    .flatten()
                    .skip(start)
                    .flat_map(|message| observe(message, cx, record))
                    .collect();
                cx.file_state
                    .insert("seen".into(), Value::from(messages.map_or(0, Vec::len)));
                signals
            }
            _ => Vec::new(),
        }
    }
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => false,
        Value::Number(number) => number.as_f64().is_some_and(|n| n != 0.0),
        Value::String(string) => !string.is_empty(),
        Value::Bool(true) | Value::Array(_) | Value::Object(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Map};
    use time::macros::datetime;

    fn parse(values: &[Value]) -> Vec<Signal> {
        let mut state = Map::new();
        let mut signals = Vec::new();
        for value in values {
            let record = Record {
                source: 0,
                file_key: "project/chats/session-test.jsonl".into(),
                offset: 0,
                value: value.clone(),
            };
            let mut cx = Cx {
                file_state: &mut state,
                now: datetime!(2026-10-03 00:00:00 UTC),
            };
            signals.extend(Gemini.parse(&record, &mut cx));
        }
        signals
    }

    #[test]
    fn user_starts_and_model_steps_without_inventing_completion() {
        let signals = parse(&[
            json!({"sessionId":"aaaaaaaa-1111-4111-8111-111111111111"}),
            json!({"id":"user-1","type":"user","timestamp":"2026-10-01T08:00:00Z","content":"private"}),
            json!({"id":"model-1","type":"gemini","timestamp":"2026-10-01T08:00:01Z","content":"private"}),
        ]);
        let turns: Vec<_> = signals
            .iter()
            .filter_map(|signal| match signal {
                Signal::Turn(turn) => {
                    assert_eq!(turn.provider, "gemini");
                    assert_eq!(turn.session_id, "aaaaaaaa-1111-4111-8111-111111111111");
                    Some(turn.phase.clone())
                }
                Signal::Event(_) | Signal::Observer(_) => None,
            })
            .collect();
        assert_eq!(turns, vec![TurnPhase::Start, TurnPhase::Step]);
        assert_eq!(
            signals
                .iter()
                .filter(|signal| !matches!(signal, Signal::Observer(_)))
                .count(),
            2
        );
    }

    #[test]
    fn invalid_ids_control_records_and_bad_dates_are_gated() {
        let signals = parse(&[
            json!({"sessionId":"aaaaaaaa-1111-4111-8111-111111111111"}),
            json!({"id":"bad id","type":"user","timestamp":"2026-10-01T08:00:00Z"}),
            json!({"$set":{"id":"user-1"},"id":"user-1","type":"user","timestamp":"2026-10-01T08:00:00Z"}),
            json!({"id":"user-2","type":"user","timestamp":"invalid"}),
        ]);
        assert!(signals.is_empty());
    }

    #[test]
    fn unsafe_counts_become_zero_and_oversized_metrics_are_skipped() {
        let signals = parse(&[
            json!({"sessionId":"aaaaaaaa-1111-4111-8111-111111111111"}),
            json!({"id":"model-1","type":"gemini","timestamp":"2026-10-01T08:00:01Z","tokens":{"input":1000000001,"cached":-5,"output":2.5,"thoughts":3}}),
        ]);
        let events: Vec<_> = signals
            .iter()
            .filter_map(|signal| match signal {
                Signal::Event(event) => Some((event.key.as_str(), event.value)),
                Signal::Turn(_) | Signal::Observer(_) => None,
            })
            .collect();
        assert_eq!(
            events,
            vec![
                ("output_tokens", 3),
                ("cache_read_tokens", 0),
                ("cache_write_tokens", 0)
            ]
        );
    }
}

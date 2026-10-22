//! Codex rollout records from active and archived sessions.

use clashofmeat_core::{
    ids, observer, Cx, Detect, Event, EventKind, Manifest, Observation, Provider, Record, Signal,
    Source, SourceFormat, TaskRef, Turn, TurnPhase, Usage,
};
use serde_json::Value;
use time::{
    format_description::well_known::Rfc3339, macros::format_description, Date, OffsetDateTime,
    UtcOffset,
};

pub struct Codex;

static MANIFEST: Manifest = Manifest {
    id: "codex",
    name: "Codex",
    detect: Detect {
        commands: &["codex"],
        apps: &["Codex"],
        paths: &["$CODEX_HOME|~/.codex"],
    },
    sources: &[
        Source {
            root: "$CODEX_HOME/sessions|~/.codex/sessions",
            glob: "**/*.jsonl",
            format: SourceFormat::Jsonl,
        },
        Source {
            root: "$CODEX_HOME/archived_sessions|~/.codex/archived_sessions",
            glob: "**/*.jsonl",
            format: SourceFormat::Jsonl,
        },
    ],
    wiring: &[],
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

// task-observer.mjs treats numeric values below 1e12 as Unix seconds.
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

fn context_key(record: &Record) -> String {
    ids::stable_id(&format!("codex-file:{}:{}", record.source, record.file_key))
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => false,
        Value::Number(number) => number.as_f64().is_some_and(|number| number != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Bool(true) | Value::Array(_) | Value::Object(_) => true,
    }
}

fn session_id(payload: &Value, cx: &Cx, record: &Record) -> String {
    let key = payload
        .get("session_id")
        .filter(|value| !value.is_null())
        .or_else(|| payload.get("thread_id").filter(|value| !value.is_null()))
        .and_then(Value::as_str)
        .or_else(|| cx.file_state.get("session_id").and_then(Value::as_str))
        .map_or_else(|| context_key(record), str::to_owned);
    ids::session_uuid("codex", &key)
}

fn turn(session_id: String, phase: TurnPhase, at: String) -> Signal {
    Signal::Turn(Turn {
        provider: "codex".into(),
        session_id,
        phase,
        at,
    })
}

impl Provider for Codex {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn parse(&self, record: &Record, cx: &mut Cx) -> Vec<Signal> {
        let value = &record.value;
        let payload = value.get("payload").unwrap_or(&Value::Null);
        let Some(at) = value
            .get("timestamp")
            .filter(|value| !value.is_null())
            .or_else(|| payload.get("started_at").filter(|value| !value.is_null()))
            .or_else(|| payload.get("completed_at"))
            .and_then(iso)
        else {
            return Vec::new();
        };
        let kind = value.get("type").and_then(Value::as_str);
        if kind == Some("session_meta") {
            if let Some(version) = payload
                .get("cli_version")
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
            }
            if let Some(id) = printable_id(payload.get("id")) {
                cx.file_state.insert("session_id".into(), Value::from(id));
            }
            let subagent = payload
                .get("source")
                .and_then(Value::as_object)
                .and_then(|source| source.get("subagent"))
                .is_some_and(truthy);
            cx.file_state
                .insert("subagent".into(), Value::Bool(subagent));
            return Vec::new();
        }

        let session_id = session_id(payload, cx, record);
        let context = context_key(record);
        let task = printable_id(payload.get("turn_id"))
            .map(|key| TaskRef::Key(key.into()))
            .unwrap_or_else(|| TaskRef::Active(context.clone()));
        if kind == Some("event_msg") {
            match payload.get("type").and_then(Value::as_str) {
                Some("task_started")
                    if printable_id(payload.get("turn_id")).is_some()
                        && cx.file_state.get("subagent") != Some(&Value::Bool(true)) =>
                {
                    let started = payload.get("started_at").and_then(iso).unwrap_or(at);
                    return vec![
                        turn(session_id.clone(), TurnPhase::Start, started.clone()),
                        Signal::Observer(Observation::Start {
                            context,
                            task_key: printable_id(payload.get("turn_id")).unwrap().into(),
                            source_key: "collector".into(),
                            session_id: cx
                                .file_state
                                .get("session_id")
                                .and_then(Value::as_str)
                                .unwrap_or(&session_id)
                                .into(),
                            at: started.clone(),
                            client_version: cx
                                .file_state
                                .get("observerVersion")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown")
                                .into(),
                            close_previous_completed: false,
                        }),
                    ];
                }
                Some("task_complete") => {
                    let completed = payload.get("completed_at").and_then(iso).unwrap_or(at);
                    return vec![
                        Signal::Observer(Observation::Finish {
                            task: TaskRef::Active(context),
                            at: completed.clone(),
                            status: "completed".into(),
                            active_guard: true,
                        }),
                        turn(session_id, TurnPhase::Stop, completed),
                    ];
                }
                Some("token_count") => {
                    let mut signals = Vec::new();
                    for (window, minutes, key, hit) in [
                        (
                            "primary",
                            300,
                            "five_hour_percent",
                            "five_hour_limit_hit_count",
                        ),
                        (
                            "secondary",
                            10080,
                            "seven_day_percent",
                            "seven_day_limit_hit_count",
                        ),
                    ] {
                        let limit = &payload["rate_limits"][window];
                        if limit["window_minutes"].as_u64() != Some(minutes) {
                            continue;
                        }
                        let Some(percent) = limit["used_percent"]
                            .as_f64()
                            .filter(|p| p.is_finite() && *p >= 0.0 && *p <= 100.0)
                        else {
                            continue;
                        };
                        let Some(reset) = limit["resets_at"]
                            .as_i64()
                            .and_then(|t| OffsetDateTime::from_unix_timestamp(t).ok())
                            .and_then(|t| t.format(&Rfc3339).ok())
                        else {
                            continue;
                        };
                        signals.push(Signal::Event(Event {
                            identity: format!("codex:quota:{session_id}:{window}:{at}"),
                            key: key.into(),
                            value: percent.round() as u64,
                            kind: EventKind::Reading,
                            session_id: session_id.clone(),
                            at: at.clone(),
                            resets_at: Some(reset.clone()),
                            model: None,
                            provider: Some("codex".into()),
                        }));
                        if percent >= 100.0 {
                            signals.push(Signal::Event(Event {
                                identity: format!("codex:quota-hit:{window}:{reset}"),
                                key: hit.into(),
                                value: 1,
                                kind: EventKind::Counter,
                                session_id: session_id.clone(),
                                at: at.clone(),
                                resets_at: None,
                                model: None,
                                provider: Some("codex".into()),
                            }));
                        }
                    }
                    return signals;
                }
                Some("turn_aborted") => {
                    return vec![
                        Signal::Observer(Observation::Finish {
                            task: TaskRef::Active(context),
                            at: at.clone(),
                            status: "interrupted".into(),
                            active_guard: true,
                        }),
                        turn(session_id, TurnPhase::Stop, at),
                    ]
                }
                _ => {}
            }
        }
        if kind == Some("response_item") {
            let mut signals = Vec::new();
            if payload["type"] == "message"
                && payload["role"] == "user"
                && cx.file_state.get("subagent") != Some(&Value::Bool(true))
            {
                let chars = payload["content"].as_array().map_or(0, |parts| {
                    parts
                        .iter()
                        .filter_map(|part| part["text"].as_str())
                        .map(|text| text.chars().count() as u64)
                        .sum()
                });
                if chars > 0 && chars <= ids::MAX_OBSERVATION_VALUE {
                    signals.push(Signal::Event(Event {
                        identity: format!("codex:prompt:{session_id}:{at}"),
                        key: "prompt_chars".into(),
                        value: chars,
                        kind: EventKind::Counter,
                        session_id: session_id.clone(),
                        at: at.clone(),
                        resets_at: None,
                        model: None,
                        provider: Some("codex".into()),
                    }));
                }
            }
            if matches!(
                payload.get("type").and_then(Value::as_str),
                Some("function_call" | "custom_tool_call")
            ) {
                if let Some(id) = printable_id(
                    payload
                        .get("call_id")
                        .filter(|v| !v.is_null())
                        .or_else(|| payload.get("id")),
                ) {
                    let name = payload.get("name").and_then(Value::as_str).unwrap_or("");
                    signals.push(Signal::Observer(Observation::Tool {
                        task,
                        tool_id: id.into(),
                        category: observer::category(name).into(),
                        subagent: ["spawn_agent", "delegate", "subagent"]
                            .iter()
                            .any(|part| name.to_ascii_lowercase().contains(part)),
                        at: at.clone(),
                    }));
                }
            }
            signals.push(turn(session_id, TurnPhase::Step, at));
            return signals;
        }
        if kind == Some("turn_context") {
            return payload
                .get("model")
                .and_then(Value::as_str)
                .map(|model| {
                    vec![Signal::Observer(Observation::Model {
                        task,
                        model: observer::model_label(model, "codex"),
                    })]
                })
                .unwrap_or_default();
        }
        if kind == Some("compacted") {
            let ordinal = value
                .get("ordinal")
                .filter(|v| !v.is_null())
                .map(|v| match v {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                })
                .unwrap_or_else(|| at.clone());
            return vec![Signal::Observer(Observation::Compaction {
                task,
                compaction_id: format!("compact:{ordinal}").chars().take(128).collect(),
                at,
            })];
        }
        if kind != Some("token_usage_record") {
            return Vec::new();
        }
        let (Some(response_id), Some(usage)) = (
            printable_id(payload.get("response_id")),
            payload
                .get("usage")
                .filter(|usage| usage.is_object() || usage.is_array()),
        ) else {
            return Vec::new();
        };
        let input = count(usage.get("input_tokens"));
        let cached = count(usage.get("cached_input_tokens"));
        let values = [
            ("input_tokens", input.saturating_sub(cached)),
            ("output_tokens", count(usage.get("output_tokens"))),
            ("cache_read_tokens", cached),
            (
                "cache_write_tokens",
                count(usage.get("cache_write_input_tokens")),
            ),
        ];
        let mut signals = Vec::with_capacity(5);
        signals.push(Signal::Observer(Observation::Request {
            task,
            response_id: response_id.into(),
            usage: Usage {
                input: input.saturating_sub(cached),
                output: count(usage.get("output_tokens")),
                cache_read: cached,
                cache_write: count(usage.get("cache_write_input_tokens")),
                reasoning: count(usage.get("reasoning_output_tokens")),
            },
            model: None,
            at: at.clone(),
        }));
        for (key, amount) in values {
            if amount > ids::MAX_OBSERVATION_VALUE {
                continue;
            }
            signals.push(Signal::Event(Event {
                identity: format!("codex:{response_id}"),
                key: key.into(),
                value: amount,
                kind: EventKind::Counter,
                session_id: session_id.clone(),
                at: at.clone(),
                resets_at: None,
                model: None,
                provider: Some("codex".into()),
            }));
        }
        signals.push(turn(session_id, TurnPhase::Step, at));
        signals
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Map};
    use time::macros::datetime;

    fn parse(value: Value, state: &mut Map<String, Value>) -> Vec<Signal> {
        Codex.parse(
            &Record {
                source: 0,
                file_key: "rollout.jsonl".into(),
                offset: 0,
                value,
            },
            &mut Cx {
                file_state: state,
                now: datetime!(2026-10-03 00:00:00 UTC),
            },
        )
    }

    fn phases(signals: &[Signal]) -> Vec<TurnPhase> {
        signals
            .iter()
            .filter_map(|signal| match signal {
                Signal::Turn(turn) => {
                    assert_eq!(turn.provider, "codex");
                    Some(turn.phase.clone())
                }
                Signal::Event(_) | Signal::Observer(_) => None,
            })
            .collect()
    }

    #[test]
    fn turn_lifecycle_and_session_state() {
        let mut state = Map::new();
        assert!(parse(json!({"type":"session_meta","timestamp":"2026-10-01T09:00:00Z","payload":{"id":"77777777-7777-4777-8777-777777777777","source":"cli"}}), &mut state).is_empty());
        let started = parse(
            json!({"type":"event_msg","timestamp":"2026-10-01T09:00:01Z","payload":{"type":"task_started","turn_id":"turn-1"}}),
            &mut state,
        );
        assert_eq!(phases(&started), vec![TurnPhase::Start]);
        let Signal::Turn(start) = &started[0] else {
            panic!("expected start")
        };
        assert_eq!(start.session_id, "77777777-7777-4777-8777-777777777777");
        let usage = parse(
            json!({"type":"token_usage_record","timestamp":"2026-10-01T09:00:02Z","payload":{"response_id":"resp-1","usage":{"input_tokens":10,"cached_input_tokens":3,"output_tokens":4}}}),
            &mut state,
        );
        assert_eq!(phases(&usage), vec![TurnPhase::Step]);
        assert_eq!(
            usage
                .iter()
                .filter(|signal| matches!(signal, Signal::Event(_)))
                .count(),
            4
        );
        assert_eq!(
            phases(&parse(
                json!({"type":"response_item","timestamp":"2026-10-01T09:00:03Z","payload":{"type":"message","text":"private"}}),
                &mut state
            )),
            vec![TurnPhase::Step]
        );
        assert_eq!(
            phases(&parse(
                json!({"type":"event_msg","timestamp":"2026-10-01T09:00:04Z","payload":{"type":"task_complete"}}),
                &mut state
            )),
            vec![TurnPhase::Stop]
        );
        assert_eq!(
            phases(&parse(
                json!({"type":"event_msg","timestamp":"2026-10-01T09:00:05Z","payload":{"type":"turn_aborted"}}),
                &mut state
            )),
            vec![TurnPhase::Stop]
        );
    }

    #[test]
    fn subagent_and_invalid_ids_do_not_start_turns_or_save_tokens() {
        let mut state = Map::new();
        parse(
            json!({"type":"session_meta","timestamp":"2026-10-01T09:00:00Z","payload":{"id":"session","source":{"subagent":{"parent_thread_id":"parent"}}}}),
            &mut state,
        );
        assert!(parse(json!({"type":"event_msg","timestamp":"2026-10-01T09:00:01Z","payload":{"type":"task_started","turn_id":"turn-1"}}), &mut state).is_empty());
        assert!(parse(json!({"type":"token_usage_record","timestamp":"2026-10-01T09:00:02Z","payload":{"response_id":"bad id","usage":{"input_tokens":10}}}), &mut state).is_empty());
    }

    #[test]
    fn invalid_counts_become_zero_and_oversized_counts_are_skipped() {
        let mut state = Map::new();
        let signals = parse(
            json!({"type":"token_usage_record","timestamp":1790845202,"payload":{"response_id":"resp-2","usage":{"input_tokens":1_000_000_001_u64,"cached_input_tokens":-1,"output_tokens":1.5,"cache_write_input_tokens":2}}}),
            &mut state,
        );
        let events: Vec<_> = signals
            .iter()
            .filter_map(|signal| match signal {
                Signal::Event(event) => Some(event),
                Signal::Turn(_) | Signal::Observer(_) => None,
            })
            .collect();
        assert_eq!(events.len(), 3);
        assert_eq!(
            events
                .iter()
                .find(|event| event.key == "output_tokens")
                .unwrap()
                .value,
            0
        );
        assert_eq!(events[0].at, "2026-10-01T09:00:02.000Z");
    }
}

#[cfg(test)]
mod quota_tests {
    use super::*;
    use serde_json::{json, Map};
    use time::macros::datetime;
    #[test]
    fn documented_quota_windows_produce_readings_and_idempotent_hits() {
        let value = json!({"type":"event_msg","timestamp":"2026-10-20T12:00:00Z","payload":{
        "type":"token_count","rate_limits":{
            "primary":{"used_percent":100.0,"window_minutes":300,"resets_at":1792501200},
            "secondary":{"used_percent":45.0,"window_minutes":10080,"resets_at":1793000000}
        }}});
        let record = Record {
            source: 0,
            file_key: "session.jsonl".into(),
            offset: 0,
            value,
        };
        let signals = Codex.parse(
            &record,
            &mut Cx {
                file_state: &mut Map::new(),
                now: datetime!(2026-10-20 12:00 UTC),
            },
        );
        let events: Vec<_> = signals
            .iter()
            .filter_map(|s| {
                if let Signal::Event(e) = s {
                    Some(e)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(events.len(), 3);
        assert_eq!(
            events
                .iter()
                .find(|e| e.key == "five_hour_percent")
                .unwrap()
                .value,
            100
        );
        assert_eq!(
            events
                .iter()
                .find(|e| e.key == "seven_day_percent")
                .unwrap()
                .value,
            45
        );
        assert_eq!(
            events
                .iter()
                .find(|e| e.key == "five_hour_limit_hit_count")
                .unwrap()
                .value,
            1
        );
        assert!(events
            .iter()
            .filter(|e| e.kind == EventKind::Reading)
            .all(|e| e.resets_at.is_some()));
    }
}

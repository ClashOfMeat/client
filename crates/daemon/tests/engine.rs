use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::time::{Duration as StdDuration, SystemTime};

use clashofmeat_core::{
    observer, Cx, Detect, Env, Event, EventKind, Manifest, Observation, Provider, Record, Signal,
    Source, SourceFormat, Store, Turn, TurnPhase,
};
use clashofmeat_daemon::engine::{Engine, SessionState};
use clashofmeat_providers::claude_code::ClaudeCode;
use clashofmeat_providers::codex::Codex;
use serde_json::json;
use tempfile::TempDir;
use time::{format_description::well_known::Rfc3339, macros::datetime, Duration, OffsetDateTime};

const SESSION: &str = "123e4567-e89b-12d3-a456-426614174000";
const PAIRED: &str = "2000-01-01T00:00:00.000Z";

struct JsonProvider;

static JSON_MANIFEST: Manifest = Manifest {
    id: "json-test",
    name: "JSON test",
    detect: Detect {
        commands: &[],
        apps: &[],
        paths: &[],
    },
    sources: &[Source {
        root: "~/json",
        glob: "**/*.json",
        format: SourceFormat::Json,
    }],
    wiring: &[],
};

impl Provider for JsonProvider {
    fn manifest(&self) -> &'static Manifest {
        &JSON_MANIFEST
    }

    fn parse(&self, record: &Record, _cx: &mut Cx) -> Vec<Signal> {
        let value = record.value["value"].as_u64().unwrap();
        let at = record.value["at"].as_str().unwrap().to_owned();
        let mut signals = vec![Signal::Event(Event {
            identity: format!("{}:{value}", record.file_key),
            key: "input_tokens".into(),
            value,
            kind: EventKind::Counter,
            session_id: SESSION.into(),
            at: at.clone(),
            resets_at: None,
            model: None,
            provider: Some("json-test".into()),
            agent_id: None,
        })];
        if let Some(phase) = record.value["phase"].as_str() {
            signals.push(Signal::Turn(Turn {
                provider: "json-test".into(),
                session_id: SESSION.into(),
                phase: match phase {
                    "start" => TurnPhase::Start,
                    "step" => TurnPhase::Step,
                    "stop" => TurnPhase::Stop,
                    _ => panic!("unexpected phase"),
                },
                at,
            }));
        }
        signals
    }
}

fn now() -> OffsetDateTime {
    datetime!(2026-10-20 12:00:00 UTC)
}
fn iso(at: OffsetDateTime) -> String {
    at.format(&Rfc3339).unwrap()
}

fn setup(paired: bool) -> (TempDir, Store, Engine) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open_in_memory().unwrap();
    if paired {
        store.set_meta("paired_at", PAIRED).unwrap();
    }
    let engine = Engine::new(
        Env::for_home(dir.path()),
        vec![Box::new(ClaudeCode), Box::new(JsonProvider)],
        "daemon-test",
    );
    (dir, store, engine)
}

#[test]
fn observer_signals_require_pairing_and_idle_finalization_requires_full_scan() {
    let (dir, mut store, mut engine) = setup(false);
    let path = claude_path(dir.path(), "observer.jsonl");
    let started = now() - Duration::minutes(11);
    fs::write(&path, prompt(started, "observer-prompt")).unwrap();
    engine.scan(&mut store, None, started).unwrap();
    assert!(observer::completed_payloads(&mut store, 10)
        .unwrap()
        .is_empty());
    store.set_meta("paired_at", PAIRED).unwrap();
    append(
        &path,
        &prompt(started + Duration::seconds(1), "paired-prompt"),
    );
    engine
        .scan(
            &mut store,
            Some(std::slice::from_ref(&path)),
            started + Duration::seconds(1),
        )
        .unwrap();
    engine
        .scan(&mut store, Some(std::slice::from_ref(&path)), now())
        .unwrap();
    assert!(observer::completed_payloads(&mut store, 10)
        .unwrap()
        .is_empty());
    engine.scan(&mut store, None, now()).unwrap();
    let tasks = observer::completed_payloads(&mut store, 10).unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0]["task"]["status"], "completed");
}

#[test]
fn codex_idle_tasks_wait_thirty_minutes() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open_in_memory().unwrap();
    store.set_meta("paired_at", PAIRED).unwrap();
    let started = now() - Duration::minutes(11);
    observer::apply(
        &mut store,
        "codex",
        &[Observation::Start {
            context: "codex-context".into(),
            task_key: "codex-task".into(),
            source_key: "collector".into(),
            session_id: SESSION.into(),
            at: iso(started),
            client_version: String::new(),
            close_previous_completed: false,
        }],
        "daemon-test",
    )
    .unwrap();
    let mut engine = Engine::new(
        Env::for_home(dir.path()),
        vec![Box::new(Codex)],
        "daemon-test",
    );
    engine.scan(&mut store, None, now()).unwrap();
    assert!(observer::completed_payloads(&mut store, 10)
        .unwrap()
        .is_empty());
    engine
        .scan(&mut store, None, started + Duration::minutes(31))
        .unwrap();
    assert_eq!(
        observer::completed_payloads(&mut store, 10).unwrap().len(),
        1
    );
}

fn claude_path(home: &Path, name: &str) -> std::path::PathBuf {
    let path = home.join(".claude/projects/project").join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    path
}

fn prompt(at: OffsetDateTime, uuid: &str) -> String {
    json!({
        "type": "user", "sessionId": SESSION, "timestamp": iso(at),
        "uuid": uuid, "message": {"content": "hello"}
    })
    .to_string()
        + "\n"
}

fn stop(at: OffsetDateTime) -> String {
    json!({
        "type": "assistant", "sessionId": SESSION, "timestamp": iso(at),
        "message": {"content": [], "stop_reason": "end_turn"}
    })
    .to_string()
        + "\n"
}

fn append(path: &Path, text: &str) {
    OpenOptions::new()
        .append(true)
        .open(path)
        .unwrap()
        .write_all(text.as_bytes())
        .unwrap();
}

fn json_path(home: &Path) -> std::path::PathBuf {
    let path = home.join("json/session.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    path
}

fn write_json(path: &Path, value: u64, phase: Option<&str>, at: OffsetDateTime) {
    fs::write(
        path,
        json!({"value": value, "phase": phase, "at": iso(at)}).to_string(),
    )
    .unwrap();
}

#[test]
fn scans_incrementally_and_accumulates_health() {
    let (dir, mut store, mut engine) = setup(true);
    let path = claude_path(dir.path(), "first.jsonl");
    fs::write(&path, prompt(now(), "prompt-1")).unwrap();
    let first = engine.scan(&mut store, None, now()).unwrap();
    assert_eq!(
        (first.files_read, first.records, first.events_saved),
        (1, 1, 3)
    );
    assert!(first.activity_changed);
    assert_eq!(store.cursors("claude-code").unwrap().len(), 1);
    assert_eq!(
        store
            .cursor("claude-code", 0, "project/first.jsonl")
            .unwrap()
            .unwrap()
            .offset,
        fs::metadata(&path).unwrap().len()
    );

    let unchanged = engine.scan(&mut store, None, now()).unwrap();
    assert_eq!(
        (
            unchanged.files_read,
            unchanged.records,
            unchanged.events_saved
        ),
        (0, 0, 0)
    );
    append(&path, &format!("bad json\n{}", prompt(now(), "prompt-2")));
    let appended = engine.scan(&mut store, None, now()).unwrap();
    assert_eq!(
        (appended.files_read, appended.records, appended.events_saved),
        (1, 1, 2)
    );
    assert_eq!(store.pending(EventKind::Counter, 20).unwrap().len(), 5);
    let health = store.health().unwrap();
    assert_eq!(
        (
            health[0].records,
            health[0].malformed,
            health[0].oversized,
            health[0].errors
        ),
        (2, 1, 0, 0)
    );
}

#[test]
fn incomplete_line_waits_for_newline() {
    let (dir, mut store, mut engine) = setup(true);
    let path = claude_path(dir.path(), "partial.jsonl");
    let line = prompt(now(), "partial");
    fs::write(&path, line.trim_end_matches('\n')).unwrap();
    let first = engine.scan(&mut store, None, now()).unwrap();
    assert_eq!((first.files_read, first.records), (1, 0));
    assert_eq!(
        store
            .cursor("claude-code", 0, "project/partial.jsonl")
            .unwrap()
            .unwrap()
            .offset,
        0
    );
    append(&path, "\n");
    let next = engine.scan(&mut store, None, now()).unwrap();
    assert_eq!((next.records, next.events_saved), (1, 3));
}

#[test]
fn changed_filter_includes_only_named_files_and_new_files() {
    let (dir, mut store, mut engine) = setup(true);
    let first = claude_path(dir.path(), "first.jsonl");
    let second = claude_path(dir.path(), "second.jsonl");
    fs::write(&first, prompt(now(), "prompt-1")).unwrap();
    fs::write(&second, prompt(now(), "prompt-2")).unwrap();
    let report = engine
        .scan(&mut store, Some(std::slice::from_ref(&first)), now())
        .unwrap();
    assert_eq!(report.files_read, 1);
    assert!(store
        .cursor("claude-code", 0, "project/first.jsonl")
        .unwrap()
        .is_some());
    assert!(store
        .cursor("claude-code", 0, "project/second.jsonl")
        .unwrap()
        .is_none());
    let third = claude_path(dir.path(), "third.jsonl");
    fs::write(&third, prompt(now(), "prompt-3")).unwrap();
    fs::File::open(&third)
        .unwrap()
        .set_modified(SystemTime::UNIX_EPOCH + StdDuration::from_secs(1))
        .unwrap();
    let report = engine
        .scan(&mut store, Some(std::slice::from_ref(&third)), now())
        .unwrap();
    assert_eq!(report.files_read, 1);
    assert!(store
        .cursor("claude-code", 0, "project/third.jsonl")
        .unwrap()
        .is_some());
}

#[test]
fn changed_scan_reads_only_matching_reported_files() {
    let (dir, mut store, mut engine) = setup(true);
    let first = claude_path(dir.path(), "first.jsonl");
    let second = claude_path(dir.path(), "second.jsonl");
    fs::write(&first, prompt(now(), "first-initial")).unwrap();
    fs::write(&second, prompt(now(), "second-initial")).unwrap();
    engine.scan(&mut store, None, now()).unwrap();
    let second_offset = store
        .cursor("claude-code", 0, "project/second.jsonl")
        .unwrap()
        .unwrap()
        .offset;

    append(&first, &prompt(now(), "first-new"));
    append(&second, &prompt(now(), "second-new"));
    let report = engine
        .scan(&mut store, Some(std::slice::from_ref(&first)), now())
        .unwrap();
    assert_eq!((report.files_read, report.records), (1, 1));
    assert_eq!(
        store
            .cursor("claude-code", 0, "project/first.jsonl")
            .unwrap()
            .unwrap()
            .offset,
        fs::metadata(&first).unwrap().len()
    );
    assert_eq!(
        store
            .cursor("claude-code", 0, "project/second.jsonl")
            .unwrap()
            .unwrap()
            .offset,
        second_offset
    );

    let outside = dir.path().join("outside.jsonl");
    fs::write(&outside, prompt(now(), "outside")).unwrap();
    let wrong_glob = claude_path(dir.path(), "wrong.json");
    fs::write(&wrong_glob, prompt(now(), "wrong-glob")).unwrap();
    let too_deep = claude_path(dir.path(), "a/b/c/d/too-deep.jsonl");
    fs::write(&too_deep, prompt(now(), "too-deep")).unwrap();
    let ignored = engine
        .scan(&mut store, Some(&[outside, wrong_glob, too_deep]), now())
        .unwrap();
    assert_eq!((ignored.files_read, ignored.records), (0, 0));
    assert_eq!(
        store
            .cursor("claude-code", 0, "project/second.jsonl")
            .unwrap()
            .unwrap()
            .offset,
        second_offset
    );
}

#[test]
fn disabled_provider_is_skipped_and_unpaired_only_advances_cursors() {
    let (dir, mut store, mut engine) = setup(true);
    let path = claude_path(dir.path(), "first.jsonl");
    fs::write(&path, prompt(now(), "prompt-1")).unwrap();
    store
        .set_meta("providers_off", "[\"claude-code\"]")
        .unwrap();
    assert_eq!(engine.scan(&mut store, None, now()).unwrap().files_read, 0);
    assert!(store.cursors("claude-code").unwrap().is_empty());
    store.delete_meta("providers_off").unwrap();
    let report = engine.scan(&mut store, None, now()).unwrap();
    assert_eq!(report.events_saved, 3);

    let (dir, mut store, mut engine) = setup(false);
    let path = claude_path(dir.path(), "unpaired.jsonl");
    fs::write(&path, prompt(now(), "prompt-2")).unwrap();
    let report = engine.scan(&mut store, None, now()).unwrap();
    assert_eq!(
        (report.files_read, report.records, report.events_saved),
        (1, 1, 0)
    );
    assert_eq!(store.cursors("claude-code").unwrap().len(), 1);
    assert!(store.pending(EventKind::Counter, 10).unwrap().is_empty());
    store.set_meta("paired_at", &iso(now())).unwrap();
    assert_eq!(engine.scan(&mut store, None, now()).unwrap().files_read, 0);
}

#[test]
fn turn_lifecycle_and_old_history() {
    let (dir, mut store, mut engine) = setup(true);
    let path = claude_path(dir.path(), "turn.jsonl");
    fs::write(&path, prompt(now(), "prompt-1")).unwrap();
    engine.scan(&mut store, None, now()).unwrap();
    let activity = engine.activity(now());
    assert!(activity.busy);
    assert_eq!(activity.sessions[0].state, SessionState::Busy);
    assert_eq!(
        engine.activity(now() + Duration::seconds(121)).sessions[0].state,
        SessionState::Idle
    );
    append(&path, &stop(now() + Duration::seconds(1)));
    assert!(
        engine
            .scan(&mut store, None, now() + Duration::seconds(1))
            .unwrap()
            .activity_changed
    );
    assert_eq!(
        engine.activity(now() + Duration::seconds(1)).sessions[0].state,
        SessionState::Done
    );
    assert_eq!(
        engine.activity(now() + Duration::seconds(12)).sessions[0].state,
        SessionState::Idle
    );
    assert!(engine
        .activity(now() + Duration::minutes(31))
        .sessions
        .is_empty());
    assert!(serde_json::to_value(engine.activity(now())).unwrap()["sessions"].is_array());

    let (dir, mut store, mut engine) = setup(true);
    let path = claude_path(dir.path(), "old.jsonl");
    fs::write(&path, prompt(now() - Duration::minutes(11), "old-prompt")).unwrap();
    let report = engine.scan(&mut store, None, now()).unwrap();
    assert!(!report.activity_changed);
    assert!(engine.activity(now()).sessions.is_empty());
}

#[test]
fn json_source_parses_once_per_change() {
    let (dir, mut store, mut engine) = setup(true);
    let path = json_path(dir.path());
    write_json(&path, 1, Some("start"), now());
    let first = engine.scan(&mut store, None, now()).unwrap();
    assert_eq!(
        (first.files_read, first.records, first.events_saved),
        (1, 1, 1)
    );
    assert!(engine.activity(now()).busy);
    assert_eq!(engine.scan(&mut store, None, now()).unwrap().files_read, 0);
    write_json(&path, 100, Some("stop"), now() + Duration::seconds(1));
    let changed = engine
        .scan(&mut store, None, now() + Duration::seconds(1))
        .unwrap();
    assert_eq!(
        (changed.files_read, changed.records, changed.events_saved),
        (1, 1, 1)
    );
    assert_eq!(
        engine.activity(now() + Duration::seconds(1)).sessions[0].state,
        SessionState::Done
    );
    assert_eq!(
        store
            .health()
            .unwrap()
            .iter()
            .find(|row| row.provider == "json-test")
            .unwrap()
            .records,
        2
    );
}

#[test]
fn additional_accounts_and_normalized_metrics_are_incremental() {
    let home = tempfile::tempdir().unwrap();
    let env = Env::for_home(home.path());
    let second = home.path().join("work-codex");
    for folder in [home.path().join(".codex"), second.clone()] {
        fs::create_dir_all(folder.join("sessions")).unwrap();
        let at = iso(now());
        fs::write(folder.join("sessions/rollout.jsonl"), [
            json!({"type":"session_meta","timestamp":at,"payload":{"id":"same-session"}}),
            json!({"type":"event_msg","timestamp":at,"payload":{"type":"task_started","turn_id":"same-turn"}}),
            json!({"type":"response_item","timestamp":at,"payload":{"type":"function_call","call_id":"same-tool","name":"spawn_agent"}}),
            json!({"type":"event_msg","timestamp":at,"payload":{"type":"task_complete"}}),
        ].iter().map(|v| format!("{v}\n")).collect::<String>()).unwrap();
    }
    clashofmeat_daemon::config::update_config(&env, |config| {
        config
            .other
            .insert("providerHomes".into(), json!({"codex":[second]}));
    })
    .unwrap();
    let mut store = Store::open_in_memory().unwrap();
    store.set_meta("paired_at", PAIRED).unwrap();
    let mut engine = Engine::new(env.clone(), vec![Box::new(Codex)], "0.1.0");
    engine.scan(&mut store, None, now()).unwrap();
    assert_eq!(
        store
            .pending(EventKind::Counter, 100)
            .unwrap()
            .iter()
            .filter(|e| e.key == "prompt_count")
            .count(),
        2
    );
    assert_eq!(
        store
            .pending(EventKind::Counter, 100)
            .unwrap()
            .iter()
            .filter(|e| e.key == "subagent_count")
            .count(),
        2
    );
    let payloads = observer::completed_payloads(&mut store, 10).unwrap();
    assert_eq!(payloads.len(), 2);
    assert_ne!(
        payloads[0]["source"]["sourceKey"],
        payloads[1]["source"]["sourceKey"]
    );
    assert!(!serde_json::to_string(&payloads)
        .unwrap()
        .contains("work-codex"));
    // The extra home's profile is a random UUID kept in config.json, not a hash of its path.
    let config = clashofmeat_daemon::config::read_config(&env).unwrap();
    let profile = config.other["providerProfiles"]["codex"][second.to_str().unwrap()]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(clashofmeat_core::ids::is_uuid(&profile));
    assert!(payloads
        .iter()
        .any(|payload| payload["source"]["sourceKey"] == format!("profile:{profile}")));
    assert!(payloads
        .iter()
        .any(|payload| payload["source"]["sourceKey"] == "collector"));
    assert_eq!(
        engine.scan(&mut store, None, now()).unwrap().events_saved,
        0
    );
}

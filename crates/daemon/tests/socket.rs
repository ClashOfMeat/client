use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use clashofmeat_core::{Env, Event, EventKind, Store};
use clashofmeat_daemon::config::store_path;
use clashofmeat_daemon::run::{self, RunOptions};
use serde_json::{json, Value};
use tempfile::tempdir;
use time::{format_description::well_known::Rfc3339, macros::datetime, OffsetDateTime, UtcOffset};

fn request(stream: &mut UnixStream, value: &Value) -> Value {
    writeln!(stream, "{value}").unwrap();
    let mut line = String::new();
    BufReader::new(stream.try_clone().unwrap())
        .read_line(&mut line)
        .unwrap();
    serde_json::from_str(&line).unwrap()
}

fn start(home: &Path) -> run::DaemonHandle {
    run::start(RunOptions {
        env: Env::for_home(home),
        version: "test".into(),
    })
    .unwrap()
}

fn token_event(identity: &str, provider: &str, key: &str, value: u64, at: OffsetDateTime) -> Event {
    Event {
        identity: identity.into(),
        key: key.into(),
        value,
        kind: EventKind::Counter,
        session_id: "123e4567-e89b-12d3-a456-426614174000".into(),
        at: at.format(&Rfc3339).unwrap(),
        resets_at: None,
        model: None,
        provider: Some(provider.into()),
    }
}

#[test]
fn today_tokens_uses_offset_and_maps_shared_claude_events() {
    let mut store = Store::open_in_memory().unwrap();
    let now = datetime!(2026-10-20 00:30 UTC);
    let events = [
        token_event(
            "claude",
            "claude",
            "input_tokens",
            5,
            now - time::Duration::hours(1),
        ),
        token_event(
            "codex",
            "codex",
            "cache_read_tokens",
            7,
            now - time::Duration::hours(1),
        ),
        token_event(
            "old",
            "codex",
            "output_tokens",
            9,
            now - time::Duration::hours(3),
        ),
    ];
    store.save_events("codex", &events, None).unwrap();
    let offset = UtcOffset::from_hms(2, 0, 0).unwrap();
    let totals = run::today_tokens(&store, now, offset);
    assert_eq!(totals.get("claude-code"), Some(&5));
    assert_eq!(totals.get("codex"), Some(&7));
    assert!(!totals.contains_key("claude-cowork"));
    assert!(run::today_tokens(&store, now, UtcOffset::UTC).is_empty());
}

#[test]
fn status_reports_persisted_today_tokens_and_accepts_local_midnight() {
    let dir = tempdir().unwrap();
    let env = Env::for_home(dir.path());
    let now = OffsetDateTime::now_utc();
    let recent = now - time::Duration::minutes(30);
    let older = now - time::Duration::hours(2);
    let mut store = Store::open(store_path(&env)).unwrap();
    let events = [
        token_event("recent", "claude", "input_tokens", 5, recent),
        token_event("older", "claude", "output_tokens", 7, older),
    ];
    store.save_events("claude-code", &events, None).unwrap();
    store
        .mark_synced(&[(events[0].id(), 5, events[0].at.clone())])
        .unwrap();
    drop(store);
    let handle = start(dir.path());
    let mut stream = UnixStream::connect(handle.socket_path()).unwrap();
    let since = (now - time::Duration::hours(1)).format(&Rfc3339).unwrap();
    let local = request(
        &mut stream,
        &json!({"id":1,"method":"status","params":{"today_since":since}}),
    );
    assert_eq!(local["result"]["today_tokens"]["claude-code"], 5);
    let claude = local["result"]["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == "claude-code")
        .unwrap();
    assert_eq!(claude["today_tokens"], 5);
    let cowork = local["result"]["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == "claude-cowork")
        .unwrap();
    assert_eq!(cowork["today_tokens"], 0);
    let utc = request(&mut stream, &json!({"id":2,"method":"status"}));
    let expected =
        5 * u64::from(recent.date() == now.date()) + 7 * u64::from(older.date() == now.date());
    assert_eq!(
        utc["result"]["today_tokens"]["claude-code"]
            .as_u64()
            .unwrap_or(0),
        expected
    );
    handle.stop().unwrap();
}

#[test]
fn socket_lifecycle_and_requests() {
    let dir = tempdir().unwrap();
    let home = dir.path();
    let handle = start(home);
    let mut stream = UnixStream::connect(handle.socket_path()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let status = request(&mut stream, &json!({"id":1,"method":"status"}));
    assert_eq!(status["result"]["paired"], false);
    assert_eq!(status["result"]["providers"].as_array().unwrap().len(), 5);
    let snapshot = request(&mut stream, &json!({"id":2,"method":"snapshot"}));
    assert_eq!(snapshot["result"]["activity"]["busy"], false);
    assert!(snapshot["result"]["hero"].is_null());
    let unknown = request(&mut stream, &json!({"id":3,"method":"unknown"}));
    assert_eq!(unknown["error"]["code"], "unknown_method");
    stream.write_all(b"not-json\n").unwrap();
    let mut line = String::new();
    BufReader::new(stream.try_clone().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap()["error"]["code"],
        "bad_request"
    );
    assert_eq!(
        request(
            &mut stream,
            &json!({"id":4,"method":"set_provider_enabled","params":{"id":"claude-code","enabled":false}})
        )["result"]["enabled"],
        false
    );
    let status = request(&mut stream, &json!({"id":5,"method":"status"}));
    let claude = status["result"]["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|provider| provider["id"] == "claude-code")
        .unwrap();
    assert_eq!(claude["enabled"], false);
    assert!(run::start(RunOptions {
        env: Env::for_home(home),
        version: "test".into()
    })
    .is_err());
    let path = handle.socket_path().to_path_buf();
    handle.stop().unwrap();
    assert!(!path.exists());
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"stale").unwrap();
    let handle = start(home);
    assert!(UnixStream::connect(handle.socket_path()).is_ok());
    handle.stop().unwrap();
}

#[test]
fn subscription_reports_transcript_activity() {
    let dir = tempdir().unwrap();
    let home = dir.path();
    let root = home.join(".claude/projects/x");
    fs::create_dir_all(&root).unwrap();
    let handle = start(home);
    let mut stream = UnixStream::connect(handle.socket_path()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    assert_eq!(
        request(&mut stream, &json!({"id":1,"method":"subscribe"}))["result"]["subscribed"],
        true
    );
    let path = root.join("12345678-1234-1234-1234-123456789abc.jsonl");
    let at = OffsetDateTime::now_utc().format(&Rfc3339).unwrap();
    let line = json!({"type":"user","sessionId":"12345678-1234-1234-1234-123456789abc","timestamp":at,"uuid":"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa","message":{"content":"hello"}});
    writeln!(
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap(),
        "{line}"
    )
    .unwrap();
    let mut reader = BufReader::new(stream);
    let mut incoming = String::new();
    loop {
        incoming.clear();
        reader.read_line(&mut incoming).unwrap();
        let event: Value = serde_json::from_str(&incoming).unwrap();
        if event["event"] == "activity" && event["data"]["busy"] == true {
            break;
        }
    }
    handle.stop().unwrap();
}

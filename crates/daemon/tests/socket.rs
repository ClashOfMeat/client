use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use clashofmeat_core::Env;
use clashofmeat_daemon::run::{self, RunOptions};
use serde_json::{json, Value};
use tempfile::tempdir;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

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

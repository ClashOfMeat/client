use std::collections::{BTreeMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use clashofmeat_core::env::Env;
use clashofmeat_core::signal::{Event, EventKind};
use clashofmeat_core::store::Store;
use clashofmeat_daemon::config::{app_origin, read_config, store_path, update_config};
use clashofmeat_daemon::run::{self, RunOptions};
use clashofmeat_daemon::sync::{mark_paired, sync_once, ApiError, Client};
use serde_json::{json, Value};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

const ID: &str = "123e4567-e89b-12d3-a456-426614174000";
const TOKEN: &str = "0123456789012345678901234567890123456789";
const NOW: &str = "2026-10-20T12:00:00Z";

fn now() -> OffsetDateTime {
    OffsetDateTime::parse(NOW, &Rfc3339).unwrap()
}

struct Request {
    path: String,
    headers: BTreeMap<String, String>,
    body: Value,
}

struct Reply {
    status: u16,
    body: Value,
    headers: Vec<(&'static str, &'static str)>,
}

impl Reply {
    fn ok(body: Value) -> Self {
        Self {
            status: 200,
            body,
            headers: vec![],
        }
    }
    fn error(status: u16, body: Value) -> Self {
        Self {
            status,
            body,
            headers: vec![],
        }
    }
}

struct Server {
    origin: String,
    received: Arc<Mutex<Vec<Request>>>,
    thread: thread::JoinHandle<()>,
}

impl Server {
    fn start(replies: Vec<Reply>) -> Self {
        Self::with_handler(replies.len(), move || {
            let mut replies: VecDeque<_> = replies.into();
            move |_| replies.pop_front().unwrap()
        })
    }

    fn with_handler<F, G>(count: usize, make: F) -> Self
    where
        F: FnOnce() -> G + Send + 'static,
        G: FnMut(&Request) -> Reply + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let received = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&received);
        let thread = thread::spawn(move || {
            let mut handle = make();
            for _ in 0..count {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_request(&mut stream);
                let reply = handle(&request);
                observed.lock().unwrap().push(request);
                let body = reply.body.to_string();
                write!(stream, "HTTP/1.1 {} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n", reply.status, body.len()).unwrap();
                for (key, value) in reply.headers {
                    write!(stream, "{key}: {value}\r\n").unwrap();
                }
                write!(stream, "\r\n{body}").unwrap();
                stream.flush().unwrap();
            }
        });
        Self {
            origin,
            received,
            thread,
        }
    }

    fn finish(self) -> Vec<Request> {
        self.thread.join().unwrap();
        Arc::try_unwrap(self.received)
            .ok()
            .unwrap()
            .into_inner()
            .unwrap()
    }
}

fn read_request(stream: &mut TcpStream) -> Request {
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut chunk = [0u8; 4096];
        let read = stream.read(&mut chunk).unwrap();
        assert!(read > 0);
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(at) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break at + 4;
        }
    };
    let header = std::str::from_utf8(&bytes[..header_end]).unwrap();
    let mut lines = header.split("\r\n");
    let path = lines
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_owned();
    let headers: BTreeMap<String, String> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    let length: usize = headers.get("content-length").unwrap().parse().unwrap();
    while bytes.len() - header_end < length {
        let mut chunk = [0u8; 4096];
        let read = stream.read(&mut chunk).unwrap();
        assert!(read > 0);
        bytes.extend_from_slice(&chunk[..read]);
    }
    let body = serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap();
    Request {
        path,
        headers,
        body,
    }
}

fn client(server: &Server) -> Client {
    Client::new(&server.origin, Some(TOKEN.into()), "0.1.0").unwrap()
}

fn event(identity: &str, kind: EventKind, at: &str, value: u64) -> Event {
    Event {
        identity: identity.into(),
        key: if kind == EventKind::Reading {
            "five_hour_percent"
        } else {
            "input_tokens"
        }
        .into(),
        value,
        kind,
        session_id: ID.into(),
        at: at.into(),
        resets_at: None,
        model: None,
        provider: None,
    }
}

#[test]
fn config_preserves_old_keys_and_private_modes() {
    let dir = tempfile::tempdir().unwrap();
    let mut env = Env::for_home(dir.path());
    env.vars.insert(
        "CLASHOFMEAT_HOME".into(),
        dir.path().join("custom").display().to_string(),
    );
    assert_eq!(
        store_path(&env),
        dir.path().join("custom/desktop/activity.sqlite")
    );
    let mut empty_env = Env::for_home(dir.path());
    empty_env
        .vars
        .insert("CLASHOFMEAT_HOME".into(), String::new());
    assert_eq!(
        store_path(&empty_env),
        dir.path().join(".clashofmeat/desktop/activity.sqlite")
    );
    std::fs::create_dir_all(dir.path().join("custom")).unwrap();
    std::fs::write(
        dir.path().join("custom/config.json"),
        r#"{"appUrl":"https://old.example","installedVersion":"1.2"}"#,
    )
    .unwrap();
    update_config(&env, |config| {
        config.app_url = Some("https://new.example".into())
    })
    .unwrap();
    let config = read_config(&env).unwrap();
    assert_eq!(config.app_url.as_deref(), Some("https://new.example"));
    assert_eq!(config.other["installedVersion"], "1.2");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(dir.path().join("custom"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(dir.path().join("custom/config.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    assert!(app_origin("http://example.com").is_err());
    assert!(app_origin("https://example.com/path").is_err());
    assert_eq!(
        app_origin("http://localhost:3100/").unwrap(),
        "http://localhost:3100"
    );
}

#[test]
fn pairing_polls_and_uses_exact_exchange_shape() {
    let server = Server::start(vec![
        Reply::ok(
            json!({"id": ID, "code": "ABCDEF1234", "exchangeSecret": "secret", "expiresAt": NOW}),
        ),
        Reply::error(404, json!({"error": "pending"})),
        Reply::ok(json!({"installationId": ID, "token": TOKEN})),
    ]);
    let client = client(&server);
    let pairing = client.pair_start("Paul's Mac", &["codex".into()]).unwrap();
    assert_eq!(
        pairing.link(&server.origin),
        format!("{}/link?code=ABCDEF1234", server.origin)
    );
    assert!(client.pair_exchange(&pairing).unwrap().is_none());
    let linked = client.pair_exchange(&pairing).unwrap().unwrap();
    let mut store = Store::open_in_memory().unwrap();
    mark_paired(&mut store, &linked, now()).unwrap();
    mark_paired(&mut store, &linked, now() + time::Duration::hours(1)).unwrap();
    assert_eq!(store.meta("paired_at").unwrap().as_deref(), Some(NOW));
    assert_eq!(store.meta("installation.id").unwrap().as_deref(), Some(ID));
    let requests = server.finish();
    assert_eq!(
        requests[0].body,
        json!({"device": "Paul's Mac", "tools": ["codex"]})
    );
    for request in &requests {
        assert_eq!(request.headers["x-clashofmeat-client"], "desktop/0.1.0");
        assert!(request.headers["content-type"].starts_with("application/json"));
    }
    assert_eq!(
        requests[1].body,
        json!({"id": ID, "exchangeSecret": "secret", "client": "collector"})
    );
    assert_eq!(requests[2].body, requests[1].body);
}

#[test]
fn pairing_rejects_declined_expired_and_bad_responses() {
    for reason in ["declined", "expired"] {
        let server = Server::start(vec![
            Reply::ok(
                json!({"id": ID, "code": "ABCDEF1234", "exchangeSecret": "secret", "expiresAt": NOW}),
            ),
            Reply::error(410, json!({"error": "gone", "code": reason})),
        ]);
        let client = client(&server);
        let pairing = client.pair_start("device", &[]).unwrap();
        assert!(
            matches!(client.pair_exchange(&pairing), Err(ApiError::Status { status: 410, reason: Some(found), .. }) if found == reason)
        );
        server.finish();
    }
    for invalid in [
        json!({"id": "bad", "code": "ABCDEF1234", "exchangeSecret": "secret", "expiresAt": NOW}),
        json!({"id": ID, "code": "abcdef1234", "exchangeSecret": "secret", "expiresAt": NOW}),
        json!({"id": ID, "code": "ABCDEF1234", "exchangeSecret": "", "expiresAt": NOW}),
        json!({"id": ID, "code": "ABCDEF1234", "exchangeSecret": "secret", "expiresAt": "bad"}),
    ] {
        let server = Server::start(vec![Reply::ok(invalid)]);
        assert!(matches!(
            client(&server).pair_start("device", &[]),
            Err(ApiError::Invalid(_))
        ));
        server.finish();
    }
}

#[test]
fn heartbeat_fallback_and_sync_meta() {
    let server = Server::start(vec![
        Reply::error(400, json!({"error": "bad adapters"})),
        Reply::ok(json!({"connected": true})),
        Reply::ok(json!({"progression": {"xp": 42}})),
    ]);
    let client = client(&server);
    assert_eq!(
        client
            .heartbeat(json!({"adapters": {"codex": {"files": 1}}}))
            .unwrap(),
        json!({"connected": true})
    );
    let mut store = Store::open_in_memory().unwrap();
    let result = sync_once(&client, &mut store, now());
    assert!(result.error.is_none());
    assert_eq!(result.heartbeat, Some(json!({"progression": {"xp": 42}})));
    assert_eq!(store.meta("heartbeat_at").unwrap().as_deref(), Some(NOW));
    assert_eq!(
        serde_json::from_str::<Value>(&store.meta("heartbeat").unwrap().unwrap()).unwrap(),
        json!({"progression": {"xp": 42}})
    );
    let requests = server.finish();
    assert_eq!(requests[0].body["adapters"]["codex"]["files"], 1);
    assert_eq!(requests[1].body, json!({}));
    assert_eq!(requests[2].body, json!({}));
}

#[test]
fn upload_shape_history_optionals_and_ack() {
    let server = Server::start(vec![Reply::ok(
        json!({"accepted": 3, "duplicates": 0, "progression": {"xp": 8}}),
    )]);
    let mut store = Store::open_in_memory().unwrap();
    let mut old = event("old", EventKind::Counter, "2026-10-01T12:00:00Z", 7);
    old.model = Some("opus".into());
    old.provider = Some("claude".into());
    let old_id = old.id();
    let live = event("live", EventKind::Counter, NOW, 2);
    let live_id = live.id();
    let mut reading = event("reading", EventKind::Reading, NOW, 50);
    reading.resets_at = Some("2026-10-20T13:00:00Z".into());
    store
        .save_events("claude", &[old, live, reading], None)
        .unwrap();
    let result = client(&server)
        .upload_observations(&mut store, now())
        .unwrap();
    assert_eq!(result.sent, 3);
    assert_eq!(result.response.unwrap()["progression"]["xp"], 8);
    assert!(store.pending(EventKind::Counter, 100).unwrap().is_empty());
    assert!(store.pending(EventKind::Reading, 50).unwrap().is_empty());
    let requests = server.finish();
    let request = &requests[0];
    assert_eq!(request.path, "/api/observations");
    assert_eq!(request.headers["authorization"], format!("Bearer {TOKEN}"));
    let counters = request.body["observations"].as_array().unwrap();
    let history = counters.iter().find(|item| item["id"] == old_id).unwrap();
    assert_eq!(
        history,
        &json!({"id": old_id, "sessionId": ID, "at": "2026-10-01T12:00:00Z", "key": "input_tokens", "value": 7, "source": "history", "model": "opus", "provider": "claude"})
    );
    let current = counters.iter().find(|item| item["id"] == live_id).unwrap();
    assert_eq!(current["source"], "live");
    assert!(current.get("model").is_none());
    assert!(current.get("provider").is_none());
    assert_eq!(
        request.body["readings"][0]["resetsAt"],
        "2026-10-20T13:00:00Z"
    );
}

#[test]
fn changed_row_during_request_stays_pending() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("activity.sqlite");
    let mut store = Store::open(&path).unwrap();
    let first = event("changed", EventKind::Counter, NOW, 1);
    store.save_events("codex", &[first], None).unwrap();
    let server = Server::with_handler(1, move || {
        move |_| {
            let mut other = Store::open(&path).unwrap();
            other
                .save_events(
                    "codex",
                    &[event("changed", EventKind::Counter, NOW, 2)],
                    None,
                )
                .unwrap();
            Reply::ok(json!({"accepted": 1}))
        }
    });
    client(&server)
        .upload_observations(&mut store, now())
        .unwrap();
    assert_eq!(store.pending(EventKind::Counter, 10).unwrap()[0].value, 2);
    server.finish();
}

#[test]
fn sync_handles_auth_block_and_rate_limit() {
    for (status, body) in [
        (
            401,
            json!({"error": "revoked", "setupUrl": "https://example.com/setup"}),
        ),
        (426, json!({"error": "update required"})),
        (429, json!({"error": "slow down"})),
    ] {
        let mut reply = Reply::error(status, body);
        if status == 429 {
            reply.headers.push(("Retry-After", "120"));
        }
        let server = Server::start(vec![reply]);
        let mut store = Store::open_in_memory().unwrap();
        store.set_meta("installation.token", TOKEN).unwrap();
        store.set_meta("installation.id", ID).unwrap();
        let client = client(&server);
        let result = sync_once(&client, &mut store, now());
        assert!(
            matches!(result.error, Some(ApiError::Status { status: found, .. }) if found == status)
        );
        match status {
            401 => {
                assert!(store.meta("installation.token").unwrap().is_none());
                assert!(store.meta("installation.id").unwrap().is_none());
                assert_eq!(store.meta("reauth").unwrap().as_deref(), Some("true"));
            }
            426 => assert_eq!(
                serde_json::from_str::<Value>(&store.meta("blocked").unwrap().unwrap()).unwrap(),
                json!({"version":"0.1.0","message":"Clash of Meat: update required"})
            ),
            429 => {
                assert_eq!(
                    store.meta("retry_after_until").unwrap().as_deref(),
                    Some("2026-10-20T12:02:00Z")
                );
                assert!(
                    sync_once(&client, &mut store, now() + time::Duration::seconds(30)).deferred
                );
            }
            _ => unreachable!(),
        }
        assert_eq!(server.finish().len(), 1);
    }
}

fn socket_request(
    stream: &mut UnixStream,
    reader: &mut BufReader<UnixStream>,
    id: u64,
    method: &str,
) -> Value {
    writeln!(stream, "{}", json!({"id":id,"method":method})).unwrap();
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let response: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response["id"], id);
    response
}

#[test]
fn blocked_previous_version_allows_sync_and_status_is_clear() {
    let dir = tempfile::tempdir().unwrap();
    let env = Env::for_home(dir.path());
    let server = Server::start(vec![Reply::ok(json!({"connected":true}))]);
    update_config(&env, |config| config.app_url = Some(server.origin.clone())).unwrap();
    let mut store = Store::open(store_path(&env)).unwrap();
    store.set_meta("installation.token", TOKEN).unwrap();
    store.set_meta("installation.id", ID).unwrap();
    store
        .set_meta(
            "blocked",
            &json!({"version":"0.1.0","message":"Update required"}).to_string(),
        )
        .unwrap();
    drop(store);

    let handle = run::start(RunOptions {
        env,
        version: "0.2.0".into(),
    })
    .unwrap();
    let mut stream = UnixStream::connect(handle.socket_path()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    assert!(socket_request(&mut stream, &mut reader, 1, "status")["result"]["blocked"].is_null());
    assert_eq!(
        socket_request(&mut stream, &mut reader, 2, "subscribe")["result"]["subscribed"],
        true
    );
    let mut command = UnixStream::connect(handle.socket_path()).unwrap();
    command
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut command_reader = BufReader::new(command.try_clone().unwrap());
    assert_eq!(
        socket_request(&mut command, &mut command_reader, 3, "sync_now")["result"]["queued"],
        true
    );
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let event: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(event["event"], "sync");
    assert_eq!(event["data"]["ok"], true);
    assert_eq!(server.finish().len(), 1);
    let store = Store::open(store_path(&Env::for_home(dir.path()))).unwrap();
    assert!(store.meta("blocked").unwrap().is_none());
    handle.stop().unwrap();
}

#[test]
fn upload_401_keeps_queue_and_unpairs() {
    let server = Server::start(vec![
        Reply::ok(json!({"connected": true})),
        Reply::error(401, json!({"error": "expired"})),
    ]);
    let mut store = Store::open_in_memory().unwrap();
    store.set_meta("installation.token", TOKEN).unwrap();
    store.set_meta("installation.id", ID).unwrap();
    store
        .save_events(
            "codex",
            &[event("pending", EventKind::Counter, NOW, 3)],
            None,
        )
        .unwrap();
    let result = sync_once(&client(&server), &mut store, now());
    assert!(matches!(
        result.error,
        Some(ApiError::Status { status: 401, .. })
    ));
    assert_eq!(store.pending(EventKind::Counter, 10).unwrap().len(), 1);
    assert!(store.meta("installation.token").unwrap().is_none());
    assert_eq!(store.meta("reauth").unwrap().as_deref(), Some("true"));
    assert_eq!(server.finish().len(), 2);
}

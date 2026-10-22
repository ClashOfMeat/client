use std::collections::{BTreeMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use clashofmeat_core::env::Env;
use clashofmeat_core::ids::is_uuid;
use clashofmeat_core::observer;
use clashofmeat_core::signal::{Event, EventKind};
use clashofmeat_core::store::Store;
use clashofmeat_core::{Observation, TaskRef};
use clashofmeat_daemon::config::{app_origin, read_config, store_path, update_config};
use clashofmeat_daemon::run::{self, RunOptions};
use clashofmeat_daemon::sync::{
    companion_session, mark_paired, sync_once, sync_once_with, sync_once_with_art, ApiError,
    Client, HeroArt,
};
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
    method: String,
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
            let mut handled = 0;
            while handled < count {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_request(&mut stream);
                // A running daemon also follows the live channel; scripts only cover the API calls.
                if request.path == "/api/live" {
                    let _ = write!(
                        stream,
                        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                    continue;
                }
                handled += 1;
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
    let mut start = lines.next().unwrap().split_whitespace();
    let method = start.next().unwrap().to_owned();
    let path = start.next().unwrap().to_owned();
    let headers: BTreeMap<String, String> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    let length: usize = headers
        .get("content-length")
        .map_or(0, |length| length.parse().unwrap());
    while bytes.len() - header_end < length {
        let mut chunk = [0u8; 4096];
        let read = stream.read(&mut chunk).unwrap();
        assert!(read > 0);
        bytes.extend_from_slice(&chunk[..read]);
    }
    let body = if length == 0 {
        Value::Null
    } else {
        serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap()
    };
    Request {
        method,
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

fn completed_task(store: &mut Store, key: &str, source: &str) -> String {
    let task = format!("task-{key}");
    observer::apply(
        store,
        "codex",
        &[
            Observation::Start {
                context: task.clone(),
                task_key: task.clone(),
                source_key: source.into(),
                session_id: ID.into(),
                at: NOW.into(),
                client_version: String::new(),
                close_previous_completed: false,
            },
            Observation::Finish {
                task: TaskRef::Key(task),
                at: NOW.into(),
                status: "completed".into(),
                active_guard: false,
            },
        ],
        "0.1.0",
    )
    .unwrap();
    observer::completed_payloads(store, 100)
        .unwrap()
        .last()
        .unwrap()["task"]["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn observer_upload_groups_by_source_and_acknowledges_duplicates() {
    let mut store = Store::open_in_memory().unwrap();
    completed_task(&mut store, "a", "first");
    completed_task(&mut store, "b", "first");
    completed_task(&mut store, "c", "second");
    let server = Server::start(vec![
        Reply::ok(json!({"accepted": 1, "duplicates": 1})),
        Reply::ok(json!({"accepted": 0, "duplicates": 1})),
    ]);
    assert_eq!(
        client(&server).upload_observer_tasks(&mut store).unwrap(),
        3
    );
    assert!(observer::completed_payloads(&mut store, 10)
        .unwrap()
        .is_empty());
    let requests = server.finish();
    assert_eq!(requests.len(), 2);
    assert!(requests
        .iter()
        .all(|request| request.path == "/api/observer/tasks"));
    assert_eq!(requests[0].body["source"]["sourceKey"], "first");
    assert_eq!(requests[0].body["tasks"].as_array().unwrap().len(), 2);
    assert_eq!(requests[1].body["source"]["sourceKey"], "second");
    assert_eq!(requests[1].body["tasks"].as_array().unwrap().len(), 1);
}

#[test]
fn observer_upload_retries_400_individually_and_rejects_bad_task() {
    let mut store = Store::open_in_memory().unwrap();
    completed_task(&mut store, "a", "first");
    completed_task(&mut store, "b", "first");
    let server = Server::start(vec![
        Reply::error(400, json!({"error": "invalid batch"})),
        Reply::ok(json!({"accepted": 1, "duplicates": 0})),
        Reply::error(400, json!({"error": "invalid task"})),
    ]);
    assert_eq!(
        client(&server).upload_observer_tasks(&mut store).unwrap(),
        1
    );
    assert!(observer::completed_payloads(&mut store, 10)
        .unwrap()
        .is_empty());
    let requests = server.finish();
    assert_eq!(
        requests
            .iter()
            .map(|request| request.body["tasks"].as_array().unwrap().len())
            .collect::<Vec<_>>(),
        vec![2, 1, 1]
    );
}

#[test]
fn observer_upload_invalid_ack_keeps_all_tasks_pending() {
    let mut store = Store::open_in_memory().unwrap();
    completed_task(&mut store, "a", "first");
    completed_task(&mut store, "b", "first");
    let server = Server::start(vec![Reply::ok(json!({"accepted": 1, "duplicates": 0}))]);
    assert!(matches!(
        client(&server).upload_observer_tasks(&mut store),
        Err(ApiError::Invalid(_))
    ));
    assert_eq!(
        observer::completed_payloads(&mut store, 10).unwrap().len(),
        2
    );
    server.finish();
}

#[test]
fn observer_upload_401_unpairs_without_acknowledging() {
    let mut store = Store::open_in_memory().unwrap();
    store.set_meta("installation.token", TOKEN).unwrap();
    store.set_meta("installation.id", ID).unwrap();
    completed_task(&mut store, "a", "first");
    let server = Server::start(vec![
        Reply::ok(json!({"connected": true})),
        Reply::error(401, json!({"error": "expired"})),
    ]);
    let result = sync_once(&client(&server), &mut store, now());
    assert!(matches!(
        result.error,
        Some(ApiError::Status { status: 401, .. })
    ));
    assert!(store.meta("installation.token").unwrap().is_none());
    assert_eq!(store.meta("reauth").unwrap().as_deref(), Some("true"));
    assert_eq!(
        observer::completed_payloads(&mut store, 10).unwrap().len(),
        1
    );
    server.finish();
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
            Reply::ok(json!({"accepted": 1, "duplicates": 0}))
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

fn full_character(revision: &str) -> Value {
    json!({"character": {"id": ID, "level": 2, "png": "iVBOR", "animation": {"frames": ["a"]}, "artRevision": revision}})
}

#[test]
fn heartbeat_reuses_unchanged_art_and_stores_it_without_frames() {
    let revision = "0123456789abcdef0123456789abcdef";
    let server = Server::start(vec![
        Reply::ok(full_character(revision)),
        Reply::ok(
            json!({"character": {"id": ID, "level": 3, "artRevision": revision, "artUnchanged": true}}),
        ),
    ]);
    let client = client(&server);
    let mut store = Store::open_in_memory().unwrap();
    let mut art = HeroArt::default();
    sync_once_with_art(&client, &mut store, now(), &mut art);
    let second = sync_once_with_art(&client, &mut store, now(), &mut art);
    let character = &second.heartbeat.unwrap()["character"];
    assert_eq!(character["level"], 3);
    assert_eq!(character["png"], "iVBOR");
    assert_eq!(character["animation"], json!({"frames": ["a"]}));
    assert!(character.get("artUnchanged").is_none());
    let stored: Value = serde_json::from_str(&store.meta("heartbeat").unwrap().unwrap()).unwrap();
    assert!(
        stored["character"].get("png").is_none() && stored["character"].get("animation").is_none()
    );
    let requests = server.finish();
    assert_eq!(requests[0].body, json!({}));
    assert_eq!(requests[1].body, json!({"artRevision": revision}));
}

#[test]
fn heartbeat_refetches_art_it_no_longer_holds() {
    let server = Server::start(vec![
        Reply::ok(
            json!({"character": {"id": ID, "artRevision": "ffffffffffffffffffffffffffffffff", "artUnchanged": true}}),
        ),
        Reply::ok(full_character("ffffffffffffffffffffffffffffffff")),
    ]);
    let client = client(&server);
    let mut store = Store::open_in_memory().unwrap();
    let mut art = HeroArt::from_view(&full_character("0123456789abcdef0123456789abcdef"));
    let result = sync_once_with_art(&client, &mut store, now(), &mut art);
    assert_eq!(result.heartbeat.unwrap()["character"]["png"], "iVBOR");
    let requests = server.finish();
    assert_eq!(
        requests[0].body,
        json!({"artRevision": "0123456789abcdef0123456789abcdef"})
    );
    assert_eq!(requests[1].body, json!({}));
}

#[test]
fn evolve_posts_the_character_and_its_level() {
    let server = Server::start(vec![Reply::ok(
        json!({"levelUp": {"version": 1}, "character": {"id": ID}}),
    )]);
    let evolved = client(&server).evolve(ID, 4).unwrap();
    assert_eq!(evolved["character"]["id"], ID);
    let requests = server.finish();
    assert_eq!(requests[0].path, "/api/companion/evolve");
    assert_eq!(requests[0].body, json!({"characterId": ID, "level": 4}));
}

#[test]
fn level_up_asks_for_the_effect_between_two_levels() {
    let server = Server::start(vec![Reply::ok(json!({"levelUp": {"version": 1}}))]);
    let effect = client(&server).level_up(12, 13).unwrap();
    assert_eq!(effect["levelUp"]["version"], 1);
    let requests = server.finish();
    assert_eq!(requests[0].path, "/api/companion/level-up");
    assert_eq!(requests[0].body, json!({"fromLevel": 12, "toLevel": 13}));
}

#[test]
fn friends_are_fetched_with_the_installation_token_and_cleaned() {
    let server = Server::start(vec![Reply::ok(json!({"friends": [
        {"handle": "gab", "name": "Gab", "online": true, "level": 12, "email": "x@y"},
        {"handle": "tom", "online": false},
        {"name": "no handle"},
    ]}))]);
    let friends = client(&server).friends().unwrap();
    assert_eq!(
        friends,
        json!({"friends": [
            {"handle": "gab", "name": "Gab", "online": true, "level": 12},
            {"handle": "tom", "name": "tom", "online": false, "level": null},
        ]})
    );
    let requests = server.finish();
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path, "/api/companion/friends");
    assert_eq!(
        requests[0].headers["authorization"],
        format!("Bearer {TOKEN}")
    );
    assert_eq!(requests[0].headers["x-clashofmeat-client"], "desktop/0.1.0");
}

#[test]
fn send_visit_posts_the_target_emote_and_request_id_and_keeps_the_server_message() {
    let server = Server::start(vec![
        Reply::ok(json!({"id": ID, "target": "gab", "expiresAt": NOW, "emote": "wave"})),
        Reply::error(409, json!({"error": "Gab's app isn't running."})),
    ]);
    let client = client(&server);
    let request_id = clashofmeat_core::ids::random_uuid();
    assert_eq!(
        client.send_visit("gab", "wave", &request_id).unwrap()["emote"],
        "wave"
    );
    let Err(ApiError::Status { status, detail, .. }) = client.send_visit("gab", "kiss", ID) else {
        panic!("expected a status error");
    };
    assert_eq!(
        (status, detail.as_deref()),
        (409, Some("Gab's app isn't running."))
    );
    let requests = server.finish();
    assert_eq!(requests[0].path, "/api/companion/kiss");
    assert_eq!(
        requests[0].body,
        json!({"target": "gab", "requestId": request_id, "emote": "wave"})
    );
}

#[test]
fn companion_session_is_created_once() {
    let mut store = Store::open_in_memory().unwrap();
    let session = companion_session(&mut store).unwrap();
    assert!(is_uuid(&session));
    assert_eq!(companion_session(&mut store).unwrap(), session);
}

#[test]
fn heartbeat_carries_the_presence_fields_on_both_attempts() {
    let server = Server::start(vec![
        Reply::ok(
            json!({"character": {"id": ID, "artRevision": "ffffffffffffffffffffffffffffffff", "artUnchanged": true}}),
        ),
        Reply::ok(json!({"connected": true})),
    ]);
    let mut store = Store::open_in_memory().unwrap();
    let mut presence = serde_json::Map::new();
    presence.insert("sessionId".into(), json!(ID));
    presence.insert("acknowledgedVisits".into(), json!([ID]));
    let mut art = HeroArt::from_view(&full_character("0123456789abcdef0123456789abcdef"));
    sync_once_with(&client(&server), &mut store, now(), &mut art, &presence);
    let requests = server.finish();
    assert_eq!(
        requests[0].body,
        json!({"artRevision": "0123456789abcdef0123456789abcdef", "sessionId": ID, "acknowledgedVisits": [ID]})
    );
    assert_eq!(
        requests[1].body,
        json!({"sessionId": ID, "acknowledgedVisits": [ID]})
    );
}

fn socket_call(
    stream: &mut UnixStream,
    reader: &mut BufReader<UnixStream>,
    id: u64,
    method: &str,
    params: Value,
) -> Value {
    writeln!(
        stream,
        "{}",
        json!({"id":id,"method":method,"params":params})
    )
    .unwrap();
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let response: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response["id"], id);
    response
}

#[test]
fn daemon_sends_visits_lists_friends_and_acknowledges_played_visits() {
    let dir = tempfile::tempdir().unwrap();
    let env = Env::for_home(dir.path());
    let server = Server::with_handler(5, || {
        |request: &Request| match request.path.as_str() {
            "/api/companion/friends" => {
                Reply::ok(json!({"friends": [{"handle": "gab", "name": "Gab", "online": true}]}))
            }
            "/api/companion/kiss" if request.body["target"] == "gab" => Reply::ok(
                json!({"id": ID, "target": "gab", "expiresAt": NOW, "emote": request.body["emote"]}),
            ),
            "/api/companion/kiss" if request.body["target"] == "lea" => {
                Reply::error(409, json!({"error": "Lea is already receiving a kiss."}))
            }
            "/api/companion/kiss" => Reply::error(403, json!({"error": "Only friends can kiss."})),
            _ => Reply::ok(json!({"connected": true})),
        }
    });
    update_config(&env, |config| config.app_url = Some(server.origin.clone())).unwrap();
    let mut store = Store::open(store_path(&env)).unwrap();
    store.set_meta("installation.token", TOKEN).unwrap();
    drop(store);
    let handle = run::start(RunOptions {
        env,
        version: "0.1.0".into(),
    })
    .unwrap();
    let mut stream = UnixStream::connect(handle.socket_path()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut call = |id, method, params| socket_call(&mut stream, &mut reader, id, method, params);

    assert_eq!(
        call(1, "friends", json!({}))["result"]["friends"][0]["handle"],
        "gab"
    );
    assert_eq!(
        call(2, "send_visit", json!({"target": "gab", "emote": "laugh"}))["result"]["emote"],
        "laugh"
    );
    let refused = call(3, "send_visit", json!({"target": "tom", "emote": "kiss"}));
    assert_eq!(refused["error"]["code"], "not_friend");
    assert_eq!(
        refused["error"]["message"],
        "Only friends can receive your hero"
    );
    let busy = call(8, "send_visit", json!({"target": "lea", "emote": "wave"}));
    assert_eq!(busy["error"]["code"], "unavailable");
    assert_eq!(
        busy["error"]["message"],
        "Lea is already receiving a visit."
    );
    // Checked locally: never reaches the server.
    assert_eq!(
        call(4, "send_visit", json!({"target": "gab", "emote": "dance"}))["error"]["code"],
        "bad_request"
    );
    assert_eq!(
        call(5, "visit_done", json!({"id": "nope"}))["error"]["code"],
        "bad_request"
    );
    assert_eq!(
        call(6, "visit_done", json!({"id": ID}))["result"]["queued"],
        true
    );
    assert_eq!(call(7, "sync_now", json!({}))["result"]["queued"], true);

    let requests = server.finish();
    let heartbeat = requests
        .iter()
        .find(|request| request.path == "/api/installations/heartbeat")
        .unwrap();
    assert_eq!(heartbeat.body["acknowledgedVisits"], json!([ID]));
    let session = heartbeat.body["sessionId"].as_str().unwrap();
    let store = Store::open(store_path(&Env::for_home(dir.path()))).unwrap();
    assert_eq!(
        store.meta("companion.session").unwrap().as_deref(),
        Some(session)
    );
    let kiss = requests
        .iter()
        .find(|request| request.path == "/api/companion/kiss")
        .unwrap();
    assert!(is_uuid(kiss.body["requestId"].as_str().unwrap()));
    handle.stop().unwrap();
}

#[test]
fn observer_backlog_is_chunked_to_server_limit() {
    let mut store = Store::open_in_memory().unwrap();
    for i in 0..51 {
        completed_task(&mut store, &format!("chunk-{i}"), "first");
    }
    let server = Server::start(vec![
        Reply::ok(json!({"accepted":50,"duplicates":0})),
        Reply::ok(json!({"accepted":1,"duplicates":0})),
    ]);
    assert_eq!(
        client(&server).upload_observer_tasks(&mut store).unwrap(),
        51
    );
    let requests = server.finish();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].body["tasks"].as_array().unwrap().len(), 50);
    assert_eq!(requests[1].body["tasks"].as_array().unwrap().len(), 1);
}

#[test]
fn observation_invalid_ack_keeps_counters_and_readings_pending() {
    for response in [
        json!({}),
        json!({"accepted":2}),
        json!({"accepted":1,"duplicates":0}),
        json!({"accepted":2,"duplicates":-1}),
    ] {
        let mut store = Store::open_in_memory().unwrap();
        store
            .save_events(
                "codex",
                &[
                    event("counter", EventKind::Counter, NOW, 5),
                    event("reading", EventKind::Reading, NOW, 50),
                ],
                None,
            )
            .unwrap();
        let server = Server::start(vec![Reply::ok(response)]);
        assert!(matches!(
            client(&server).upload_observations(&mut store, now()),
            Err(ApiError::Invalid(_))
        ));
        assert_eq!(store.pending(EventKind::Counter, 10).unwrap().len(), 1);
        assert_eq!(store.pending(EventKind::Reading, 10).unwrap().len(), 1);
        server.finish();
    }
}

#[test]
fn expired_readings_do_not_block_live_counters() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .save_events(
            "codex",
            &[
                event("fresh", EventKind::Counter, NOW, 8),
                event(
                    "old-reading",
                    EventKind::Reading,
                    "2026-10-01T12:00:00Z",
                    50,
                ),
            ],
            None,
        )
        .unwrap();
    let server = Server::start(vec![Reply::ok(json!({"accepted":1,"duplicates":0}))]);
    assert_eq!(
        client(&server)
            .upload_observations(&mut store, now())
            .unwrap()
            .sent,
        1
    );
    assert_eq!(store.pending_count(EventKind::Counter).unwrap(), 0);
    assert_eq!(store.pending_count(EventKind::Reading).unwrap(), 0);
    assert_eq!(
        server.finish()[0].body["readings"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

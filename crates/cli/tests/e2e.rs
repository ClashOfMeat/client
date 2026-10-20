use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

const ID: &str = "123e4567-e89b-12d3-a456-426614174000";
const TOKEN: &str = "0123456789012345678901234567890123456789";

struct Request {
    path: String,
    headers: BTreeMap<String, String>,
    body: Value,
}

struct Server {
    origin: String,
    received: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn start() -> Self {
        Self::with_pair_failure(None)
    }

    fn with_pair_failure(failure: Option<&'static str>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let received = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&received);
        let stopping = Arc::clone(&stop);
        let exchanges = AtomicUsize::new(0);
        let thread = thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(error) => panic!("accept: {error}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let request = read_request(&mut stream);
                let (status, body) = match request.path.as_str() {
                    "/api/pairings" => (
                        200,
                        json!({"id":ID,"code":"ABCDEF1234","exchangeSecret":"secret","expiresAt":(OffsetDateTime::now_utc()+time::Duration::minutes(10)).format(&Rfc3339).unwrap()}),
                    ),
                    "/api/pairings/exchange" if exchanges.fetch_add(1, Ordering::Relaxed) == 0 => {
                        (404, json!({"error":"pending"}))
                    }
                    "/api/pairings/exchange" if failure.is_some() => {
                        (410, json!({"error":"gone","code":failure.unwrap()}))
                    }
                    "/api/pairings/exchange" => (200, json!({"installationId":ID,"token":TOKEN})),
                    "/api/installations/heartbeat" => (200, json!({"connected":true})),
                    "/api/observations" => (200, json!({"accepted":10,"duplicates":0})),
                    other => panic!("unexpected request {other}"),
                };
                observed.lock().unwrap().push(request);
                let body = body.to_string();
                write!(stream, "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                stream.flush().unwrap();
            }
        });
        Self {
            origin,
            received,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn read_request(stream: &mut TcpStream) -> Request {
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut chunk = [0; 4096];
        let count = stream.read(&mut chunk).unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break index + 4;
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
    let length: usize = headers["content-length"].parse().unwrap();
    while bytes.len() - header_end < length {
        let mut chunk = [0; 4096];
        let count = stream.read(&mut chunk).unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
    }
    Request {
        path,
        headers,
        body: serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap(),
    }
}

struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn command(home: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_clashofmeat"));
    command
        .env("HOME", home)
        .env("CLASHOFMEAT_HOME", home.join(".clashofmeat"))
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME");
    command
}

#[test]
fn binary_pairs_and_uploads_real_claude_activity() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join(".claude/projects/demo");
    std::fs::create_dir_all(&dir).unwrap();
    let missing = command(home.path()).arg("status").output().unwrap();
    assert_eq!(missing.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("isn't running"));

    let server = Server::start();
    let daemon = Daemon(command(home.path()).arg("daemon").spawn().unwrap());
    let socket = home.path().join(".clashofmeat/daemon.sock");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !socket.exists() {
        assert!(Instant::now() < deadline, "daemon socket did not appear");
        assert!(daemon.0.id() > 0);
        thread::sleep(Duration::from_millis(25));
    }
    let linked = command(home.path())
        .args(["connect", "--app", &server.origin, "--no-open"])
        .output()
        .unwrap();
    assert!(
        linked.status.success(),
        "connect: {}",
        String::from_utf8_lossy(&linked.stderr)
    );
    assert!(String::from_utf8_lossy(&linked.stdout)
        .contains("Connected. Activity from now on is collected."));

    let at = OffsetDateTime::now_utc().format(&Rfc3339).unwrap();
    let session = "123e4567-e89b-12d3-a456-426614174001";
    let line = json!({"type":"user","sessionId":session,"uuid":"123e4567-e89b-12d3-a456-426614174002","timestamp":at,"message":{"content":"hello"}});
    let transcript = dir.join(format!("{session}.jsonl"));
    std::fs::write(&transcript, format!("{line}\n")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut last_write = Instant::now();
    let mut last_sync = None::<Instant>;
    loop {
        let status = command(home.path())
            .args(["--json", "status"])
            .output()
            .unwrap();
        let value: Value = serde_json::from_slice(&status.stdout).unwrap();
        let scanned = value["providers"].as_array().is_some_and(|providers| {
            providers.iter().any(|provider| {
                provider["id"] == "claude-code"
                    && provider["health"]["records"].as_u64().unwrap_or(0) > 0
            })
        });
        if scanned && last_sync.is_none_or(|at| at.elapsed() >= Duration::from_secs(2)) {
            let synced = command(home.path()).arg("sync").output().unwrap();
            assert!(
                synced.status.success(),
                "sync: {}",
                String::from_utf8_lossy(&synced.stderr)
            );
            last_sync = Some(Instant::now());
        }
        let found = server.received.lock().unwrap().iter().any(|request| {
            request.path == "/api/observations"
                && request.body["observations"]
                    .as_array()
                    .is_some_and(|items| {
                        items
                            .iter()
                            .any(|item| item["key"] == "prompt_count" && item["value"] == 1)
                    })
        });
        if found {
            break;
        }
        if !scanned && last_write.elapsed() >= Duration::from_secs(1) {
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(&transcript)
                .unwrap();
            writeln!(file, "{line}").unwrap();
            last_write = Instant::now();
        }
        if Instant::now() >= deadline {
            let paths: Vec<_> = server
                .received
                .lock()
                .unwrap()
                .iter()
                .map(|request| format!("{} {}", request.path, request.body))
                .collect();
            let status = command(home.path())
                .args(["--json", "status"])
                .output()
                .unwrap();
            panic!(
                "prompt_count upload missing; requests={paths:?}; status={}",
                String::from_utf8_lossy(&status.stdout)
            );
        }
        thread::sleep(Duration::from_millis(200));
    }
    let status = command(home.path())
        .args(["--json", "status"])
        .output()
        .unwrap();
    assert!(status.status.success());
    let value: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(value["paired"], true);
    assert_eq!(value["app_url"], server.origin);
    for request in server.received.lock().unwrap().iter() {
        assert_eq!(
            request.headers["x-clashofmeat-client"],
            format!("desktop/{}", env!("CARGO_PKG_VERSION"))
        );
    }
}

#[test]
fn declined_and_expired_pairing_use_old_messages() {
    for (reason, expected) in [
        (
            "declined",
            "Clash of Meat: Connection refused in the browser.",
        ),
        (
            "expired",
            "Clash of Meat: This code expired. Run the command again.",
        ),
    ] {
        let home = tempfile::tempdir().unwrap();
        let server = Server::with_pair_failure(Some(reason));
        let daemon = Daemon(command(home.path()).arg("daemon").spawn().unwrap());
        let socket = home.path().join(".clashofmeat/daemon.sock");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !socket.exists() {
            assert!(Instant::now() < deadline, "daemon socket did not appear");
            thread::sleep(Duration::from_millis(25));
        }
        let result = command(home.path())
            .args(["connect", "--app", &server.origin, "--no-open"])
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(1));
        assert_eq!(String::from_utf8_lossy(&result.stderr).trim(), expected);
        drop(daemon);
    }
}

//! A hero switched on the web reaches the overlay through the live channel within seconds.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use clashofmeat_core::env::Env;
use clashofmeat_core::store::Store;
use clashofmeat_daemon::config::{store_path, update_config};
use clashofmeat_daemon::run::{self, RunOptions};
use serde_json::{json, Value};

const TOKEN: &str = "0123456789012345678901234567890123456789";
const HERO_A: &str = "11111111-2222-4333-8444-555555555555";
const HERO_B: &str = "66666666-7777-4888-8999-aaaaaaaaaaaa";

/// Serves the heartbeat with the active hero and holds `/api/live` open until told to announce
/// a `companion:` revision, as the web app does when the player switches hero.
struct Server {
    origin: String,
    switched: Arc<AtomicBool>,
    live: mpsc::Receiver<()>,
    announce: Mutex<Option<mpsc::Sender<()>>>,
}

impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let switched = Arc::new(AtomicBool::new(false));
        let (live_tx, live) = mpsc::channel();
        let (announce_tx, announce_rx) = mpsc::channel::<()>();
        let announce_rx = Arc::new(Mutex::new(announce_rx));
        let active = switched.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let (active, live_tx, announce_rx) =
                    (active.clone(), live_tx.clone(), announce_rx.clone());
                thread::spawn(move || serve(stream, &active, &live_tx, &announce_rx));
            }
        });
        Self {
            origin,
            switched,
            live,
            announce: Mutex::new(Some(announce_tx)),
        }
    }

    fn switch_hero(&self) {
        self.switched.store(true, Ordering::SeqCst);
        if let Some(announce) = self.announce.lock().unwrap().take() {
            announce.send(()).unwrap();
        }
    }
}

fn serve(
    mut stream: TcpStream,
    switched: &AtomicBool,
    live: &mpsc::Sender<()>,
    announce: &Mutex<mpsc::Receiver<()>>,
) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut first = String::new();
    if reader.read_line(&mut first).is_err() {
        return;
    }
    let mut length = 0;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0; length];
    let _ = reader.read_exact(&mut body);
    if first.starts_with("GET /api/live") {
        let _ = stream.write_all(
            b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\nretry: 3000\n\n",
        );
        let _ = stream.flush();
        let _ = live.send(());
        if announce.lock().unwrap().recv().is_ok() {
            let _ = stream.write_all(
                b"event: revision\ndata: [{\"topic\":\"companion:player\",\"revision\":\"2\"}]\n\n",
            );
            let _ = stream.flush();
        }
        // Held open like a real channel, until the daemon goes away.
        thread::sleep(Duration::from_secs(30));
        return;
    }
    let reply = if first.starts_with("POST /api/installations/heartbeat") {
        let id = if switched.load(Ordering::SeqCst) {
            HERO_B
        } else {
            HERO_A
        };
        json!({"connected": true, "character": {"id": id, "level": 1}, "player": {"handle": "paul"}})
    } else {
        json!({})
    };
    let body = reply.to_string();
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
}

fn socket_request(
    stream: &mut UnixStream,
    reader: &mut BufReader<UnixStream>,
    method: &str,
) -> Value {
    writeln!(stream, "{}", json!({"id": 1, "method": method})).unwrap();
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

/// The id of the next hero the daemon broadcasts.
fn next_hero(reader: &mut BufReader<UnixStream>) -> String {
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).expect("a heartbeat event");
        let event: Value = serde_json::from_str(&line).unwrap();
        if event["event"] == "heartbeat" {
            return event["data"]["character"]["id"]
                .as_str()
                .unwrap()
                .to_owned();
        }
    }
}

#[test]
fn a_hero_switch_on_the_web_reaches_the_overlay_within_seconds() {
    let dir = tempfile::tempdir().unwrap();
    let env = Env::for_home(dir.path());
    let server = Server::start();
    update_config(&env, |config| config.app_url = Some(server.origin.clone())).unwrap();
    let mut store = Store::open(store_path(&env)).unwrap();
    store.set_meta("installation.token", TOKEN).unwrap();
    store
        .set_meta("installation.id", "123e4567-e89b-12d3-a456-426614174000")
        .unwrap();
    drop(store);

    let handle = run::start(RunOptions {
        env,
        version: "0.2.0".into(),
    })
    .unwrap();
    let mut events = UnixStream::connect(handle.socket_path()).unwrap();
    events
        .set_read_timeout(Some(Duration::from_secs(8)))
        .unwrap();
    let mut reader = BufReader::new(events.try_clone().unwrap());
    socket_request(&mut events, &mut reader, "subscribe");
    let mut command = UnixStream::connect(handle.socket_path()).unwrap();
    let mut command_reader = BufReader::new(command.try_clone().unwrap());
    socket_request(&mut command, &mut command_reader, "sync_now");
    assert_eq!(next_hero(&mut reader), HERO_A);
    server
        .live
        .recv_timeout(Duration::from_secs(10))
        .expect("the daemon follows the live channel");

    // Right after a sync, where the player topic would wait 10 s or more.
    let switched_at = Instant::now();
    server.switch_hero();
    assert_eq!(next_hero(&mut reader), HERO_B);
    let took = switched_at.elapsed();
    eprintln!("hero switch reached the overlay in {took:?}");
    assert!(took < Duration::from_secs(4), "took {took:?}");
    handle.stop().unwrap();
}

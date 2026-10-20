//! Local socket fixture for exercising the desktop overlay without a server.
//! Run with `cargo run --offline -p clashofmeat-daemon --example fake_daemon -- --home /tmp/meat-fake --script cycle`.

#[path = "fake/sprite.rs"]
mod sprite;

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

const HERO_ID: &str = "11111111-2222-4333-8444-555555555555";
const VISITOR_ID: &str = "66666666-7777-4888-8999-aaaaaaaaaaaa";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Script {
    Idle,
    Busy,
    Cycle,
    Xp,
    Visit,
    Offline,
}

impl Script {
    fn parse(name: &str) -> Result<Self> {
        match name {
            "idle" => Ok(Self::Idle),
            "busy" => Ok(Self::Busy),
            "cycle" => Ok(Self::Cycle),
            "xp" => Ok(Self::Xp),
            "visit" => Ok(Self::Visit),
            "offline" => Ok(Self::Offline),
            _ => {
                bail!("unknown script {name:?}; expected idle, busy, cycle, xp, visit, or offline")
            }
        }
    }

    fn cycles(self) -> bool {
        !matches!(self, Self::Idle | Self::Busy)
    }
}

struct State {
    script: Script,
    activity: Value,
    hero: Value,
    providers_off: Vec<String>,
    subscribers: Vec<UnixStream>,
    clients: HashMap<u64, UnixStream>,
    next_client: u64,
    started: Instant,
    cycle: u64,
    cycle_phase: u8,
    heartbeats: u64,
    accepting_since: Option<Instant>,
}

fn main() -> Result<()> {
    let (home, script) = args()?;
    let mut listener = Some(bind(&home)?);
    println!(
        "fake daemon listening on {} ({})",
        home.join("daemon.sock").display(),
        script_name(script)
    );
    let animation = sprite::animation();
    let hero = hero(&animation, 12, 6200, 8000, None, None);
    let activity = if script == Script::Busy {
        busy("claude-code")
    } else {
        idle()
    };
    let state = Arc::new(Mutex::new(State {
        script,
        activity,
        hero,
        providers_off: Vec::new(),
        subscribers: Vec::new(),
        clients: HashMap::new(),
        next_client: 0,
        started: Instant::now(),
        cycle: 0,
        cycle_phase: u8::MAX,
        heartbeats: 0,
        accepting_since: None,
    }));

    loop {
        if let Some(socket) = &listener {
            match socket.accept() {
                Ok((stream, _)) => {
                    // macOS can inherit O_NONBLOCK from the listener. Large PNG replies
                    // need a blocking client stream or writes can stop mid-JSON.
                    stream.set_nonblocking(false)?;
                    let client_id = {
                        let mut shared = state.lock().expect("state lock");
                        let id = shared.next_client;
                        shared.next_client += 1;
                        shared.clients.insert(id, stream.try_clone()?);
                        if script == Script::Offline && shared.accepting_since.is_none() {
                            shared.accepting_since = Some(Instant::now());
                        }
                        id
                    };
                    let state = Arc::clone(&state);
                    thread::spawn(move || serve_client(stream, state, client_id));
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error.into()),
            }
        }

        let elapsed = state.lock().expect("state lock").started.elapsed();
        if script.cycles() {
            tick_cycle(&state, elapsed);
        }
        if matches!(script, Script::Xp | Script::Visit) {
            tick_heartbeat(&state, elapsed);
        }
        if script == Script::Offline {
            let accepted_for = state
                .lock()
                .expect("state lock")
                .accepting_since
                .map(|at| at.elapsed());
            if listener.is_some()
                && accepted_for.is_some_and(|duration| duration >= Duration::from_secs(10))
            {
                listener = None;
                let mut shared = state.lock().expect("state lock");
                for (_, client) in shared.clients.drain() {
                    let _ = client.shutdown(Shutdown::Both);
                }
                shared.subscribers.clear();
                let _ = fs::remove_file(home.join("daemon.sock"));
                println!("socket offline for 5s");
            } else if listener.is_none()
                && accepted_for.is_some_and(|duration| duration >= Duration::from_secs(15))
            {
                listener = Some(bind(&home)?);
                state.lock().expect("state lock").accepting_since = None;
                println!("socket listening again");
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn args() -> Result<(PathBuf, Script)> {
    let mut args = env::args().skip(1);
    let mut home = None;
    let mut script = Script::Cycle;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--home" => {
                home = Some(PathBuf::from(
                    args.next().context("--home needs a directory")?,
                ))
            }
            "--script" => script = Script::parse(&args.next().context("--script needs a name")?)?,
            "--help" | "-h" => {
                println!(
                    "Usage: fake_daemon --home <dir> --script <idle|busy|cycle|xp|visit|offline>"
                );
                std::process::exit(0);
            }
            _ => bail!("unknown argument {arg:?}"),
        }
    }
    Ok((home.context("--home is required")?, script))
}

fn script_name(script: Script) -> &'static str {
    match script {
        Script::Idle => "idle",
        Script::Busy => "busy",
        Script::Cycle => "cycle",
        Script::Xp => "xp",
        Script::Visit => "visit",
        Script::Offline => "offline",
    }
}

fn bind(home: &Path) -> Result<UnixListener> {
    fs::create_dir_all(home)?;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(home, fs::Permissions::from_mode(0o700))?;
    let path = home.join("daemon.sock");
    if path.exists() {
        if UnixStream::connect(&path).is_ok() {
            bail!("another daemon is already running at {}", path.display());
        }
        fs::remove_file(&path)
            .with_context(|| format!("removing stale socket {}", path.display()))?;
    }
    let listener = UnixListener::bind(&path)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

fn serve_client(stream: UnixStream, state: Arc<Mutex<State>>, client_id: u64) {
    serve_client_inner(stream, &state);
    state.lock().expect("state lock").clients.remove(&client_id);
}

fn serve_client_inner(mut stream: UnixStream, state: &Arc<Mutex<State>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    let mut reader = match stream.try_clone() {
        Ok(clone) => BufReader::new(clone),
        Err(_) => return,
    };
    let mut line = String::new();
    loop {
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                if line.len() > 1_048_576 {
                    break;
                }
                continue;
            }
            Err(_) => break,
        }
        let parsed = serde_json::from_str::<Value>(&line);
        let Ok(request) = parsed else {
            if write_line(
                &mut stream,
                &json!({"id":null,"error":{"code":"bad_request","message":"invalid request"}}),
            )
            .is_err()
            {
                break;
            }
            line.clear();
            continue;
        };
        let Some(method) = request.get("method").and_then(Value::as_str) else {
            if write_line(
                &mut stream,
                &json!({"id":null,"error":{"code":"bad_request","message":"invalid request"}}),
            )
            .is_err()
            {
                break;
            }
            line.clear();
            continue;
        };
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let is_subscribe = method == "subscribe";
        let response = {
            let mut shared = state.lock().expect("state lock");
            match handle(&mut shared, method, request.get("params")) {
                Ok(value) => json!({"id":id,"result":value}),
                Err((code, message)) => json!({"id":id,"error":{"code":code,"message":message}}),
            }
        };
        if let Err(error) = write_line(&mut stream, &response) {
            eprintln!("fake daemon socket write: {error}");
            break;
        }
        line.clear();
        if is_subscribe && response.get("result").is_some() {
            if let Ok(clone) = stream.try_clone() {
                let _ = clone.set_write_timeout(Some(Duration::from_millis(500)));
                state.lock().expect("state lock").subscribers.push(clone);
            }
            // The real daemon dedicates a subscribed connection to events.
            loop {
                match reader.read_line(&mut line) {
                    Ok(0) => return,
                    Ok(_) => line.clear(),
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                    {
                        continue
                    }
                    Err(_) => return,
                }
            }
        }
    }
}

fn handle(
    state: &mut State,
    method: &str,
    params: Option<&Value>,
) -> std::result::Result<Value, (&'static str, &'static str)> {
    match method {
        "status" => Ok(status(state)),
        "snapshot" => Ok(json!({"activity":state.activity,"hero":state.hero})),
        "subscribe" => Ok(json!({"subscribed":true})),
        "sync_now" => Ok(json!({"queued":true})),
        "pair_start" => Ok(
            json!({"link":"http://localhost:3000/link?code=FAKE123456","code":"FAKE123456","expires_at":timestamp(300)}),
        ),
        "pair_status" => Ok(json!({"status":"paired"})),
        "set_provider_enabled" => {
            let id = params
                .and_then(|value| value.get("id"))
                .and_then(Value::as_str);
            let enabled = params
                .and_then(|value| value.get("enabled"))
                .and_then(Value::as_bool);
            let (Some(id), Some(enabled)) = (id, enabled) else {
                return Err(("bad_request", "id and enabled are required"));
            };
            if !PROVIDERS.iter().any(|(provider, _)| *provider == id) {
                return Err(("bad_request", "unknown provider"));
            }
            state.providers_off.retain(|provider| provider != id);
            if !enabled {
                state.providers_off.push(id.to_string());
            }
            Ok(json!({"id":id,"enabled":enabled}))
        }
        _ => Err(("unknown_method", "unknown method")),
    }
}

const PROVIDERS: [(&str, &str); 5] = [
    ("claude-code", "Claude Code"),
    ("codex", "Codex"),
    ("cursor", "Cursor"),
    ("claude-cowork", "Claude Cowork"),
    ("gemini", "Gemini CLI"),
];

fn status(state: &State) -> Value {
    let providers: Vec<Value> = PROVIDERS.iter().map(|(id, name)| json!({
        "id":id,"name":name,"detected":true,"enabled":!state.providers_off.iter().any(|off| off == id),
        "health":{"records":42,"malformed":0,"oversized":0,"errors":0,"last_at":timestamp(0)}
    })).collect();
    json!({
        "version":"fake", "paired":true, "app_url":"http://localhost:3000",
        "blocked":null, "reauth":false, "pending":{"counters":0,"readings":0},
        "last_sync_at":timestamp(0), "providers":providers
    })
}

fn hero(
    animation: &Value,
    level: u64,
    xp: u64,
    next_level_at: u64,
    level_up: Option<Value>,
    visit: Option<Value>,
) -> Value {
    json!({
        "character":{"id":HERO_ID,"heroId":"barbarian","png":animation["frames"][0],
            "level":level,"xp":xp,"nextLevelAt":next_level_at,"name":"Barbarian"},
        "animation":animation, "levelUp":level_up.unwrap_or_else(|| sprite::level_up(11, 12, 0)),
        "progression":{"todayTokens":12540 + xp - 6200},
        "player":{"name":"Barbarian"}, "visit":visit
    })
}

fn idle() -> Value {
    json!({"busy":false,"sessions":[]})
}

fn busy(provider: &str) -> Value {
    json!({"busy":true,"sessions":[{"provider":provider,"session_id":format!("fake-{provider}"),
        "state":"busy","last_at":timestamp(0)}]})
}

fn done(provider: &str) -> Value {
    json!({"busy":false,"sessions":[{"provider":provider,"session_id":format!("fake-{provider}"),
        "state":"done","last_at":timestamp(0)}]})
}

fn tick_cycle(state: &Arc<Mutex<State>>, elapsed: Duration) {
    let mut shared = state.lock().expect("state lock");
    let in_cycle = elapsed.as_secs() % 60;
    let cycle = elapsed.as_secs() / 60;
    let phase = if in_cycle < 20 {
        0
    } else if in_cycle < 21 {
        1
    } else {
        2
    };
    if shared.cycle != cycle || shared.cycle_phase != phase {
        let provider = if cycle.is_multiple_of(2) {
            "claude-code"
        } else {
            "codex"
        };
        let activity = match phase {
            0 => busy(provider),
            1 => done(provider),
            _ => idle(),
        };
        if shared.activity != activity {
            shared.activity = activity.clone();
            broadcast(&mut shared, "activity", activity);
        }
        shared.cycle = cycle;
        shared.cycle_phase = phase;
    }
}

fn tick_heartbeat(state: &Arc<Mutex<State>>, elapsed: Duration) {
    let due = elapsed.as_secs() / 15;
    let mut shared = state.lock().expect("state lock");
    if due <= shared.heartbeats {
        return;
    }
    shared.heartbeats = due;
    let old_level = shared.hero["character"]["level"].as_u64().unwrap_or(12);
    let level = old_level + u64::from(due.is_multiple_of(3));
    let xp = 6200 + due * 350;
    let next = 8000 + (level - 12) * 2000;
    let level_up = due
        .is_multiple_of(3)
        .then(|| sprite::level_up(old_level, level, due));
    let visit = if shared.script == Script::Visit && due == 3 {
        let animation = &shared.hero["animation"];
        Some(
            json!({"id":VISITOR_ID,"name":"Rogue","guild":"The Party","expiresAt":timestamp(120),
            "character":{"id":VISITOR_ID,"png":animation["frames"][0],"animation":animation,
                "level":9,"heroId":"rogue"}}),
        )
    } else {
        None
    };
    let animation = shared.hero["animation"].clone();
    shared.hero = hero(&animation, level, xp, next, level_up, visit);
    let payload = shared.hero.clone();
    broadcast(&mut shared, "heartbeat", payload);
}

fn broadcast(state: &mut State, event: &str, data: Value) {
    let message = json!({"event":event,"data":data});
    state
        .subscribers
        .retain_mut(|subscriber| write_line(subscriber, &message).is_ok());
    if event == "heartbeat" {
        println!(
            "event heartbeat xp={} level={} visit={}",
            data["character"]["xp"],
            data["character"]["level"],
            !data["visit"].is_null()
        );
    } else {
        println!(
            "event activity busy={} sessions={}",
            data["busy"],
            data["sessions"].as_array().map_or(0, Vec::len)
        );
    }
}

fn write_line(stream: &mut UnixStream, value: &Value) -> std::io::Result<()> {
    serde_json::to_writer(&mut *stream, value)?;
    stream.write_all(b"\n")
}

fn timestamp(seconds_from_now: i64) -> String {
    (OffsetDateTime::now_utc() + time::Duration::seconds(seconds_from_now))
        .format(&Rfc3339)
        .expect("RFC3339 timestamp")
}

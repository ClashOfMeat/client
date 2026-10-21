use std::env;
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
#[cfg(windows)]
use uds_windows::UnixStream;

#[derive(Default)]
pub struct DaemonState(pub Mutex<DesktopState>);

#[derive(Default, Clone, Serialize)]
pub struct DesktopState {
    pub connected: bool,
    pub status: Option<Value>,
    pub snapshot: Option<Value>,
    pub heartbeat: Option<Value>,
}

pub fn state(app: &AppHandle) -> DesktopState {
    app.state::<DaemonState>().0.lock().unwrap().clone()
}

#[derive(Clone, Serialize)]
struct DaemonMessage {
    kind: &'static str,
    data: Value,
}

#[derive(Debug, PartialEq)]
enum ParsedLine {
    Result {
        id: u64,
        data: Value,
    },
    Event {
        name: String,
        data: Value,
    },
    Error {
        id: u64,
        code: String,
        message: String,
    },
}

fn parse_line(line: &str) -> Result<ParsedLine, String> {
    let value: Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
    if let Some(name) = value.get("event").and_then(Value::as_str) {
        return Ok(ParsedLine::Event {
            name: name.to_owned(),
            data: value.get("data").cloned().unwrap_or(Value::Null),
        });
    }
    let id = value
        .get("id")
        .and_then(Value::as_u64)
        .ok_or("missing response id")?;
    if let Some(result) = value.get("result") {
        return Ok(ParsedLine::Result {
            id,
            data: result.clone(),
        });
    }
    let error = value.get("error").ok_or("missing result or error")?;
    Ok(ParsedLine::Error {
        id,
        code: error["code"].as_str().unwrap_or("unknown").to_owned(),
        message: error["message"]
            .as_str()
            .unwrap_or("daemon error")
            .to_owned(),
    })
}

fn home() -> PathBuf {
    env::var_os("CLASHOFMEAT_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            env::var_os("HOME")
                .or_else(|| env::var_os("USERPROFILE"))
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".clashofmeat")
        })
}

fn socket_path() -> PathBuf {
    home().join("daemon.sock")
}

/// Windows has no LaunchAgent: the app starts the daemon (installed next to it) when none answers.
/// It runs without a console window and outlives the app, like the macOS agent.
#[cfg(windows)]
fn spawn_daemon() {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    let Some(exe) = env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join("clashofmeat.exe")))
    else {
        return;
    };
    let logs = home().join("logs");
    let _ = std::fs::create_dir_all(&logs);
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(logs.join("daemon.log"));
    let mut command = std::process::Command::new(exe);
    command
        .arg("daemon")
        .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
        .stdin(std::process::Stdio::null());
    if let Ok(log) = log {
        if let Ok(copy) = log.try_clone() {
            command.stdout(copy);
        }
        command.stderr(log);
    }
    if let Err(error) = command.spawn() {
        eprintln!("daemon start: {error}");
    }
}

fn request(stream: &mut UnixStream, id: u64, method: &str, params: &Value) -> Result<(), String> {
    writeln!(
        stream,
        "{}",
        json!({"id":id,"method":method,"params":params})
    )
    .map_err(|e| e.to_string())
}

fn emit(app: &AppHandle, kind: &'static str, data: Value) {
    {
        let binding = app.state::<DaemonState>();
        let mut state = binding.0.lock().unwrap();
        match kind {
            "snapshot" => state.snapshot = Some(data.clone()),
            "status" if data.get("connected").is_some() => {
                state.connected = data["connected"].as_bool().unwrap_or(false)
            }
            "status" => state.status = Some(data.clone()),
            "event" => {
                if data["event"] == "heartbeat" {
                    state.heartbeat = Some(data["data"].clone());
                }
                if data["event"] == "activity" {
                    if let Some(snapshot) = state.snapshot.as_mut() {
                        snapshot["activity"] = data["data"].clone();
                    }
                }
                if data["event"] == "paired" {
                    if let Some(status) = state.status.as_mut() {
                        status["paired"] = true.into();
                    }
                }
            }
            _ => {}
        }
    }
    let _ = app.emit("daemon", DaemonMessage { kind, data });
}

fn connect(app: &AppHandle) -> Result<(), String> {
    let mut stream = UnixStream::connect(socket_path()).map_err(|e| e.to_string())?;
    request(&mut stream, 1, "snapshot", &json!({}))?;
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    let mut line = String::new();
    if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
        return Err("daemon closed before snapshot".into());
    }
    match parse_line(&line)? {
        ParsedLine::Result { id: 1, data } => emit(app, "snapshot", data),
        ParsedLine::Error { message, .. } => return Err(message),
        _ => return Err("unexpected snapshot response".into()),
    }
    request(&mut stream, 2, "subscribe", &json!({}))?;
    line.clear();
    if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
        return Err("daemon closed before subscribe".into());
    }
    match parse_line(&line)? {
        ParsedLine::Result { id: 2, .. } => {
            emit(app, "status", json!({"connected":true}));
            refresh_status(app);
        }
        ParsedLine::Error { message, .. } => return Err(message),
        _ => return Err("unexpected subscribe response".into()),
    }
    loop {
        line.clear();
        if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            return Err("daemon connection closed".into());
        }
        match parse_line(&line)? {
            ParsedLine::Event { name, data } => {
                let refresh = matches!(name.as_str(), "sync" | "paired" | "pair_failed");
                emit(app, "event", json!({"event":name,"data":data}));
                if refresh {
                    refresh_status(app);
                }
            }
            _ => return Err("unexpected subscription line".into()),
        }
    }
}

pub fn start(app: AppHandle) {
    thread::spawn(move || {
        let mut delay = Duration::from_millis(500);
        #[cfg(windows)]
        let mut spawned_at: Option<std::time::Instant> = None;
        loop {
            if let Err(error) = connect(&app) {
                eprintln!("daemon connection: {error}");
                #[cfg(windows)]
                if spawned_at.is_none_or(|at| at.elapsed() > Duration::from_secs(30)) {
                    spawn_daemon();
                    spawned_at = Some(std::time::Instant::now());
                }
            }
            emit(&app, "status", json!({"connected":false}));
            thread::sleep(delay);
            delay = (delay * 2).min(Duration::from_secs(5));
        }
    });
}

pub fn send(method: &str) -> Result<Value, String> {
    send_with_params(method, json!({}))
}

pub fn send_with_params(method: &str, params: Value) -> Result<Value, String> {
    let mut stream = UnixStream::connect(socket_path()).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(
            if matches!(method, "pair_start" | "evolve" | "level_up") {
                30
            } else {
                5
            },
        )))
        .map_err(|e| e.to_string())?;
    request(&mut stream, 1, method, &params)?;
    let mut line = String::new();
    if BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?
        == 0
    {
        return Err("daemon closed connection".into());
    }
    match parse_line(&line)? {
        ParsedLine::Result { id: 1, data } => Ok(data),
        ParsedLine::Error { message, .. } => Err(message),
        _ => Err("unexpected daemon response".into()),
    }
}

pub fn refresh_status(app: &AppHandle) {
    if let Ok(value) = send("status") {
        emit(app, "status", value);
    }
}

pub fn publish_status(app: &AppHandle, value: Value) {
    emit(app, "status", value);
}

#[cfg(test)]
mod tests {
    use super::{parse_line, ParsedLine};
    use serde_json::json;

    #[test]
    fn parses_result_event_and_error_lines() {
        assert_eq!(
            parse_line("{\"id\":1,\"result\":{\"busy\":true}}\n"),
            Ok(ParsedLine::Result {
                id: 1,
                data: json!({"busy":true})
            })
        );
        assert_eq!(
            parse_line("{\"event\":\"activity\",\"data\":{\"busy\":false}}"),
            Ok(ParsedLine::Event {
                name: "activity".into(),
                data: json!({"busy":false})
            })
        );
        assert_eq!(
            parse_line("{\"id\":2,\"error\":{\"code\":\"bad\",\"message\":\"oops\"}}"),
            Ok(ParsedLine::Error {
                id: 2,
                code: "bad".into(),
                message: "oops".into()
            })
        );
    }

    #[test]
    fn rejects_malformed_lines() {
        for line in ["", "[]", "{}", "{\"id\":1}", "{\"event\":9}"] {
            assert!(parse_line(line).is_err(), "{line}");
        }
    }
}

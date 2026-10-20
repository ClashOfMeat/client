use std::env;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

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

fn socket_path() -> PathBuf {
    let home = env::var_os("CLASHOFMEAT_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".clashofmeat")
        });
    home.join("daemon.sock")
}

fn request(stream: &mut UnixStream, id: u64, method: &str) -> Result<(), String> {
    writeln!(stream, "{}", json!({"id":id,"method":method,"params":{}})).map_err(|e| e.to_string())
}

fn emit(app: &AppHandle, kind: &'static str, data: Value) {
    let _ = app.emit_to("hero", "daemon", DaemonMessage { kind, data });
}

fn connect(app: &AppHandle) -> Result<(), String> {
    let mut stream = UnixStream::connect(socket_path()).map_err(|e| e.to_string())?;
    request(&mut stream, 1, "snapshot")?;
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
    request(&mut stream, 2, "subscribe")?;
    line.clear();
    if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
        return Err("daemon closed before subscribe".into());
    }
    match parse_line(&line)? {
        ParsedLine::Result { id: 2, .. } => emit(app, "status", json!({"connected":true})),
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
                emit(app, "event", json!({"event":name,"data":data}))
            }
            _ => return Err("unexpected subscription line".into()),
        }
    }
}

pub fn start(app: AppHandle) {
    thread::spawn(move || {
        let mut delay = Duration::from_millis(500);
        loop {
            if let Err(error) = connect(&app) {
                eprintln!("daemon connection: {error}");
            }
            emit(&app, "status", json!({"connected":false}));
            thread::sleep(delay);
            delay = (delay * 2).min(Duration::from_secs(5));
        }
    });
}

pub fn send(method: &str) -> Result<Value, String> {
    let mut stream = UnixStream::connect(socket_path()).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    request(&mut stream, 1, method)?;
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

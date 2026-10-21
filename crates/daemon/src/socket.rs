//! Local newline-delimited JSON socket transport.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use clashofmeat_core::ipc::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use crossbeam_channel::{bounded, Receiver, Sender};
use serde_json::{json, Value};

/// A parsed socket request sent to the single-owner main loop.
pub struct Request {
    pub method: String,
    pub params: Value,
    pub reply: Sender<Result<Value, SocketError>>,
    pub subscriber: Option<Sender<Value>>,
}

/// An API error returned on the socket.
#[derive(Debug)]
pub struct SocketError {
    pub code: &'static str,
    pub message: String,
}

impl SocketError {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Bound socket listener, started before daemon threads.
pub struct BoundSocket {
    listener: UnixListener,
    path: PathBuf,
}

impl BoundSocket {
    /// Binds a private socket, replacing a stale endpoint.
    pub fn bind(home: &Path) -> Result<Self> {
        fs::create_dir_all(home)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(home, fs::Permissions::from_mode(0o700))?;
        }
        let path = home.join("daemon.sock");
        if path.exists() {
            if UnixStream::connect(&path).is_ok() {
                bail!("another daemon is already running at {}", path.display());
            }
            fs::remove_file(&path)
                .with_context(|| format!("removing stale socket {}", path.display()))?;
        }
        let listener = UnixListener::bind(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        }
        listener.set_nonblocking(true)?;
        Ok(Self { listener, path })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn serve(
        self,
        requests: Sender<crate::run::Message>,
        stopping: Arc<AtomicBool>,
    ) -> JoinHandle<()> {
        thread::spawn(move || {
            let mut clients = Vec::new();
            while !stopping.load(Ordering::Relaxed) {
                match self.listener.accept() {
                    Ok((stream, _)) => {
                        if stream.set_nonblocking(false).is_err() {
                            continue;
                        }
                        let requests = requests.clone();
                        let stopping = stopping.clone();
                        clients.push(thread::spawn(move || {
                            serve_client(stream, requests, stopping)
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(50))
                    }
                    Err(error) => {
                        eprintln!("daemon socket accept: {error}");
                        break;
                    }
                }
            }
            for client in clients {
                let _ = client.join();
            }
        })
    }
}

impl Drop for BoundSocket {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn serve_client(
    mut stream: UnixStream,
    requests: Sender<crate::run::Message>,
    stopping: Arc<AtomicBool>,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    let mut reader = match stream.try_clone() {
        Ok(stream) => BufReader::new(stream),
        Err(_) => return,
    };
    let mut line = String::new();
    while !stopping.load(Ordering::Relaxed) {
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error)
                if error.kind() == std::io::ErrorKind::TimedOut
                    || error.kind() == std::io::ErrorKind::WouldBlock =>
            {
                if line.len() > 1_048_576 {
                    break;
                }
                continue;
            }
            Err(_) => break,
        }
        let parsed = serde_json::from_str::<Value>(&line);
        let value = match parsed {
            Ok(value)
                if value.is_object() && value.get("method").and_then(Value::as_str).is_some() =>
            {
                value
            }
            _ => {
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
            }
        };
        let id = value.get("id").cloned().unwrap_or(Value::Null);
        let method = value["method"].as_str().unwrap_or_default().to_string();
        let (reply, response) = bounded(1);
        let (event_tx, event_rx) = bounded(128);
        let is_subscribe = method == "subscribe";
        let request = Request {
            method,
            params: value.get("params").cloned().unwrap_or_else(|| json!({})),
            reply,
            subscriber: is_subscribe.then_some(event_tx),
        };
        if requests.send(crate::run::Message::Socket(request)).is_err() {
            break;
        }
        let result = match response.recv() {
            Ok(result) => result,
            Err(_) => break,
        };
        let response = match result {
            Ok(result) => json!({"id":id,"result":result}),
            Err(error) => json!({"id":id,"error":{"code":error.code,"message":error.message}}),
        };
        if write_line(&mut stream, &response).is_err() {
            break;
        }
        line.clear();
        if is_subscribe && response.get("result").is_some() {
            stream_events(&mut stream, event_rx, &stopping);
            break;
        }
    }
}

fn stream_events(stream: &mut UnixStream, events: Receiver<Value>, stopping: &AtomicBool) {
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    while !stopping.load(Ordering::Relaxed) {
        match events.recv_timeout(Duration::from_millis(200)) {
            Ok(event) => {
                if write_line(stream, &event).is_err() {
                    break;
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(_) => break,
        }
    }
}

fn write_line(stream: &mut UnixStream, value: &Value) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(value)?;
    line.push(b'\n');
    stream.write_all(&line)
}

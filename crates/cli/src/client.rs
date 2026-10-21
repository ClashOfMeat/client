use std::fmt;
use std::io::{BufRead, BufReader, Write};
use clashofmeat_core::ipc::UnixStream;
use std::time::Duration;

use anyhow::{anyhow, Result};
use clashofmeat_core::Env;
use clashofmeat_daemon::config;
use serde_json::{json, Value};

#[derive(Debug)]
pub struct NotRunning;

impl fmt::Display for NotRunning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Clash of Meat isn't running. Start it with `clashofmeat daemon` (the app starts it for you).")
    }
}
impl std::error::Error for NotRunning {}

pub struct Client {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
    next_id: u64,
}

impl Client {
    pub fn connect(env: &Env) -> Result<Self> {
        let stream =
            UnixStream::connect(config::home(env).join("daemon.sock")).map_err(|_| NotRunning)?;
        stream.set_read_timeout(Some(Duration::from_secs(20)))?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Self {
            stream,
            reader,
            next_id: 0,
        })
    }

    pub fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        self.next_id += 1;
        serde_json::to_writer(
            &mut self.stream,
            &json!({"id":self.next_id,"method":method,"params":params}),
        )?;
        self.stream.write_all(b"\n")?;
        let mut line = String::new();
        if self.reader.read_line(&mut line)? == 0 {
            return Err(anyhow!("Daemon closed the socket"));
        }
        let response: Value = serde_json::from_str(&line)?;
        if response["id"] != self.next_id {
            return Err(anyhow!("Daemon response ID mismatch"));
        }
        if let Some(error) = response.get("error") {
            return Err(anyhow!(
                "{}",
                error["message"].as_str().unwrap_or("Daemon request failed")
            ));
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| anyhow!("Daemon response missing result"))
    }

    pub fn subscribe(&mut self) -> Result<()> {
        self.request("subscribe", json!({}))?;
        Ok(())
    }

    pub fn next_event(&mut self, timeout: Duration) -> Result<Option<Value>> {
        self.reader.get_ref().set_read_timeout(Some(timeout))?;
        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            Ok(0) => Err(anyhow!("Daemon closed the event stream")),
            Ok(_) => Ok(Some(serde_json::from_str(&line)?)),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                Ok(None)
            }
            Err(error) => Err(error.into()),
        }
    }
}

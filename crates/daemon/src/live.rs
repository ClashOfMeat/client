//! Follows the player's live channel so changes made on the web, such as a hero
//! switch, reach the desktop on the next sync instead of the next heartbeat.

use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use clashofmeat_core::{Env, Store};

use crate::{config, sync};

/// Calls `changed` for every revision the server announces. Returns when
/// `stopping` is set or `changed` returns false.
pub fn follow(env: Env, version: String, stopping: Arc<AtomicBool>, changed: impl Fn() -> bool) {
    let store = match Store::open(config::store_path(&env)) {
        Ok(store) => store,
        Err(error) => {
            eprintln!("daemon live store: {error}");
            return;
        }
    };
    let mut backoff = Duration::from_secs(5);
    while !stopping.load(Ordering::Relaxed) {
        let started = Instant::now();
        let token = store.meta("installation.token").ok().flatten();
        let app_url = config::read_config(&env)
            .ok()
            .and_then(|config| config.app_url);
        if let (Some(token), Some(url)) = (token, app_url) {
            match sync::Client::new(&url, Some(token), &version).and_then(|client| client.live()) {
                Ok(reader) => {
                    if !read_events(reader, &stopping, &changed) {
                        return;
                    }
                }
                Err(sync::ApiError::Status {
                    status: 401 | 403, ..
                }) => backoff = Duration::from_secs(300),
                Err(_) => {}
            }
        }
        // A stream that lasted a while was healthy; reconnect soon after it drops.
        if started.elapsed() > Duration::from_secs(60) {
            backoff = Duration::from_secs(5);
        }
        let until = Instant::now() + backoff;
        while Instant::now() < until && !stopping.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_secs(1));
        }
        backoff = (backoff * 2).min(Duration::from_secs(300));
    }
}

/// Reads one event stream until it ends. False when the daemon no longer listens.
fn read_events(
    reader: Box<dyn std::io::Read + Send + Sync>,
    stopping: &AtomicBool,
    changed: &impl Fn() -> bool,
) -> bool {
    let mut event = String::new();
    for line in BufReader::new(reader).lines() {
        let Ok(line) = line else { break };
        if stopping.load(Ordering::Relaxed) {
            break;
        }
        if let Some(name) = line.strip_prefix("event:") {
            event = name.trim().to_owned();
        } else if line.starts_with("data:") && event == "revision" && !changed() {
            return false;
        } else if line.is_empty() {
            event.clear();
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn only_revision_events_count() {
        let stream = "retry: 3000\nevent: snapshot\ndata: [{\"topic\":\"player:x\",\"revision\":\"1\"}]\n\n: heartbeat\n\nevent: revision\ndata: [{\"topic\":\"player:x\",\"revision\":\"2\"}]\n\n";
        let seen = Cell::new(0);
        let reader: Box<dyn std::io::Read + Send + Sync> =
            Box::new(std::io::Cursor::new(stream.as_bytes().to_vec()));
        assert!(read_events(reader, &AtomicBool::new(false), &|| {
            seen.set(seen.get() + 1);
            true
        }));
        assert_eq!(seen.get(), 1);
    }
}

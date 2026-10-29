//! Follows the player's live channel so changes made on the web, such as a hero
//! switch, reach the desktop on the next sync instead of the next heartbeat.

use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use clashofmeat_core::{Env, Store};

use crate::{config, sync};

/// Calls `changed` for every revision the server announces, with whether it came on the
/// desktop's own `companion:<id>` topic. Returns when `stopping` is set or `changed` returns false.
pub fn follow(
    env: Env,
    version: String,
    stopping: Arc<AtomicBool>,
    changed: impl Fn(bool) -> bool,
) {
    let mut store = match Store::open(config::store_path(&env)) {
        Ok(store) => store,
        Err(error) => {
            eprintln!("daemon live store: {error}");
            return;
        }
    };
    let mut backoff = Duration::from_secs(5);
    while !stopping.load(Ordering::Relaxed) {
        let started = Instant::now();
        let app_url = config::read_config(&env)
            .ok()
            .and_then(|config| config.app_url);
        let token = sync::installation_token(&mut store, app_url.as_deref())
            .ok()
            .flatten();
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
        // Jittered, so a server restart doesn't bring every client back in the same second.
        let until = Instant::now() + crate::cadence::jitter(backoff);
        while Instant::now() < until && !stopping.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_secs(1));
        }
        backoff = (backoff * 2).min(Duration::from_secs(300));
    }
}

/// Reads one event stream until it ends. False when the daemon no longer listens. Every
/// revision counts, whatever its topic: `player:<id>`, or `companion:<id>` on newer servers.
/// Only `companion:` revisions are flagged: the server never puts our own uploads there.
fn read_events(
    reader: Box<dyn std::io::Read + Send + Sync>,
    stopping: &AtomicBool,
    changed: &impl Fn(bool) -> bool,
) -> bool {
    let mut event = String::new();
    for line in BufReader::new(reader).lines() {
        let Ok(line) = line else { break };
        if stopping.load(Ordering::Relaxed) {
            break;
        }
        if let Some(name) = line.strip_prefix("event:") {
            event = name.trim().to_owned();
        } else if let Some(data) = line.strip_prefix("data:").filter(|_| event == "revision") {
            if !changed(companion(data)) {
                return false;
            }
        } else if line.is_empty() {
            event.clear();
        }
    }
    true
}

/// Whether a revision batch, `[{"topic":..,"revision":..}]`, names a `companion:` topic.
fn companion(data: &str) -> bool {
    serde_json::from_str::<Vec<serde_json::Value>>(data.trim()).is_ok_and(|changes| {
        changes.iter().any(|change| {
            change["topic"]
                .as_str()
                .is_some_and(|topic| topic.starts_with("companion:"))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn only_revision_events_count_whatever_their_topic() {
        let stream = "retry: 3000\nevent: snapshot\ndata: [{\"topic\":\"player:x\",\"revision\":\"1\"}]\n\n: heartbeat\n\nevent: revision\ndata: [{\"topic\":\"player:x\",\"revision\":\"2\"}]\n\nevent: revision\ndata: [{\"topic\":\"companion:x\",\"revision\":\"3\"}]\n\n";
        let seen = RefCell::new(Vec::new());
        let reader: Box<dyn std::io::Read + Send + Sync> =
            Box::new(std::io::Cursor::new(stream.as_bytes().to_vec()));
        assert!(read_events(reader, &AtomicBool::new(false), &|companion| {
            seen.borrow_mut().push(companion);
            true
        }));
        // Only the companion revision is flagged, so the hero switch it carries is never
        // mistaken for the echo of an upload.
        assert_eq!(*seen.borrow(), [false, true]);
    }
}

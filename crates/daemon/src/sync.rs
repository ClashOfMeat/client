use std::cell::Cell;
use std::fmt;
use std::time::Duration;

use clashofmeat_core::env::Env;
use clashofmeat_core::ids::is_uuid;
use clashofmeat_core::observer;
use clashofmeat_core::signal::EventKind;
use clashofmeat_core::store::{Store, StoredEvent};
use serde_json::{json, Map, Value};
use time::format_description::well_known::{Rfc2822, Rfc3339};
use time::OffsetDateTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    Status {
        status: u16,
        message: String,
        /// The server's own words (`{error}`), when it sent any.
        detail: Option<String>,
        reason: Option<String>,
        retry_after_s: Option<u64>,
        setup_url: Option<String>,
    },
    Transport(String),
    Invalid(String),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Status { message, .. } | Self::Transport(message) | Self::Invalid(message) => {
                f.write_str(message)
            }
        }
    }
}

impl std::error::Error for ApiError {}

pub struct Client {
    pub origin: String,
    pub token: Option<String>,
    pub version: String,
    agent: ureq::Agent,
    /// The server's clock minus ours, in seconds, from the last answer's `Date` header.
    clock_skew: Cell<Option<i64>>,
}

/// A refused batch is halved until the refused observations stand alone; this caps the requests
/// one upload may make doing so (enough to isolate one or two bad rows among 150).
const MAX_SPLIT_REQUESTS: usize = 24;

impl Client {
    pub fn new(origin: &str, token: Option<String>, version: &str) -> Result<Self, ApiError> {
        Ok(Self {
            origin: crate::config::app_origin(origin)
                .map_err(|e| ApiError::Invalid(e.to_string()))?,
            token,
            version: version.to_owned(),
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(15))
                .redirects(0)
                .build(),
            clock_skew: Cell::new(None),
        })
    }

    /// How far the server's clock is ahead of ours, from its last `Date` header.
    pub fn clock_skew(&self) -> Option<i64> {
        self.clock_skew.get()
    }

    fn note_date(&self, result: &Result<ureq::Response, ureq::Error>) {
        let response = match result {
            Ok(response) | Err(ureq::Error::Status(_, response)) => response,
            Err(_) => return,
        };
        if let Some(date) = response
            .header("Date")
            .and_then(|date| OffsetDateTime::parse(date, &Rfc2822).ok())
        {
            self.clock_skew
                .set(Some((date - OffsetDateTime::now_utc()).whole_seconds()));
        }
    }

    /// Opens the player's live channel (server-sent events) on its own long-lived
    /// connection; the server sends a keepalive every 25 s.
    pub fn live(&self) -> Result<Box<dyn std::io::Read + Send + Sync>, ApiError> {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_secs(60))
            .redirects(0)
            .build();
        let mut request = agent
            .get(&format!("{}/api/live", self.origin))
            .set("x-clashofmeat-client", &format!("desktop/{}", self.version))
            .set("accept", "text/event-stream");
        if let Some(token) = &self.token {
            request = request.set("authorization", &format!("Bearer {token}"));
        }
        match request.call() {
            Ok(response) => Ok(Box::new(response.into_reader())),
            Err(ureq::Error::Status(status, _)) => Err(ApiError::Status {
                status,
                message: format!("Clash of Meat live channel returned {status}."),
                detail: None,
                reason: None,
                retry_after_s: None,
                setup_url: None,
            }),
            Err(error) => Err(ApiError::Transport(error.to_string())),
        }
    }

    fn post(&self, path: &str, body: &Value) -> Result<Value, ApiError> {
        let request = self
            .authorized(self.agent.post(&format!("{}{path}", self.origin)))
            .set("content-type", "application/json");
        let result = request.send_json(body);
        self.note_date(&result);
        answer(result)
    }

    fn get(&self, path: &str) -> Result<Value, ApiError> {
        let result = self
            .authorized(self.agent.get(&format!("{}{path}", self.origin)))
            .call();
        self.note_date(&result);
        answer(result)
    }

    fn authorized(&self, request: ureq::Request) -> ureq::Request {
        let request = request.set("x-clashofmeat-client", &format!("desktop/{}", self.version));
        match &self.token {
            Some(token) => request.set("authorization", &format!("Bearer {token}")),
            None => request,
        }
    }

    pub fn pair_start(&self, device: &str, tools: &[String]) -> Result<Pairing, ApiError> {
        let response = self.post("/api/pairings", &json!({"device": device, "tools": tools}))?;
        let id = required_string(&response, "id")?;
        let code = required_string(&response, "code")?;
        let exchange_secret = required_string(&response, "exchangeSecret")?;
        let expires_at = required_string(&response, "expiresAt")?;
        if !is_uuid(&id)
            || code.len() != 10
            || !code
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            || exchange_secret.is_empty()
            || OffsetDateTime::parse(&expires_at, &Rfc3339).is_err()
        {
            return Err(ApiError::Invalid(
                "Invalid Clash of Meat pairing response.".into(),
            ));
        }
        Ok(Pairing {
            id,
            code,
            exchange_secret,
            expires_at,
        })
    }

    pub fn pair_exchange(&self, pairing: &Pairing) -> Result<Option<Linked>, ApiError> {
        let response = match self.post(
            "/api/pairings/exchange",
            &json!({
                "id": pairing.id, "exchangeSecret": pairing.exchange_secret, "client": "collector"
            }),
        ) {
            Ok(response) => response,
            Err(ApiError::Status { status: 404, .. }) => return Ok(None),
            Err(error) => return Err(error),
        };
        let installation_id = required_string(&response, "installationId")?;
        let token = required_string(&response, "token")?;
        if !is_uuid(&installation_id) || token.len() < 32 {
            return Err(ApiError::Invalid(
                "Invalid Clash of Meat connection response.".into(),
            ));
        }
        Ok(Some(Linked {
            installation_id,
            token,
            origin: self.origin.clone(),
        }))
    }

    pub fn heartbeat(&self, extra: Value) -> Result<Value, ApiError> {
        let non_empty = extra.as_object().is_some_and(|fields| !fields.is_empty());
        match self.post("/api/installations/heartbeat", &extra) {
            Err(ApiError::Status { status: 400, .. }) if non_empty => {
                self.post("/api/installations/heartbeat", &json!({}))
            }
            result => result,
        }
    }

    /// Revokes this installation's token. Servers from before this endpoint answer 404 or 405.
    pub fn revoke(&self) -> Result<(), ApiError> {
        let result = self
            .authorized(
                self.agent
                    .delete(&format!("{}/api/installations/self", self.origin)),
            )
            .call();
        match result {
            Ok(_) | Err(ureq::Error::Status(404 | 405, _)) => Ok(()),
            Err(error) => answer(Err(error)).map(|_| ()),
        }
    }

    /// Evolves the active hero; the answer carries the new skin and its level-up effect.
    pub fn evolve(&self, character_id: &str, level: u64) -> Result<Value, ApiError> {
        self.post(
            "/api/companion/evolve",
            &json!({"characterId": character_id, "level": level}),
        )
    }

    /// The gold level-up effect for an ordinary level gain; it changes nothing on the server.
    pub fn level_up(&self, from: u64, to: u64) -> Result<Value, ApiError> {
        self.post(
            "/api/companion/level-up",
            &json!({"fromLevel": from, "toLevel": to}),
        )
    }

    /// The player's friends, those whose app is running first.
    pub fn friends(&self) -> Result<Value, ApiError> {
        let response = self.get("/api/companion/friends")?;
        let friends = response
            .get("friends")
            .and_then(Value::as_array)
            .ok_or_else(|| ApiError::Invalid("invalid response field: friends".into()))?;
        let text = |friend: &Value, key: &str| {
            friend
                .get(key)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty() && s.len() <= 80)
                .map(str::to_owned)
        };
        let friends: Vec<Value> = friends
            .iter()
            .filter_map(|friend| {
                let handle = text(friend, "handle")?;
                let name = text(friend, "name").unwrap_or_else(|| handle.clone());
                Some(json!({
                    "handle": handle, "name": name,
                    "online": friend.get("online").and_then(Value::as_bool).unwrap_or(false),
                    "level": friend.get("level").and_then(Value::as_u64),
                }))
            })
            .collect();
        Ok(json!({ "friends": friends }))
    }

    /// Sends the player's hero to a friend's screen to play one emote.
    pub fn send_visit(
        &self,
        target: &str,
        emote: &str,
        request_id: &str,
    ) -> Result<Value, ApiError> {
        self.post(
            "/api/companion/kiss",
            &json!({"target": target, "requestId": request_id, "emote": emote}),
        )
    }

    /// Uploads the oldest queued observations: up to 100 counters and 50 readings. A batch the
    /// server refuses (400 or 413) is halved until the refused observations stand alone; those
    /// are set aside locally (`Store::reject`, logged) so they no longer hold up the queue. When
    /// nothing at all is accepted, the refusal is more likely about the request than the data (a
    /// server older than this client, say), so everything stays queued.
    pub fn upload_observations(
        &self,
        store: &mut Store,
        now: OffsetDateTime,
    ) -> Result<UploadOutcome, ApiError> {
        let counters = store
            .pending(EventKind::Counter, 100)
            .map_err(store_error)?;
        let mut readings = store.pending(EventKind::Reading, 50).map_err(store_error)?;
        let expired: Vec<_> = readings
            .iter()
            .filter(|event| {
                OffsetDateTime::parse(&event.at, &Rfc3339)
                    .is_ok_and(|at| at < now - time::Duration::days(7))
            })
            .map(snapshot)
            .collect();
        // Quota snapshots have no historical upload mode; retain them locally after their upload window closes.
        store.mark_synced(&expired).map_err(store_error)?;
        readings.retain(|event| {
            OffsetDateTime::parse(&event.at, &Rfc3339)
                .is_ok_and(|at| at >= now - time::Duration::days(7))
        });
        let mut outcome = UploadOutcome {
            response: None,
            sent: 0,
            counters: counters.len(),
            readings: readings.len(),
        };
        if counters.is_empty() && readings.is_empty() {
            return Ok(outcome);
        }
        let batch = Batch {
            now,
            server_now: server_now(store, now),
            agent_ids: server_feature(store, "agentId"),
        };
        let events: Vec<&StoredEvent> = counters.iter().chain(readings.iter()).collect();
        let mut refused = Vec::new();
        let mut requests = 0;
        self.send_split(
            store,
            &batch,
            &events,
            &mut outcome,
            &mut refused,
            &mut requests,
        )?;
        if let Some((_, error)) = refused.first() {
            if outcome.sent == 0 {
                return Err(error.clone());
            }
            for (event, error) in &refused {
                eprintln!(
                    "daemon upload: set aside observation {} ({} at {}), refused by the server: {error}",
                    event.id, event.key, event.at
                );
            }
            let snapshots: Vec<_> = refused.iter().map(|(event, _)| snapshot(event)).collect();
            store.reject(&snapshots).map_err(store_error)?;
        }
        Ok(outcome)
    }

    fn send_split<'a>(
        &self,
        store: &mut Store,
        batch: &Batch,
        events: &[&'a StoredEvent],
        outcome: &mut UploadOutcome,
        refused: &mut Vec<(&'a StoredEvent, ApiError)>,
        requests: &mut usize,
    ) -> Result<(), ApiError> {
        *requests += 1;
        match self.send_observations(store, batch, events) {
            Ok(response) => {
                outcome.sent += events.len();
                outcome.response = Some(response);
                Ok(())
            }
            Err(
                error @ ApiError::Status {
                    status: 400 | 413, ..
                },
            ) if *requests < MAX_SPLIT_REQUESTS => {
                if let [event] = events {
                    refused.push((event, error));
                    return Ok(());
                }
                let (first, second) = events.split_at(events.len() / 2);
                self.send_split(store, batch, first, outcome, refused, requests)?;
                self.send_split(store, batch, second, outcome, refused, requests)
            }
            Err(error) => Err(error),
        }
    }

    /// Posts one batch and marks it synced once the server acknowledged every observation.
    fn send_observations(
        &self,
        store: &mut Store,
        batch: &Batch,
        events: &[&StoredEvent],
    ) -> Result<Value, ApiError> {
        let (counters, readings): (Vec<&StoredEvent>, Vec<&StoredEvent>) = events
            .iter()
            .partition(|event| event.kind == EventKind::Counter);
        let observations: Vec<Value> = counters
            .iter()
            .map(|event| {
                let mut item = Map::new();
                item.insert("id".into(), json!(event.id));
                item.insert("sessionId".into(), json!(event.session_id));
                item.insert("at".into(), json!(batch.at(&event.at)));
                item.insert("key".into(), json!(event.key));
                item.insert("value".into(), json!(event.value));
                let history = OffsetDateTime::parse(&event.at, &Rfc3339)
                    .is_ok_and(|at| at < batch.now - time::Duration::days(7));
                item.insert(
                    "source".into(),
                    json!(if history { "history" } else { "live" }),
                );
                if let Some(model) = &event.model {
                    item.insert("model".into(), json!(model));
                }
                if let Some(provider) = &event.provider {
                    item.insert("provider".into(), json!(provider));
                }
                // Older servers refuse unknown fields, so only one that lists the feature gets it.
                if let Some(agent_id) = event.agent_id.as_ref().filter(|_| batch.agent_ids) {
                    item.insert("agentId".into(), json!(agent_id));
                }
                Value::Object(item)
            })
            .collect();
        let reading_payload: Vec<Value> = readings
            .iter()
            .map(|event| {
                json!({
                    "id": event.id, "at": batch.at(&event.at), "key": event.key, "value": event.value,
                    "resetsAt": event.resets_at,
                })
            })
            .collect();
        let response = self.post(
            "/api/observations",
            &json!({
                "observations": observations, "readings": reading_payload,
            }),
        )?;
        let accepted = response["accepted"].as_u64();
        let duplicates = response["duplicates"].as_u64();
        if accepted.zip(duplicates).and_then(|(a, b)| a.checked_add(b)) != Some(events.len() as u64)
        {
            return Err(ApiError::Invalid(
                "Clash of Meat returned an invalid observation acknowledgement.".into(),
            ));
        }
        let snapshots: Vec<(String, u64, String)> =
            events.iter().map(|event| snapshot(event)).collect();
        store.mark_synced(&snapshots).map_err(store_error)?;
        Ok(response)
    }

    /// Uploads only the completed payloads assembled by the private observer.
    pub fn upload_observer_tasks(&self, store: &mut Store) -> Result<usize, ApiError> {
        let payloads = observer::completed_payloads(store, 100).map_err(store_error)?;
        let mut groups: Vec<(Value, Vec<Value>)> = Vec::new();
        for payload in payloads {
            let source = payload["source"].clone();
            let task = payload["task"].clone();
            if let Some((_, tasks)) = groups.iter_mut().find(|(key, _)| *key == source) {
                tasks.push(task);
            } else {
                groups.push((source, vec![task]));
            }
        }
        let mut uploaded = 0;
        for (source, tasks) in groups {
            for tasks in tasks.chunks(50) {
                match self.send_observer_tasks(store, &source, tasks) {
                    Ok(()) => uploaded += tasks.len(),
                    Err(ApiError::Status {
                        status: 400 | 413, ..
                    }) => {
                        for task in tasks {
                            match self.send_observer_tasks(
                                store,
                                &source,
                                std::slice::from_ref(task),
                            ) {
                                Ok(()) => uploaded += 1,
                                Err(ApiError::Status {
                                    status: 400 | 413, ..
                                }) => {
                                    observer::reject(
                                        store,
                                        task["id"].as_str().unwrap_or_default(),
                                    )
                                    .map_err(store_error)?;
                                }
                                Err(error) => return Err(error),
                            }
                        }
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        Ok(uploaded)
    }

    fn send_observer_tasks(
        &self,
        store: &mut Store,
        source: &Value,
        tasks: &[Value],
    ) -> Result<(), ApiError> {
        let response = self.post(
            "/api/observer/tasks",
            &json!({"source": source, "tasks": tasks}),
        )?;
        let accepted = response["accepted"].as_u64();
        let duplicates = response["duplicates"].as_u64();
        if accepted.zip(duplicates).and_then(|(a, b)| a.checked_add(b)) != Some(tasks.len() as u64)
        {
            return Err(ApiError::Invalid(
                "Clash of Meat returned an invalid observer acknowledgement.".into(),
            ));
        }
        for task in tasks {
            let id = task["id"]
                .as_str()
                .ok_or_else(|| ApiError::Invalid("observer payload has no task ID".into()))?;
            observer::acknowledge(store, id).map_err(store_error)?;
        }
        Ok(())
    }
}

/// Reads a JSON answer, or the server's error: `{error}` from the web app's `handle()`.
fn answer(result: Result<ureq::Response, ureq::Error>) -> Result<Value, ApiError> {
    match result {
        Ok(response) => response
            .into_json()
            .map_err(|e| ApiError::Invalid(format!("invalid response JSON: {e}"))),
        Err(ureq::Error::Status(status, response)) => {
            let retry_header = response
                .header("Retry-After")
                .and_then(|s| s.parse::<u64>().ok());
            let body: Option<Value> = response.into_json().ok();
            let detail = body
                .as_ref()
                .and_then(|v| v.get("error"))
                .and_then(Value::as_str)
                .filter(|s| s.len() <= 200);
            let message = detail.map_or_else(
                || format!("Clash of Meat returned {status}. Pending activity is saved locally."),
                |s| format!("Clash of Meat: {s}"),
            );
            let reason = body
                .as_ref()
                .and_then(|v| v.get("code").or_else(|| v.get("reason")))
                .and_then(Value::as_str)
                .filter(|s| {
                    s.len() <= 40
                        && !s.is_empty()
                        && s.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
                })
                .map(str::to_owned);
            let body_retry = body
                .as_ref()
                .and_then(|v| v.get("retryAfter"))
                .and_then(Value::as_u64);
            let retry = retry_header.or(body_retry).filter(|s| *s > 0);
            // A 429 always waits (a minute when unsaid); a 503 only when the server says how long.
            let retry_after_s = match status {
                429 => Some(retry.unwrap_or(60).min(3600)),
                503 => retry.map(|s| s.min(3600)),
                _ => None,
            };
            let setup_url = body
                .as_ref()
                .and_then(|v| v.get("setupUrl"))
                .and_then(Value::as_str)
                .filter(|s| s.len() <= 500)
                .map(str::to_owned);
            Err(ApiError::Status {
                status,
                message,
                detail: detail.map(str::to_owned),
                reason,
                retry_after_s,
                setup_url,
            })
        }
        Err(ureq::Error::Transport(error)) => Err(ApiError::Transport(error.to_string())),
    }
}

/// One upload's view of time.
struct Batch {
    now: OffsetDateTime,
    /// Our clock corrected by the server's `Date`, when they disagree.
    server_now: OffsetDateTime,
    /// The server accepts `agentId` on observations.
    agent_ids: bool,
}

impl Batch {
    /// A timestamp ahead of the server's clock (our clock runs fast) would be refused as in the
    /// future; it goes up as the server's "now" instead. The stored row keeps its own time.
    fn at(&self, at: &str) -> String {
        match OffsetDateTime::parse(at, &Rfc3339) {
            Ok(parsed) if parsed > self.server_now => {
                let ms = self.server_now.nanosecond() / 1_000_000 * 1_000_000;
                self.server_now
                    .replace_nanosecond(ms)
                    .unwrap_or(self.server_now)
                    .format(&Rfc3339)
                    .unwrap_or_else(|_| at.to_owned())
            }
            _ => at.to_owned(),
        }
    }
}

/// Whether the last heartbeat listed this optional upload field in its `features`. A server
/// without the list gets none of them.
fn server_feature(store: &Store, feature: &str) -> bool {
    store
        .meta("heartbeat")
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|heartbeat| {
            heartbeat["features"]
                .as_array()
                .map(|features| features.iter().any(|f| f.as_str() == Some(feature)))
        })
        .unwrap_or(false)
}

/// `now` on the server's clock, from the skew the last heartbeat measured.
fn server_now(store: &Store, now: OffsetDateTime) -> OffsetDateTime {
    let skew = store
        .meta("clock_skew_s")
        .ok()
        .flatten()
        .and_then(|raw| raw.parse::<i64>().ok())
        .unwrap_or(0);
    now + time::Duration::seconds(skew)
}

/// Remembers the server's clock offset for uploads. A few seconds is the `Date` header's own
/// resolution and network delay, not a wrong clock.
fn save_clock_skew(store: &mut Store, skew: Option<i64>) -> Result<(), ApiError> {
    match skew {
        Some(skew) if skew.abs() >= 5 => store.set_meta("clock_skew_s", &skew.to_string()),
        Some(_) => store.delete_meta("clock_skew_s"),
        None => Ok(()),
    }
    .map_err(store_error)
}

fn snapshot(event: &StoredEvent) -> (String, u64, String) {
    (event.id.clone(), event.value, event.at.clone())
}

fn required_string(value: &Value, key: &str) -> Result<String, ApiError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ApiError::Invalid(format!("invalid response field: {key}")))
}

fn store_error(error: impl fmt::Display) -> ApiError {
    ApiError::Invalid(format!("local store: {error}"))
}

pub struct Pairing {
    pub id: String,
    pub code: String,
    pub exchange_secret: String,
    pub expires_at: String,
}

impl Pairing {
    pub fn link(&self, origin: &str) -> String {
        format!("{origin}/link?code={}", self.code)
    }
}

pub struct Linked {
    pub installation_id: String,
    pub token: String,
    /// The app that issued the token; it is sent nowhere else.
    pub origin: String,
}

pub fn mark_paired(
    store: &mut Store,
    linked: &Linked,
    now: OffsetDateTime,
) -> Result<(), ApiError> {
    store
        .set_meta("installation.token", &linked.token)
        .map_err(store_error)?;
    store
        .set_meta("installation.id", &linked.installation_id)
        .map_err(store_error)?;
    store
        .set_meta("installation.origin", &linked.origin)
        .map_err(store_error)?;
    if store.meta("paired_at").map_err(store_error)?.is_none() {
        store
            .set_meta("paired_at", &now.format(&Rfc3339).map_err(store_error)?)
            .map_err(store_error)?;
    }
    store.set_meta("reauth", "false").map_err(store_error)
}

/// The installation token, but only for the app that issued it. Pairing records the app's origin
/// next to the token; once the configured app URL points elsewhere (say, a reinstall with
/// another `--app`), the token is dropped so it never reaches another server. A token from before
/// origins were recorded adopts the current app.
pub fn installation_token(
    store: &mut Store,
    app_url: Option<&str>,
) -> Result<Option<String>, ApiError> {
    let Some(token) = store.meta("installation.token").map_err(store_error)? else {
        return Ok(None);
    };
    let Some(origin) = app_url.and_then(|url| crate::config::app_origin(url).ok()) else {
        return Ok(None);
    };
    match store.meta("installation.origin").map_err(store_error)? {
        Some(paired) if paired == origin => Ok(Some(token)),
        Some(_) => {
            forget_installation(store)?;
            Ok(None)
        }
        None => {
            store
                .set_meta("installation.origin", &origin)
                .map_err(store_error)?;
            Ok(Some(token))
        }
    }
}

/// Forgets the installation's token and what the server last said, keeping queued activity.
pub fn forget_installation(store: &mut Store) -> Result<(), ApiError> {
    for key in [
        "installation.token",
        "installation.id",
        "installation.origin",
        "heartbeat",
        "clock_skew_s",
    ] {
        store.delete_meta(key).map_err(store_error)?;
    }
    Ok(())
}

/// Revokes this installation on the server (`uninstall`, `connect --force`), then forgets its
/// token. Older servers have no revoke endpoint and an unreachable one must not block an
/// uninstall, so a failed revoke is only reported. True when there was a token.
pub fn revoke_installation(env: &Env, version: &str) -> anyhow::Result<bool> {
    let path = crate::config::store_path(env);
    if !path.is_file() {
        return Ok(false);
    }
    let mut store = Store::open(path)?;
    let app_url = crate::config::read_config(env)?.app_url;
    let Some(token) = installation_token(&mut store, app_url.as_deref())? else {
        return Ok(false);
    };
    if let Err(error) = Client::new(app_url.as_deref().unwrap_or_default(), Some(token), version)
        .and_then(|client| client.revoke())
    {
        eprintln!("Could not revoke this computer's connection on the server: {error}");
    }
    forget_installation(&mut store)?;
    Ok(true)
}

pub struct UploadOutcome {
    pub response: Option<Value>,
    pub sent: usize,
    pub counters: usize,
    pub readings: usize,
}

pub struct SyncResult {
    pub heartbeat: Option<Value>,
    pub uploaded: usize,
    pub error: Option<ApiError>,
    pub deferred: bool,
    /// The last batch was full: more observations are queued.
    pub more: bool,
    /// The server's pace for the next sync, `nextSyncInSeconds`.
    pub next_sync: Option<Duration>,
    /// What the heartbeat's numbers already include: effective tokens acknowledged since this
    /// time. Local activity beyond them moves the overlay before the next sync.
    pub tokens_base: Option<(String, u64)>,
}

impl SyncResult {
    fn new() -> Self {
        Self {
            heartbeat: None,
            uploaded: 0,
            error: None,
            deferred: false,
            more: false,
            next_sync: None,
            tokens_base: None,
        }
    }
}

/// The last full hero art. Heartbeats send its revision so the server can leave the frames out.
#[derive(Default)]
pub struct HeroArt {
    revision: Option<String>,
    png: Value,
    animation: Value,
}

impl HeroArt {
    /// Starts from a saved hero view (`hero/cache.json`).
    pub fn from_view(view: &Value) -> Self {
        let mut art = Self::default();
        let mut heartbeat = view.clone();
        art.keep(&mut heartbeat);
        art
    }

    fn request(&self) -> Value {
        match &self.revision {
            Some(revision) => json!({ "artRevision": revision }),
            None => json!({}),
        }
    }

    /// Fills unchanged art back in, or remembers new art. False when the server
    /// skipped art this client no longer holds.
    fn keep(&mut self, heartbeat: &mut Value) -> bool {
        let Some(character) = heartbeat
            .get_mut("character")
            .and_then(Value::as_object_mut)
        else {
            return true;
        };
        let revision = character
            .get("artRevision")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if character.remove("artUnchanged") == Some(Value::Bool(true)) {
            if revision.is_none() || revision != self.revision {
                return false;
            }
            character.insert("png".into(), self.png.clone());
            character.insert("animation".into(), self.animation.clone());
            return true;
        }
        match (revision, character.get("png"), character.get("animation")) {
            (Some(revision), Some(png), Some(animation)) => {
                self.png = png.clone();
                self.animation = animation.clone();
                self.revision = Some(revision);
            }
            _ => *self = Self::default(),
        }
        true
    }
}

/// The emotes a hero can play on a friend's screen.
pub const VISIT_EMOTES: [&str; 5] = ["kiss", "wave", "laugh", "victory", "angry"];

/// This install's companion session, created once. Heartbeats carry it so the server only
/// sends visits to an app that is running.
pub fn companion_session(store: &mut Store) -> Result<String, ApiError> {
    if let Some(session) = store
        .meta("companion.session")
        .map_err(store_error)?
        .filter(|session| is_uuid(session))
    {
        return Ok(session);
    }
    let session = clashofmeat_core::ids::random_uuid();
    store
        .set_meta("companion.session", &session)
        .map_err(store_error)?;
    Ok(session)
}

/// The stored heartbeat without hero art, which the hero cache already holds.
fn without_art(heartbeat: &Value) -> Value {
    let mut light = heartbeat.clone();
    if let Some(character) = light.get_mut("character").and_then(Value::as_object_mut) {
        character.remove("png");
        character.remove("animation");
    }
    light
}

pub fn sync_once(client: &Client, store: &mut Store, now: OffsetDateTime) -> SyncResult {
    sync_once_with_art(client, store, now, &mut HeroArt::default())
}

pub fn sync_once_with_art(
    client: &Client,
    store: &mut Store,
    now: OffsetDateTime,
    art: &mut HeroArt,
) -> SyncResult {
    sync_once_with(client, store, now, art, &Map::new())
}

/// Like `sync_once_with_art`; `presence` adds fields to the heartbeat, such as the companion
/// session and the visits played since the last one.
pub fn sync_once_with(
    client: &Client,
    store: &mut Store,
    now: OffsetDateTime,
    art: &mut HeroArt,
    presence: &Map<String, Value>,
) -> SyncResult {
    let with_presence = |mut body: Value| {
        if let Some(fields) = body.as_object_mut() {
            fields.extend(presence.clone());
        }
        body
    };
    let mut result = SyncResult::new();
    if let Some(deferred) = deferred(client, store, now) {
        return deferred;
    }
    let mut heartbeat = match client.heartbeat(with_presence(art.request())) {
        Ok(value) => value,
        Err(error) => return fail(client, store, now, result, error),
    };
    if !art.keep(&mut heartbeat) {
        *art = HeroArt::default();
        heartbeat = match client.heartbeat(with_presence(json!({}))) {
            Ok(value) => value,
            Err(error) => return fail(client, store, now, result, error),
        };
        art.keep(&mut heartbeat);
    }
    let timestamp = match now.format(&Rfc3339) {
        Ok(timestamp) => timestamp,
        Err(error) => {
            return fail(
                client,
                store,
                now,
                result,
                ApiError::Invalid(error.to_string()),
            )
        }
    };
    if let Err(error) = store
        .set_meta("heartbeat", &without_art(&heartbeat).to_string())
        .and_then(|()| store.set_meta("heartbeat_at", &timestamp))
        .map_err(store_error)
        .and_then(|()| save_clock_skew(store, client.clock_skew()))
    {
        return fail(client, store, now, result, error);
    }
    result.next_sync = crate::cadence::next_sync_hint(&heartbeat);
    result.heartbeat = Some(heartbeat);
    // Measured before this sync's upload, which the heartbeat's numbers don't include yet.
    result.tokens_base = (now - time::Duration::days(2))
        .format(&Rfc3339)
        .ok()
        .and_then(|since| {
            let synced = store.effective_tokens_since(&since, true).ok()?;
            Some((since, synced))
        });
    // One batch per sync; the sync thread drains a backlog at its own pace (`drain_once`).
    if let Err(error) = upload_batch(client, store, now, &mut result) {
        return fail(client, store, now, result, error);
    }
    match client.upload_observer_tasks(store) {
        Ok(uploaded) => result.uploaded += uploaded,
        Err(error @ ApiError::Status { status: 401, .. }) => {
            return fail(client, store, now, result, error);
        }
        // Calibration errors leave observer tasks queued without failing the sync.
        Err(_) => {}
    }
    result
}

/// Uploads one more batch of a backlog, without a heartbeat.
pub fn drain_once(client: &Client, store: &mut Store, now: OffsetDateTime) -> SyncResult {
    let mut result = SyncResult::new();
    if let Some(deferred) = deferred(client, store, now) {
        return deferred;
    }
    match upload_batch(client, store, now, &mut result) {
        Ok(()) => result,
        Err(error) => fail(client, store, now, result, error),
    }
}

fn upload_batch(
    client: &Client,
    store: &mut Store,
    now: OffsetDateTime,
    result: &mut SyncResult,
) -> Result<(), ApiError> {
    let outcome = client.upload_observations(store, now)?;
    result.uploaded += outcome.sent;
    result.more = outcome.counters >= 100 || outcome.readings >= 50;
    if let Some(hint) = outcome
        .response
        .as_ref()
        .and_then(crate::cadence::next_sync_hint)
    {
        result.next_sync = Some(hint);
    }
    Ok(())
}

/// A deferred result while the server's Retry-After lasts.
fn deferred(client: &Client, store: &mut Store, now: OffsetDateTime) -> Option<SyncResult> {
    match store.meta("retry_after_until") {
        Ok(Some(until))
            if OffsetDateTime::parse(&until, &Rfc3339).is_ok_and(|until| now < until) =>
        {
            let mut result = SyncResult::new();
            result.deferred = true;
            Some(result)
        }
        Err(error) => Some(fail(
            client,
            store,
            now,
            SyncResult::new(),
            store_error(error),
        )),
        _ => None,
    }
}

fn fail(
    client: &Client,
    store: &mut Store,
    now: OffsetDateTime,
    mut result: SyncResult,
    error: ApiError,
) -> SyncResult {
    if let ApiError::Status {
        status,
        retry_after_s,
        message,
        ..
    } = &error
    {
        match status {
            401 => {
                let _ = forget_installation(store);
                let _ = store.set_meta("reauth", "true");
            }
            426 => {
                let _ = store.set_meta(
                    "blocked",
                    &json!({"version":client.version,"message":message}).to_string(),
                );
            }
            429 | 503 => {
                if let Some(seconds) = retry_after_s {
                    // Spread out, so throttled clients don't all return at the same second.
                    let wait = crate::cadence::spread(Duration::from_secs(*seconds));
                    let until = now + time::Duration::milliseconds(wait.as_millis() as i64);
                    if let Ok(text) = until.format(&Rfc3339) {
                        let _ = store.set_meta("retry_after_until", &text);
                    }
                }
            }
            _ => {}
        }
    }
    result.error = Some(error);
    result
}

pub(crate) fn blocked_message(raw: &str, version: &str) -> Option<String> {
    match serde_json::from_str::<Value>(raw) {
        Ok(Value::Object(fields)) => match (fields.get("version"), fields.get("message")) {
            (Some(Value::String(blocked_version)), Some(Value::String(message))) => {
                (blocked_version == version).then(|| message.clone())
            }
            _ => Some(raw.to_owned()),
        },
        _ => Some(raw.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::blocked_message;

    #[test]
    fn blocked_message_applies_to_its_version_and_legacy_values() {
        let versioned = r#"{"version":"0.1.0","message":"Update required"}"#;
        assert_eq!(
            blocked_message(versioned, "0.1.0").as_deref(),
            Some("Update required")
        );
        assert_eq!(blocked_message(versioned, "0.2.0"), None);
        assert_eq!(
            blocked_message("Legacy update required", "0.2.0").as_deref(),
            Some("Legacy update required")
        );
    }
}

use std::fmt;
use std::time::Duration;

use clashofmeat_core::ids::is_uuid;
use clashofmeat_core::observer;
use clashofmeat_core::signal::EventKind;
use clashofmeat_core::store::{Store, StoredEvent};
use serde_json::{json, Map, Value};
use time::format_description::well_known::Rfc3339;
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
}

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
        })
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
        answer(request.send_json(body))
    }

    fn get(&self, path: &str) -> Result<Value, ApiError> {
        answer(
            self.authorized(self.agent.get(&format!("{}{path}", self.origin)))
                .call(),
        )
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

    /// Checks whether an outbound visit is still awaiting its recipient's playback.
    pub fn visit_status(&self, id: &str) -> Result<Value, ApiError> {
        self.get(&format!("/api/companion/visit-status?visitId={id}"))
    }

    pub fn upload_observations(
        &self,
        store: &mut Store,
        now: OffsetDateTime,
    ) -> Result<UploadOutcome, ApiError> {
        let counters = store
            .pending(EventKind::Counter, 100)
            .map_err(store_error)?;
        let readings = store.pending(EventKind::Reading, 50).map_err(store_error)?;
        if counters.is_empty() && readings.is_empty() {
            return Ok(UploadOutcome {
                response: None,
                sent: 0,
                counters: 0,
                readings: 0,
            });
        }
        let observations: Vec<Value> = counters
            .iter()
            .map(|event| {
                let mut item = Map::new();
                item.insert("id".into(), json!(event.id));
                item.insert("sessionId".into(), json!(event.session_id));
                item.insert("at".into(), json!(event.at));
                item.insert("key".into(), json!(event.key));
                item.insert("value".into(), json!(event.value));
                let history = OffsetDateTime::parse(&event.at, &Rfc3339)
                    .is_ok_and(|at| at < now - time::Duration::days(7));
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
                Value::Object(item)
            })
            .collect();
        let reading_payload: Vec<Value> = readings
            .iter()
            .map(|event| {
                json!({
                    "id": event.id, "at": event.at, "key": event.key, "value": event.value,
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
        let snapshots: Vec<(String, u64, String)> = counters
            .iter()
            .chain(readings.iter())
            .map(snapshot)
            .collect();
        store.mark_synced(&snapshots).map_err(store_error)?;
        Ok(UploadOutcome {
            response: Some(response),
            sent: snapshots.len(),
            counters: counters.len(),
            readings: readings.len(),
        })
    }

    /// Uploads only the completed payloads assembled by the private observer.
    pub fn upload_observer_tasks(&self, store: &mut Store) -> Result<usize, ApiError> {
        let payloads = observer::completed_payloads(store, usize::MAX).map_err(store_error)?;
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
            match self.send_observer_tasks(store, &source, &tasks) {
                Ok(()) => uploaded += tasks.len(),
                Err(ApiError::Status { status: 400, .. }) => {
                    for task in tasks {
                        match self.send_observer_tasks(store, &source, std::slice::from_ref(&task))
                        {
                            Ok(()) => uploaded += 1,
                            Err(ApiError::Status { status: 400, .. }) => {
                                observer::reject(store, task["id"].as_str().unwrap_or_default())
                                    .map_err(store_error)?;
                            }
                            Err(error) => return Err(error),
                        }
                    }
                }
                Err(error) => return Err(error),
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
            let retry_after_s = if status == 429 {
                let body_retry = body
                    .as_ref()
                    .and_then(|v| v.get("retryAfter"))
                    .and_then(Value::as_u64);
                Some(
                    retry_header
                        .or(body_retry)
                        .filter(|s| *s > 0)
                        .unwrap_or(60)
                        .min(3600),
                )
            } else {
                None
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
    if store.meta("paired_at").map_err(store_error)?.is_none() {
        store
            .set_meta("paired_at", &now.format(&Rfc3339).map_err(store_error)?)
            .map_err(store_error)?;
    }
    store.set_meta("reauth", "false").map_err(store_error)
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
pub const VISIT_EMOTES: [&str; 3] = ["flex", "kiss", "angry"];

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
    let mut result = SyncResult {
        heartbeat: None,
        uploaded: 0,
        error: None,
        deferred: false,
    };
    match store.meta("retry_after_until") {
        Ok(Some(until))
            if OffsetDateTime::parse(&until, &Rfc3339).is_ok_and(|until| now < until) =>
        {
            result.deferred = true;
            return result;
        }
        Err(error) => return fail(client, store, now, result, store_error(error)),
        _ => {}
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
    {
        return fail(client, store, now, result, store_error(error));
    }
    result.heartbeat = Some(heartbeat);
    loop {
        match client.upload_observations(store, now) {
            Ok(outcome) => {
                result.uploaded += outcome.sent;
                if outcome.counters < 100 && outcome.readings < 50 {
                    break;
                }
            }
            Err(error) => return fail(client, store, now, result, error),
        }
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
                let _ = store.delete_meta("installation.token");
                let _ = store.delete_meta("installation.id");
                let _ = store.set_meta("reauth", "true");
            }
            426 => {
                let _ = store.set_meta(
                    "blocked",
                    &json!({"version":client.version,"message":message}).to_string(),
                );
            }
            429 => {
                let until = now
                    + time::Duration::seconds(
                        i64::try_from(retry_after_s.unwrap_or(60)).unwrap_or(60),
                    );
                if let Ok(text) = until.format(&Rfc3339) {
                    let _ = store.set_meta("retry_after_until", &text);
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

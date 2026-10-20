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

    fn post(&self, path: &str, body: &Value) -> Result<Value, ApiError> {
        let mut request = self
            .agent
            .post(&format!("{}{path}", self.origin))
            .set("x-clashofmeat-client", &format!("desktop/{}", self.version))
            .set("content-type", "application/json");
        if let Some(token) = &self.token {
            request = request.set("authorization", &format!("Bearer {token}"));
        }
        match request.send_json(body) {
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
                    || {
                        format!(
                            "Clash of Meat returned {status}. Pending activity is saved locally."
                        )
                    },
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
                    reason,
                    retry_after_s,
                    setup_url,
                })
            }
            Err(ureq::Error::Transport(error)) => Err(ApiError::Transport(error.to_string())),
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

pub fn sync_once(client: &Client, store: &mut Store, now: OffsetDateTime) -> SyncResult {
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
    let heartbeat = match client.heartbeat(json!({})) {
        Ok(value) => value,
        Err(error) => return fail(client, store, now, result, error),
    };
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
        .set_meta("heartbeat", &heartbeat.to_string())
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

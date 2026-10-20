//! Long-running daemon coordination.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::Result;
use clashofmeat_core::{Env, EventKind, Store};
use crossbeam_channel::{unbounded, Receiver, Sender};
use serde_json::{json, Value};
use time::{format_description::well_known::Rfc3339, OffsetDateTime, UtcOffset};

use crate::{
    config, detect,
    engine::Engine,
    hero,
    socket::{BoundSocket, Request, SocketError},
    sync, watch, wiring,
};

/// Inputs required to start a daemon.
pub struct RunOptions {
    /// Filesystem and environment used by providers.
    pub env: Env,
    /// Client version sent to the service.
    pub version: String,
}

/// A running daemon and its owned threads.
pub struct DaemonHandle {
    socket_path: PathBuf,
    main: Option<JoinHandle<()>>,
    socket: Option<JoinHandle<()>>,
    watcher: Option<JoinHandle<()>>,
    ticker: Option<JoinHandle<()>>,
    sync: Option<JoinHandle<()>>,
    sender: Sender<Message>,
    watch_stop: Sender<()>,
    sync_stop: Sender<SyncCommand>,
    stopping: Arc<AtomicBool>,
}

impl DaemonHandle {
    /// Returns the active local socket path.
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Stops every daemon thread and removes its socket.
    pub fn stop(mut self) -> Result<()> {
        self.stop_inner();
        Ok(())
    }

    fn stop_inner(&mut self) {
        self.stopping.store(true, Ordering::Relaxed);
        let _ = self.sender.send(Message::Stop);
        let _ = self.watch_stop.send(());
        let _ = self.sync_stop.send(SyncCommand::Stop);
        for handle in [
            &mut self.ticker,
            &mut self.watcher,
            &mut self.socket,
            &mut self.sync,
            &mut self.main,
        ] {
            if let Some(handle) = handle.take() {
                let _ = handle.join();
            }
        }
        let _ = fs::remove_file(&self.socket_path);
    }
}

impl Drop for DaemonHandle {
    fn drop(&mut self) {
        self.stop_inner();
    }
}

pub(crate) enum Message {
    FsChanged(Vec<PathBuf>),
    Tick,
    Socket(Request),
    SyncDone(sync::SyncResult),
    PairState(Value),
    Stop,
}

enum SyncCommand {
    Now,
    PairStart {
        device: String,
        tools: Vec<String>,
        reply: Sender<std::result::Result<Value, SocketError>>,
    },
    Stop,
}

/// Starts a daemon and returns a handle for shutdown.
pub fn start(opts: RunOptions) -> Result<DaemonHandle> {
    let home = config::home(&opts.env);
    let bound = BoundSocket::bind(&home)?;
    let socket_path = bound.path().to_path_buf();
    let mut store = Store::open(config::store_path(&opts.env))?;
    let mut engine = Engine::new(
        opts.env.clone(),
        clashofmeat_providers::all(),
        &opts.version,
    );
    engine.scan(&mut store, None, OffsetDateTime::now_utc())?;
    let cached = hero::load_cache(&home).unwrap_or(None);
    let (sender, inbox) = unbounded();
    let (watch_refresh, refresh_rx) = unbounded();
    let (watch_stop, stop_rx) = unbounded();
    let (sync_tx, sync_rx) = unbounded();
    let stopping = Arc::new(AtomicBool::new(false));
    let socket = bound.serve(sender.clone(), stopping.clone());
    let (ready_tx, ready_rx) = crossbeam_channel::bounded(1);
    let watcher = watch::start(
        opts.env.clone(),
        sender.clone(),
        refresh_rx,
        stop_rx,
        ready_tx,
    );
    let _ = ready_rx.recv_timeout(Duration::from_secs(2));
    let ticker = {
        let sender = sender.clone();
        let stopping = stopping.clone();
        thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_secs(1));
                if !stopping.load(Ordering::Relaxed) && sender.send(Message::Tick).is_err() {
                    break;
                }
            }
        })
    };
    let busy_recent = Arc::new(AtomicBool::new(false));
    let sync = {
        let env = opts.env.clone();
        let version = opts.version.clone();
        let sender = sender.clone();
        let busy_recent = busy_recent.clone();
        thread::spawn(move || sync_loop(env, version, sync_rx, sender, busy_recent))
    };
    let main_sync = sync_tx.clone();
    let main = thread::spawn(move || {
        main_loop(
            opts,
            store,
            engine,
            cached,
            inbox,
            watch_refresh,
            main_sync,
            busy_recent,
        )
    });
    // The main loop owns a clone of sync_tx; keep another sender for shutdown.
    Ok(DaemonHandle {
        socket_path,
        main: Some(main),
        socket: Some(socket),
        watcher: Some(watcher),
        ticker: Some(ticker),
        sync: Some(sync),
        sender,
        watch_stop,
        sync_stop: sync_tx,
        stopping,
    })
}

/// Runs a daemon until its process is terminated.
pub fn run_forever(opts: RunOptions) -> Result<()> {
    let _handle = start(opts)?;
    loop {
        thread::park();
    }
}

#[allow(clippy::too_many_arguments)]
fn main_loop(
    opts: RunOptions,
    mut store: Store,
    mut engine: Engine,
    mut cached: Option<Value>,
    inbox: Receiver<Message>,
    watch_refresh: Sender<()>,
    sync_tx: Sender<SyncCommand>,
    busy_recent: Arc<AtomicBool>,
) {
    let mut subscribers: Vec<Sender<Value>> = Vec::new();
    let mut activity = serde_json::to_value(engine.activity(OffsetDateTime::now_utc()))
        .unwrap_or_else(|_| json!({"busy":false,"sessions":[]}));
    let mut recent_busy = None::<Instant>;
    let mut changed = HashSet::new();
    let mut changed_at = None::<Instant>;
    let mut full_at = Instant::now();
    let mut pair_state = json!({"status":"unpaired"});
    loop {
        let full_deadline = full_at + Duration::from_secs(600);
        let deadline = changed_at
            .map(|at| (at + Duration::from_millis(300)).min(full_deadline))
            .unwrap_or(full_deadline);
        let message = inbox.recv_timeout(deadline.saturating_duration_since(Instant::now()));
        match message {
            Ok(Message::Stop) | Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            Ok(Message::FsChanged(paths)) => {
                changed.extend(paths);
                changed_at = Some(Instant::now());
            }
            Ok(Message::Tick) => {
                let next = serde_json::to_value(engine.activity(OffsetDateTime::now_utc()))
                    .unwrap_or(Value::Null);
                if next != activity {
                    activity = next.clone();
                    broadcast(&mut subscribers, "activity", next);
                }
                if activity["busy"] == true {
                    recent_busy = Some(Instant::now());
                }
                busy_recent.store(
                    recent_busy.is_some_and(|at| at.elapsed() < Duration::from_secs(300)),
                    Ordering::Relaxed,
                );
            }
            Ok(Message::Socket(request)) => {
                let reply = handle_request(
                    request,
                    &opts,
                    &mut store,
                    &activity,
                    cached.as_ref(),
                    &pair_state,
                    &sync_tx,
                    &mut subscribers,
                );
                if let Some((sender, value)) = reply {
                    let _ = sender.send(value);
                }
            }
            Ok(Message::SyncDone(result)) => {
                if result.deferred {
                    continue;
                }
                let ok = result.error.is_none();
                let error = result.error.as_ref().map(ToString::to_string);
                broadcast(
                    &mut subscribers,
                    "sync",
                    json!({"ok":ok,"uploaded":result.uploaded,"error":error}),
                );
                if let Some(heartbeat) = result.heartbeat {
                    let view = hero::hero_view(&heartbeat);
                    if let Err(error) = hero::save_cache(&config::home(&opts.env), &view) {
                        eprintln!("daemon hero cache: {error}");
                    }
                    cached = Some(view.clone());
                    broadcast(&mut subscribers, "heartbeat", view);
                }
            }
            Ok(Message::PairState(state)) => {
                pair_state = state.clone();
                match state["status"].as_str() {
                    Some("paired") => broadcast(&mut subscribers, "paired", state),
                    Some("failed") => broadcast(&mut subscribers, "pair_failed", state),
                    _ => {}
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
        }
        if changed_at.is_some_and(|at| at.elapsed() >= Duration::from_millis(300)) {
            let paths: Vec<_> = changed.drain().collect();
            changed_at = None;
            if !paths.is_empty() {
                scan_and_broadcast(
                    &mut engine,
                    &mut store,
                    Some(&paths),
                    &mut activity,
                    &mut subscribers,
                );
            }
        }
        if full_at.elapsed() >= Duration::from_secs(600) {
            scan_and_broadcast(
                &mut engine,
                &mut store,
                None,
                &mut activity,
                &mut subscribers,
            );
            let _ = watch_refresh.send(());
            full_at = Instant::now();
        }
    }
}

fn scan_and_broadcast(
    engine: &mut Engine,
    store: &mut Store,
    paths: Option<&[PathBuf]>,
    activity: &mut Value,
    subscribers: &mut Vec<Sender<Value>>,
) {
    if let Err(error) = engine.scan(store, paths, OffsetDateTime::now_utc()) {
        eprintln!("daemon scan: {error}");
    }
    let next =
        serde_json::to_value(engine.activity(OffsetDateTime::now_utc())).unwrap_or(Value::Null);
    if next != *activity {
        *activity = next.clone();
        broadcast(subscribers, "activity", next);
    }
}

fn broadcast(subscribers: &mut Vec<Sender<Value>>, event: &str, data: Value) {
    let value = json!({"event":event,"data":data});
    subscribers.retain(|sender| sender.try_send(value.clone()).is_ok());
}

type SocketReply = (
    Sender<std::result::Result<Value, SocketError>>,
    std::result::Result<Value, SocketError>,
);

#[allow(clippy::too_many_arguments)]
fn handle_request(
    request: Request,
    opts: &RunOptions,
    store: &mut Store,
    activity: &Value,
    cached: Option<&Value>,
    pair_state: &Value,
    sync_tx: &Sender<SyncCommand>,
    subscribers: &mut Vec<Sender<Value>>,
) -> Option<SocketReply> {
    let Request {
        method,
        params,
        reply,
        subscriber,
    } = request;
    let result = match method.as_str() {
        "status" => status(opts, store, &params),
        "snapshot" => Ok(json!({"activity":activity,"hero":cached})),
        "subscribe" => {
            if let Some(subscriber) = subscriber {
                subscribers.push(subscriber);
            }
            Ok(json!({"subscribed":true}))
        }
        "sync_now" => sync_tx
            .send(SyncCommand::Now)
            .map(|()| json!({"queued":true}))
            .map_err(internal),
        "pair_start" => {
            let device = params
                .get("device")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(detect::device_name);
            let tools = params
                .get("tools")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_else(|| detect::tools(&opts.env));
            if sync_tx
                .send(SyncCommand::PairStart {
                    device,
                    tools,
                    reply: reply.clone(),
                })
                .is_err()
            {
                return Some((
                    reply,
                    Err(SocketError::new("internal", "sync thread stopped")),
                ));
            }
            return None;
        }
        "pair_status" => Ok(pair_state.clone()),
        "wire_provider" => match params.get("id").and_then(Value::as_str) {
            Some("cursor") => {
                let writer =
                    clashofmeat_core::ConfigWriter::new(config::home(&opts.env).join("backup"));
                wiring::wire_cursor(&opts.env, &writer)
                    .map(|outcome| json!({"id":"cursor","wiring":wiring::cursor_wiring_state(&opts.env),"changed":matches!(outcome, clashofmeat_core::WriteOutcome::Written { .. })}))
                    .map_err(internal)
            }
            Some(_) => Err(SocketError::new("not_wirable", "provider cannot be wired")),
            None => Err(SocketError::new("bad_request", "id is required")),
        },
        "set_provider_enabled" => {
            let id = params.get("id").and_then(Value::as_str);
            let enabled = params.get("enabled").and_then(Value::as_bool);
            if let (Some(id), Some(enabled)) = (id, enabled) {
                if !clashofmeat_providers::all()
                    .iter()
                    .any(|p| p.manifest().id == id)
                {
                    Err(SocketError::new("bad_request", "unknown provider"))
                } else {
                    let mut off: HashSet<String> = store
                        .meta("providers_off")
                        .ok()
                        .flatten()
                        .and_then(|raw| serde_json::from_str(&raw).ok())
                        .unwrap_or_default();
                    if enabled {
                        off.remove(id);
                    } else {
                        off.insert(id.to_string());
                    }
                    let mut off: Vec<_> = off.into_iter().collect();
                    off.sort();
                    store
                        .set_meta("providers_off", &json!(off).to_string())
                        .map(|()| json!({"id":id,"enabled":enabled}))
                        .map_err(internal)
                }
            } else {
                Err(SocketError::new(
                    "bad_request",
                    "id and enabled are required",
                ))
            }
        }
        _ => Err(SocketError::new("unknown_method", "unknown method")),
    };
    Some((reply, result))
}

/// Returns today's persisted token counters by provider ID.
/// Claude Code and Cowork both store `claude`; the store cannot identify their origin,
/// so their shared total is reported under `claude-code`.
pub fn today_tokens(
    store: &Store,
    now: OffsetDateTime,
    utc_offset: UtcOffset,
) -> HashMap<String, u64> {
    let midnight = now
        .to_offset(utc_offset)
        .date()
        .midnight()
        .assume_offset(utc_offset);
    token_totals(store, midnight, now).unwrap_or_default()
}

fn token_totals(
    store: &Store,
    since: OffsetDateTime,
    now: OffsetDateTime,
) -> std::result::Result<HashMap<String, u64>, SocketError> {
    let mut totals = HashMap::new();
    for (provider, value) in store
        .token_totals_between(
            &since.format(&Rfc3339).map_err(internal)?,
            &now.format(&Rfc3339).map_err(internal)?,
        )
        .map_err(internal)?
    {
        let key = if provider == "claude" {
            "claude-code"
        } else {
            &provider
        };
        *totals.entry(key.to_owned()).or_insert(0) += value;
    }
    Ok(totals)
}

fn status(
    opts: &RunOptions,
    store: &Store,
    params: &Value,
) -> std::result::Result<Value, SocketError> {
    let now = OffsetDateTime::now_utc();
    let since = match params.get("today_since") {
        Some(Value::String(value)) => OffsetDateTime::parse(value, &Rfc3339).map_err(|_| {
            SocketError::new("bad_request", "today_since must be ISO-8601 with offset")
        })?,
        Some(_) => {
            return Err(SocketError::new(
                "bad_request",
                "today_since must be a timestamp",
            ))
        }
        None => now.date().midnight().assume_utc(),
    };
    let today = token_totals(store, since, now)?;
    let meta = |key| store.meta(key).map_err(internal);
    let blocked = meta("blocked")?.and_then(|raw| sync::blocked_message(&raw, &opts.version));
    let off: HashSet<String> = meta("providers_off")?
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    let health = store.health().map_err(internal)?;
    let providers: Vec<Value> = clashofmeat_providers::all().iter().map(|provider| {
        let manifest = provider.manifest();
        let row = health.iter().find(|row| row.provider == manifest.id);
        let mut value = json!({"id":manifest.id,"name":manifest.name,"detected":detect::detect(&opts.env,manifest),"enabled":!off.contains(manifest.id),"today_tokens":today.get(manifest.id).copied().unwrap_or(0),"health":row.map(|row| json!({"records":row.records,"malformed":row.malformed,"oversized":row.oversized,"errors":row.errors,"last_at":row.last_at})).unwrap_or_else(|| json!({"records":0,"malformed":0,"oversized":0,"errors":0,"last_at":null}))});
        if !manifest.wiring.is_empty() && manifest.id == "cursor" {
            value["wiring"] = json!(wiring::cursor_wiring_state(&opts.env));
        }
        value
    }).collect();
    let counters = store
        .pending(EventKind::Counter, usize::MAX)
        .map_err(internal)?
        .len();
    let readings = store
        .pending(EventKind::Reading, usize::MAX)
        .map_err(internal)?
        .len();
    Ok(
        json!({"version":opts.version,"paired":meta("installation.token")?.is_some(),"app_url":config::read_config(&opts.env).map_err(internal)?.app_url,"blocked":blocked,"reauth":meta("reauth")?.as_deref()==Some("true"),"pending":{"counters":counters,"readings":readings},"last_sync_at":meta("last_sync_at")?,"providers":providers,"today_tokens":today}),
    )
}

fn internal(error: impl std::fmt::Display) -> SocketError {
    SocketError::new("internal", error.to_string())
}

fn sync_loop(
    env: Env,
    version: String,
    commands: Receiver<SyncCommand>,
    main: Sender<Message>,
    busy_recent: Arc<AtomicBool>,
) {
    let mut store = match Store::open(config::store_path(&env)) {
        Ok(store) => store,
        Err(error) => {
            eprintln!("daemon sync store: {error}");
            return;
        }
    };
    let mut last_heartbeat = Instant::now();
    let mut last_upload = Instant::now();
    let mut pairing: Option<sync::Pairing> = None;
    let mut next_poll = Instant::now();
    loop {
        let mut forced = false;
        match commands.recv_timeout(Duration::from_secs(1)) {
            Ok(SyncCommand::Stop) | Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            Ok(SyncCommand::Now) => forced = true,
            Ok(SyncCommand::PairStart {
                device,
                tools,
                reply,
            }) => {
                let result = make_client(&env, &version, None).and_then(|client| {
                    client
                        .pair_start(&device, &tools)
                        .map(|pair| (client, pair))
                        .map_err(internal)
                });
                match result {
                    Ok((client, pair)) => {
                        let response = json!({"link":pair.link(&client.origin),"code":pair.code,"expires_at":pair.expires_at});
                        let _ = reply.send(Ok(response));
                        let _ = main.send(Message::PairState(json!({"status":"pending","code":pair.code,"expires_at":pair.expires_at})));
                        pairing = Some(pair);
                        next_poll = Instant::now() + Duration::from_secs(3);
                    }
                    Err(error) => {
                        let _ = reply.send(Err(error));
                    }
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
        }
        if pairing.is_some() && Instant::now() >= next_poll {
            let pair = pairing.as_ref().expect("checked");
            let expired = OffsetDateTime::parse(&pair.expires_at, &Rfc3339)
                .is_ok_and(|at| OffsetDateTime::now_utc() >= at);
            if expired {
                pairing = None;
                let _ = main.send(Message::PairState(
                    json!({"status":"failed","reason":"expired","message":"Clash of Meat: This code expired. Run the command again."}),
                ));
            } else if let Ok(client) = make_client(&env, &version, None) {
                match client.pair_exchange(pair) {
                    Ok(Some(linked)) => {
                        match sync::mark_paired(&mut store, &linked, OffsetDateTime::now_utc()) {
                            Ok(()) => {
                                pairing = None;
                                let _ = main.send(Message::PairState(json!({"status":"paired"})));
                                forced = true;
                            }
                            Err(error) => {
                                pairing = None;
                                let _ = main.send(Message::PairState(
                                    json!({"status":"failed","reason":null,"message":error.to_string()}),
                                ));
                            }
                        }
                    }
                    Ok(None) => next_poll = Instant::now() + Duration::from_secs(3),
                    Err(sync::ApiError::Status {
                        status: 400..=499,
                        message,
                        reason,
                        ..
                    }) => {
                        pairing = None;
                        let _ = main.send(Message::PairState(
                            json!({"status":"failed","reason":reason,"message":message}),
                        ));
                    }
                    Err(_) => next_poll = Instant::now() + Duration::from_secs(3),
                }
            }
        }
        let interval = if busy_recent.load(Ordering::Relaxed) {
            Duration::from_secs(30)
        } else {
            Duration::from_secs(300)
        };
        let pending = last_upload.elapsed() >= Duration::from_secs(30)
            && (store
                .pending(EventKind::Counter, 1)
                .is_ok_and(|v| !v.is_empty())
                || store
                    .pending(EventKind::Reading, 1)
                    .is_ok_and(|v| !v.is_empty()));
        if !forced && last_heartbeat.elapsed() < interval && !pending {
            continue;
        }
        let token = match store.meta("installation.token") {
            Ok(Some(token)) => token,
            _ => continue,
        };
        if let Ok(Some(raw)) = store.meta("blocked") {
            if sync::blocked_message(&raw, &version).is_some() {
                continue;
            }
            let _ = store.delete_meta("blocked");
        }
        if store
            .meta("retry_after_until")
            .ok()
            .flatten()
            .and_then(|at| OffsetDateTime::parse(&at, &Rfc3339).ok())
            .is_some_and(|at| at > OffsetDateTime::now_utc())
        {
            continue;
        }
        let client = match make_client(&env, &version, Some(token)) {
            Ok(client) => client,
            Err(_) => continue,
        };
        let result = sync::sync_once(&client, &mut store, OffsetDateTime::now_utc());
        last_heartbeat = Instant::now();
        last_upload = Instant::now();
        if result.error.is_none() && !result.deferred {
            let _ = store.delete_meta("blocked");
            let _ = store.set_meta("reauth", "false");
            if let Ok(at) = OffsetDateTime::now_utc().format(&Rfc3339) {
                let _ = store.set_meta("last_sync_at", &at);
            }
        }
        let _ = main.send(Message::SyncDone(result));
    }
}

fn make_client(
    env: &Env,
    version: &str,
    token: Option<String>,
) -> std::result::Result<sync::Client, SocketError> {
    let url = config::read_config(env)
        .map_err(internal)?
        .app_url
        .ok_or_else(|| SocketError::new("not_configured", "app_url is not configured"))?;
    sync::Client::new(&url, token, version).map_err(internal)
}

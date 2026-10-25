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
use serde_json::{json, Map, Value};
use time::{format_description::well_known::Rfc3339, OffsetDateTime, UtcOffset};

use crate::{
    cadence::{Cadence, Due, Outcome},
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

/// Agent activity, as the main loop last saw it, for the sync thread's pace.
#[derive(Default)]
struct Busy {
    /// An agent is working right now.
    now: AtomicBool,
    /// An agent worked in the last five minutes.
    recent: AtomicBool,
}

pub(crate) enum Message {
    FsChanged(Vec<PathBuf>),
    Tick,
    Socket(Request),
    SyncDone(sync::SyncResult),
    PairState(Value),
    /// The server's evolve answer: the new skin and its level-up effect.
    Evolved(Value),
    Stop,
}

enum SyncCommand {
    Now,
    /// The live channel announced a change for this player.
    Live,
    Evolve {
        character_id: String,
        level: u64,
        reply: Sender<std::result::Result<Value, SocketError>>,
    },
    LevelUp {
        from: u64,
        to: u64,
        reply: Sender<std::result::Result<Value, SocketError>>,
    },
    PairStart {
        device: String,
        tools: Vec<String>,
        reply: Sender<std::result::Result<Value, SocketError>>,
    },
    /// The overlay finished playing this visit; the next heartbeat acknowledges it.
    VisitDone {
        id: String,
    },
    Stop,
}

/// Starts a daemon and returns a handle for shutdown.
pub fn start(opts: RunOptions) -> Result<DaemonHandle> {
    let home = config::home(&opts.env);
    // The Cursor hook writes its spool here; watched from the start, its first record isn't missed.
    let _ = fs::create_dir_all(home.join("spool"));
    let bound = BoundSocket::bind(&home)?;
    let socket_path = bound.path().to_path_buf();
    let mut store = Store::open(config::store_path(&opts.env))?;
    let mut engine = Engine::new(
        opts.env.clone(),
        clashofmeat_providers::all(),
        &opts.version,
    );
    let first = engine.scan(&mut store, None, OffsetDateTime::now_utc())?;
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
    let busy = Arc::new(Busy::default());
    let sync = {
        let env = opts.env.clone();
        let version = opts.version.clone();
        let sender = sender.clone();
        let busy = busy.clone();
        thread::spawn(move || sync_loop(env, version, sync_rx, sender, busy))
    };
    {
        // Not joined on stop: a blocked read ends within the 60 s read timeout.
        let env = opts.env.clone();
        let version = opts.version.clone();
        let stopping = stopping.clone();
        let live = sync_tx.clone();
        thread::spawn(move || {
            crate::live::follow(env, version, stopping, || {
                live.send(SyncCommand::Live).is_ok()
            })
        });
    }
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
            busy,
            first.recent,
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
    busy: Arc<Busy>,
    recent: Vec<PathBuf>,
) {
    let mut subscribers: Vec<Sender<Value>> = Vec::new();
    let mut activity = serde_json::to_value(engine.activity(OffsetDateTime::now_utc()))
        .unwrap_or_else(|_| json!({"busy":false,"sessions":[]}));
    let mut recent_busy = None::<Instant>;
    let mut changed = HashSet::new();
    let mut changed_at = None::<Instant>;
    let mut full_at = Instant::now();
    let mut open_files = OpenFiles::default();
    open_files.track(&recent);
    let mut polled_at = Instant::now();
    let mut pair_state = json!({"status":"unpaired"});
    let mut local = LocalTokens::default();
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
                let working = activity["busy"] == true;
                if working {
                    recent_busy = Some(Instant::now());
                }
                busy.now.store(working, Ordering::Relaxed);
                busy.recent.store(
                    recent_busy.is_some_and(|at| at.elapsed() < Duration::from_secs(300)),
                    Ordering::Relaxed,
                );
                if polled_at.elapsed() >= Duration::from_secs(2) {
                    polled_at = Instant::now();
                    let grown = open_files.grown();
                    if !grown.is_empty() {
                        changed.extend(grown);
                        changed_at.get_or_insert_with(Instant::now);
                    }
                }
            }
            Ok(Message::Socket(request)) => {
                let rescan = request.method == "set_provider_homes";
                let reply = handle_request(
                    request,
                    &opts,
                    &mut store,
                    &activity,
                    local.pending,
                    cached.as_ref(),
                    &pair_state,
                    &sync_tx,
                    &mut subscribers,
                );
                if let Some((sender, value)) = reply {
                    let _ = sender.send(value);
                }
                if rescan {
                    let recent = scan_and_broadcast(
                        &mut engine,
                        &mut store,
                        None,
                        &mut activity,
                        &mut subscribers,
                        &mut local,
                    );
                    open_files.track(&recent);
                    let _ = watch_refresh.send(());
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
                    let mut view = hero::hero_view(&heartbeat);
                    if let Err(error) = hero::save_cache(&config::home(&opts.env), &view) {
                        eprintln!("daemon hero cache: {error}");
                    }
                    cached = Some(view.clone());
                    // The overlay adds this machine's activity since the heartbeat to its numbers.
                    local.base = result.tokens_base;
                    local.refresh(&store);
                    view["localTokens"] = json!(local.pending);
                    broadcast(&mut subscribers, "heartbeat", view);
                }
            }
            Ok(Message::Evolved(evolved)) => {
                let mut view = cached.clone().unwrap_or_else(|| json!({}));
                if let (Some(view), Some(character)) =
                    (view.as_object_mut(), evolved.get("character"))
                {
                    view.insert("character".into(), character.clone());
                    view.remove("levelUp");
                    view.remove("level_up");
                }
                if let Err(error) = hero::save_cache(&config::home(&opts.env), &view) {
                    eprintln!("daemon hero cache: {error}");
                }
                cached = Some(view.clone());
                // The effect plays once, so it goes out with the event but is never cached.
                if let (Some(event), Some(level_up)) =
                    (view.as_object_mut(), evolved.get("levelUp"))
                {
                    event.insert("levelUp".into(), level_up.clone());
                }
                broadcast(&mut subscribers, "heartbeat", view);
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
                let recent = scan_and_broadcast(
                    &mut engine,
                    &mut store,
                    Some(&paths),
                    &mut activity,
                    &mut subscribers,
                    &mut local,
                );
                open_files.track(&recent);
            }
        }
        if full_at.elapsed() >= Duration::from_secs(600) {
            let recent = scan_and_broadcast(
                &mut engine,
                &mut store,
                None,
                &mut activity,
                &mut subscribers,
                &mut local,
            );
            open_files.track(&recent);
            let _ = watch_refresh.send(());
            full_at = Instant::now();
        }
    }
}

/// Scans, broadcasts any activity change and returns the files written in the last six hours.
fn scan_and_broadcast(
    engine: &mut Engine,
    store: &mut Store,
    paths: Option<&[PathBuf]>,
    activity: &mut Value,
    subscribers: &mut Vec<Sender<Value>>,
    local: &mut LocalTokens,
) -> Vec<PathBuf> {
    let recent = match engine.scan(store, paths, OffsetDateTime::now_utc()) {
        Ok(report) => {
            if report.events_saved > 0 {
                broadcast(
                    subscribers,
                    "observations",
                    json!({"changed": report.events_saved}),
                );
                if local.refresh(store) {
                    broadcast(
                        subscribers,
                        "local_tokens",
                        json!({"effective": local.pending}),
                    );
                }
            }
            report.recent
        }
        Err(error) => {
            eprintln!("daemon scan: {error}");
            Vec::new()
        }
    };
    let next =
        serde_json::to_value(engine.activity(OffsetDateTime::now_utc())).unwrap_or(Value::Null);
    if next != *activity {
        *activity = next.clone();
        broadcast(subscribers, "activity", next);
    }
    recent
}

/// This machine's effective tokens that the last heartbeat's numbers don't include yet, so the
/// overlay's XP bar and token count move right away; each heartbeat starts over.
#[derive(Default)]
struct LocalTokens {
    /// The last heartbeat's `tokens_base`: since when, and how much was acknowledged then.
    base: Option<(String, u64)>,
    pending: u64,
}

impl LocalTokens {
    /// Recounts; true when the number changed.
    fn refresh(&mut self, store: &Store) -> bool {
        let pending = self.base.as_ref().map_or(0, |(since, synced)| {
            store
                .effective_tokens_since(since, false)
                .map_or(0, |total| total.saturating_sub(*synced))
        });
        std::mem::replace(&mut self.pending, pending) != pending
    }
}

/// Provider files written in the last six hours, with the size last seen. Codex keeps its
/// session file open, even through long pauses, and macOS reports no change to an open file
/// until it is closed, so the main loop checks these few sizes every 2 s on top of the watcher.
#[derive(Default)]
struct OpenFiles(HashMap<PathBuf, (u64, Instant)>);

impl OpenFiles {
    fn track(&mut self, paths: &[PathBuf]) {
        for path in paths {
            if let Ok(metadata) = fs::metadata(path) {
                self.0
                    .insert(path.clone(), (metadata.len(), Instant::now()));
            }
        }
    }

    /// The files that grew since the last look. One quiet for six hours drops out.
    fn grown(&mut self) -> Vec<PathBuf> {
        self.0
            .retain(|_, (_, at)| at.elapsed() < Duration::from_secs(6 * 3600));
        let mut grown = Vec::new();
        for (path, (len, at)) in &mut self.0 {
            let Ok(metadata) = fs::metadata(path) else {
                continue;
            };
            if metadata.len() != *len {
                *len = metadata.len();
                *at = Instant::now();
                grown.push(path.clone());
            }
        }
        grown
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
    local_tokens: u64,
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
        "updater_config" => {
            let endpoint = config::read_config(&opts.env)
                .map_err(internal)
                .map(|config| {
                    config.app_url.map(|url| format!(
                        "{}/api/clients/desktop/update?target={{{{target}}}}&arch={{{{arch}}}}&current={{{{current_version}}}}",
                        url.trim_end_matches('/')
                    ))
                });
            endpoint.and_then(|endpoint| {
                let url = config::read_config(&opts.env).map_err(internal)?.app_url;
                sync::installation_token(store, url.as_deref()).map_err(internal).map(|token| {
                    json!({"endpoint": endpoint, "authorization": token.map(|token| format!("Bearer {token}"))})
                })
            })
        }
        "snapshot" => Ok(json!({"activity":activity,"hero":cached,"localTokens":local_tokens})),
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
        "evolve" => {
            let character_id = params.get("characterId").and_then(Value::as_str);
            let level = params.get("level").and_then(Value::as_u64);
            let (Some(character_id), Some(level)) = (character_id, level) else {
                return Some((
                    reply,
                    Err(SocketError::new(
                        "bad_request",
                        "characterId and level are required",
                    )),
                ));
            };
            if sync_tx
                .send(SyncCommand::Evolve {
                    character_id: character_id.to_owned(),
                    level,
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
        "level_up" => {
            let from = params.get("fromLevel").and_then(Value::as_u64);
            let to = params.get("toLevel").and_then(Value::as_u64);
            let (Some(from), Some(to)) = (from, to) else {
                return Some((
                    reply,
                    Err(SocketError::new(
                        "bad_request",
                        "fromLevel and toLevel are required",
                    )),
                ));
            };
            if sync_tx
                .send(SyncCommand::LevelUp {
                    from,
                    to,
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
        "friends" => return friend_request(opts, store, reply, |client| client.friends()),
        "send_visit" => {
            let target = params
                .get("target")
                .and_then(Value::as_str)
                .filter(|target| !target.is_empty() && target.len() <= 80)
                .map(str::to_owned);
            let emote = params
                .get("emote")
                .and_then(Value::as_str)
                .filter(|emote| sync::VISIT_EMOTES.contains(emote))
                .map(str::to_owned);
            let (Some(target), Some(emote)) = (target, emote) else {
                return Some((
                    reply,
                    Err(SocketError::new(
                        "bad_request",
                        "target and a known emote are required",
                    )),
                ));
            };
            return friend_request(opts, store, reply, move |client| {
                client
                    .send_visit(&target, &emote, &clashofmeat_core::ids::random_uuid())
                    .map_err(visit_wording)
            });
        }
        "visit_done" => match params.get("id").and_then(Value::as_str) {
            Some(id) if clashofmeat_core::ids::is_uuid(id) => sync_tx
                .send(SyncCommand::VisitDone { id: id.to_owned() })
                .map(|()| json!({"queued":true}))
                .map_err(internal),
            _ => Err(SocketError::new("bad_request", "id must be a visit ID")),
        },
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
        "set_provider_homes" => (|| {
            let id = params["id"]
                .as_str()
                .filter(|id| config::provider_variable(id).is_some())
                .ok_or_else(|| {
                    SocketError::new("bad_request", "Choose Claude Code, Codex or Gemini CLI")
                })?;
            let homes = params["homes"]
                .as_array()
                .filter(|homes| homes.len() <= 8)
                .ok_or_else(|| {
                    SocketError::new("bad_request", "Choose up to eight account folders")
                })?;
            let mut paths = Vec::new();
            for home in homes {
                let home = home
                    .as_str()
                    .filter(|home| home.len() <= 4096)
                    .ok_or_else(|| SocketError::new("bad_request", "Invalid account folder"))?;
                let path = PathBuf::from(home);
                if !path.is_absolute() || !path.is_dir() {
                    return Err(SocketError::new(
                        "bad_request",
                        "Choose an existing absolute account folder",
                    ));
                }
                let path = path
                    .canonicalize()
                    .map_err(internal)?
                    .to_string_lossy()
                    .into_owned();
                if !paths.contains(&path) {
                    paths.push(path);
                }
            }
            config::update_config(&opts.env, |config| {
                let map = config
                    .other
                    .entry("providerHomes")
                    .or_insert_with(|| json!({}));
                if !map.is_object() {
                    *map = json!({});
                }
                map[id] = json!(paths);
            })
            .and_then(|_| config::ensure_profiles(&opts.env))
            .map_err(internal)?;
            Ok(json!({"id": id, "homes": paths}))
        })(),
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

/// Calls the web app for the overlay's friend panel on its own thread, so a slow answer never
/// holds up the main loop or a heartbeat.
fn friend_request(
    opts: &RunOptions,
    store: &mut Store,
    reply: Sender<std::result::Result<Value, SocketError>>,
    call: impl FnOnce(&sync::Client) -> std::result::Result<Value, sync::ApiError> + Send + 'static,
) -> Option<SocketReply> {
    let client = match make_client(&opts.env, &opts.version, Some(store)) {
        Ok(client) if client.token.is_some() => client,
        Ok(_) => {
            return Some((
                reply,
                Err(SocketError::new(
                    "not_paired",
                    "Connect this computer to Clash of Meat first.",
                )),
            ))
        }
        Err(error) => return Some((reply, Err(error))),
    };
    thread::spawn(move || {
        let _ = reply.send(call(&client).map_err(friend_error));
    });
    None
}

/// Short panel texts for a refused visit. The server's 409 and 429 texts say "kiss" whatever
/// the emote, so a 409 keeps its words with "visit" instead.
fn visit_wording(mut error: sync::ApiError) -> sync::ApiError {
    if let sync::ApiError::Status { status, detail, .. } = &mut error {
        let text = match *status {
            403 => Some("Only friends can receive your hero".to_owned()),
            404 => Some("Player not found".to_owned()),
            409 => detail
                .as_deref()
                .map(|text| text.replace("kiss", "visit").replace("Kiss", "Visit")),
            429 => Some("Wait a few seconds before sending again".to_owned()),
            _ => None,
        };
        if text.is_some() {
            *detail = text;
        }
    }
    error
}

/// Keeps the server's own words ("Gab's app isn't running") so the overlay can show them.
fn friend_error(error: sync::ApiError) -> SocketError {
    match error {
        sync::ApiError::Status { status, detail, .. } => SocketError::new(
            match status {
                400 => "bad_request",
                401 => "unauthorized",
                403 => "not_friend",
                404 => "unknown_player",
                409 => "unavailable",
                429 => "cooldown",
                _ => "server_error",
            },
            detail.unwrap_or_else(|| format!("Clash of Meat returned {status}.")),
        ),
        sync::ApiError::Transport(_) => {
            SocketError::new("offline", "Couldn't reach Clash of Meat.")
        }
        sync::ApiError::Invalid(message) => SocketError::new("invalid_response", message),
    }
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
    store: &mut Store,
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
    let config = config::read_config(&opts.env).map_err(internal)?;
    let paired = sync::installation_token(store, config.app_url.as_deref())
        .map_err(internal)?
        .is_some();
    let meta = |key| store.meta(key).map_err(internal);
    let blocked = meta("blocked")?.and_then(|raw| sync::blocked_message(&raw, &opts.version));
    let off: HashSet<String> = meta("providers_off")?
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    let health = store.health().map_err(internal)?;
    let limits = store
        .usage_limits(&now.format(&Rfc3339).map_err(internal)?)
        .map_err(internal)?;
    let providers: Vec<Value> = clashofmeat_providers::all().iter().map(|provider| {
        let manifest = provider.manifest();
        let row = health.iter().find(|row| row.provider == manifest.id);
        let mut value = json!({"id":manifest.id,"name":manifest.name,"detected":detect::detect(&opts.env,manifest),"enabled":!off.contains(manifest.id),"today_tokens":today.get(manifest.id).copied().unwrap_or(0),"health":row.map(|row| json!({"records":row.records,"malformed":row.malformed,"oversized":row.oversized,"errors":row.errors,"last_at":row.last_at})).unwrap_or_else(|| json!({"records":0,"malformed":0,"oversized":0,"errors":0,"last_at":null}))});
        if !manifest.wiring.is_empty() && manifest.id == "cursor" {
            value["wiring"] = json!(wiring::cursor_wiring_state(&opts.env));
        }
        value["limits"] = json!(limits.iter().filter(|(provider,_,_,_)| provider == manifest.id).map(|(_,key,used,reset)| json!({"key":key,"used":used,"resets_at":reset})).collect::<Vec<_>>());
        value["homes"] = config.other.get("providerHomes").and_then(|homes| homes.get(manifest.id)).cloned().unwrap_or_else(|| json!([]));
        if value["homes"].as_array().is_some_and(|homes| !homes.is_empty()) {
            value["detected"] = json!(true);
        }
        if manifest.id == "cursor" && !today.contains_key("cursor") {
            value["tokens_available"] = json!(false);
        }
        value
    }).collect();
    let counters = store.pending_count(EventKind::Counter).map_err(internal)?;
    let readings = store.pending_count(EventKind::Reading).map_err(internal)?;
    Ok(
        json!({"version":opts.version,"paired":paired,"app_url":config.app_url,"blocked":blocked,"reauth":meta("reauth")?.as_deref()==Some("true"),"pending":{"counters":counters,"readings":readings},"last_sync_at":meta("last_sync_at")?,"providers":providers,"today_tokens":today}),
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
    busy: Arc<Busy>,
) {
    let mut store = match Store::open(config::store_path(&env)) {
        Ok(store) => store,
        Err(error) => {
            eprintln!("daemon sync store: {error}");
            return;
        }
    };
    let mut cadence = Cadence::new(Instant::now());
    // The queue size, and when it last changed: the final flush waits for it to settle.
    let mut queued = (0, Instant::now());
    let mut art = hero::load_cache(&config::home(&env))
        .ok()
        .flatten()
        .map(|view| sync::HeroArt::from_view(&view))
        .unwrap_or_default();
    let mut pairing: Option<sync::Pairing> = None;
    let mut next_poll = Instant::now();
    let session = sync::companion_session(&mut store)
        .map_err(|error| eprintln!("daemon companion session: {error}"))
        .ok();
    // Visits the overlay played, acknowledged by the next heartbeat (the server re-sends a
    // visit until then).
    let mut played: Vec<String> = Vec::new();
    loop {
        let mut forced = false;
        match commands.recv_timeout(Duration::from_secs(1)) {
            Ok(SyncCommand::Stop) | Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            Ok(SyncCommand::Now) => forced = true,
            Ok(SyncCommand::Evolve {
                character_id,
                level,
                reply,
            }) => {
                let result = make_client(&env, &version, Some(&mut store)).and_then(|client| {
                    client
                        .evolve(&character_id, level)
                        .map_err(|error| SocketError::new("evolve_failed", error.to_string()))
                });
                if let Ok(evolved) = &result {
                    let _ = main.send(Message::Evolved(evolved.clone()));
                    // The new skin's art differs; refetch it, then refresh progress.
                    art = sync::HeroArt::default();
                    forced = true;
                }
                let _ = reply.send(result.map(|_| json!({"evolved":true})));
            }
            Ok(SyncCommand::LevelUp { from, to, reply }) => {
                let result = make_client(&env, &version, Some(&mut store)).and_then(|client| {
                    client
                        .level_up(from, to)
                        .map_err(|error| SocketError::new("level_up_failed", error.to_string()))
                });
                let _ = reply.send(result);
            }
            Ok(SyncCommand::Live) => {
                cadence.on_live(Instant::now());
            }
            Ok(SyncCommand::VisitDone { id }) => {
                if !played.contains(&id) {
                    if played.len() >= 20 {
                        played.remove(0);
                    }
                    played.push(id);
                }
                cadence.nudge(Instant::now());
            }
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
        let pending = store.pending_count(EventKind::Counter).unwrap_or(0)
            + store.pending_count(EventKind::Reading).unwrap_or(0);
        if pending != queued.0 {
            queued = (pending, Instant::now());
        }
        let due = if forced {
            Due::Sync
        } else {
            cadence.due(
                Instant::now(),
                busy.now.load(Ordering::Relaxed),
                pending > 0,
                queued.1.elapsed(),
            )
        };
        if due == Due::Wait {
            continue;
        }
        let client = match make_client(&env, &version, Some(&mut store)) {
            Ok(client) if client.token.is_some() => client,
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
        let mut presence = Map::new();
        if let Some(session) = &session {
            presence.insert("sessionId".into(), json!(session));
        }
        if !played.is_empty() {
            presence.insert("acknowledgedVisits".into(), json!(played));
        }
        let result = if due == Due::Drain {
            sync::drain_once(&client, &mut store, OffsetDateTime::now_utc())
        } else {
            sync::sync_once_with(
                &client,
                &mut store,
                OffsetDateTime::now_utc(),
                &mut art,
                &presence,
            )
        };
        if result.heartbeat.is_some() {
            played.clear();
        }
        cadence.after(
            Instant::now(),
            Outcome {
                due,
                uploaded: result.uploaded,
                more: result.more,
                error: result.error.as_ref(),
                next_sync: result.next_sync,
                busy: busy.recent.load(Ordering::Relaxed),
            },
        );
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

/// A client for the configured app. Given the store, it carries the installation token, when that
/// app issued it.
fn make_client(
    env: &Env,
    version: &str,
    store: Option<&mut Store>,
) -> std::result::Result<sync::Client, SocketError> {
    let url = config::read_config(env)
        .map_err(internal)?
        .app_url
        .ok_or_else(|| SocketError::new("not_configured", "app_url is not configured"))?;
    let token = match store {
        Some(store) => sync::installation_token(store, Some(&url)).map_err(internal)?,
        None => None,
    };
    sync::Client::new(&url, token, version).map_err(internal)
}

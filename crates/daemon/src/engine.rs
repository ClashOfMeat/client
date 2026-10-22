//! Turns provider source files into durable observations and live activity.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::Result;
use clashofmeat_core::{
    observer, reader, Cx, Env, Event, FileCursor, FileEntry, Observation, Provider, Record, Signal,
    SourceFormat, Store, Turn, TurnPhase,
};
use serde::{Serialize, Serializer};
use time::{format_description::well_known::Rfc3339, Duration, OffsetDateTime};

pub struct Engine {
    env: Env,
    providers: Vec<Box<dyn Provider>>,
    client_version: String,
    sessions: HashMap<(String, String), SessionActivity>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct ScanReport {
    pub files_read: usize,
    pub records: u64,
    pub events_saved: usize,
    pub activity_changed: bool,
    /// Files written in the last six hours. A writer that keeps its file open (Codex) raises no
    /// watcher event on macOS until it closes the file, so the daemon polls these for growth.
    pub recent: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionActivity {
    pub provider: String,
    pub session_id: String,
    pub phase: TurnPhase,
    #[serde(serialize_with = "serialize_time")]
    pub last_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize)]
pub struct Activity {
    pub busy: bool,
    pub sessions: Vec<SessionView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionView {
    pub provider: String,
    pub session_id: String,
    pub state: SessionState,
    #[serde(serialize_with = "serialize_time")]
    pub last_at: OffsetDateTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionState {
    Busy,
    Done,
    Idle,
}

fn serialize_time<S: Serializer>(
    at: &OffsetDateTime,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.serialize_str(&at.format(&Rfc3339).map_err(serde::ser::Error::custom)?)
}

impl Engine {
    pub fn new(env: Env, providers: Vec<Box<dyn Provider>>, client_version: &str) -> Self {
        Self {
            env,
            providers,
            client_version: client_version.to_owned(),
            sessions: HashMap::new(),
        }
    }

    pub fn scan(
        &mut self,
        store: &mut Store,
        changed: Option<&[PathBuf]>,
        now: OffsetDateTime,
    ) -> Result<ScanReport> {
        let paired_at = store.meta("paired_at")?;
        let since_ms = paired_at
            .as_deref()
            .and_then(|at| OffsetDateTime::parse(at, &Rfc3339).ok())
            .unwrap_or(now - Duration::hours(1))
            .unix_timestamp_nanos()
            / 1_000_000;
        let since_ms = i64::try_from(since_ms).unwrap_or(i64::MIN);
        let recent_ms =
            i64::try_from((now - Duration::hours(6)).unix_timestamp_nanos() / 1_000_000)
                .unwrap_or(i64::MIN);
        let off: HashSet<String> = store
            .meta("providers_off")?
            .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
            .unwrap_or_default()
            .into_iter()
            .collect();
        let changed: Option<HashSet<PathBuf>> =
            changed.map(|paths| paths.iter().map(|path| canonical_or_self(path)).collect());
        let now_iso = now.format(&Rfc3339)?;
        let mut report = ScanReport::default();
        self.sessions
            .retain(|_, session| now - session.last_at <= Duration::minutes(30));

        let config = crate::config::read_config(&self.env)?;
        for provider in &self.providers {
            let manifest = provider.manifest();
            if off.contains(manifest.id) {
                continue;
            }
            for (source_index, source) in manifest.sources.iter().enumerate() {
                for (root, profile) in
                    crate::config::source_roots(&self.env, &config, manifest.id, source.root)
                {
                    if !root.is_dir() {
                        continue;
                    }
                    // A file reported by the watcher may have an old mtime (for example, an
                    // atomic rename). Check only the reported paths, without the since gate.
                    let files = if let Some(paths) = changed.as_ref() {
                        changed_files(&root, source.glob, paths)
                    } else {
                        reader::list_files(&root, source.glob, 4, Some(since_ms))
                    };
                    for file in files {
                        let file_key = file
                            .path
                            .strip_prefix(&root)?
                            .to_string_lossy()
                            .into_owned();
                        let file_key = profile
                            .as_ref()
                            .map_or(file_key.clone(), |key| format!("@profile/{key}/{file_key}"));
                        let mut cursor = store
                            .cursor(manifest.id, source_index, &file_key)?
                            .unwrap_or_default();
                        if file.mtime_ms >= recent_ms {
                            report.recent.push(file.path.clone());
                        }
                        if cursor_complete(&cursor, &file, source.format) {
                            continue;
                        }
                        report.files_read += 1;
                        let mut counts = FileCounts::default();
                        let read_result = scan_file(
                            provider.as_ref(),
                            source_index,
                            &file_key,
                            &file.path,
                            source.format,
                            &mut cursor,
                            &mut self.sessions,
                            store,
                            paired_at.as_deref(),
                            now,
                            &mut report,
                            &mut counts,
                            &self.client_version,
                        );
                        if read_result.is_err() {
                            counts.errors += 1;
                        }
                        report.records += counts.records;
                        store.record_health(
                            manifest.id,
                            counts.records,
                            counts.malformed,
                            counts.oversized,
                            counts.errors,
                            Some(&now_iso),
                        )?;
                        // Reader I/O is file-local; store failures still abort the scan.
                        if let Err(error) = read_result {
                            if !error.is::<std::io::Error>() {
                                return Err(error);
                            }
                        }
                    }
                }
            }
            if changed.is_none() {
                let idle = match manifest.id {
                    "codex" | "cursor" => std::time::Duration::from_secs(30 * 60),
                    _ => std::time::Duration::from_secs(10 * 60),
                };
                observer::finalize_idle(store, manifest.id, now, idle)?;
            }
        }
        Ok(report)
    }

    pub fn activity(&self, now: OffsetDateTime) -> Activity {
        let mut sessions: Vec<_> = self
            .sessions
            .values()
            .filter_map(|session| {
                let age = now - session.last_at;
                if age > Duration::minutes(30) {
                    return None;
                }
                let state = match session.phase {
                    TurnPhase::Start | TurnPhase::Step if age <= Duration::seconds(120) => {
                        SessionState::Busy
                    }
                    TurnPhase::Stop if age <= Duration::seconds(10) => SessionState::Done,
                    _ => SessionState::Idle,
                };
                Some(SessionView {
                    provider: session.provider.clone(),
                    session_id: session.session_id.clone(),
                    state,
                    last_at: session.last_at,
                })
            })
            .collect();
        sessions.sort_by(|a, b| (&a.provider, &a.session_id).cmp(&(&b.provider, &b.session_id)));
        let busy = sessions
            .iter()
            .any(|session| session.state == SessionState::Busy);
        Activity { busy, sessions }
    }
}

#[derive(Default)]
struct FileCounts {
    records: u64,
    malformed: u64,
    oversized: u64,
    errors: u64,
}

#[allow(clippy::too_many_arguments)]
fn scan_file(
    provider: &dyn Provider,
    source_index: usize,
    file_key: &str,
    path: &Path,
    format: SourceFormat,
    cursor: &mut FileCursor,
    sessions: &mut HashMap<(String, String), SessionActivity>,
    store: &mut Store,
    paired_at: Option<&str>,
    now: OffsetDateTime,
    report: &mut ScanReport,
    counts: &mut FileCounts,
    client_version: &str,
) -> Result<()> {
    match format {
        SourceFormat::Jsonl | SourceFormat::Spool => loop {
            let before = cursor.offset;
            let batch = reader::read_jsonl(path, cursor)?;
            counts.records += batch.lines.len() as u64;
            counts.malformed += u64::from(batch.malformed);
            counts.oversized += u64::from(batch.oversized);
            let mut events = Vec::new();
            let mut observations = Vec::new();
            for line in batch.lines {
                collect_signals(
                    provider,
                    source_index,
                    file_key,
                    line.offset,
                    line.value,
                    cursor,
                    sessions,
                    now,
                    report,
                    &mut events,
                    &mut observations,
                );
            }
            save_pass(
                provider,
                store,
                cursor,
                source_index,
                file_key,
                paired_at,
                &events,
                &observations,
                client_version,
                report,
            )?;
            if !batch.more || cursor.offset == before {
                break;
            }
        },
        SourceFormat::Json => {
            // read_json checks length and mtime, so an atomic replacement with the
            // same length and timestamp needs its old length cleared first.
            let metadata = fs::metadata(path)?;
            if cursor.inode != file_inode(&metadata) {
                cursor.len = 0;
            }
            if let Some(value) = reader::read_json(path, cursor)? {
                counts.records += 1;
                let mut events = Vec::new();
                let mut observations = Vec::new();
                collect_signals(
                    provider,
                    source_index,
                    file_key,
                    0,
                    value,
                    cursor,
                    sessions,
                    now,
                    report,
                    &mut events,
                    &mut observations,
                );
                cursor.offset = cursor.len;
                save_pass(
                    provider,
                    store,
                    cursor,
                    source_index,
                    file_key,
                    paired_at,
                    &events,
                    &observations,
                    client_version,
                    report,
                )?;
            } else {
                if cursor.len > 32 * 1024 * 1024 {
                    counts.oversized += 1;
                } else {
                    counts.malformed += 1;
                }
                cursor.offset = cursor.len;
                store.put_cursor(provider.manifest().id, source_index, file_key, cursor)?;
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn collect_signals(
    provider: &dyn Provider,
    source_index: usize,
    file_key: &str,
    offset: u64,
    value: serde_json::Value,
    cursor: &mut FileCursor,
    sessions: &mut HashMap<(String, String), SessionActivity>,
    now: OffsetDateTime,
    report: &mut ScanReport,
    events: &mut Vec<Event>,
    observations: &mut Vec<Observation>,
) {
    let record = Record {
        source: source_index,
        file_key: file_key.to_owned(),
        offset,
        value,
    };
    let mut cx = Cx {
        file_state: &mut cursor.state,
        now,
    };
    for mut signal in provider.parse(&record, &mut cx) {
        if let Some(profile) = file_key
            .strip_prefix("@profile/")
            .and_then(|key| key.split('/').next())
        {
            namespace_signal(&mut signal, profile);
        }
        match signal {
            Signal::Event(event) => events.push(event),
            Signal::Observer(observation) => {
                if !matches!(provider.manifest().id, "claude-code" | "claude-cowork") {
                    let metric = match &observation {
                        Observation::Start {
                            task_key,
                            session_id,
                            at,
                            ..
                        } => Some(("prompt_count", task_key.as_str(), session_id.as_str(), at)),
                        Observation::Tool {
                            task,
                            tool_id,
                            subagent: true,
                            at,
                            ..
                        } => {
                            let session = match task {
                                clashofmeat_core::TaskRef::Active(key)
                                | clashofmeat_core::TaskRef::Key(key) => key,
                            };
                            Some(("subagent_count", tool_id.as_str(), session.as_str(), at))
                        }
                        _ => None,
                    };
                    if let Some((key, identity, session, at)) = metric {
                        events.push(Event {
                            identity: format!("{}:{key}:{identity}", provider.manifest().id),
                            key: key.into(),
                            value: 1,
                            kind: clashofmeat_core::EventKind::Counter,
                            session_id: clashofmeat_core::ids::session_uuid(
                                provider.manifest().id,
                                session,
                            ),
                            at: at.clone(),
                            resets_at: None,
                            model: None,
                            provider: Some(provider.manifest().id.into()),
                        });
                    }
                }
                observations.push(observation);
            }
            Signal::Turn(turn) => {
                report.activity_changed |= apply_turn(sessions, turn, now);
            }
        }
    }
}

// Keep different accounts' source-local IDs separate without uploading account paths.
fn namespace_signal(signal: &mut Signal, profile: &str) {
    let key = |value: &mut String| {
        *value = clashofmeat_core::ids::stable_id(&format!("{profile}:{value}"))
    };
    let task = |value: &mut clashofmeat_core::TaskRef| match value {
        clashofmeat_core::TaskRef::Active(value) | clashofmeat_core::TaskRef::Key(value) => {
            key(value)
        }
    };
    match signal {
        Signal::Event(event) => {
            key(&mut event.identity);
            key(&mut event.session_id);
        }
        Signal::Turn(turn) => key(&mut turn.session_id),
        Signal::Observer(observation) => match observation {
            Observation::Start {
                context,
                task_key,
                source_key,
                session_id,
                ..
            } => {
                key(context);
                key(task_key);
                key(session_id);
                *source_key = format!("profile:{profile}");
            }
            Observation::Request {
                task: reference,
                response_id,
                ..
            } => {
                task(reference);
                key(response_id);
            }
            Observation::Tool {
                task: reference,
                tool_id,
                ..
            } => {
                task(reference);
                key(tool_id);
            }
            Observation::Compaction {
                task: reference,
                compaction_id,
                ..
            } => {
                task(reference);
                key(compaction_id);
            }
            Observation::Model {
                task: reference, ..
            }
            | Observation::Finish {
                task: reference, ..
            }
            | Observation::ClientVersion {
                task: reference, ..
            } => task(reference),
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn save_pass(
    provider: &dyn Provider,
    store: &mut Store,
    cursor: &FileCursor,
    source_index: usize,
    file_key: &str,
    paired_at: Option<&str>,
    events: &[Event],
    observations: &[Observation],
    client_version: &str,
    report: &mut ScanReport,
) -> Result<()> {
    if let Some(paired_at) = paired_at {
        report.events_saved +=
            store.save_events(provider.manifest().id, events, Some(paired_at))?;
        observer::apply(store, provider.manifest().id, observations, client_version)?;
    }
    store.put_cursor(provider.manifest().id, source_index, file_key, cursor)?;
    Ok(())
}

fn apply_turn(
    sessions: &mut HashMap<(String, String), SessionActivity>,
    turn: Turn,
    now: OffsetDateTime,
) -> bool {
    let Ok(at) = OffsetDateTime::parse(&turn.at, &Rfc3339) else {
        return false;
    };
    if at < now - Duration::minutes(10) {
        return false;
    }
    let key = (turn.provider.clone(), turn.session_id.clone());
    if sessions
        .get(&key)
        .is_some_and(|session| at < session.last_at)
    {
        return false;
    }
    let changed = sessions
        .get(&key)
        .is_none_or(|session| session.last_at != at || session.phase != turn.phase);
    sessions.insert(
        key,
        SessionActivity {
            provider: turn.provider,
            session_id: turn.session_id,
            phase: turn.phase,
            last_at: at,
        },
    );
    changed
}

fn canonical_or_self(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn changed_files(root: &Path, glob: &str, paths: &HashSet<PathBuf>) -> Vec<FileEntry> {
    let canonical_root = canonical_or_self(root);
    let mut files = Vec::new();
    for path in paths {
        let Ok(relative) = path.strip_prefix(&canonical_root) else {
            continue;
        };
        if !reader::depth_ok(relative, 4) || !reader::glob_matches(glob, relative) {
            continue;
        }
        let Ok(metadata) = fs::metadata(path) else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        let mtime_ms = match modified.duration_since(UNIX_EPOCH) {
            Ok(duration) => i64::try_from(duration.as_millis()).unwrap_or(i64::MAX),
            Err(error) => -i64::try_from(error.duration().as_millis()).unwrap_or(i64::MAX),
        };
        files.push(FileEntry {
            path: root.join(relative),
            len: metadata.len(),
            mtime_ms,
        });
    }
    files.sort_by(|a, b| {
        b.mtime_ms
            .cmp(&a.mtime_ms)
            .then_with(|| a.path.cmp(&b.path))
    });
    files
}

fn cursor_complete(cursor: &FileCursor, file: &FileEntry, format: SourceFormat) -> bool {
    let Ok(metadata) = fs::metadata(&file.path) else {
        return false;
    };
    cursor.inode == file_inode(&metadata)
        && cursor.len == file.len
        && cursor.offset == file.len
        && (format != SourceFormat::Json || cursor.mtime_ms == file.mtime_ms)
}

fn file_inode(metadata: &fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        0
    }
}

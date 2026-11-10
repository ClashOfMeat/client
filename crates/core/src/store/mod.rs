//! Durable local observations and reader state.

pub mod migrations;

use std::fs;
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use thiserror::Error;
use time::{format_description::well_known::Rfc3339, OffsetDateTime, UtcOffset};

use crate::ids::MAX_OBSERVATION_VALUE;
use crate::reader::FileCursor;
use crate::signal::{Event, EventKind};

/// Errors while opening or using the local store.
#[derive(Debug, Error)]
pub enum StoreError {
    /// Filesystem operation failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// SQLite operation failed.
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
    /// Cursor JSON operation failed.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// The database was written by a newer client.
    #[error("unsupported database version {0}")]
    UnsupportedVersion(i64),
    /// The source index does not fit in SQLite's integer type.
    #[error("source index is out of range")]
    SourceOutOfRange,
}

/// Result of a local store operation.
pub type Result<T> = std::result::Result<T, StoreError>;

/// Observation held in the upload queue.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredEvent {
    /// Stable observation identifier.
    pub id: String,
    /// Session identifier, if present.
    pub session_id: Option<String>,
    /// Observation timestamp.
    pub at: String,
    /// Metric key.
    pub key: String,
    /// Largest observed value.
    pub value: u64,
    /// Rate-limit reset timestamp, if present.
    pub resets_at: Option<String>,
    /// Aggregation behavior.
    pub kind: EventKind,
    /// Model family, if present.
    pub model: Option<String>,
    /// Originating provider, if present.
    pub provider: Option<String>,
    /// The subagent that produced it, if known.
    pub agent_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    fn event(identity: &str, key: &str, value: u64, at: &str) -> Event {
        Event {
            identity: identity.into(),
            key: key.into(),
            value,
            kind: EventKind::Counter,
            session_id: "session".into(),
            at: at.into(),
            resets_at: None,
            model: None,
            provider: Some("codex".into()),
            agent_id: None,
        }
    }

    #[test]
    fn reopen_preserves_data_and_migration_version() {
        let dir = tempdir().unwrap();
        let parent = dir.path().join("private");
        let path = parent.join("activity.sqlite");
        {
            let mut store = Store::open(&path).unwrap();
            store.set_meta("key", "value").unwrap();
            assert_eq!(
                store
                    .conn
                    .pragma_query_value(None, "user_version", |row| row.get::<_, usize>(0))
                    .unwrap(),
                migrations::MIGRATIONS.len()
            );
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.meta("key").unwrap().as_deref(), Some("value"));
        assert_eq!(
            store
                .conn
                .pragma_query_value(None, "user_version", |row| row.get::<_, usize>(0))
                .unwrap(),
            migrations::MIGRATIONS.len()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&parent).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn pending_counts_follow_acknowledgements_without_loading_rows() {
        let mut store = Store::open_in_memory().unwrap();
        let counter = event("count", "input_tokens", 10, "2026-01-01T00:00:00Z");
        store
            .save_events("codex", std::slice::from_ref(&counter), None)
            .unwrap();
        assert_eq!(store.pending_count(EventKind::Counter).unwrap(), 1);
        assert_eq!(store.pending_count(EventKind::Reading).unwrap(), 0);
        store
            .mark_synced(&[(counter.id(), counter.value, counter.at)])
            .unwrap();
        assert_eq!(store.pending_count(EventKind::Counter).unwrap(), 0);
    }

    #[test]
    fn quota_status_uses_latest_unexpired_report() {
        let mut store = Store::open_in_memory().unwrap();
        let mut first = event("quota-1", "five_hour_percent", 80, "2026-01-01T01:00:00Z");
        first.kind = EventKind::Reading;
        first.resets_at = Some("2026-01-01T05:00:00Z".into());
        let mut latest = first.clone();
        latest.identity = "quota-2".into();
        latest.at = "2026-01-01T02:00:00Z".into();
        latest.value = 90;
        store.save_events("codex", &[first, latest], None).unwrap();
        assert_eq!(
            store.usage_limits("2026-01-01T03:00:00Z").unwrap(),
            vec![(
                "codex".into(),
                "five_hour_percent".into(),
                90,
                "2026-01-01T05:00:00Z".into()
            )]
        );
        assert!(store
            .usage_limits("2026-01-01T05:00:00Z")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn effective_tokens_count_a_tenth_of_cache_reads_and_skip_refused_rows() {
        let mut store = Store::open_in_memory().unwrap();
        let at = "2026-01-01T12:00:00Z";
        let rows = [
            event("r", "input_tokens", 100, at),
            event("r", "cache_read_tokens", 1000, at),
            event("r", "cache_write_tokens", 10, at),
            event("s", "output_tokens", 5, at),
            event("old", "output_tokens", 7, "2025-12-01T00:00:00Z"),
            event("p", "prompt_count", 1, at),
        ];
        store.save_events("codex", &rows, None).unwrap();
        let since = "2026-01-01T00:00:00Z";
        assert_eq!(store.effective_tokens_since(since).unwrap(), 215);
        assert_eq!(store.acknowledged_tokens_since(since).unwrap(), 0);
        let snapshot = |e: &Event| (e.id(), e.value, e.at.clone());
        store.mark_synced(&[snapshot(&rows[0])]).unwrap();
        store.reject(&[snapshot(&rows[3])]).unwrap();
        assert_eq!(store.acknowledged_tokens_since(since).unwrap(), 100);
        assert_eq!(store.effective_tokens_since(since).unwrap(), 210);
    }

    #[test]
    fn token_changes_follow_new_grown_and_refused_rows_and_sums_skip_old_days() {
        let mut store = Store::open_in_memory().unwrap();
        let old = event("old", "output_tokens", 7, "2025-12-01T00:00:00Z");
        store.save_events("codex", &[old], None).unwrap();
        assert!(store.take_token_changes().is_empty());
        store.track_token_changes();
        let at = "2026-01-01T12:00:00+02:00";
        let mut read = event("r", "cache_read_tokens", 1000, at);
        let count = event("p", "prompt_count", 1, at);
        store
            .save_events("codex", &[read.clone(), count], None)
            .unwrap();
        assert_eq!(
            store.take_token_changes(),
            vec![TokenChange {
                before: None,
                after: (at.into(), 1000)
            }]
        );
        // Unchanged: nothing to add. Refused, then grown: it counts in full again.
        store.save_events("codex", &[read.clone()], None).unwrap();
        assert!(store.take_token_changes().is_empty());
        store.reject(&[(read.id(), 1000, at.into())]).unwrap();
        read.value = 1500;
        store.begin().unwrap();
        store.save_events("codex", &[read], None).unwrap();
        store.commit().unwrap();
        assert_eq!(
            store.take_token_changes(),
            vec![TokenChange {
                before: None,
                after: (at.into(), 1500)
            }]
        );
        // 10:00 UTC, written with an offset: after a 09:00 UTC start, before an 11:00 one.
        assert_eq!(
            store
                .effective_tenths_since("2026-01-01T09:00:00Z")
                .unwrap(),
            1500
        );
        assert_eq!(
            store
                .effective_tenths_since("2026-01-01T11:00:00Z")
                .unwrap(),
            0
        );
    }

    #[test]
    fn a_row_queued_again_still_counts_what_the_server_acknowledged() {
        let mut store = Store::open_in_memory().unwrap();
        let since = "2026-01-01T00:00:00Z";
        let mut read = event("r", "cache_read_tokens", 1000, "2026-01-01T12:00:00Z");
        let mut output = event("r", "output_tokens", 40, "2026-01-01T12:00:00Z");
        store
            .save_events("codex", &[read.clone(), output.clone()], None)
            .unwrap();
        let snapshot = |e: &Event| (e.id(), e.value, e.at.clone());
        store
            .mark_synced(&[snapshot(&read), snapshot(&output)])
            .unwrap();
        // A later line of the same message: the same cache read, more output.
        read.at = "2026-01-01T12:00:05Z".into();
        output.at = read.at.clone();
        output.value = 90;
        store.save_events("codex", &[read, output], None).unwrap();
        assert_eq!(store.pending_count(EventKind::Counter).unwrap(), 2);
        // Only the 50 new output tokens are unknown to the server.
        assert_eq!(store.effective_tokens_since(since).unwrap(), 190);
        assert_eq!(store.acknowledged_tokens_since(since).unwrap(), 140);
    }

    #[test]
    fn upsert_follows_collector_conflict_rule() {
        let mut store = Store::open_in_memory().unwrap();
        let mut tokens = event("same", "input_tokens", 5, "2026-01-01T00:00:00Z");
        let id = tokens.id();
        assert_eq!(
            store.save_events("codex", &[tokens.clone()], None).unwrap(),
            1
        );
        store
            .mark_synced(&[(id.clone(), 5, tokens.at.clone())])
            .unwrap();
        tokens.value = 3;
        tokens.model = Some("new-model".into());
        assert_eq!(
            store.save_events("codex", &[tokens.clone()], None).unwrap(),
            0
        );
        assert_eq!(store.pending(EventKind::Counter, 10).unwrap().len(), 0);
        tokens.at = "2026-01-02T00:00:00Z".into();
        assert_eq!(
            store.save_events("codex", &[tokens.clone()], None).unwrap(),
            1
        );
        let pending = store.pending(EventKind::Counter, 10).unwrap();
        assert_eq!(
            (
                pending[0].value,
                pending[0].at.as_str(),
                pending[0].model.as_deref()
            ),
            (5, "2026-01-02T00:00:00Z", Some("new-model"))
        );
        store
            .mark_synced(&[(id.clone(), 5, tokens.at.clone())])
            .unwrap();
        tokens.value = 6;
        tokens.model = None;
        assert_eq!(store.save_events("codex", &[tokens], None).unwrap(), 1);
        assert_eq!(
            store.pending(EventKind::Counter, 10).unwrap()[0]
                .model
                .as_deref(),
            Some("new-model")
        );

        let mut count = event("other", "session_count", 1, "2026-01-01T00:00:00Z");
        store.save_events("codex", &[count.clone()], None).unwrap();
        count.at = "2026-01-03T00:00:00Z".into();
        count.value = 2;
        store.save_events("codex", &[count], None).unwrap();
        assert_eq!(
            store
                .pending(EventKind::Counter, 10)
                .unwrap()
                .iter()
                .find(|row| row.key == "session_count")
                .unwrap()
                .at,
            "2026-01-01T00:00:00Z"
        );
    }

    #[test]
    fn gates_order_limit_and_acknowledges_only_unchanged_rows() {
        let mut store = Store::open_in_memory().unwrap();
        let old = event("old", "session_count", 1, "2026-01-01T00:00:00Z");
        let first = event("first", "session_count", 1, "2026-01-02T00:00:00Z");
        let second = event("second", "session_count", 1, "2026-01-03T00:00:00Z");
        let oversized = event(
            "huge",
            "session_count",
            MAX_OBSERVATION_VALUE + 1,
            "2026-01-04T00:00:00Z",
        );
        assert_eq!(
            store
                .save_events(
                    "codex",
                    &[second.clone(), old, oversized, first.clone()],
                    Some("2026-01-02T00:00:00Z")
                )
                .unwrap(),
            2
        );
        let snapshot = store.pending(EventKind::Counter, 1).unwrap();
        assert_eq!(snapshot[0].id, first.id());
        assert_eq!(store.pending(EventKind::Counter, 10).unwrap().len(), 2);
        let mut changed = first.clone();
        changed.value = 2;
        store.save_events("codex", &[changed], None).unwrap();
        store
            .mark_synced(&[(
                snapshot[0].id.clone(),
                snapshot[0].value,
                snapshot[0].at.clone(),
            )])
            .unwrap();
        assert_eq!(store.pending(EventKind::Counter, 10).unwrap().len(), 2);
        let current = store.pending(EventKind::Counter, 10).unwrap();
        store
            .mark_synced(
                &current
                    .iter()
                    .map(|row| (row.id.clone(), row.value, row.at.clone()))
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        assert!(store.pending(EventKind::Counter, 10).unwrap().is_empty());
    }

    #[test]
    fn token_totals_include_synced_events_and_filter_time_and_keys() {
        let mut store = Store::open_in_memory().unwrap();
        let mut input = event("today-input", "input_tokens", 5, "2026-10-20T00:00:00Z");
        input.provider = Some("claude".into());
        let mut output = event("today-output", "output_tokens", 7, "2026-10-20T01:00:00Z");
        output.provider = Some("codex".into());
        let old = event("old", "cache_read_tokens", 20, "2026-10-19T23:59:59Z");
        let other = event("other", "session_count", 10, "2026-10-20T02:00:00Z");
        let future = event("future", "cache_write_tokens", 30, "2026-10-21T00:00:00Z");
        store
            .save_events("codex", &[input.clone(), output, old, other, future], None)
            .unwrap();
        store
            .mark_synced(&[(input.id(), input.value, input.at)])
            .unwrap();
        assert_eq!(
            store
                .token_totals_between("2026-10-20T00:00:00Z", "2026-10-20T12:00:00Z")
                .unwrap(),
            vec![("claude".into(), 5), ("codex".into(), 7)]
        );
    }

    #[test]
    fn cursor_meta_and_health_roundtrip() {
        let mut store = Store::open_in_memory().unwrap();
        let mut cursor = FileCursor {
            offset: 42,
            ..FileCursor::default()
        };
        cursor
            .state
            .insert("nested".into(), json!({"seen": [1, 2]}));
        store.put_cursor("codex", 1, "file", &cursor).unwrap();
        assert_eq!(
            store.cursor("codex", 1, "file").unwrap(),
            Some(cursor.clone())
        );
        assert_eq!(
            store.cursors("codex").unwrap(),
            vec![(1, "file".into(), cursor)]
        );
        assert_eq!(store.cursor("other", 1, "file").unwrap(), None);
        store.set_meta("k", "a").unwrap();
        store.set_meta("k", "b").unwrap();
        assert_eq!(store.meta("k").unwrap().as_deref(), Some("b"));
        store.delete_meta("k").unwrap();
        assert_eq!(store.meta("k").unwrap(), None);
        store
            .record_health("codex", 1, 2, 3, 4, Some("2026-01-01T00:00:00Z"))
            .unwrap();
        store.record_health("codex", 2, 3, 4, 5, None).unwrap();
        let health = store.health().unwrap();
        assert_eq!(
            (
                health[0].records,
                health[0].malformed,
                health[0].oversized,
                health[0].errors
            ),
            (3, 5, 7, 9)
        );
        assert_eq!(health[0].last_at.as_deref(), Some("2026-01-01T00:00:00Z"));
    }
}

/// Accumulated parser health for a provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthRow {
    /// Provider identifier.
    pub provider: String,
    /// Number of records processed.
    pub records: u64,
    /// Number of malformed records.
    pub malformed: u64,
    /// Number of oversized records.
    pub oversized: u64,
    /// Number of errors.
    pub errors: u64,
    /// Most recent health timestamp, if present.
    pub last_at: Option<String>,
}

/// SQLite backed local store.
pub struct Store {
    conn: Connection,
    /// Token changes saved since the last `take_token_changes`, once tracking is on.
    token_changes: Option<Vec<TokenChange>>,
}

/// How one saved token observation changed, in tenths of an effective token (a cache read
/// counts a tenth of a token). The daemon adds these to its running totals instead of summing
/// the table again after every scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenChange {
    /// The observation's time and amount before the save; `None` when it was new or refused.
    pub before: Option<(String, u64)>,
    /// Its time and amount after the save.
    pub after: (String, u64),
}

const TOKEN_KEYS: [&str; 4] = [
    "input_tokens",
    "output_tokens",
    "cache_read_tokens",
    "cache_write_tokens",
];

fn tenths(key: &str, value: u64) -> u64 {
    if key == "cache_read_tokens" {
        value
    } else {
        value.saturating_mul(10)
    }
}

/// A date every timestamp at or after `since` sorts after as text, whatever its UTC offset:
/// the day before `since`. It lets the `at` index skip older rows before the exact
/// `julianday` comparison. Empty (no skipping) when `since` cannot be read.
fn day_before(since: &str) -> String {
    OffsetDateTime::parse(since, &Rfc3339)
        .ok()
        .and_then(|at| at.to_offset(UtcOffset::UTC).date().previous_day())
        .map(|day| day.to_string())
        .unwrap_or_default()
}

impl Store {
    /// Connection for the private task observer's normalized tables.
    pub(crate) fn observer_connection(&mut self) -> &mut Connection {
        &mut self.conn
    }
    /// Opens a database, creating private parent and database paths as needed.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            // Never open a descriptor on an existing database: closing any descriptor on a file
            // drops every POSIX lock this process holds on it, SQLite's included. Another process
            // closing the database (the CLI while the daemon runs) would then think it is the last
            // connection and delete the WAL that this process's connections still write to.
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
            {
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        }
        let conn = Connection::open(path)?;
        Self::initialize(conn)
    }

    /// Opens an in-memory database with the same schema and settings.
    pub fn open_in_memory() -> Result<Self> {
        Self::initialize(Connection::open_in_memory()?)
    }

    fn initialize(mut conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.busy_timeout(std::time::Duration::from_millis(5000))?;
        // In WAL mode this syncs at checkpoints instead of at every commit. A power cut can lose
        // the last commits, never corrupt the file; cursors and events are saved together, so
        // the lost lines are read again.
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version < 0 || version as usize > migrations::MIGRATIONS.len() {
            return Err(StoreError::UnsupportedVersion(version));
        }
        for (index, sql) in migrations::MIGRATIONS
            .iter()
            .enumerate()
            .skip(version as usize)
        {
            let tx = conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", index + 1)?;
            tx.commit()?;
        }
        Ok(Self {
            conn,
            token_changes: None,
        })
    }

    /// Saves accepted events atomically and returns the number inserted or changed.
    /// Starts recording how saved token observations change (see `take_token_changes`).
    pub fn track_token_changes(&mut self) {
        self.token_changes.get_or_insert_with(Vec::new);
    }

    /// The token changes saved since the last call; empty unless tracking is on.
    pub fn take_token_changes(&mut self) -> Vec<TokenChange> {
        self.token_changes
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
    }

    /// Opens a write transaction that the saves up to `commit` share: one commit for a pass
    /// over a file instead of one per statement.
    pub fn begin(&mut self) -> Result<()> {
        Ok(self.conn.execute_batch("BEGIN IMMEDIATE")?)
    }

    /// Commits the transaction `begin` opened.
    pub fn commit(&mut self) -> Result<()> {
        Ok(self.conn.execute_batch("COMMIT")?)
    }

    /// Drops the transaction `begin` opened, if one is still open.
    pub fn rollback(&mut self) {
        if !self.conn.is_autocommit() {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
    }

    pub fn save_events(
        &mut self,
        _provider: &str,
        events: &[Event],
        paired_at: Option<&str>,
    ) -> Result<usize> {
        let track = self.token_changes.is_some();
        let mut changes = Vec::new();
        // A savepoint, so it also works inside `begin` ... `commit`.
        let tx = self.conn.savepoint()?;
        let mut changed = 0;
        {
            let mut look = tx.prepare_cached("SELECT at, value, synced FROM events WHERE id=?1")?;
            let mut row = |id: &str| {
                look.query_row([id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, u64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                })
                .optional()
            };
            let mut statement = tx.prepare_cached(
                "INSERT INTO events (id, session_id, at, key, value, resets_at, kind, source, model, provider, agent_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'live', ?8, ?9, ?10)
                 ON CONFLICT(id) DO UPDATE SET
                   value=MAX(events.value, excluded.value),
                   model=COALESCE(excluded.model, events.model),
                   agent_id=COALESCE(excluded.agent_id, events.agent_id),
                   at=CASE WHEN events.key IN ('input_tokens','output_tokens','cache_read_tokens','cache_write_tokens','model_step_count')
                     THEN MAX(events.at, excluded.at) ELSE events.at END,
                   synced=0
                 WHERE excluded.value > events.value OR
                   (events.key IN ('input_tokens','output_tokens','cache_read_tokens','cache_write_tokens','model_step_count')
                    AND excluded.at > events.at)",
            )?;
            for event in events {
                if paired_at.is_some_and(|paired| event.at.as_str() < paired)
                    || event.value > MAX_OBSERVATION_VALUE
                {
                    continue;
                }
                let kind = match event.kind {
                    EventKind::Counter => "counter",
                    EventKind::Reading => "reading",
                };
                let id = event.id();
                let tokens = track
                    && matches!(event.kind, EventKind::Counter)
                    && TOKEN_KEYS.contains(&event.key.as_str());
                let before = if tokens { row(&id)? } else { None };
                let saved = statement.execute(params![
                    id,
                    event.session_id,
                    event.at,
                    event.key,
                    event.value,
                    event.resets_at,
                    kind,
                    event.model,
                    event.provider,
                    event.agent_id,
                ])?;
                changed += saved;
                if tokens && saved > 0 {
                    if let Some((at, value, _)) = row(&id)? {
                        changes.push(TokenChange {
                            before: before
                                .filter(|(_, _, synced)| *synced != 2)
                                .map(|(at, value, _)| (at, tenths(&event.key, value))),
                            after: (at, tenths(&event.key, value)),
                        });
                    }
                }
            }
        }
        tx.commit()?;
        if let Some(list) = &mut self.token_changes {
            list.extend(changes);
        }
        Ok(changed)
    }

    /// Counts queued observations without loading their payloads.
    pub fn pending_count(&self, kind: EventKind) -> Result<u64> {
        let kind = match kind {
            EventKind::Counter => "counter",
            EventKind::Reading => "reading",
        };
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM events WHERE synced=0 AND kind=?1",
            [kind],
            |row| row.get(0),
        )?)
    }

    /// Returns unsynced observations of one kind, oldest first.
    pub fn pending(&self, kind: EventKind, limit: usize) -> Result<Vec<StoredEvent>> {
        let kind = match kind {
            EventKind::Counter => "counter",
            EventKind::Reading => "reading",
        };
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut statement = self.conn.prepare(
            "SELECT id, session_id, at, key, value, resets_at, kind, model, provider, agent_id
             FROM events WHERE synced=0 AND kind=?1 ORDER BY at, id LIMIT ?2",
        )?;
        let rows = statement.query_map(params![kind, limit], |row| {
            let kind: String = row.get(6)?;
            Ok(StoredEvent {
                id: row.get(0)?,
                session_id: row.get(1)?,
                at: row.get(2)?,
                key: row.get(3)?,
                value: row.get(4)?,
                resets_at: row.get(5)?,
                kind: if kind == "reading" {
                    EventKind::Reading
                } else {
                    EventKind::Counter
                },
                model: row.get(7)?,
                provider: row.get(8)?,
                agent_id: row.get(9)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// The most recent unexpired, provider-reported quota windows, never estimated from tokens.
    pub fn usage_limits(&self, now: &str) -> Result<Vec<(String, String, u64, String)>> {
        let mut statement = self.conn.prepare(
            "SELECT provider,key,value,resets_at FROM (
              SELECT provider,key,value,resets_at,ROW_NUMBER() OVER(PARTITION BY provider,key ORDER BY julianday(at) DESC,id DESC) AS position
              FROM events WHERE kind='reading' AND provider IS NOT NULL
                AND key IN ('five_hour_percent','seven_day_percent')
                AND julianday(at)<=julianday(?1) AND julianday(resets_at)>julianday(?1)
            ) WHERE position=1 ORDER BY provider,key",
        )?;
        let rows = statement.query_map([now], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Sums persisted counter tokens by event provider, including uploaded rows.
    pub fn token_totals_between(&self, since: &str, until: &str) -> Result<Vec<(String, u64)>> {
        let mut statement = self.conn.prepare(
            "SELECT provider, SUM(value) FROM events INDEXED BY events_at
             WHERE at>=?3 AND kind='counter' AND provider IS NOT NULL
               AND key IN ('input_tokens','output_tokens','cache_read_tokens','cache_write_tokens')
               AND julianday(at)>=julianday(?1) AND julianday(at)<=julianday(?2)
             GROUP BY provider ORDER BY provider",
        )?;
        let rows = statement.query_map(params![since, until, day_before(since)], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Effective tokens of observations since `since`, counted as the server scores them: input,
    /// output and cache writes in full, a tenth of cache reads. Observations the server refused
    /// are left out.
    pub fn effective_tokens_since(&self, since: &str) -> Result<u64> {
        Ok(self.effective_tenths_since(since)? / 10)
    }

    /// `effective_tokens_since` in tenths of a token, the unit of `TokenChange`.
    pub fn effective_tenths_since(&self, since: &str) -> Result<u64> {
        self.effective_tenths("value", since)
    }

    /// The part of `effective_tokens_since` the server has acknowledged. An observation queued
    /// again still counts what was acknowledged before, so the difference between the two is
    /// only what the server has not seen.
    pub fn acknowledged_tokens_since(&self, since: &str) -> Result<u64> {
        Ok(self.effective_tenths("acked", since)? / 10)
    }

    fn effective_tenths(&self, column: &str, since: &str) -> Result<u64> {
        Ok(self.conn.query_row(
            &format!(
                "SELECT COALESCE(SUM(CASE key WHEN 'cache_read_tokens' THEN {column} ELSE {column} * 10 END), 0)
                 FROM events INDEXED BY events_at
                 WHERE at>=?2
                   AND kind='counter'
                   AND key IN ('input_tokens','output_tokens','cache_read_tokens','cache_write_tokens')
                   AND julianday(at)>=julianday(?1)
                   AND synced IN (0,1)"
            ),
            params![since, day_before(since)],
            |row| row.get(0),
        )?)
    }

    /// Acknowledges only observations whose value and timestamp still match the snapshot.
    pub fn mark_synced(&mut self, snapshots: &[(String, u64, String)]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut statement = tx.prepare_cached(
                "UPDATE events SET synced=1, acked=value WHERE id=?1 AND value=?2 AND at=?3",
            )?;
            for (id, value, at) in snapshots {
                statement.execute(params![id, value, at])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Sets aside observations the server refused, so they leave the upload queue but still count
    /// locally. A later, larger value of the same observation queues it again.
    pub fn reject(&mut self, snapshots: &[(String, u64, String)]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut statement =
                tx.prepare_cached("UPDATE events SET synced=2 WHERE id=?1 AND value=?2 AND at=?3")?;
            for (id, value, at) in snapshots {
                statement.execute(params![id, value, at])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Returns the persisted reader cursor for one source file.
    pub fn cursor(
        &self,
        provider: &str,
        source: usize,
        file_key: &str,
    ) -> Result<Option<FileCursor>> {
        let source = i64::try_from(source).map_err(|_| StoreError::SourceOutOfRange)?;
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT cursor FROM cursors WHERE provider=?1 AND source=?2 AND file_key=?3",
                params![provider, source, file_key],
                |row| row.get(0),
            )
            .optional()?;
        json.map(|json| serde_json::from_str(&json).map_err(Into::into))
            .transpose()
    }

    /// Persists one reader cursor and its JSON state.
    pub fn put_cursor(
        &mut self,
        provider: &str,
        source: usize,
        file_key: &str,
        cursor: &FileCursor,
    ) -> Result<()> {
        let source = i64::try_from(source).map_err(|_| StoreError::SourceOutOfRange)?;
        let json = serde_json::to_string(cursor)?;
        self.conn.execute(
            "INSERT INTO cursors(provider, source, file_key, cursor, updated_at)
             VALUES (?1, ?2, ?3, ?4, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
             ON CONFLICT(provider, source, file_key) DO UPDATE SET cursor=excluded.cursor, updated_at=excluded.updated_at",
            params![provider, source, file_key, json],
        )?;
        Ok(())
    }

    /// Lists all saved source cursors for a provider.
    pub fn cursors(&self, provider: &str) -> Result<Vec<(usize, String, FileCursor)>> {
        let mut statement = self.conn.prepare(
            "SELECT source, file_key, cursor FROM cursors WHERE provider=?1 ORDER BY source, file_key",
        )?;
        let rows = statement.query_map([provider], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        rows.map(|row| {
            let (source, file_key, json) = row?;
            let source = usize::try_from(source).map_err(|_| StoreError::SourceOutOfRange)?;
            Ok((source, file_key, serde_json::from_str(&json)?))
        })
        .collect()
    }

    /// Returns a stored metadata value.
    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row("SELECT value FROM meta WHERE key=?1", [key], |row| {
                row.get(0)
            })
            .optional()
            .map_err(Into::into)
    }

    /// Sets a metadata value.
    pub fn set_meta(&mut self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Deletes a metadata value.
    pub fn delete_meta(&mut self, key: &str) -> Result<()> {
        self.conn.execute("DELETE FROM meta WHERE key=?1", [key])?;
        Ok(())
    }

    /// Adds parser health counts for a provider.
    pub fn record_health(
        &mut self,
        provider: &str,
        records: u64,
        malformed: u64,
        oversized: u64,
        errors: u64,
        at: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO health(provider, records, malformed, oversized, errors, last_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(provider) DO UPDATE SET
               records=health.records+excluded.records, malformed=health.malformed+excluded.malformed,
               oversized=health.oversized+excluded.oversized, errors=health.errors+excluded.errors,
               last_at=CASE WHEN excluded.last_at IS NULL THEN health.last_at
                 WHEN health.last_at IS NULL THEN excluded.last_at
                 ELSE MAX(health.last_at, excluded.last_at) END",
            params![provider, records, malformed, oversized, errors, at],
        )?;
        Ok(())
    }

    /// Returns health counters for all providers.
    pub fn health(&self) -> Result<Vec<HealthRow>> {
        let mut statement = self.conn.prepare(
            "SELECT provider, records, malformed, oversized, errors, last_at FROM health ORDER BY provider",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(HealthRow {
                provider: row.get(0)?,
                records: row.get(1)?,
                malformed: row.get(2)?,
                oversized: row.get(3)?,
                errors: row.get(4)?,
                last_at: row.get(5)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
}

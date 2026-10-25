/// SQL migrations applied in order, one transaction per entry.
pub const MIGRATIONS: &[&str] = &[
    r#"
CREATE TABLE events (
    id TEXT PRIMARY KEY,
    session_id TEXT,
    at TEXT NOT NULL,
    key TEXT NOT NULL,
    value INTEGER NOT NULL,
    resets_at TEXT,
    kind TEXT NOT NULL,
    source TEXT NOT NULL DEFAULT 'live',
    model TEXT,
    provider TEXT,
    synced INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX events_pending ON events(synced, kind);
CREATE TABLE cursors (
    provider TEXT NOT NULL,
    source INTEGER NOT NULL,
    file_key TEXT NOT NULL,
    cursor TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY(provider, source, file_key)
);
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE health (
    provider TEXT PRIMARY KEY,
    records INTEGER NOT NULL DEFAULT 0,
    malformed INTEGER NOT NULL DEFAULT 0,
    oversized INTEGER NOT NULL DEFAULT 0,
    errors INTEGER NOT NULL DEFAULT 0,
    last_at TEXT
);
CREATE TABLE observer_tasks (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    source_key TEXT NOT NULL,
    session_id TEXT NOT NULL,
    started_at TEXT NOT NULL,
    completed_at TEXT,
    status TEXT,
    model_family TEXT NOT NULL DEFAULT 'unknown',
    model_release TEXT NOT NULL DEFAULT 'unknown',
    client_version TEXT NOT NULL,
    adapter_version TEXT NOT NULL,
    schema_version INTEGER NOT NULL,
    synced INTEGER NOT NULL DEFAULT 0,
    last_at TEXT
);
CREATE TABLE observer_requests (
    task_id TEXT NOT NULL,
    response_id TEXT NOT NULL,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    cache_write_tokens INTEGER NOT NULL DEFAULT 0,
    reasoning_tokens INTEGER NOT NULL DEFAULT 0,
    total_tokens INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(task_id, response_id)
);
CREATE TABLE observer_tools (
    task_id TEXT NOT NULL,
    tool_id TEXT NOT NULL,
    category TEXT NOT NULL,
    subagent INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(task_id, tool_id)
);
CREATE TABLE observer_compactions (
    task_id TEXT NOT NULL,
    compaction_id TEXT NOT NULL,
    PRIMARY KEY(task_id, compaction_id)
);
CREATE TABLE observer_active (
    provider TEXT NOT NULL,
    context_key TEXT NOT NULL,
    task_id TEXT NOT NULL,
    PRIMARY KEY(provider, context_key)
);
CREATE INDEX observer_tasks_pending ON observer_tasks(synced, completed_at);
"#,
    r#"
ALTER TABLE events ADD COLUMN agent_id TEXT;
"#,
];

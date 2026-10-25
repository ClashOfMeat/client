//! Private, content-free task telemetry compatible with the original collector.
use std::time::Duration;

use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::{
    ids::observer_task_id,
    signal::{Observation, TaskRef},
    store::{Result, Store},
};

const ADAPTER_VERSION: &str = "0.5.0";
type TaskRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    i64,
);

/// Classifies a tool name without retaining it in telemetry.
pub fn category(name: &str) -> &'static str {
    let name = name.to_ascii_lowercase();
    if ["web", "browser", "fetch", "search_query"]
        .iter()
        .any(|part| name.contains(part))
    {
        "web"
    } else if ["test", "spec", "verify"]
        .iter()
        .any(|part| name.contains(part))
    {
        "test"
    } else if ["write", "edit", "patch", "replace", "create"]
        .iter()
        .any(|part| name.contains(part))
    {
        "write"
    } else if ["read", "find", "grep", "glob", "search", "list", "view"]
        .iter()
        .any(|part| name.contains(part))
    {
        "read"
    } else {
        "other"
    }
}

fn printable(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.:-".contains(&byte))
}

fn task_id(conn: &rusqlite::Connection, provider: &str, task: &TaskRef) -> Result<Option<String>> {
    let id = match task {
        TaskRef::Active(context) => conn
            .query_row(
                "SELECT task_id FROM observer_active WHERE provider=?1 AND context_key=?2",
                params![provider, context],
                |row| row.get(0),
            )
            .optional()?,
        TaskRef::Key(key) => Some(observer_task_id(provider, key)),
    };
    id.map(|id| {
        conn.query_row("SELECT id FROM observer_tasks WHERE id=?1", [&id], |row| {
            row.get(0)
        })
        .optional()
    })
    .transpose()
    .map(Option::flatten)
    .map_err(Into::into)
}

fn touch(conn: &rusqlite::Connection, id: &str, at: &str) -> Result<()> {
    conn.execute(
        "UPDATE observer_tasks SET last_at=MAX(COALESCE(last_at,started_at),?1) WHERE id=?2",
        params![at, id],
    )?;
    Ok(())
}

fn finish(conn: &rusqlite::Connection, id: &str, at: &str, status: &str) -> Result<()> {
    if !matches!(status, "completed" | "interrupted" | "failed") {
        return Ok(());
    }
    conn.execute("UPDATE observer_tasks SET completed_at=MAX(started_at,?1), status=?2, synced=0 WHERE id=?3 AND completed_at IS NULL", params![at,status,id])?;
    Ok(())
}

fn model_release(model: &str, provider: &str) -> (String, String) {
    let lower = model.to_ascii_lowercase();
    if provider == "claude" {
        let found = ["fable", "opus", "sonnet", "haiku"]
            .into_iter()
            .filter_map(|family| lower.find(family).map(|pos| (pos, family)))
            .min_by_key(|(pos, _)| *pos);
        if let Some((pos, family)) = found {
            let suffix = lower[pos + family.len()..].strip_prefix('-').unwrap_or("");
            let release = if suffix.starts_with(|c: char| c.is_ascii_digit()) {
                format!(
                    "{family}-{}",
                    suffix
                        .chars()
                        .take_while(|c| c.is_ascii_lowercase()
                            || c.is_ascii_digit()
                            || matches!(c, '.' | '-'))
                        .collect::<String>()
                )
            } else {
                family.to_owned()
            };
            return (family.to_owned(), release.chars().take(64).collect());
        }
        return ("unknown".into(), "unknown".into());
    }
    let safe: String = lower
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | '-'))
        .take(64)
        .collect();
    let family = if safe.contains("codex") {
        "codex"
    } else if safe.starts_with("gpt-") {
        "gpt"
    } else {
        safe.split(['.', '-'])
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("unknown")
    };
    (
        family.chars().take(32).collect(),
        if safe.is_empty() {
            "unknown".into()
        } else {
            safe
        },
    )
}

/// Reduces a raw provider model to the allowed release label before it enters a signal.
pub fn model_label(model: &str, provider: &str) -> String {
    model_release(model, provider).1
}

/// Preserves a short public client release or falls back to unknown.
pub fn client_label(version: &str) -> &str {
    if !version.is_empty()
        && version.len() <= 64
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.+-".contains(&b))
    {
        version
    } else {
        "unknown"
    }
}

fn set_model(conn: &rusqlite::Connection, id: &str, model: &str, provider: &str) -> Result<()> {
    let (family, release) = model_release(model, provider);
    if family != "unknown" {
        conn.execute(
            "UPDATE observer_tasks SET model_family=?1,model_release=?2,synced=0 WHERE id=?3",
            params![family, release, id],
        )?;
    }
    Ok(())
}

/// Applies ordered observer signals to one installation's private store.
pub fn apply(
    store: &mut Store,
    provider: &str,
    observations: &[Observation],
    client_version: &str,
) -> Result<()> {
    let provider = if matches!(provider, "claude-code" | "claude-cowork") {
        "claude"
    } else {
        provider
    };
    let paired = store.meta("paired_at")?;
    let conn = store.observer_connection();
    for observation in observations {
        match observation {
            Observation::Start {
                context,
                task_key,
                source_key,
                session_id,
                at,
                client_version: version,
                close_previous_completed,
            } => {
                if !printable(context)
                    || !printable(session_id)
                    || paired.as_deref().is_some_and(|p| at.as_str() < p)
                {
                    continue;
                }
                let id = observer_task_id(provider, task_key);
                let previous: Option<String> = conn
                    .query_row(
                        "SELECT task_id FROM observer_active WHERE provider=?1 AND context_key=?2",
                        params![provider, context],
                        |r| r.get(0),
                    )
                    .optional()?;
                if previous.as_deref() == Some(&id) {
                    continue;
                }
                if let Some(previous) = previous {
                    let end = if *close_previous_completed {
                        conn.query_row(
                            "SELECT COALESCE(last_at,started_at) FROM observer_tasks WHERE id=?1",
                            [&previous],
                            |r| r.get::<_, String>(0),
                        )
                        .optional()?
                        .unwrap_or_else(|| at.clone())
                    } else {
                        at.clone()
                    };
                    finish(
                        conn,
                        &previous,
                        &end,
                        if *close_previous_completed {
                            "completed"
                        } else {
                            "interrupted"
                        },
                    )?;
                }
                conn.execute("INSERT OR IGNORE INTO observer_tasks (id,provider,source_key,session_id,started_at,client_version,adapter_version,schema_version) VALUES (?1,?2,?3,?4,?5,?6,?7,1)", params![id,provider,source_key,session_id,at,if version.is_empty(){client_version}else{version},ADAPTER_VERSION])?;
                conn.execute("INSERT INTO observer_active(provider,context_key,task_id) VALUES (?1,?2,?3) ON CONFLICT(provider,context_key) DO UPDATE SET task_id=excluded.task_id",params![provider,context,id])?;
            }
            Observation::Request {
                task,
                response_id,
                usage,
                model,
                at,
            } => {
                if !printable(response_id) || paired.as_deref().is_some_and(|p| at.as_str() < p) {
                    continue;
                }
                if let Some(id) = task_id(conn, provider, task)? {
                    touch(conn, &id, at)?;
                    let total = usage
                        .input
                        .saturating_add(usage.output)
                        .saturating_add(usage.cache_read)
                        .saturating_add(usage.cache_write);
                    conn.execute("INSERT INTO observer_requests(task_id,response_id,input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,reasoning_tokens,total_tokens) VALUES (?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(task_id,response_id) DO UPDATE SET input_tokens=MAX(input_tokens,excluded.input_tokens),output_tokens=MAX(output_tokens,excluded.output_tokens),cache_read_tokens=MAX(cache_read_tokens,excluded.cache_read_tokens),cache_write_tokens=MAX(cache_write_tokens,excluded.cache_write_tokens),reasoning_tokens=MAX(reasoning_tokens,excluded.reasoning_tokens),total_tokens=MAX(total_tokens,excluded.total_tokens)",params![id,response_id,usage.input,usage.output,usage.cache_read,usage.cache_write,usage.reasoning,total])?;
                    if let Some(model) = model {
                        set_model(conn, &id, model, provider)?;
                    }
                }
            }
            Observation::Tool {
                task,
                tool_id,
                category,
                subagent,
                at,
            } => {
                if !printable(tool_id)
                    || !matches!(
                        category.as_str(),
                        "read" | "write" | "test" | "web" | "other"
                    )
                {
                    continue;
                }
                if let Some(id) = task_id(conn, provider, task)? {
                    touch(conn, &id, at)?;
                    conn.execute("INSERT OR IGNORE INTO observer_tools(task_id,tool_id,category,subagent) VALUES (?1,?2,?3,?4)",params![id,tool_id,category,*subagent as i64])?;
                }
            }
            Observation::Compaction {
                task,
                compaction_id,
                at,
            } => {
                if !printable(compaction_id) {
                    continue;
                }
                if let Some(id) = task_id(conn, provider, task)? {
                    conn.execute("INSERT OR IGNORE INTO observer_compactions(task_id,compaction_id) VALUES (?1,?2)",params![id,compaction_id])?;
                    touch(conn, &id, at)?;
                }
            }
            Observation::Model { task, model } => {
                if let Some(id) = task_id(conn, provider, task)? {
                    set_model(conn, &id, model, provider)?;
                }
            }
            Observation::Finish {
                task,
                at,
                status,
                active_guard,
            } => {
                if let Some(id) = task_id(conn, provider, task)? {
                    let started: String = conn.query_row(
                        "SELECT started_at FROM observer_tasks WHERE id=?1",
                        [&id],
                        |r| r.get(0),
                    )?;
                    if !*active_guard || started <= *at {
                        finish(conn, &id, at, status)?;
                    }
                }
            }
            Observation::ClientVersion {
                task,
                client_version,
            } => {
                if let Some(id) = task_id(conn, provider, task)? {
                    conn.execute("UPDATE observer_tasks SET client_version=?1 WHERE id=?2 AND client_version='unknown'",params![client_version,id])?;
                }
            }
        }
    }
    Ok(())
}

/// Completes tasks whose last activity precedes the idle cutoff.
pub fn finalize_idle(
    store: &mut Store,
    provider: &str,
    now: OffsetDateTime,
    idle: Duration,
) -> Result<usize> {
    let provider = if matches!(provider, "claude-code" | "claude-cowork") {
        "claude"
    } else {
        provider
    };
    let cutoff = now - time::Duration::try_from(idle).unwrap_or(time::Duration::MAX);
    let cutoff = cutoff
        .format(&time::macros::format_description!(
            "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
        ))
        .unwrap();
    let conn = store.observer_connection();
    let tasks: Vec<(String, String)> = {
        let mut statement=conn.prepare("SELECT id,COALESCE(last_at,started_at) FROM observer_tasks WHERE provider=?1 AND completed_at IS NULL AND COALESCE(last_at,started_at) < ?2")?;
        {
            let rows = statement
                .query_map(params![provider, cutoff], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<std::result::Result<_, _>>()?;
            rows
        }
    };
    for (id, at) in &tasks {
        finish(conn, id, at, "completed")?;
    }
    Ok(tasks.len())
}

/// Returns completed unsynced payloads in the collector wire shape.
pub fn completed_payloads(store: &mut Store, limit: usize) -> Result<Vec<Value>> {
    let conn = store.observer_connection();
    let tasks: Vec<TaskRow> = {
        let mut s=conn.prepare("SELECT id,provider,source_key,session_id,started_at,completed_at,status,model_family,model_release,client_version,schema_version FROM observer_tasks WHERE completed_at IS NOT NULL AND synced=0 ORDER BY completed_at LIMIT ?1")?;
        {
            let rows = s
                .query_map([i64::try_from(limit).unwrap_or(i64::MAX)], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                        r.get(8)?,
                        r.get(9)?,
                        r.get(10)?,
                    ))
                })?
                .collect::<std::result::Result<_, _>>()?;
            rows
        }
    };
    tasks.into_iter().map(|(id,provider,source_key,session_id,started,completed,status,family,release,version,schema)| {
        let usage:(i64,i64,i64,i64,i64,i64,i64)=conn.query_row("SELECT COALESCE(sum(input_tokens),0),COALESCE(sum(output_tokens),0),COALESCE(sum(cache_read_tokens),0),COALESCE(sum(cache_write_tokens),0),COALESCE(sum(reasoning_tokens),0),COALESCE(sum(total_tokens),0),count(*) FROM observer_requests WHERE task_id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?)))?;
        let count_category=|category:&str| -> Result<i64> {Ok(conn.query_row("SELECT count(*) FROM observer_tools WHERE task_id=?1 AND category=?2",params![id,category],|r|r.get(0))?)};
        let read=count_category("read")?;let write=count_category("write")?;let test=count_category("test")?;let web=count_category("web")?;let other=count_category("other")?;
        let subagents:i64=conn.query_row("SELECT count(*) FROM observer_tools WHERE task_id=?1 AND subagent=1",[&id],|r|r.get(0))?;
        let compactions:i64=conn.query_row("SELECT count(*) FROM observer_compactions WHERE task_id=?1",[&id],|r|r.get(0))?;
        let duration=match (OffsetDateTime::parse(&started,&Rfc3339),OffsetDateTime::parse(&completed,&Rfc3339)) {(Ok(a),Ok(b))=>(b-a).whole_milliseconds().clamp(0,604_800_000) as i64,_=>0};
        Ok(json!({"source":{"provider":provider,"sourceKey":source_key,"clientVersion":version,"adapterVersion":ADAPTER_VERSION,"capabilities":if provider != "cursor" || usage.6 > 0 { vec!["tasks", "exactTokens"] } else { vec!["tasks"] }},"task":{"id":id,"sessionId":session_id,"modelFamily":family,"modelRelease":release,"startedAt":started,"completedAt":completed,"durationMs":duration,"status":status,"inputTokens":usage.0,"outputTokens":usage.1,"cacheReadTokens":usage.2,"cacheWriteTokens":usage.3,"reasoningTokens":usage.4,"totalTokens":usage.5,"modelCalls":usage.6,"userTurns":1,"toolCalls":read+write+test+web+other,"subagentCalls":subagents,"compactions":compactions,"readOperations":read,"writeOperations":write,"testOperations":test,"webOperations":web,"otherOperations":other,"schemaVersion":schema}}))
    }).collect()
}

/// Marks a completed task uploaded.
pub fn acknowledge(store: &mut Store, task_id: &str) -> Result<()> {
    store.observer_connection().execute(
        "UPDATE observer_tasks SET synced=1 WHERE id=?1 AND completed_at IS NOT NULL",
        [task_id],
    )?;
    Ok(())
}
/// Drops a completed task rejected by the server.
pub fn reject(store: &mut Store, task_id: &str) -> Result<()> {
    store.observer_connection().execute(
        "UPDATE observer_tasks SET synced=2 WHERE id=?1 AND completed_at IS NOT NULL",
        [task_id],
    )?;
    Ok(())
}

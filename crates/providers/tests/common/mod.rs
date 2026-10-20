use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use clashofmeat_core::{
    Cx, Env, EventKind, FileCursor, Provider, Record, Signal, SourceFormat, Store, CHUNK,
};
use serde_json::{json, Value};
use time::{macros::datetime, OffsetDateTime};

fn fixture_dir(fixture: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(fixture)
}

fn copy_recursive(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_recursive(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn normalize_numbers(value: &mut Value) {
    match value {
        Value::Number(number) => {
            if let Some(integer) = number.as_u64() {
                *value = Value::from(integer);
            } else if let Some(integer) = number.as_i64().and_then(|n| u64::try_from(n).ok()) {
                *value = Value::from(integer);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(normalize_numbers),
        Value::Object(fields) => fields.values_mut().for_each(normalize_numbers),
        _ => {}
    }
}

pub fn run_fixture(provider: &dyn Provider, fixture: &str, oversized_file: &str) -> Vec<Value> {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    copy_recursive(&fixture_dir(fixture).join("input"), &home);
    let mut oversized = fs::OpenOptions::new()
        .append(true)
        .open(home.join(oversized_file))
        .unwrap();
    oversized
        .write_all(&vec![b'x'; CHUNK as usize + 128])
        .unwrap();
    oversized
        .write_all(b"\n{\"type\":\"fixture-ignored\",\"event\":\"fixture-ignored\"}\n")
        .unwrap();
    drop(oversized);

    let env = Env::for_home(&home);
    let mut store = Store::open_in_memory().unwrap();
    let now: OffsetDateTime = datetime!(2026-10-03 00:00:00 UTC);
    for (source_index, source) in provider.manifest().sources.iter().enumerate() {
        let root = env.resolve(source.root);
        for file in clashofmeat_core::reader::list_files(&root, source.glob, 4, None) {
            let file_key = file
                .path
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let mut cursor = FileCursor::default();
            match source.format {
                SourceFormat::Jsonl => loop {
                    let batch =
                        clashofmeat_core::reader::read_jsonl(&file.path, &mut cursor).unwrap();
                    for line in batch.lines {
                        let record = Record {
                            source: source_index,
                            file_key: file_key.clone(),
                            offset: line.offset,
                            value: line.value,
                        };
                        let mut cx = Cx {
                            file_state: &mut cursor.state,
                            now,
                        };
                        let events: Vec<_> = provider
                            .parse(&record, &mut cx)
                            .into_iter()
                            .filter_map(|signal| match signal {
                                Signal::Event(event) => Some(event),
                                Signal::Turn(_) | Signal::Observer(_) => None,
                            })
                            .collect();
                        store
                            .save_events(
                                provider.manifest().id,
                                &events,
                                Some("2000-01-01T00:00:00.000Z"),
                            )
                            .unwrap();
                    }
                    if !batch.more {
                        break;
                    }
                },
                SourceFormat::Json => {
                    if let Some(value) =
                        clashofmeat_core::reader::read_json(&file.path, &mut cursor).unwrap()
                    {
                        let record = Record {
                            source: source_index,
                            file_key: file_key.clone(),
                            offset: 0,
                            value,
                        };
                        let mut cx = Cx {
                            file_state: &mut cursor.state,
                            now,
                        };
                        let events: Vec<_> = provider
                            .parse(&record, &mut cx)
                            .into_iter()
                            .filter_map(|signal| match signal {
                                Signal::Event(event) => Some(event),
                                Signal::Turn(_) | Signal::Observer(_) => None,
                            })
                            .collect();
                        store
                            .save_events(
                                provider.manifest().id,
                                &events,
                                Some("2000-01-01T00:00:00.000Z"),
                            )
                            .unwrap();
                    }
                }
                other => panic!("unsupported fixture source format: {other:?}"),
            }
        }
    }
    let mut rows = Vec::new();
    for kind in [EventKind::Counter, EventKind::Reading] {
        for row in store.pending(kind, usize::MAX).unwrap() {
            let mut value = json!({
                "id": row.id,
                "session_id": row.session_id,
                "at": row.at,
                "key": row.key,
                "value": row.value,
                "kind": row.kind,
                "provider": row.provider,
                "model": row.model,
            });
            normalize_numbers(&mut value);
            rows.push(value);
        }
    }
    rows.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
    rows
}

pub fn expected(fixture: &str) -> Vec<Value> {
    let path = fixture_dir(fixture).join("expected.json");
    let mut rows: Vec<Value> = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    rows.iter_mut().for_each(normalize_numbers);
    rows
}

#[allow(dead_code)]
pub fn run_fixture_tasks(
    provider: &dyn Provider,
    fixture: &str,
    oversized_file: &str,
    idle: std::time::Duration,
) -> Vec<Value> {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    copy_recursive(&fixture_dir(fixture).join("input"), &home);
    let mut oversized = fs::OpenOptions::new()
        .append(true)
        .open(home.join(oversized_file))
        .unwrap();
    oversized
        .write_all(&vec![b'x'; CHUNK as usize + 128])
        .unwrap();
    oversized
        .write_all(b"\n{\"type\":\"fixture-ignored\",\"event\":\"fixture-ignored\"}\n")
        .unwrap();
    drop(oversized);
    let env = Env::for_home(&home);
    let mut store = Store::open_in_memory().unwrap();
    store
        .set_meta("paired_at", "2000-01-01T00:00:00.000Z")
        .unwrap();
    let now: OffsetDateTime = datetime!(2026-10-03 00:00:00 UTC);
    for (source_index, source) in provider.manifest().sources.iter().enumerate() {
        let root = env.resolve(source.root);
        for file in clashofmeat_core::reader::list_files(&root, source.glob, 4, None) {
            let file_key = file
                .path
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let mut cursor = FileCursor::default();
            let mut handle = |value: Value, offset: u64, cursor: &mut FileCursor| {
                let record = Record {
                    source: source_index,
                    file_key: file_key.clone(),
                    offset,
                    value,
                };
                let mut cx = Cx {
                    file_state: &mut cursor.state,
                    now,
                };
                let observations: Vec<_> = provider
                    .parse(&record, &mut cx)
                    .into_iter()
                    .filter_map(|signal| match signal {
                        Signal::Observer(o) => Some(o),
                        _ => None,
                    })
                    .collect();
                clashofmeat_core::observer::apply(
                    &mut store,
                    provider.manifest().id,
                    &observations,
                    "unknown",
                )
                .unwrap();
            };
            match source.format {
                SourceFormat::Jsonl => loop {
                    let batch =
                        clashofmeat_core::reader::read_jsonl(&file.path, &mut cursor).unwrap();
                    for line in batch.lines {
                        handle(line.value, line.offset, &mut cursor);
                    }
                    if !batch.more {
                        break;
                    }
                },
                SourceFormat::Json => {
                    if let Some(value) =
                        clashofmeat_core::reader::read_json(&file.path, &mut cursor).unwrap()
                    {
                        handle(value, 0, &mut cursor);
                    }
                }
                other => panic!("unsupported fixture source format: {other:?}"),
            }
        }
    }
    clashofmeat_core::observer::finalize_idle(&mut store, provider.manifest().id, now, idle)
        .unwrap();
    let mut rows = clashofmeat_core::observer::completed_payloads(&mut store, 1000).unwrap();
    rows.sort_by(|a, b| a["task"]["id"].as_str().cmp(&b["task"]["id"].as_str()));
    rows
}

#[allow(dead_code)]
pub fn expected_tasks(fixture: &str) -> Vec<Value> {
    serde_json::from_slice(&fs::read(fixture_dir(fixture).join("expected-tasks.json")).unwrap())
        .unwrap()
}

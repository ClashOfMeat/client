//! Incremental local file reading. Offsets always point to the next complete line.

use std::fs::{self, File, Metadata};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use walkdir::WalkDir;

pub const CHUNK: u64 = 4 * 1024 * 1024;
const MAX_JSON: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FileCursor {
    pub inode: u64,
    pub offset: u64,
    pub len: u64,
    pub mtime_ms: i64,
    pub skip_line: bool,
    pub state: Map<String, Value>,
}

#[derive(Debug)]
pub struct Line {
    pub offset: u64,
    pub value: Value,
}

#[derive(Debug)]
pub struct ReadBatch {
    pub lines: Vec<Line>,
    pub malformed: u32,
    pub oversized: u32,
    pub more: bool,
}

#[derive(Debug)]
pub struct FileEntry {
    pub path: PathBuf,
    pub len: u64,
    pub mtime_ms: i64,
}

#[cfg(unix)]
fn inode(metadata: &Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    metadata.ino()
}

#[cfg(not(unix))]
fn inode(_metadata: &Metadata) -> u64 {
    0
}

fn mtime_ms(metadata: &Metadata) -> io::Result<i64> {
    let modified = metadata.modified()?;
    Ok(match modified.duration_since(UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_millis()).unwrap_or(i64::MAX),
        Err(error) => -i64::try_from(error.duration().as_millis()).unwrap_or(i64::MAX),
    })
}

/// Read at most one chunk, retaining an incomplete trailing line for the next pass.
pub fn read_jsonl(path: &Path, cursor: &mut FileCursor) -> io::Result<ReadBatch> {
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    let len = metadata.len();
    let current_inode = inode(&metadata);
    let current_mtime = mtime_ms(&metadata)?;
    if cursor.inode != 0 && cursor.inode != current_inode || len < cursor.offset {
        cursor.offset = 0;
        cursor.skip_line = false;
    }
    cursor.inode = current_inode;
    cursor.len = len;
    cursor.mtime_ms = current_mtime;

    let mut batch = ReadBatch {
        lines: Vec::new(),
        malformed: 0,
        oversized: 0,
        more: false,
    };
    if len == cursor.offset {
        return Ok(batch);
    }

    let start = cursor.offset;
    file.seek(SeekFrom::Start(start))?;
    let size = usize::try_from((len - start).min(CHUNK)).expect("chunk fits in usize");
    let mut bytes = Vec::with_capacity(size);
    file.take(size as u64).read_to_end(&mut bytes)?;
    let read = bytes.len();

    if let Some(last_newline) = bytes.iter().rposition(|byte| *byte == b'\n') {
        let complete = last_newline + 1;
        let mut line_start = 0;
        for (index, byte) in bytes[..complete].iter().enumerate() {
            if *byte != b'\n' {
                continue;
            }
            if cursor.skip_line {
                cursor.skip_line = false;
            } else if index > line_start {
                match serde_json::from_slice(&bytes[line_start..index]) {
                    Ok(value) => batch.lines.push(Line {
                        offset: start + line_start as u64,
                        value,
                    }),
                    Err(_) => batch.malformed += 1,
                }
            }
            line_start = index + 1;
        }
        cursor.offset = start + complete as u64;
    } else if read as u64 == CHUNK {
        if !cursor.skip_line {
            batch.oversized = 1;
        }
        cursor.skip_line = true;
        cursor.offset = start + read as u64;
    }
    batch.more = cursor.offset < len;
    Ok(batch)
}

/// Read a whole JSON document only when its length or modification time changes.
pub fn read_json(path: &Path, cursor: &mut FileCursor) -> io::Result<Option<Value>> {
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    let len = metadata.len();
    let modified = mtime_ms(&metadata)?;
    if len == cursor.len && modified == cursor.mtime_ms {
        return Ok(None);
    }
    cursor.len = len;
    cursor.mtime_ms = modified;
    cursor.inode = inode(&metadata);
    if len > MAX_JSON {
        return Ok(None);
    }

    let mut bytes = Vec::with_capacity(len as usize);
    file.by_ref().take(MAX_JSON + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_JSON {
        return Ok(None);
    }
    Ok(serde_json::from_slice(&bytes).ok())
}

/// List matching regular files, with root-relative globs and directory depth.
pub fn list_files(
    root: &Path,
    glob: &str,
    max_depth: usize,
    since_ms: Option<i64>,
) -> Vec<FileEntry> {
    let mut files = Vec::new();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .max_depth(max_depth.saturating_add(1))
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let Some(relative) = entry.path().strip_prefix(root).ok() else {
            continue;
        };
        if !glob_matches(glob, relative) {
            continue;
        }
        let Ok(metadata) = fs::metadata(entry.path()) else {
            continue;
        };
        let Ok(modified) = mtime_ms(&metadata) else {
            continue;
        };
        if since_ms.is_some_and(|since| modified < since) {
            continue;
        }
        files.push(FileEntry {
            path: entry.path().to_path_buf(),
            len: metadata.len(),
            mtime_ms: modified,
        });
    }
    files.sort_by(|a, b| {
        b.mtime_ms
            .cmp(&a.mtime_ms)
            .then_with(|| a.path.cmp(&b.path))
    });
    files
}

fn glob_matches(glob: &str, path: &Path) -> bool {
    let pattern: Vec<_> = glob.split('/').collect();
    let relative = path.to_string_lossy();
    let segments: Vec<_> = relative.split('/').collect();
    match_segments(&pattern, &segments)
}

fn match_segments(pattern: &[&str], path: &[&str]) -> bool {
    if pattern.is_empty() {
        return path.is_empty();
    }
    if pattern[0] == "**" {
        return (0..=path.len()).any(|skip| match_segments(&pattern[1..], &path[skip..]));
    }
    !path.is_empty()
        && match_segment(pattern[0].as_bytes(), path[0].as_bytes())
        && match_segments(&pattern[1..], &path[1..])
}

fn match_segment(pattern: &[u8], name: &[u8]) -> bool {
    if pattern.is_empty() {
        return name.is_empty();
    }
    if pattern[0] == b'*' {
        return (0..=name.len()).any(|skip| match_segment(&pattern[1..], &name[skip..]));
    }
    !name.is_empty() && pattern[0] == name[0] && match_segment(&pattern[1..], &name[1..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File, OpenOptions};
    use std::io::Write;
    use std::time::{Duration, SystemTime};
    use tempfile::tempdir;

    #[test]
    fn complete_and_partial_lines() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("log.jsonl");
        fs::write(&path, b"{\"a\":1}\n{\"a\":2}").unwrap();
        let mut cursor = FileCursor::default();
        let batch = read_jsonl(&path, &mut cursor).unwrap();
        assert_eq!(batch.lines.len(), 1);
        assert_eq!(batch.lines[0].offset, 0);
        assert_eq!(batch.lines[0].value, serde_json::json!({"a": 1}));
        assert_eq!(cursor.offset, 8);
        assert!(batch.more);
        assert!(read_jsonl(&path, &mut cursor).unwrap().more);
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"\n")
            .unwrap();
        let batch = read_jsonl(&path, &mut cursor).unwrap();
        assert_eq!(batch.lines[0].offset, 8);
        assert_eq!(batch.lines[0].value, serde_json::json!({"a": 2}));
        assert!(!batch.more);
    }

    #[test]
    fn truncation_restarts_and_keeps_state() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("log.jsonl");
        fs::write(&path, b"{\"long\":123}\n").unwrap();
        let mut cursor = FileCursor::default();
        cursor.state.insert("seen".into(), Value::Bool(true));
        read_jsonl(&path, &mut cursor).unwrap();
        fs::write(&path, b"1\n").unwrap();
        let batch = read_jsonl(&path, &mut cursor).unwrap();
        assert_eq!(batch.lines[0].offset, 0);
        assert_eq!(batch.lines[0].value, Value::from(1));
        assert_eq!(cursor.state["seen"], true);
        assert!(!cursor.skip_line);
    }

    #[cfg(unix)]
    #[test]
    fn replacement_inode_restarts_even_if_new_file_is_longer() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("log.jsonl");
        fs::write(&path, b"1\n").unwrap();
        let mut cursor = FileCursor::default();
        read_jsonl(&path, &mut cursor).unwrap();
        let old_inode = cursor.inode;
        let replacement = dir.path().join("replacement");
        fs::write(&replacement, b"{\"new\":true}\n").unwrap();
        fs::rename(&replacement, &path).unwrap();
        let batch = read_jsonl(&path, &mut cursor).unwrap();
        assert_ne!(cursor.inode, old_inode);
        assert_eq!(batch.lines[0].offset, 0);
        assert_eq!(batch.lines[0].value, serde_json::json!({"new": true}));
    }

    #[test]
    fn malformed_and_empty_lines_are_skipped() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("log.jsonl");
        fs::write(&path, b"\nwrong\nnull\n").unwrap();
        let batch = read_jsonl(&path, &mut FileCursor::default()).unwrap();
        assert_eq!(batch.malformed, 1);
        assert_eq!(batch.lines.len(), 1);
        assert_eq!(batch.lines[0].offset, 7);
        assert_eq!(batch.lines[0].value, Value::Null);
    }

    #[test]
    fn oversized_line_is_dropped_and_following_line_is_read() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("log.jsonl");
        let mut file = File::create(&path).unwrap();
        file.write_all(&vec![b'x'; CHUNK as usize + 3]).unwrap();
        file.write_all(b"\n{\"ok\":true}\n").unwrap();
        drop(file);
        let mut cursor = FileCursor::default();
        let first = read_jsonl(&path, &mut cursor).unwrap();
        assert_eq!(first.oversized, 1);
        assert!(first.more);
        assert!(cursor.skip_line);
        let second = read_jsonl(&path, &mut cursor).unwrap();
        assert_eq!(second.oversized, 0);
        assert_eq!(second.lines.len(), 1);
        assert_eq!(second.lines[0].offset, CHUNK + 4);
        assert_eq!(second.lines[0].value, serde_json::json!({"ok": true}));
        assert!(!second.more);
        assert!(!cursor.skip_line);
    }

    #[test]
    fn json_change_detection_and_malformed_document() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("doc.json");
        fs::write(&path, b"{\"ok\":true}").unwrap();
        let mut cursor = FileCursor::default();
        assert_eq!(
            read_json(&path, &mut cursor).unwrap(),
            Some(serde_json::json!({"ok": true}))
        );
        assert_eq!(read_json(&path, &mut cursor).unwrap(), None);
        fs::write(&path, b"bad").unwrap();
        assert_eq!(read_json(&path, &mut cursor).unwrap(), None);
        assert_eq!(cursor.len, 3);
        assert_eq!(read_json(&path, &mut cursor).unwrap(), None);
        fs::write(&path, b"true").unwrap();
        assert_eq!(
            read_json(&path, &mut cursor).unwrap(),
            Some(Value::Bool(true))
        );
    }

    #[test]
    fn json_oversized_document_is_skipped_until_changed() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("doc.json");
        let file = File::create(&path).unwrap();
        file.set_len(MAX_JSON + 1).unwrap();
        let mut cursor = FileCursor::default();
        assert_eq!(read_json(&path, &mut cursor).unwrap(), None);
        assert_eq!(cursor.len, MAX_JSON + 1);
        assert_eq!(read_json(&path, &mut cursor).unwrap(), None);
    }

    #[test]
    fn file_globs_depth_since_and_order() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("agent/chats/deep")).unwrap();
        fs::create_dir_all(root.join("other/chats")).unwrap();
        let old = root.join("root.jsonl");
        let new = root.join("agent/chats/session-new.json");
        let middle = root.join("other/chats/session-old.json");
        let deep = root.join("agent/chats/deep/log.jsonl");
        for path in [&old, &new, &middle, &deep] {
            fs::write(path, b"x").unwrap();
        }
        for (path, seconds) in [(&old, 10), (&middle, 20), (&new, 30), (&deep, 40)] {
            File::open(path)
                .unwrap()
                .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
                .unwrap();
        }
        let jsonl = list_files(root, "**/*.jsonl", 2, None);
        assert_eq!(jsonl.len(), 1);
        assert_eq!(jsonl[0].path, old);
        let sessions = list_files(root, "*/chats/session-*.json", 2, None);
        assert_eq!(
            sessions.iter().map(|entry| &entry.path).collect::<Vec<_>>(),
            vec![&new, &middle]
        );
        assert_eq!(list_files(root, "*/chats/session-*.json", 1, None).len(), 0);
        let recent = list_files(root, "*/chats/session-*.json", 2, Some(25_000));
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].path, new);
        assert_eq!(list_files(root, "**/*.jsonl", 3, None)[0].path, deep);
    }
}

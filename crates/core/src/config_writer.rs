//! The single path for changing another tool's configuration.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("invalid JSON in {path}", path = .path.display())]
    InvalidJson { path: PathBuf },
}

#[derive(Debug, PartialEq, Eq)]
pub enum WriteOutcome {
    Unchanged,
    Written { backup: PathBuf },
}

#[derive(Debug, PartialEq, Eq)]
pub enum RestoreOutcome {
    NothingToRestore,
    Restored,
}

#[derive(Debug, Clone)]
pub struct ConfigWriter {
    backup_dir: PathBuf,
}

impl ConfigWriter {
    pub fn new(backup_dir: PathBuf) -> Self {
        Self { backup_dir }
    }

    pub fn write(&self, path: &Path, contents: &[u8]) -> Result<WriteOutcome, ConfigError> {
        let target = resolve_target(path)?;
        let previous = match fs::read(&target) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        if previous.as_deref() == Some(contents) {
            return Ok(WriteOutcome::Unchanged);
        }

        let mode = match previous {
            Some(_) => file_mode(&target)?,
            None => 0o644,
        };
        let backup = self.backup_path(&target);
        self.ensure_backup(&target, &backup)?;
        atomic_replace(&target, contents, mode)?;
        Ok(WriteOutcome::Written { backup })
    }

    pub fn restore(&self, path: &Path) -> Result<RestoreOutcome, ConfigError> {
        let target = resolve_target(path)?;
        let backup = self.backup_path(&target);
        if !backup.exists() {
            return Ok(RestoreOutcome::NothingToRestore);
        }
        verify_backup_path(&backup, &target)?;
        let original = backup.join("original");
        if original.exists() {
            let mode = file_mode(&original)?;
            atomic_replace(&target, &fs::read(&original)?, mode)?;
        } else if backup.join("absent").exists() {
            match fs::remove_file(&target) {
                Ok(()) => sync_parent(&target)?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        } else {
            return Err(
                io::Error::new(io::ErrorKind::InvalidData, "incomplete config backup").into(),
            );
        }
        fs::remove_dir_all(backup)?;
        Ok(RestoreOutcome::Restored)
    }

    pub fn update_json(
        &self,
        path: &Path,
        edit: impl FnOnce(&mut Value) -> bool,
    ) -> Result<WriteOutcome, ConfigError> {
        let mut value = match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| ConfigError::InvalidJson {
                path: path.to_path_buf(),
            })?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => serde_json::json!({}),
            Err(error) => return Err(error.into()),
        };
        if !edit(&mut value) {
            return Ok(WriteOutcome::Unchanged);
        }
        let mut serialized = serde_json::to_vec_pretty(&value).map_err(io::Error::other)?;
        serialized.push(b'\n');
        self.write(path, &serialized)
    }

    fn backup_path(&self, target: &Path) -> PathBuf {
        #[cfg(unix)]
        let bytes = {
            use std::os::unix::ffi::OsStrExt;
            target.as_os_str().as_bytes()
        };
        #[cfg(not(unix))]
        let bytes = target.to_string_lossy();
        #[cfg(unix)]
        let digest = hex::encode(Sha256::digest(bytes));
        #[cfg(not(unix))]
        let digest = hex::encode(Sha256::digest(bytes.as_bytes()));
        self.backup_dir.join(&digest[..16])
    }

    fn ensure_backup(&self, target: &Path, backup: &Path) -> Result<(), ConfigError> {
        if backup.exists() {
            verify_backup_path(backup, target)?;
            if backup.join("original").exists() || backup.join("absent").exists() {
                return Ok(());
            }
            return Err(
                io::Error::new(io::ErrorKind::InvalidData, "incomplete config backup").into(),
            );
        }
        create_private_dirs(&self.backup_dir)?;
        fs::create_dir(backup)?;
        set_mode(backup, 0o700)?;
        let mut path_file = open_new(&backup.join("path.txt"))?;
        path_file.write_all(target.to_string_lossy().as_bytes())?;
        path_file.sync_all()?;
        match fs::read(target) {
            Ok(bytes) => {
                let mode = file_mode(target)?;
                let original = backup.join("original");
                let mut file = open_new(&original)?;
                set_mode(&original, mode)?;
                file.write_all(&bytes)?;
                file.sync_all()?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let file = open_new(&backup.join("absent"))?;
                file.sync_all()?;
            }
            Err(error) => return Err(error.into()),
        }
        sync_dir(backup)?;
        sync_dir(&self.backup_dir)?;
        Ok(())
    }
}

fn verify_backup_path(backup: &Path, target: &Path) -> io::Result<()> {
    if fs::read(backup.join("path.txt"))? == target.to_string_lossy().as_bytes() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "config backup path mismatch",
        ))
    }
}

fn resolve_target(path: &Path) -> io::Result<PathBuf> {
    let mut candidate = path.to_path_buf();
    for _ in 0..40 {
        match fs::canonicalize(&candidate) {
            Ok(target) => return Ok(target),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        if fs::symlink_metadata(&candidate).is_ok_and(|meta| meta.file_type().is_symlink()) {
            let link = fs::read_link(&candidate)?;
            candidate = if link.is_absolute() {
                link
            } else {
                candidate
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or(Path::new("."))
                    .join(link)
            };
            continue;
        }
        let parent = candidate
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        create_private_dirs(parent)?;
        let name = candidate.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "config path has no file name")
        })?;
        return Ok(fs::canonicalize(parent)?.join(name));
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "too many config symlinks",
    ))
}

fn create_private_dirs(path: &Path) -> io::Result<()> {
    if path.as_os_str().is_empty() {
        return Ok(());
    }
    if path.is_dir() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        if parent != path {
            create_private_dirs(parent)?;
        }
    }
    match fs::create_dir(path) {
        Ok(()) => set_mode(path, 0o700),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists && path.is_dir() => Ok(()),
        Err(error) => Err(error),
    }
}

fn atomic_replace(target: &Path, contents: &[u8], mode: u32) -> io::Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "config path has no parent"))?;
    let (temporary, mut file) = loop {
        let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(".clashofmeat-{}-{id}.tmp", std::process::id()));
        match open_new(&path) {
            Ok(file) => break (path, file),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };
    let result = (|| {
        set_mode(&temporary, mode)?;
        file.write_all(contents)?;
        file.sync_all()?;
        fs::rename(&temporary, target)?;
        sync_dir(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn sync_parent(path: &Path) -> io::Result<()> {
    sync_dir(
        path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "config path has no parent")
        })?,
    )
}

fn open_new(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

#[cfg(unix)]
fn sync_dir(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_dir(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn file_mode(path: &Path) -> io::Result<u32> {
    Ok(fs::metadata(path)?.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
fn file_mode(_path: &Path) -> io::Result<u32> {
    Ok(0o644)
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn new_file_has_absent_backup_and_restore_deletes_it() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("new/config.json");
        let writer = ConfigWriter::new(dir.path().join("backup"));
        let WriteOutcome::Written { backup } = writer.write(&path, b"new").unwrap() else {
            panic!("expected write")
        };
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert!(backup.join("absent").exists());
        assert!(!backup.join("original").exists());
        #[cfg(unix)]
        {
            assert_eq!(file_mode(&path).unwrap() & 0o777, 0o644);
            assert_eq!(file_mode(path.parent().unwrap()).unwrap() & 0o777, 0o700);
        }
        assert_eq!(writer.restore(&path).unwrap(), RestoreOutcome::Restored);
        assert!(!path.exists());
        assert!(!backup.exists());
    }

    #[test]
    fn existing_file_keeps_mode_and_first_backup() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, b"first").unwrap();
        set_mode(&path, 0o600).unwrap();
        let writer = ConfigWriter::new(dir.path().join("backup"));
        let WriteOutcome::Written { backup } = writer.write(&path, b"second").unwrap() else {
            panic!("expected write")
        };
        writer.write(&path, b"third").unwrap();
        assert_eq!(fs::read(backup.join("original")).unwrap(), b"first");
        assert_eq!(fs::read(&path).unwrap(), b"third");
        #[cfg(unix)]
        assert_eq!(file_mode(&path).unwrap() & 0o777, 0o600);
        assert_eq!(writer.restore(&path).unwrap(), RestoreOutcome::Restored);
        assert_eq!(fs::read(&path).unwrap(), b"first");
        #[cfg(unix)]
        assert_eq!(file_mode(&path).unwrap() & 0o777, 0o600);
        assert!(!backup.exists());
    }

    #[test]
    fn unchanged_contents_do_not_make_backup() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, b"same").unwrap();
        let backup = dir.path().join("backup");
        let writer = ConfigWriter::new(backup.clone());
        assert_eq!(
            writer.write(&path, b"same").unwrap(),
            WriteOutcome::Unchanged
        );
        assert!(!backup.exists());
        assert_eq!(
            writer.restore(&path).unwrap(),
            RestoreOutcome::NothingToRestore
        );
    }

    #[cfg(unix)]
    #[test]
    fn write_through_symlink_keeps_link_and_restores_target() {
        use std::os::unix::fs::symlink;
        let dir = tempdir().unwrap();
        let target = dir.path().join("target.json");
        let link = dir.path().join("link.json");
        fs::write(&target, b"old").unwrap();
        symlink(&target, &link).unwrap();
        let writer = ConfigWriter::new(dir.path().join("backup"));
        writer.write(&link, b"new").unwrap();
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert_eq!(writer.restore(&link).unwrap(), RestoreOutcome::Restored);
        assert_eq!(fs::read(&target).unwrap(), b"old");
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn invalid_json_is_untouched() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, b"{oops").unwrap();
        let writer = ConfigWriter::new(dir.path().join("backup"));
        assert!(matches!(
            writer.update_json(&path, |_| true),
            Err(ConfigError::InvalidJson { .. })
        ));
        assert_eq!(fs::read(&path).unwrap(), b"{oops");
        assert!(!dir.path().join("backup").exists());
    }

    #[cfg(unix)]
    #[test]
    fn write_through_dangling_symlink_keeps_link() {
        use std::os::unix::fs::symlink;
        let dir = tempdir().unwrap();
        let target = dir.path().join("target.json");
        let link = dir.path().join("link.json");
        symlink(&target, &link).unwrap();
        let writer = ConfigWriter::new(dir.path().join("backup"));
        writer.write(&link, b"new").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        writer.restore(&link).unwrap();
        assert!(!target.exists());
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn json_edit_returning_false_writes_nothing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, b"{\"version\":1}").unwrap();
        let writer = ConfigWriter::new(dir.path().join("backup"));
        assert_eq!(
            writer.update_json(&path, |_| false).unwrap(),
            WriteOutcome::Unchanged
        );
        assert_eq!(fs::read(&path).unwrap(), b"{\"version\":1}");
        assert!(!dir.path().join("backup").exists());
    }

    #[test]
    fn json_edit_writes_two_space_indentation_and_newline() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        let writer = ConfigWriter::new(dir.path().join("backup"));
        writer
            .update_json(&path, |value| {
                value["version"] = Value::from(1);
                true
            })
            .unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"{\n  \"version\": 1\n}\n");
    }
}

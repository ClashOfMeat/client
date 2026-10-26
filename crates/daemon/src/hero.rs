//! Cached hero data exposed to the desktop client.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use serde_json::{Map, Value};

/// Keeps only the heartbeat fields used by the hero view.
pub fn hero_view(heartbeat: &Value) -> Value {
    let mut view = Map::new();
    for key in [
        "character",
        "animation",
        "levelUp",
        "level_up",
        "progression",
        "player",
        "visit",
        "outgoingVisit",
    ] {
        if let Some(value) = heartbeat.get(key) {
            view.insert(key.to_string(), value.clone());
        }
    }
    Value::Object(view)
}

/// Atomically saves a private hero view under the daemon home.
pub fn save_cache(home: &Path, view: &Value) -> Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = home.join("hero");
    fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    }
    let temp = dir.join(format!(
        ".cache.{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let write = (|| -> Result<()> {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        serde_json::to_writer(&mut file, view)?;
        file.flush()?;
        file.sync_all()?;
        fs::rename(&temp, dir.join("cache.json"))?;
        Ok(())
    })();
    if write.is_err() {
        let _ = fs::remove_file(&temp);
    }
    write
}

/// Loads the saved hero view, if one exists.
pub fn load_cache(home: &Path) -> Result<Option<Value>> {
    match fs::read(home.join("hero/cache.json")) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::{hero_view, load_cache, save_cache};
    use serde_json::json;

    #[test]
    fn outgoing_visit_survives_hero_cache_for_restart_recovery() {
        let heartbeat = json!({
            "character": {"id": "hero"},
            "outgoingVisit": {
                "id": "123e4567-e89b-12d3-a456-426614174000",
                "status": "delivered",
                "expiresAt": "2026-10-26T12:02:00Z"
            },
            "unrelated": "discard"
        });
        let view = hero_view(&heartbeat);
        assert_eq!(view["outgoingVisit"], heartbeat["outgoingVisit"]);
        assert!(view.get("unrelated").is_none());
        let home = tempfile::tempdir().unwrap();
        save_cache(home.path(), &view).unwrap();
        assert_eq!(
            load_cache(home.path()).unwrap().unwrap()["outgoingVisit"],
            heartbeat["outgoingVisit"]
        );
    }
}

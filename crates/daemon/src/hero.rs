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

/// Saves a private hero view under the daemon home. The hero's art (over 1 MB of frames) goes
/// to its own file, written only when its revision changes; the small rest is rewritten each
/// time. A heartbeat every 30 s would otherwise rewrite the same megabyte all day.
pub fn save_cache(home: &Path, view: &Value) -> Result<()> {
    let dir = home.join("hero");
    fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    }
    let mut light = view.clone();
    let art = light
        .get_mut("character")
        .and_then(Value::as_object_mut)
        .and_then(|character| {
            let revision = character.get("artRevision")?.as_str()?.to_owned();
            let png = character.remove("png")?;
            let animation = character.remove("animation").unwrap_or(Value::Null);
            Some((revision, png, animation))
        });
    match art {
        Some((revision, png, animation)) => {
            if saved_art_revision(&dir).as_deref() != Some(&revision) {
                let art = serde_json::json!({"artRevision": revision, "png": png, "animation": animation});
                // The marker goes first: if the art write fails, it names art that is not there
                // and the next save writes again.
                let _ = fs::remove_file(dir.join("art.revision"));
                write_private(&dir, "art.json", &art)?;
                fs::write(dir.join("art.revision"), &revision)?;
            }
            write_private(&dir, "cache.json", &light)
        }
        // Without a revision to tell art apart, keep everything in one file.
        None => write_private(&dir, "cache.json", view),
    }
}

/// The revision of the saved art file, kept beside it so nobody has to parse the frames to know.
fn saved_art_revision(dir: &Path) -> Option<String> {
    if !dir.join("art.json").is_file() {
        return None;
    }
    fs::read_to_string(dir.join("art.revision")).ok()
}

fn write_private(dir: &Path, name: &str, value: &Value) -> Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let temp = dir.join(format!(
        ".{name}.{}.{}.tmp",
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
        serde_json::to_writer(&mut file, value)?;
        file.flush()?;
        file.sync_all()?;
        fs::rename(&temp, dir.join(name))?;
        Ok(())
    })();
    if write.is_err() {
        let _ = fs::remove_file(&temp);
    }
    write
}

/// Loads the saved hero view, if one exists, with its art put back in.
pub fn load_cache(home: &Path) -> Result<Option<Value>> {
    let dir = home.join("hero");
    let mut view: Value = match fs::read(dir.join("cache.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if let Some(character) = view.get_mut("character").and_then(Value::as_object_mut) {
        if !character.contains_key("png") {
            // A missing or older art file leaves the view without art; the next heartbeat
            // then asks the server for it again.
            let art = fs::read(dir.join("art.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .filter(|art| art.get("artRevision") == character.get("artRevision"));
            if let Some(Value::Object(mut art)) = art {
                for key in ["png", "animation"] {
                    if let Some(value) = art.remove(key).filter(|value| !value.is_null()) {
                        character.insert(key.into(), value);
                    }
                }
            }
        }
    }
    Ok(Some(view))
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

    #[test]
    fn art_is_written_once_per_revision_and_loaded_back() {
        let home = tempfile::tempdir().unwrap();
        let view = |xp: u64, revision: &str, png: &str| json!({"character": {"id": "hero", "xp": xp, "artRevision": revision, "png": png, "animation": {"frames": [png]}}});
        save_cache(home.path(), &view(1, "a", "one")).unwrap();
        let art = home.path().join("hero/art.json");
        let written = std::fs::metadata(&art).unwrap().modified().unwrap();
        let light = std::fs::read_to_string(home.path().join("hero/cache.json")).unwrap();
        assert!(!light.contains("one"));
        // Same art, new numbers: only the small file changes.
        save_cache(home.path(), &view(2, "a", "one")).unwrap();
        assert_eq!(
            std::fs::metadata(&art).unwrap().modified().unwrap(),
            written
        );
        assert_eq!(
            load_cache(home.path()).unwrap().unwrap(),
            view(2, "a", "one")
        );
        save_cache(home.path(), &view(3, "b", "two")).unwrap();
        assert_eq!(
            load_cache(home.path()).unwrap().unwrap(),
            view(3, "b", "two")
        );
        // A cache from before the split holds its art inline and loads as it is.
        std::fs::write(
            home.path().join("hero/cache.json"),
            view(4, "old", "inline").to_string(),
        )
        .unwrap();
        assert_eq!(
            load_cache(home.path()).unwrap().unwrap(),
            view(4, "old", "inline")
        );
    }

    #[test]
    fn notification_text_is_not_written_to_the_hero_cache() {
        let heartbeat = json!({
            "character": {"id": "hero"},
            "notifications": {"attention": {"unread": 1}, "items": [{"message": "private message"}]}
        });
        let view = hero_view(&heartbeat);
        assert!(view.get("notifications").is_none());
        let home = tempfile::tempdir().unwrap();
        save_cache(home.path(), &view).unwrap();
        let bytes = std::fs::read(home.path().join("hero/cache.json")).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("private message"));
    }
}

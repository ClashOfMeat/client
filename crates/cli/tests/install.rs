use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use clashofmeat_core::{Env, Store};
use clashofmeat_daemon::{config, engine::Engine};
use rusqlite::Connection;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use time::OffsetDateTime;

struct Fixture {
    root: TempDir,
    launchctl: PathBuf,
    log: PathBuf,
    _socket: UnixListener,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let home = root.path();
        let launchctl = home.join("launchctl");
        let log = home.join("launchctl.log");
        fs::write(
            &launchctl,
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$CLASHOFMEAT_LAUNCHCTL_LOG\"\n",
        )
        .unwrap();
        fs::set_permissions(&launchctl, fs::Permissions::from_mode(0o755)).unwrap();
        fs::create_dir_all(home.join(".clashofmeat")).unwrap();
        let socket = UnixListener::bind(home.join(".clashofmeat/daemon.sock")).unwrap();
        Self {
            root,
            launchctl,
            log,
            _socket: socket,
        }
    }

    fn home(&self) -> &Path {
        self.root.path()
    }

    fn run(&self, args: &[&str]) -> Output {
        self.run_with(args, &[])
    }

    fn run_with(&self, args: &[&str], vars: &[(&str, &str)]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_clashofmeat"))
            .args(args)
            .envs(vars.iter().copied())
            .env("HOME", self.home())
            .env("CLASHOFMEAT_LAUNCHCTL", &self.launchctl)
            .env("CLASHOFMEAT_LAUNCHCTL_LOG", &self.log)
            .env("CLASHOFMEAT_NO_OPEN", "1")
            .output()
            .unwrap()
    }

    fn plist(&self) -> PathBuf {
        self.home()
            .join("Library/LaunchAgents/com.clashofmeat.daemon.plist")
    }
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn install_is_idempotent_and_uninstall_restores_cursor_hooks() {
    let fixture = Fixture::new();
    let hooks = fixture.home().join(".cursor/hooks.json");
    fs::create_dir_all(hooks.parent().unwrap()).unwrap();
    let original = json!({"version":1,"hooks":{"stop":[{"command":"user-hook"},{"command":"node clashofmeat.mjs cursor-hook"}]}});
    let original_bytes = serde_json::to_vec_pretty(&original).unwrap();
    fs::write(&hooks, &original_bytes).unwrap();
    let args = ["install", "--app", "https://example.com", "--no-connect"];
    assert_success(&fixture.run_with(
        &args,
        &[
            ("CODEX_HOME", "/Users/me/work & codex"),
            ("GEMINI_CLI_HOME", ""),
        ],
    ));
    let plist = fs::read_to_string(fixture.plist()).unwrap();
    assert!(!plist.contains("GEMINI_CLI_HOME"));
    for expected in [
        "<key>CODEX_HOME</key><string>/Users/me/work &amp; codex</string>",
        "<string>com.clashofmeat.daemon</string>",
        "<string>daemon</string>",
        "<key>KeepAlive</key><true/>",
        "<key>ProcessType</key><string>Background</string>",
        "daemon.log",
        "<key>HOME</key>",
    ] {
        assert!(plist.contains(expected), "missing {expected}");
    }
    let first_hooks = fs::read(&hooks).unwrap();
    let wired: Value = serde_json::from_slice(&first_hooks).unwrap();
    for event in [
        "beforeSubmitPrompt",
        "postToolUse",
        "afterAgentResponse",
        "stop",
    ] {
        assert_eq!(
            wired["hooks"][event]
                .as_array()
                .unwrap()
                .iter()
                .filter(|entry| entry["command"]
                    .as_str()
                    .unwrap_or("")
                    .contains("cursor-hook"))
                .count(),
            1
        );
    }
    assert!(wired["hooks"]["stop"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["command"] == "user-hook"));
    assert_success(&fixture.run(&args));
    assert_eq!(fs::read(&hooks).unwrap(), first_hooks);
    let calls = fs::read_to_string(&fixture.log).unwrap();
    let calls: Vec<_> = calls.lines().collect();
    assert_eq!(calls.len(), 6);
    assert!(calls[0].starts_with("bootout gui/"));
    assert!(calls[1].starts_with("bootstrap gui/"));
    assert!(calls[2].starts_with("kickstart gui/"));
    assert_success(&fixture.run(&["uninstall"]));
    assert!(!fixture.plist().exists());
    assert_eq!(fs::read(&hooks).unwrap(), original_bytes);

    assert_success(&fixture.run(&args));
    let mut edited: Value = serde_json::from_slice(&fs::read(&hooks).unwrap()).unwrap();
    edited["hooks"]["stop"]
        .as_array_mut()
        .unwrap()
        .push(json!({"command":"new-user-hook"}));
    fs::write(&hooks, serde_json::to_vec_pretty(&edited).unwrap()).unwrap();
    assert_success(&fixture.run(&["uninstall"]));
    let after: Value = serde_json::from_slice(&fs::read(&hooks).unwrap()).unwrap();
    assert!(after["hooks"]["stop"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["command"] == "new-user-hook"));
    assert!(after["hooks"]["stop"]
        .as_array()
        .unwrap()
        .iter()
        .all(|entry| !entry["command"]
            .as_str()
            .unwrap_or("")
            .contains("cursor-hook")));
}

#[test]
fn migration_imports_pairing_and_complete_file_offset() {
    let fixture = Fixture::new();
    let home = fixture.home();
    let origin = "https://example.com";
    let claude = home.join(".claude/projects/demo/chat.jsonl");
    fs::create_dir_all(claude.parent().unwrap()).unwrap();
    fs::write(&claude, format!("{}\n", json!({"type":"user","sessionId":"123e4567-e89b-12d3-a456-426614174000","timestamp":"2026-10-20T12:00:00Z","uuid":"prompt-1","message":{"content":"hello"}}))).unwrap();
    let old_dir = home
        .join(".clashofmeat")
        .join(&hex::encode(Sha256::digest(origin.as_bytes()))[..16]);
    fs::create_dir_all(&old_dir).unwrap();
    let old = Connection::open(old_dir.join("collector.sqlite")).unwrap();
    old.execute_batch("CREATE TABLE accounts(id TEXT PRIMARY KEY, plan TEXT, token TEXT, installation_id TEXT); CREATE TABLE metadata(key TEXT PRIMARY KEY, value TEXT); CREATE TABLE provider_files(path TEXT PRIMARY KEY, offset INTEGER, state TEXT); CREATE TABLE files(path TEXT PRIMARY KEY, offset INTEGER);").unwrap();
    old.execute(
        "INSERT INTO accounts VALUES (?1,'collector',?2,?3)",
        ("account-1", "secret-installation-token", "install-id"),
    )
    .unwrap();
    old.execute(
        "INSERT INTO metadata VALUES ('paired-at:account-1','2026-10-01T00:00:00Z')",
        [],
    )
    .unwrap();
    old.execute(
        "INSERT INTO provider_files VALUES (?1,?2,'{}')",
        (
            claude.to_string_lossy().as_ref(),
            fs::metadata(&claude).unwrap().len(),
        ),
    )
    .unwrap();
    old.execute(
        "INSERT INTO files VALUES (?1,?2)",
        (
            claude.to_string_lossy().as_ref(),
            fs::metadata(&claude).unwrap().len(),
        ),
    )
    .unwrap();
    drop(old);
    let old_agent = home.join("Library/LaunchAgents/com.clashofmeat.collector.plist");
    fs::create_dir_all(old_agent.parent().unwrap()).unwrap();
    fs::write(&old_agent, b"old").unwrap();
    assert_success(&fixture.run(&["install", "--app", origin, "--no-connect"]));
    assert!(!old_agent.exists());
    let env = Env::for_home(home);
    let mut store = Store::open(config::store_path(&env)).unwrap();
    assert_eq!(
        store.meta("installation.token").unwrap().as_deref(),
        Some("secret-installation-token")
    );
    assert_eq!(
        store.meta("installation.id").unwrap().as_deref(),
        Some("install-id")
    );
    assert_eq!(
        store.meta("paired_at").unwrap().as_deref(),
        Some("2026-10-01T00:00:00Z")
    );
    assert_eq!(
        store
            .cursor("claude-code", 0, "demo/chat.jsonl")
            .unwrap()
            .unwrap()
            .offset,
        fs::metadata(&claude).unwrap().len()
    );
    let mut engine = Engine::new(env, clashofmeat_providers::all(), "test");
    let report = engine
        .scan(&mut store, None, OffsetDateTime::now_utc())
        .unwrap();
    assert_eq!(report.events_saved, 0);
    assert_eq!(report.files_read, 0);
    drop(store);
    assert_success(&fixture.run(&["uninstall", "--keep-data"]));
    assert!(config::store_path(&Env::for_home(home)).exists());
    assert_success(&fixture.run(&["uninstall"]));
    assert!(!config::store_path(&Env::for_home(home)).exists());
}

#[test]
fn uninstall_removes_only_its_app_symlink_and_data() {
    let fixture = Fixture::new();
    let bin = fixture.home().join(".local/bin");
    fs::create_dir_all(&bin).unwrap();
    symlink(
        fixture
            .home()
            .join("Applications/Clash of Meat.app/Contents/MacOS/clashofmeat"),
        bin.join("clashofmeat"),
    )
    .unwrap();
    fs::create_dir_all(fixture.home().join(".clashofmeat/desktop")).unwrap();
    fs::write(fixture.home().join(".clashofmeat/desktop/data"), b"x").unwrap();
    assert_success(&fixture.run(&["uninstall"]));
    assert!(!bin.join("clashofmeat").exists());
    assert!(!fixture.home().join(".clashofmeat/desktop").exists());
}

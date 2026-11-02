//! The daemon keeps several stores open on one database while the CLI opens and closes it from
//! another process (`connect --force`, `uninstall`). The CLI closing must not delete the WAL the
//! daemon still writes to, or every later write is lost when the daemon restarts.
#![cfg(unix)]

use std::path::Path;
use std::process::Command;

use clashofmeat_core::Store;

/// Run as a separate process by `another_process_closing_the_store_keeps_our_writes`.
#[test]
#[ignore = "helper: opens and closes the store from another process"]
fn helper_open_and_close_store() {
    if let Ok(path) = std::env::var("CLASHOFMEAT_STORE_LOCK_TEST_DB") {
        let store = Store::open(&path).unwrap();
        assert!(store.meta("before").unwrap().is_some());
        drop(store);
    }
}

fn other_process_opens_and_closes(path: &Path) {
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "helper_open_and_close_store",
            "--ignored",
            "--quiet",
        ])
        .env("CLASHOFMEAT_STORE_LOCK_TEST_DB", path)
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn another_process_closing_the_store_keeps_our_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("desktop/activity.sqlite");
    // Like the daemon: one store, then a second one on the same file in the same process.
    let mut first = Store::open(&path).unwrap();
    first.set_meta("before", "1").unwrap();
    let second = Store::open(&path).unwrap();

    other_process_opens_and_closes(&path);
    assert!(
        path.with_extension("sqlite-wal").exists(),
        "another process deleted the WAL this process still writes to"
    );

    first.set_meta("after", "2").unwrap();
    drop(second);
    // A daemon killed by `systemctl restart` or `launchctl kickstart -k` never closes cleanly.
    std::mem::forget(first);
    let reopened = Store::open(&path).unwrap();
    assert_eq!(reopened.meta("after").unwrap().as_deref(), Some("2"));
}

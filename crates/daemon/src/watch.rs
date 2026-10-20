//! Provider source root watcher.

use std::collections::HashSet;
use std::path::PathBuf;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use clashofmeat_core::Env;
use crossbeam_channel::{Receiver, Sender};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::run::Message;

/// Starts the filesystem watcher and periodically resolves missing roots.
pub(crate) fn start(
    env: Env,
    main: Sender<Message>,
    refresh: Receiver<()>,
    stop: Receiver<()>,
    ready: Sender<()>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let (events, received) = crossbeam_channel::unbounded();
        let mut watcher = match notify::recommended_watcher(move |event| {
            let _ = events.send(event);
        }) {
            Ok(watcher) => watcher,
            Err(error) => {
                eprintln!("daemon watcher: {error}");
                let _ = ready.send(());
                return;
            }
        };
        let mut watched = HashSet::new();
        add_roots(&env, &mut watcher, &mut watched);
        let _ = ready.send(());
        loop {
            crossbeam_channel::select! {
                recv(stop) -> _ => break,
                recv(refresh) -> _ => add_roots(&env, &mut watcher, &mut watched),
                recv(received) -> event => match event {
                    Ok(Ok(event)) => {
                        if matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) {
                            if !event.paths.is_empty() { let _ = main.send(Message::FsChanged(event.paths)); }
                            add_roots(&env, &mut watcher, &mut watched);
                        }
                    }
                    Ok(Err(error)) => eprintln!("daemon watcher event: {error}"),
                    Err(_) => break,
                },
                default(Duration::from_secs(1800)) => add_roots(&env, &mut watcher, &mut watched),
            }
        }
    })
}

fn add_roots(env: &Env, watcher: &mut RecommendedWatcher, watched: &mut HashSet<PathBuf>) {
    for provider in clashofmeat_providers::all() {
        for source in provider.manifest().sources {
            let root = env.resolve(source.root);
            let mut path = root.as_path();
            while !path.is_dir() {
                match path.parent() {
                    Some(parent) => path = parent,
                    None => break,
                }
            }
            if path.is_dir() && watched.insert(path.to_path_buf()) {
                let mode = if path == root {
                    RecursiveMode::Recursive
                } else {
                    RecursiveMode::NonRecursive
                };
                if let Err(error) = watcher.watch(path, mode) {
                    watched.remove(path);
                    eprintln!("daemon watcher root {}: {error}", path.display());
                }
            }
        }
    }
}

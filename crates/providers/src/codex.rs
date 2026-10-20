//! Codex records. Stub: the module task replaces this file.

use clashofmeat_core::{Cx, Detect, Manifest, Provider, Record, Signal};

pub struct Codex;

static MANIFEST: Manifest = Manifest {
    id: "codex",
    name: "Codex",
    detect: Detect {
        commands: &[],
        apps: &[],
        paths: &[],
    },
    sources: &[],
    wiring: &[],
};

impl Provider for Codex {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn parse(&self, _record: &Record, _cx: &mut Cx) -> Vec<Signal> {
        Vec::new()
    }
}

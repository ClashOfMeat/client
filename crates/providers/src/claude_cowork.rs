//! Claude Cowork records. Stub: the module task replaces this file.

use clashofmeat_core::{Cx, Detect, Manifest, Provider, Record, Signal};

pub struct ClaudeCowork;

static MANIFEST: Manifest = Manifest {
    id: "claude-cowork",
    name: "Claude Cowork",
    detect: Detect {
        commands: &[],
        apps: &[],
        paths: &[],
    },
    sources: &[],
    wiring: &[],
};

impl Provider for ClaudeCowork {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn parse(&self, _record: &Record, _cx: &mut Cx) -> Vec<Signal> {
        Vec::new()
    }
}

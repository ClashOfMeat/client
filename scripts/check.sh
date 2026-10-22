#!/usr/bin/env bash
# Run every check a release needs: Rust tests and lints, UI and installer tests, installer syntax.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

step() { echo "check: $*"; }
step "cargo test --workspace"
cargo test --workspace
step "cargo clippy --workspace --all-targets -- -D warnings"
cargo clippy --workspace --all-targets -- -D warnings
step "node --test (hero, overlay, settings, installer)"
node --test apps/desktop/ui/hero/tests/*.test.mjs apps/desktop/ui/overlay/tests/*.test.mjs apps/desktop/ui/settings/tests/*.test.mjs install/tests/*.test.mjs
step "sh -n install/install.sh"
sh -n install/install.sh
echo "check: all passed"

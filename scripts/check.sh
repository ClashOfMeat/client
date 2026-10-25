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
if cargo audit --version >/dev/null 2>&1; then
  step "cargo audit (workspace and desktop app lock files)"
  cargo audit
  cargo audit --file apps/desktop/src-tauri/Cargo.lock
else
  echo "check: note: cargo-audit is not installed (cargo install cargo-audit), so no advisory scan ran"
fi
echo "check: all passed"

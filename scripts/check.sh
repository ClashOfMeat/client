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
if [[ $(uname) == Darwin ]]; then
  # The desktop app is its own workspace and macOS-only here, so the workspace steps above never compile it.
  step "cargo clippy (desktop app) -- -D warnings"
  sidecar="apps/desktop/src-tauri/binaries/clashofmeat-$(rustc -vV | sed -n 's/^host: //p')"
  # tauri-build insists the bundled CLI exists; a lint never runs it, so an empty stand-in will do.
  if [[ ! -e $sidecar ]]; then
    mkdir -p "$(dirname "$sidecar")"
    : >"$sidecar"
    trap 'rm -f "$sidecar"' EXIT
  fi
  cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings
fi
step "node --test (hero, overlay, settings, signs, installer)"
node --test apps/desktop/ui/hero/tests/*.test.mjs apps/desktop/ui/overlay/tests/*.test.mjs apps/desktop/ui/settings/tests/*.test.mjs apps/desktop/ui/signs/tests/*.test.mjs install/tests/*.test.mjs
if [[ -f ../clashofmeat-app/packages/core/src/heroes/hero-contract.json ]]; then
  step "exported game hero payloads"
  (cd ../clashofmeat-app && node --import remix/node-tsx "$root/scripts/check-hero-emote-payload.mjs" . --all)
fi
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

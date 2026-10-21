# Scripts

Everything is built and released by hand on a Mac. [docs/building.md](../docs/building.md) is the full guide:
prerequisites, the updater key, building each target and cutting a release.

| Script | What it does |
|---|---|
| `check.sh` | `cargo test`, `cargo clippy -D warnings`, the UI and installer `node --test` suites, `sh -n install/install.sh`. |
| `package.sh <version> <target> [--out DIR] [--app URL] [--tauri-config JSON]` | Builds one installer: `.app.tar.gz` for `aarch64-apple-darwin` / `x86_64-apple-darwin`, NSIS `-setup.exe` for `x86_64-pc-windows-msvc` (cross-compiled with cargo-xwin). Writes a `.sha256`, and an updater `.sig` when `TAURI_SIGNING_PRIVATE_KEY_PATH` or `TAURI_SIGNING_PRIVATE_KEY` is set. |
| `update-manifest.sh <dist> <version> <base-url> [--min-version X] [--security] [--mac-only]` | Checks the three installers (only the two Mac archives with `--mac-only`), checksums and signatures, then writes `manifest.json` (for `install.sh`) and `update.json` (Tauri updater). `--self-test` checks it against fixtures. |
| `release.sh <version> --app <url> [--min-version X] [--security] [--mac-only] [--dry-run]` | Runs the checks, packages all three targets (the two Mac targets with `--mac-only`), signs them, writes the manifests, and publishes GitHub release `desktop-v<version>`. `--dry-run` stops before publishing. |
| `budget.sh [--window S] [--daemon-pid PID] [--app-pid PID]` | Samples the running daemon and app with `ps` and fails over 25 MB / 1% CPU (daemon) or 150 MB / 3% CPU (app). |
| `e2e-mac.sh [--app URL] [--port PORT] [--keep]` | Local end-to-end test with a throwaway key and a test identifier: install 0.1.0, pairing link, update to 0.1.1, bad signature refused. |
| `parity/` | Generator for the provider parity fixtures (see `parity/README.md`). |

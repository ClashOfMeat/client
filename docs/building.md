# Building and releasing Clash of Meat

There is no build pipeline. Every build and release is made by hand on Paul's Mac with the scripts in `scripts/`.
One Mac builds all three installers:

| Target | Installer | Who uses it |
|---|---|---|
| `aarch64-apple-darwin` | `Clash.of.Meat_<v>_aarch64.app.tar.gz` | Apple Silicon Macs |
| `x86_64-apple-darwin` | `Clash.of.Meat_<v>_x86_64.app.tar.gz` | Intel Macs |
| `x86_64-pc-windows-msvc` | `Clash.of.Meat_<v>_x64-setup.exe` (NSIS) | Windows x64 (ARM64 PCs run it under emulation) |

The Windows installer is cross-compiled on the Mac with `cargo-xwin`.

The GitHub repo `ClashOfMeat/client` is private for now. Release downloads (the Mac `curl` install and
the Windows `.exe`) only work for people with access to the repo until it is made public.

## 1. Prerequisites (once)

1. **Rust** (rustup) with the three targets:

   ```sh
   rustup target add aarch64-apple-darwin x86_64-apple-darwin x86_64-pc-windows-msvc
   rustup component add clippy
   ```

2. **Tauri CLI 2.12.2**, through cargo:

   ```sh
   cargo install tauri-cli --version 2.12.2 --locked
   ```

3. **Xcode command line tools** (`xcode-select --install`) for the macOS builds and `codesign`.

4. **LLVM, LLD and NSIS** for the Windows build:

   ```sh
   brew install llvm lld nsis
   ```

   Homebrew's `llvm` no longer ships `lld-link`; it comes from `lld`. Both are keg-only, so they are not on `PATH` by
   default. `scripts/package.sh` adds both `bin` directories to `PATH` itself for Windows builds. To run
   `cargo xwin` by hand, add them to your shell profile:

   ```sh
   export PATH="$(brew --prefix llvm)/bin:$(brew --prefix lld)/bin:$PATH"
   ```

5. **cargo-xwin**:

   ```sh
   cargo install --locked cargo-xwin
   ```

   On first use, cargo-xwin downloads the Microsoft CRT and Windows SDK into its cache. By using it you accept
   Microsoft's license for those files.

6. **Node.js** (any current LTS) for the UI and installer tests.

7. **GitHub CLI**, logged in with access to `ClashOfMeat/client`:

   ```sh
   brew install gh
   gh auth login
   ```

## 2. Run the app in development

```sh
cargo build -p clashofmeat-cli
cd apps/desktop/src-tauri
cargo tauri dev
```

`cargo tauri dev` expects the sidecar at `apps/desktop/src-tauri/binaries/clashofmeat-<host-triple>`. If it is
missing, copy `target/debug/clashofmeat` there under that name, or run `scripts/package.sh` once (it copies the
release build there).

## 3. Run the checks

```sh
scripts/check.sh
```

It runs, in order, and stops at the first failure:

- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `node --test apps/desktop/ui/hero/tests/*.test.mjs apps/desktop/ui/overlay/tests/*.test.mjs install/tests/*.test.mjs`
- `sh -n install/install.sh`

`scripts/update-manifest.sh --self-test` checks the release manifest generation on its own.

### Resource budget

```sh
scripts/budget.sh [--window 60] [--interval 5] [--daemon-pid PID] [--app-pid PID] [--only daemon|app]
```

It samples the daemon (`clashofmeat daemon`) and the app (`clashofmeat-desktop`) with `ps` for the window and
exits 1 when one is over budget (2 on a usage error or a missing process). Without `--daemon-pid`/`--app-pid` it
finds each by name and stops if several match. Average CPU is the CPU time used during the window divided by its
length. RSS is the largest sample.

| Process | Limit | Measured 2026-10-21 (Apple Silicon) |
|---|---|---|
| daemon | 25 MB RSS, 1% average CPU | release, idle: 4–7 MB, 0.07–0.09%. Debug, with agents writing logs: 26.5 MB, 1.1% |
| app | 150 MB RSS, 3% average CPU | release, hero shown: 84 MB, 0.42%. Debug: 44 MB, 1.6% |

The design target is a daemon idling at about 10 MB and 0% CPU, and an overlay that redraws only while the hero
moves. The limits apply to release builds while no agent session is writing logs: the daemon parses provider logs
300 ms after each change, so it costs more while you work. The limits leave room for a burst of parsing or a
moving hero and still catch a leak or a loop that never stops. The WebView's WebKit helper processes run under
launchd, so `ps` cannot tie them to the app and they are not counted. Override a limit with `--daemon-rss-mb`,
`--daemon-cpu`, `--app-rss-mb` or `--app-cpu`.

## 4. Build one installer

```sh
scripts/package.sh <version> <target> [--out DIR] [--app URL] [--tauri-config JSON]
```

Examples:

```sh
scripts/package.sh 0.1.0 aarch64-apple-darwin
scripts/package.sh 0.1.0 x86_64-apple-darwin
scripts/package.sh 0.1.0 x86_64-pc-windows-msvc --app https://clashofmeat.example
```

What it does:

1. Builds the `clashofmeat` CLI for the target (`cargo build`, or `cargo xwin build` for Windows) and copies it into
   `apps/desktop/src-tauri/binaries/` as the Tauri sidecar.
2. Runs `cargo tauri build` for the target: an `.app` bundle on macOS, an NSIS installer on Windows
   (`--runner cargo-xwin --bundles nsis`). The signing variables are unset for this step.
3. macOS: ad-hoc signs the `.app` (`codesign --sign -`) and packs it as `.app.tar.gz`. Windows: copies the
   `-setup.exe`. There is no code signing on Windows.
4. Writes a `.sha256` next to the installer.
5. If `TAURI_SIGNING_PRIVATE_KEY_PATH` or `TAURI_SIGNING_PRIVATE_KEY` is set, signs the installer with
   `cargo tauri signer sign --app-version <version>` and writes the updater `.sig`. If neither is set, it skips
   this and says so.

`--app URL` (or the `CLASHOFMEAT_DEFAULT_APP_URL` environment variable) is compiled into the CLI as its default app
URL. The Windows installer runs `clashofmeat.exe install` without `--app`, so a Windows build without it cannot pair.
URLs must be `https://`; `http://localhost` and `http://127.0.0.1` are allowed for local testing.

`--tauri-config JSON` is merged over `tauri.conf.json` for this build (after the version). The local end-to-end
test uses it to build under a test identifier (`{"identifier":"com.clashofmeat.e2e"}`); releases do not use it.

Output goes to `dist/<version>/` (ignored by git) unless you pass `--out`. Tauri's own bundles stay under
`apps/desktop/src-tauri/target/<target>/release/bundle/`.

## 5. Updater key (once)

The app only accepts updates signed with the updater key. Until the real public key is in `tauri.conf.json`, the
app reports "Update signing key is not configured" and updates stay off.

1. Generate the key pair and choose a password:

   ```sh
   cargo tauri signer generate -w ~/.tauri/clashofmeat.key
   ```

   This writes the private key to `~/.tauri/clashofmeat.key` and the public key to `~/.tauri/clashofmeat.key.pub`.

2. Put the contents of `~/.tauri/clashofmeat.key.pub` into `apps/desktop/src-tauri/tauri.conf.json` at
   `plugins.updater.pubkey`, replacing `REPLACE_WITH_TAURI_UPDATER_PUBLIC_KEY`. Also put it in
   `apps/desktop/src-tauri/updater.pub` so the two agree (`release.sh` warns when they differ). Commit both.

3. Never commit the private key or its password. Back the key up somewhere safe (a password manager). If it is
   lost, installed apps can no longer be updated and users must reinstall by hand.

At release time, set:

```sh
export TAURI_SIGNING_PRIVATE_KEY_PATH="$HOME/.tauri/clashofmeat.key"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD='…'   # optional: release.sh asks once if it is unset
```

`TAURI_SIGNING_PRIVATE_KEY` (the key contents) works instead of the path.

## 6. Cut a release

```sh
scripts/release.sh <version> --app https://<web app URL> [--min-version x.y.z] [--security] [--mac-only] [--dry-run] [--out DIR]
```

- `--app` is required. It becomes the default app URL compiled into the CLI (see section 4).
- `--min-version` is optional. Use it only when older installed apps must update before they keep running.
- `--security` marks the update as a security update in `update.json`.
- `--mac-only` packages only `aarch64-apple-darwin` and `x86_64-apple-darwin`. `manifest.json` and `update.json`
  then list only the two Mac entries, and the GitHub release has no Windows installer. A Windows app that reads this
  `update.json` finds a newer version with no `windows-x86_64` entry, so its update check fails (Tauri's
  "target not found") and retries; do not offer a Mac-only release to Windows apps. Without the flag, all three
  targets are built.
- `--dry-run` does everything except creating the GitHub release. Problems that would stop a real release (dirty
  tree, placeholder public key, missing signing key, gh not logged in) are reported and skipped instead.
- `--out` changes the output directory (default `dist/<version>/`).

Release from a clean, pushed commit. The script:

1. Checks that the updater public key is set, the signing key is available, tracked files are clean, the tag
   `desktop-v<version>` does not exist locally or on GitHub (tags are never reused), no release with that tag exists,
   and `HEAD` is on GitHub.
2. Runs `scripts/check.sh`.
3. Runs `scripts/package.sh` for `aarch64-apple-darwin`, `x86_64-apple-darwin` and `x86_64-pc-windows-msvc`, which
   builds and signs each installer.
4. Runs `scripts/update-manifest.sh` to check every checksum and signature and write:
   - `manifest.json`: file, sha256 and size per target. `install.sh` reads it.
   - `update.json`: Tauri v2 updater static JSON with `darwin-aarch64`, `darwin-x86_64` and `windows-x86_64`
     entries (URL and signature), plus `security` and the optional `min_version`.
5. Creates the GitHub release `desktop-v<version>` on `ClashOfMeat/client` at `HEAD` with the two Mac
   archives, their `.sha256` and `.sig`, the Windows `-setup.exe` with its `.sha256` and `.sig`, and `install.sh`.
   Then it uploads `manifest.json` and `update.json` last, so nobody reads a manifest whose files are not there yet.

Try a dry run first:

```sh
scripts/release.sh 0.1.0 --app https://clashofmeat.example --dry-run
```

After the release exists, stage it in the web app: set `CLASHOFMEAT_DESKTOP_CANDIDATE_VERSION`,
`CLASHOFMEAT_DESKTOP_CANDIDATE_MANIFEST` (the release's `update.json` URL) and `CLASHOFMEAT_DESKTOP_CANDIDATE_PERCENT`
(0–100), then redeploy. To promote it, set `CLASHOFMEAT_DESKTOP_STABLE_VERSION` and
`CLASHOFMEAT_DESKTOP_STABLE_MANIFEST` to the same version and URL, clear the candidate variables, and redeploy.
`CLASHOFMEAT_DESKTOP_MINIMUM` is the separate server-side minimum for normal client calls. The web app's
`docs/releasing.md` section "Desktop app" covers rollout and recovery.

## 7. Where things land

| What | Where |
|---|---|
| Release files | `dist/<version>/` (or `--out`) |
| Tauri bundles | `apps/desktop/src-tauri/target/<target>/release/bundle/{macos,nsis}/` |
| CLI builds | `target/<target>/release/clashofmeat[.exe]` |
| Sidecars | `apps/desktop/src-tauri/binaries/clashofmeat-<target>[.exe]` (ignored by git) |
| Published release | `https://github.com/ClashOfMeat/client/releases/tag/desktop-v<version>` |

## 8. How users install

**Mac.** Run the command from the setup page. The web app serves `install.sh` with its own URL and the stable
version filled in. A release copy also works:

```sh
curl -fsSL https://github.com/ClashOfMeat/client/releases/download/desktop-v<version>/install.sh \
  | sh -s -- --app https://<web app URL> --version <version>
```

The installer picks the archive for the Mac's CPU, checks its size and SHA-256 against `manifest.json`, installs
`Clash of Meat.app` in `~/Applications`, links the CLI at `~/.local/bin/clashofmeat`, then runs
`clashofmeat install`. See `install/README.md`. The app is not signed with a Developer ID or notarized; `curl`
downloads are not quarantined, so Gatekeeper does not block it.

**Windows.** Download `Clash.of.Meat_<version>_x64-setup.exe` from the release and run it. The installer is not
code signed, so SmartScreen shows "Windows protected your PC". Click **More info**, then **Run anyway**. It installs
for the current user only and needs no admin rights. It runs `clashofmeat.exe install --no-connect`, which records the
app URL compiled into the build and wires Cursor; pairing happens in the app.

Windows has no LaunchAgent. The app starts the daemon itself, hidden, and registers itself to start at sign-in
(`tauri-plugin-autostart`, a `Clash of Meat` value under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`).
The installer hooks are in `apps/desktop/src-tauri/windows/hooks.nsh`. Before an install or update they stop the
running `clashofmeat.exe`, because the daemon holds the file open. On uninstall they run `clashofmeat.exe uninstall`
and remove the sign-in entry.

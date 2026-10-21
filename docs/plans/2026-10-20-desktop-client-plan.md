# Desktop client implementation plan

**Goal:** build the client described in [the design](2026-10-20-desktop-client-design.md): a Rust daemon with provider
modules plus a Tauri hero overlay, installed and updated end to end with the web app.

**Execution model (Paul, 2026-10-20):** Codex writes all the code. Claude owns the specs, runs each task through
`codex-exec`, reviews every diff against this plan and the design, and commits. One task = one `codex-exec` call with
its own GOAL / FILES / CONSTRAINTS / ACCEPTANCE. Specs for later milestones are sharpened just before dispatch, once
the interfaces they build on exist.

**Tech stack:** Rust 1.99 workspace (std threads, no tokio in the daemon), `rusqlite` (bundled), `notify` (FSEvents),
`ureq` (rustls), `clap`, `serde_json`; Tauri 2.12 with plain HTML/CSS/JS (no npm, no bundler) for the app.

## Rules for every task

- **Sandbox:** Codex has no network. Claude adds dependencies to `Cargo.toml` and runs `cargo fetch` before
  dispatching; specs say "no new dependencies" and every cargo command uses `--offline`.
- **Scope:** `--files` lists the exact paths; new folders are committed as a skeleton first so the scope check sees
  files, not a collapsed `??` folder.
- **Checks** run bare commands (no `| tail`), so their exit code counts.
- **Reference code** is read from `../guildbyte-plugin` (working tree, Clash of Meat rename uncommitted). It is read
  only: no task writes there.
- **Privacy** (from `guildbyte-app/docs/provider-observer-plan.md`): nothing uploaded or stored may contain prompts,
  responses, code, paths, file or repo names, commands or tool arguments. Only model families, never raw model ids.
- **Ids** never change: events `stableId("guildbyte-v1:" + value)`, observer tasks `"guildbyte-observer-v1:"`, both
  formatted as the JS does (`worker.mjs:28`, `task-observer.mjs:8`).
- **Style:** small modules, doc comments on public items only, `anyhow` at binaries, `thiserror` in libraries,
  `cargo clippy --offline -- -D warnings` clean.
- **Review loop:** exit 0 → Claude reads the diff, runs the check again, commits. Exit 3/4/5 → sharpen the spec, retry
  once; a second failure is done by Claude and noted.

## Workspace layout

```
crates/core       ids, signals, manifest + Provider trait, Env, incremental reader, SQLite store, observer
                  aggregation, atomic config writer
crates/providers  claude_code, claude_cowork, codex, gemini, cursor (+ registry)
crates/daemon     engine (scan → parse → store), watcher, sync client, hero cache, socket server
crates/cli        `clashofmeat` binary: daemon, connect, status, sync, uninstall, cursor-hook
apps/desktop      Tauri app (src-tauri/ + ui/)                                   (milestone 3)
install/          install.sh                                                      (milestone 4)
fixtures/<provider>/{input/, expected.json}    parity fixtures
scripts/parity/   generator that runs the old JS parsers over fixtures
```

`CLASHOFMEAT_HOME` overrides `~/.clashofmeat`; provider roots resolve through `Env { home, vars }` so every test runs
against a temp `HOME`.

## Milestone 1 — core, daemon, Claude Code

### T1. Parity fixtures and generator
- **Files:** `scripts/parity/generate.mjs`, `scripts/parity/README.md`, `fixtures/**`
- **Goal:** sanitized input fixtures for all five providers and `expected.json` files produced by the *old* JS code.
- **Spec:** for each provider (`claude-code`, `claude-cowork`, `codex`, `gemini`, `cursor`) write synthetic inputs
  under `fixtures/<provider>/input/` mirroring the real on-disk layout relative to `HOME` (formats from the old
  adapters and their tests; no real user content). Cover: progressive usage of one response, several sessions,
  task start/complete/abort (Codex), legacy JSON (Gemini), oversized and malformed lines, tool calls, subagents,
  compaction, an unknown model. `generate.mjs --plugin ../guildbyte-plugin` runs the old adapters against a temp
  HOME + temp DB and writes rows `{id, session_id, at, key, value, kind, provider, model}` sorted by id to
  `fixtures/<provider>/expected.json`, plus completed observer task payloads to `expected-tasks.json`.
- **Acceptance:** `node scripts/parity/generate.mjs --plugin ../guildbyte-plugin --check` exits 0 (regenerates in
  memory and diffs against the committed files).

### T2. Core types and ids
- **Files:** `crates/core/src/{lib,ids,signal,manifest,provider,env}.rs`
- **Spec:** `stable_id`, `observer_task_id`, `session_uuid`, `is_uuid`, `model_family` ported exactly; test vectors
  computed with `node` from the old JS and embedded. `Signal` (Usage / Turn / Tool / Compaction / Reading / Counter
  per the design), `Manifest { id, name, detect, sources, wiring }`, `Source { roots, glob, format: Jsonl | Json |
  Spool }`, `Record { source, file_key, offset, value }`, `Cx { file_state: serde_json::Map, now }`, the `Provider`
  trait. `Env::resolve(root)` handles `$VAR|~/default`.
- **Acceptance:** `cargo test --offline -p clashofmeat-core`.

### T3. Incremental reader
- **Files:** `crates/core/src/reader.rs`
- **Spec:** `FileCursor { inode, offset, state }`; `read_jsonl(path, &mut FileCursor) -> ReadBatch { records,
  skipped_malformed, skipped_oversized }` keeps a trailing partial line unconsumed, restarts on truncation or inode
  change, skips lines above the old cap (`lines.mjs`). `read_json(path, cursor)` yields the whole document when
  mtime/len changed. `list_files(root, glob, since)` via walkdir. One implementation replaces `scanFile`, `scanLines`
  and the three walkers.
- **Acceptance:** `cargo test --offline -p clashofmeat-core reader`.

### T4. Store
- **Files:** `crates/core/src/store.rs`, `crates/core/src/store/migrations.rs`
- **Spec:** `PRAGMA user_version` migrations. Tables: `events` (old columns, `value INTEGER`, upsert keeps the max),
  `cursors (source, file_key, inode, offset, state)`, `meta (key, value)`, `health (provider, ok, malformed,
  oversized, errors, last_at)`, and the observer tables `task-observer.mjs` actually uses. No deletes on open. API:
  `open`, `save_signals(provider, &[Signal])`, `pending(kind, limit)`, `mark_synced(ids)`, cursor and meta get/put,
  `health_snapshot`.
- **Acceptance:** `cargo test --offline -p clashofmeat-core store`.

### T5. Claude Code provider
- **Files:** `crates/providers/src/{lib,claude_code}.rs`, `crates/providers/tests/parity_claude_code.rs`
- **Spec:** port `ingestRecord` + `collector/providers/claude.mjs` (Claude part only). Turn signals: human prompt →
  `Start`, assistant message → `Step`, assistant `stop_reason: end_turn` (or the next human prompt) → `Stop`. The
  parity test runs reader + provider + store over `fixtures/claude-code/input` and compares to `expected.json`.
- **Acceptance:** `cargo test --offline -p clashofmeat-providers`.

### T6. Sync client
- **Files:** `crates/daemon/src/{lib,config,sync}.rs`, `crates/daemon/tests/sync.rs`
- **Spec:** `config.json` compatible with the old collector (`collector/config.mjs`, `paths.mjs`). Pairing create +
  exchange (404 waiting, 410 gone), observations upload in old batch sizes, observer tasks, heartbeat (response saved
  to `meta.heartbeat`). Header `x-clashofmeat-client: desktop/<version>`. 401 → unpair, 426 → stop and keep the
  server message for the UI, 429 → honour `Retry-After`. Tests use an in-process fake server on `127.0.0.1:0`.
- **Acceptance:** `cargo test --offline -p clashofmeat-daemon --test sync`.

### T7. Engine
- **Files:** `crates/daemon/src/engine.rs`, `crates/daemon/tests/engine.rs`
- **Spec:** `Engine::scan(&mut self, changed: Option<&[PathBuf]>)` lists sources for every enabled provider, reads
  new records, parses, stores, updates cursors and health, and returns `Activity` changes (per provider/session:
  busy since, last step, stopped). Opt-out list in `meta.providers_off`.
- **Acceptance:** `cargo test --offline -p clashofmeat-daemon --test engine`.

### T8. Watcher, loops and socket
- **Files:** `crates/daemon/src/{watch,run,socket,hero}.rs`, `crates/daemon/tests/socket.rs`
- **Spec:** FSEvents on resolved roots (debounce 300 ms, rescan every 10 min, detect new roots every 30 min);
  upload when pending and 30 s since the last one; heartbeat every 30 s while any agent was active in the last 5 min,
  else every 5 min. Socket `$HOME/daemon.sock` (0600), NDJSON `{id, method, params}` → `{id, result|error}`, methods
  `status`, `snapshot`, `subscribe` (streams `activity`, `heartbeat`, `sync` events), `sync_now`, `pair_start`,
  `pair_status`. Hero cache: sprites from the heartbeat written to `$HOME/hero/`.
- **Acceptance:** `cargo test --offline -p clashofmeat-daemon`.

### T9. CLI and end-to-end test
- **Files:** `crates/cli/src/main.rs`, `crates/cli/tests/e2e.rs`
- **Spec:** `clashofmeat daemon | connect --app URL | status [--json] | sync`; exit codes 0/1/2/3. The e2e test
  starts the daemon on a temp HOME and fake server, appends Claude transcript lines, and asserts the socket streams
  `activity` busy → stopped and the server receives the observations.
- **Acceptance:** `cargo test --offline --workspace` and `cargo clippy --offline --workspace -- -D warnings`.

## Milestone 2 — remaining providers and migration

- **T10–T13.** `claude_cowork`, `codex`, `gemini`, `cursor` modules, one task each, each with its parity test. Run in
  parallel worktrees (independent files) after T9.
- **T14. Cursor wiring:** `clashofmeat cursor-hook` writes the content-free spool line; `core::config_writer` (atomic,
  symlink-aware, mode-preserving, backup) installs and removes the hook in `~/.cursor/hooks.json`.
- **T15. Observer aggregation:** port `task-observer.mjs` onto signals; parity against `expected-tasks.json`.
- **T16. Migration:** import token and cursors from the old collector, stop `com.clashofmeat.collector`, detect the
  plugin.

## Milestone 3 — Tauri app

- **T17. Hero state machine (JS):** `apps/desktop/ui/hero/*.mjs` ported from `companion-animation.mjs`,
  `companion-pixels.mjs` and the motion code, with `node --test` tests; level and XP uncapped.
- **T18. Overlay window:** transparent, click-through except the hero (hit test), always on top, all Spaces, moves
  along the bottom edge above the Dock, hides behind full-screen apps, multi-screen setting.
- **T19. Menu bar and settings:** status, pause, sync now, providers on/off, pairing, about.
- **T20. Fake daemon:** `clashofmeat daemon --fake <script>` for UI work.
- **T21. Resource budget script.**

## Milestone 4 — install, update, web app

- **T22. install.sh and uninstall.**
- **T23. Updater:** Tauri updater plugin with minisign, check cadence, Update ready · Restart, `minimum` block,
  daemon kickstart after relaunch.
- **T24. Release workflow:** manual dispatch, macOS arm64 + x86_64, separate `sign` job, GitHub release
  `desktop-v<ver>`.
  *(2026-10-21: replaced by local scripts. No CI; `scripts/release.sh` checks, packages macOS arm64 + x86_64 and
  Windows x64, signs and publishes from Paul's Mac. See `docs/building.md`.)*
- **T25–T27 (guildbyte-app worktree off `onboarding`):** `desktop` client + both headers; `CLIENT_RELEASES.desktop`
  and `GET /api/clients/desktop/update`; `GET /install.sh`, setup page, docs.

## Milestone 5 — cutover

- **T28.** Local end-to-end run against the web app (port 3100+, local docker DB), update to a locally signed build,
  resource budget, then the setup page switch and plugin phase-out notice.

## Remaining work (2026-10-21)

- [ ] **Make the GitHub repo public** (planned within days). Audit secrets and history, add a LICENSE and a README
  for outsiders. Until then, release downloads only work for people with repo access.
- [ ] **Generate the updater key** (Paul) and set the real pubkey in `tauri.conf.json` (`docs/building.md` §5).
  Updates stay off until then.
- [ ] **Web app** (guildbyte-app, branch `feat/desktop-hero-sync`, not pushed):
  - [ ] Accept a `desktop` client in the `X-Clashofmeat-Client` header parser. Today `desktop/x.y.z` falls back to
    "legacy plugin 0.0.0".
  - [ ] Add `CLIENT_RELEASES.desktop`.
  - [ ] Add the updater endpoint `GET /api/clients/desktop/update`.
  - [ ] Serve `/install.sh` with the app URL and stable version substituted.
  - [ ] Switch the setup page to the Mac install command and a Windows `.exe` download. It still points to
    guildbyte-plugin, whose download URL returns 404.
  - [ ] Pairing: device and tools fields, and 410 for declined or expired pairings. Needs migration 066 or later;
    Paul applies migrations.
- [ ] **Apply migration `db/066_shared_guild_wagers.sql` locally** (Paul).
- [ ] **Build the first Windows installer** once the web app, with the desktop branch, is deployed to production:
  `scripts/package.sh <v> x86_64-pc-windows-msvc` with `CLASHOFMEAT_DEFAULT_APP_URL` set to the production URL (Paul
  will give it). Paul then installs it on his Windows PC. It has not been built yet, on purpose.
- [x] **Create the GitHub repo and push** (2026-10-21): `ClashOfMeat/client`, private. The org was renamed from
  PromptWar; old `PromptWar/...` URLs redirect.
- [ ] **Web app repo names.** The web app still names `PromptWar/guildbyte-plugin` (setup page, collector manifest
  check). GitHub redirects it to `ClashOfMeat/plugin`; switch to the new names when the setup page moves to this client.
- [ ] **Mac-only release.** `release.sh` needs all three installers. Add a `--mac-only` flag if a Mac release has to go
  out before the Windows build is ready.
- [ ] **Cursor hook on Windows.** The hook command quotes the `clashofmeat.exe` path for `cmd.exe`; check how Cursor
  runs hooks on Windows during the first test.
- [ ] **First Windows test** on Paul's Windows PC: install the setup `.exe`, pair, hero overlay, token counting for
  Claude Code and Codex, autostart after reboot, uninstall.
- [ ] **Code signing.** A Windows code signing certificate removes the SmartScreen warning. Apple Developer ID
  signing and notarization are needed for a downloaded `.app`; the `curl` install works without them.
- [ ] **Windows ARM64 build.** x64 runs under emulation until then.
- [ ] **End-to-end run:** install → pair → update to a locally signed build (old T28), and the resource budget script
  (old T21).
- [ ] **Retire guildbyte-plugin.** Once the setup page points to the new client, archive the repo (read-only, do not
  delete) so installed plugins and collectors keep resolving. First review its 48 uncommitted local changes on branch
  `feat/leaderboard-progression-v1` (rename to clashofmeat, `collector/`, `install/` incl. `install.ps1`) before
  anything is discarded.
- [ ] **Push the web branch and open its PR.**
- [ ] **Clean up** the local demo processes and `~/.com-visual` when Paul is done.

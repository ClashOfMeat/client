# Clash of Meat desktop client — design

Validated 2026-10-20. Replaces the Claude Code plugin (`guildbyte-plugin/mods/clashofmeat`) and the Node collector
(`guildbyte-plugin/collector`) with one Mac app: a Rust daemon that collects coding-agent token usage from every
supported provider, and a Tauri app that shows the hero walking on the screen. Architecture borrowed from
`tandem-ai/session-mesh` (daemon + thin UI over a Unix socket, launchd, Tauri updater) and `tandem-ai/mesh-extensions`
(one provider module per agent, described by a manifest).

## Goals

- **Light.** Daemon idle at ~10 MB RSS and ~0% CPU; no polling loops. Overlay redraws only while the hero moves.
- **Hero on the screen.** The companion walks along the bottom edge of the screen, above the Dock, reacting live to
  agent activity from any provider.
- **Modular providers.** The core knows no provider. Each provider is one module behind one interface; adding one is a
  module plus fixtures.
- **Wired end to end with the web app**, including staged, signed self-updates.

Non-goals for v1: new providers beyond the five existing ones; Developer ID signing / notarization / `.dmg`; Linux and
Windows UI; walking on top of windows; WASM-sandboxed providers.

## 1. Architecture

```
┌─ Clash of Meat.app (Tauri 2) ─────────────────────────────────────────┐
│  overlay window   256 px, transparent, click-through except the hero, │
│                   always on top, all Spaces; the window moves          │
│  hero menu        hide hero, stop walking, settings, quit (no menu bar)│
│  settings window  pairing, providers on/off, about/update              │
│  Contents/MacOS/clashofmeat   one Rust binary: CLI + daemon            │
└───────────────────────────────┬───────────────────────────────────────┘
                                │ NDJSON over ~/.clashofmeat/daemon.sock
┌─ clashofmeat daemon (LaunchAgent com.clashofmeat.daemon, KeepAlive) ──┐
│  watcher    FSEvents on provider source roots                         │
│  providers  one module per tool (§2)                                  │
│  store      SQLite queue: events, tasks, file offsets; versioned       │
│             migrations                                                │
│  sync       POST /api/observations, /api/observer/tasks, heartbeat    │
│  hero       caches character, sprites, progression; pushes activity,  │
│             XP and level-up events to the app                         │
└───────────────────────────────────────────────────────────────────────┘
```

- The app holds no state: it renders a view model built from the daemon's snapshot and event stream, and reconnects
  with backoff (0.5 s → 5 s) when the daemon restarts. Quitting the app or pausing the hero never stops collection.
- Socket API: newline-delimited JSON requests `{id, method, params}` → `{id, result | error}`; `subscribe` streams
  events. Every CLI command takes `--json` and prints the raw result. Exit codes: 0 ok, 1 error, 2 usage, 3 daemon not
  running.
- **No hooks for usage or activity.** Provider session files grow during a turn and record task boundaries, so
  FSEvents plus parsing gives busy/done within ~1 s without touching any agent config. Cursor is the only exception: it
  writes no usage to disk, so its `stop` hook stays (§2).
- **Kept from today:** the frozen `stableId("guildbyte-v1:…")` id seed (the server dedupes events already uploaded by
  the old collector or plugin, so switching clients never double counts); the pairing flow; the server API.

## 2. Provider modules

```rust
pub trait Provider: Send + Sync {
    fn manifest(&self) -> &'static Manifest;               // id, name, detect, sources, wiring
    fn parse(&self, rec: &Record, cx: &mut Cx) -> Vec<Signal>;
    fn wiring(&self, env: &Env) -> Option<Wiring> { None }  // Cursor only in v1
}

enum Signal {
    Usage { session, model, input, output, cache_read, cache_write, at, key },
    Turn  { session, phase: Start | Step | Stop },          // drives the hero
    Task  { .. },                                           // observer aggregates, as today
    Tool  { session, category }, Compaction { session },
}
```

Modules are pure: records in, signals out. No filesystem, network or SQLite access. The core owns, once:

- detection (at start, every 30 min, and when a source root appears) and an opt-out list;
- FSEvents and the single incremental reader (offsets, truncation/rotation, partial lines, oversized-line cap);
- stable ids from `key`; per-provider health counters (heartbeat `adapters`);
- every config write: atomic, symlink-aware, mode-preserving, backed up to `~/.clashofmeat/backup/`.

Modules are compiled in, not WASM: there are no third-party authors, and the pure interface keeps a later move to WASM
open.

| Module | Source | Covers |
|---|---|---|
| `claude-code` | `$CLAUDE_CONFIG_DIR` or `~/.claude/projects/**/*.jsonl` | CLI, VS Code and JetBrains extensions |
| `claude-cowork` | `~/Library/Application Support/Claude/local-agent-mode-sessions/**/audit.jsonl` | Claude desktop agent mode |
| `codex` | `$CODEX_HOME` or `~/.codex/{sessions,archived_sessions}` | CLI, IDE extension, app |
| `gemini` | `~/.gemini/tmp/*/chats` (JSONL and legacy JSON) | Gemini CLI |
| `cursor` | spool `~/.clashofmeat/spool/cursor.jsonl`, written by its `stop` hook in `~/.cursor/hooks.json` | Cursor IDE agent |

Every module reports `provider` and `model` (today only Claude does). Parsing rules come from
`guildbyte-app/docs/provider-observer-plan.md` §3 and the current JS parsers.

## 3. Hero overlay

```
┌──────────────────────────────── main screen ───────────────────────────────┐
│                                                  ┌─────────────────────┐   │
│                                                  │ Lv 12 Barbarian     │   │  click → stats bubble
│                         +350 XP                  │ ███████░░░ 6.2k XP  │   │
│                          ▲                       │ today 1.4M tokens   │   │
│              ◄── 🧍 ────                          └─────────▲───────────┘   │
│                                                             🧍             │
│▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁│
│                      [ Dock ]                                              │
└────────────────────────────────────────────────────────────────────────────┘
```

State machine ported from the plugin (`companionActivity`, `companionFrame`, `visitPose`, level-up and XP canvases):

| Signal | Hero |
|---|---|
| Any provider `Turn` start/step | walks back and forth along the bottom edge, turning at each end |
| `Turn` stop | `talk` briefly, then idle → `sit` after 20 s → `sleep` after 90 s |
| Heartbeat XP increase | pixel-font "+N XP" bubble floats up |
| Level-up / evolve | 18-frame aura effect |
| Heartbeat `visit` | visitor walks in, kisses, walks out |

- Interaction: click the hero for the stats bubble; drag to move (patrol resumes from there); right-click for pause /
  settings / quit overlay. Hidden while a full-screen app is in front. Main screen by default; a setting picks another
  screen or follows the cursor.
- Rendering: the 256 px window moves with the hero, rather than covering the screen. Frames are decoded once into
  `ImageBitmap`s and drawn on a canvas at each clip's frame durations; still frames stop the timer.
- Sprites and animation clips come from the heartbeat (format: `companion-animation.mjs`), cached on disk by the
  daemon so an offline launch still shows the hero.
- The client accepts any hero level and uncapped XP (today's validator rejects level > 5 and XP > 1600, which hides the
  hero for players past level 5).

## 4. Install, migration, update

**Install:** `curl -fsSL https://<app>/install.sh | sh`

1. Download `Clash of Meat.app.tar.gz` (arm64 or x86_64) from the GitHub release; verify sha256 and the minisign
   signature.
2. Install to `~/Applications` (no admin); link `~/.local/bin/clashofmeat` to `Contents/MacOS/clashofmeat`.
3. Write `~/Library/LaunchAgents/com.clashofmeat.daemon.plist` (`RunAtLoad`, `KeepAlive`, `ProcessType=Background`,
   logs to `~/.clashofmeat/logs/`); `launchctl bootout` → `bootstrap gui/$uid` → `kickstart`.
4. Register the app as a login item; run `clashofmeat connect` (opens `/link?code=…`).

**Uninstall:** `clashofmeat uninstall [--keep-data]` reverses each step, restoring the `hooks.json` backup.

**Migration:** the installer finds the old collector (`~/.clashofmeat`, LaunchAgent `com.clashofmeat.collector`),
reuses its installation token and file offsets, then removes its LaunchAgent. If the Claude plugin is installed, the
app asks the user to run `claude plugin uninstall clashofmeat`; the shared stable ids keep any overlap from double
counting.

**Update** (mesh's model plus the web app's staged rollout):

- Tauri updater, minisign key. The public key is compiled in; the private key is a CI secret readable only by a
  separate `sign` job that checks out no code.
- Check 3 s after launch, then every 6 h with up to 30 min of jitter (10 min after a failure). Download and verify in
  the background, then **Update ready · Restart** in Settings. Nothing installs without a click. Below `minimum`
  the app blocks with **Update and restart**; the daemon keeps collecting. A `security` flag highlights the row.
- Endpoint: `GET /api/clients/desktop/update?target={{target}}&arch={{arch}}&current={{current_version}}` with the
  installation bearer token. It reuses the stable/candidate split by `sha256(installationId)` and answers in Tauri's
  format, or `204` for no update.
- After relaunch the app restarts the daemon through `launchctl kickstart` once no upload is in flight. App, CLI and
  daemon move together. Versions only go up; a rollback is a newer version built from old code.

v1 limit: no Developer ID signing or notarization; `curl` downloads are not quarantined by Gatekeeper.

## 5. Web app changes

Land on a branch off `guildbyte-app` `onboarding` (it carries the rename, client versioning, `/api/clients` and
`/api/installations/plugin`), in a separate worktree; merge `origin/main` in at integration.

| Area | Change |
|---|---|
| Handshake | `desktop` in `CLIENT_NAMES`; accept `x-clashofmeat-client` and `x-guildbyte-client` during the rename |
| Releases | `CLIENT_RELEASES.desktop = { minimum, stable, candidate }`; assets only from `github.com/PromptWar/clashofmeat-client/releases/download/desktop-v<ver>/…` |
| Update route | `GET /api/clients/desktop/update` (§4) |
| Installer route | `GET /install.sh` with the stable version and app URL filled in |
| Setup page | the `curl … \| sh` command; plugin instructions move to "legacy" |
| Pairing | `tools` lists detected providers (existing field) |
| Heartbeat | unchanged |
| Plugin phase-out | once desktop is stable, plugin users see a switch notice; later `*_PLUGIN_MINIMUM` turns old plugins away |
| Docs | `docs/releasing.md` (desktop channel, signing key), `docs/app-api.md` (update and install routes) |

## 6. Testing

- **Provider parity:** a script in `guildbyte-plugin` runs today's JS parsers over sanitized transcript fixtures and
  writes expected signals as JSON; `cargo test -p providers` must reproduce them exactly (ids and values).
- **Core:** reader (partial lines, truncation, rotation), store migrations, upload retry/ack, stable-id compatibility,
  `hooks.json` write and rollback.
- **Daemon end to end:** temp `HOME` with fake provider roots and a fake web server; appended transcript lines must
  yield the right socket events and uploads.
- **Overlay:** `clashofmeat daemon --fake` replays scripted states (busy, idle, XP, level-up, visit); view-model unit
  tests in Rust.
- **Against the real web app:** local app on port 3100+ with local docker Postgres: pairing, observations,
  heartbeat, and an update to a locally signed 0.9.1.
- **Resource budget:** a script measures daemon RSS/CPU idle and overlay RSS/CPU walking/sleeping, and fails over
  budget.
- **CI:** PR job runs `cargo test`, `clippy` and parity checks. Release workflow is manual dispatch with a separate
  signing job.

## 7. Milestones

1. **Core, daemon, Claude Code:** store, reader, sync, pairing, CLI, Claude parity. Replaces the collector for Claude
   users.
2. **Other four providers** and migration from the old collector.
3. **Tauri app:** overlay, menu bar, settings, hero state machine.
4. **Install and update:** installer, LaunchAgent, updater, release workflow, web routes on `onboarding`.
5. **Cutover:** setup page switches to the new install command; plugin phase-out.

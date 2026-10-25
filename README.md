# Clash of Meat desktop client

Clash of Meat turns the work of your AI coding agents into a game. This repo is the desktop client. It puts a small
hero on your screen that walks along the bottom edge while your agents work, and it counts the tokens they use.

The client reads the local session files of these agents:

- Claude Code
- Claude Cowork
- Codex
- Cursor (through a hook the client adds to Cursor's settings)
- Gemini CLI

It counts supported activity on your machine and sends content-free counts to your Clash of Meat account. Cursor hooks report tasks, prompts and tools; exact tokens are unavailable unless the hook supplies usage. Codex session logs also report usage-limit windows when present. Other providers show limits as unavailable rather than estimating them. Your hero gains XP from them.

## Platforms

- macOS 12 or later (Apple Silicon and Intel)
- Windows 10 version 1803 or later, and Windows 11 (x64; ARM64 PCs run it under emulation)

## Install

Sign in to the Clash of Meat web app and open its setup page.

- **Mac:** copy the install command from the setup page and run it in Terminal. It installs the app in
  `~/Applications`, the `clashofmeat` command in `~/.local/bin`, and links the app to your account.
- **Windows:** download the setup `.exe` from the setup page and run it. It installs for your user only.

To remove the client on a Mac, run `clashofmeat uninstall`. On Windows, uninstall it from Settings > Apps.

## Multiple accounts

In client Settings, expand **Additional accounts** and add the local folder used by each Claude Code, Codex or Gemini CLI account. The normal account remains enabled. Use the account's `.claude` or `.codex` directory; for Gemini, use the home containing `.gemini` (the value of `GEMINI_CLI_HOME`). Up to eight extra homes per agent are watched simultaneously. Removing a home stops collecting it and preserves its queued counts. Account folder paths stay local; uploads use a random profile ID per folder, kept in `~/.clashofmeat/config.json`.

On a Mac, if your default account lives elsewhere through `CLAUDE_CONFIG_DIR`, `CODEX_HOME` or `GEMINI_CLI_HOME`, have those variables set in the shell that runs the install command: the background service keeps the values it had then. After changing them, run `clashofmeat install` again, or add the folders under **Additional accounts**.

## Privacy

The client reads agent session files, but it never uploads their content. It keeps a local database of counts and
syncs it to your account.

What it sends, and where (every request goes to the Clash of Meat web app it was linked to, over HTTPS, with the header
`x-clashofmeat-client: desktop/<version>` and, once linked, the installation token):

- **Observations** (`POST /api/observations`, at most every 30 s while an agent works, then once shortly after it
  stops). Each one has a hashed ID, the agent's session ID (or a hash of it), a timestamp in milliseconds, a key, a
  number, whether it is live or history, the provider, and for Claude the model family. The keys are:
  - token counts per response: `input_tokens`, `output_tokens`, `cache_read_tokens`, `cache_write_tokens`, and
    `model_step_count` (one per response);
  - `prompt_count` (one per prompt you send) and `prompt_chars`, the length of each prompt in characters (Claude Code,
    Claude Cowork and Cursor). The length is computed locally; the text is never read into the upload;
  - `session_count`, `tool_call_count`, `subagent_count` and `compaction_count` (one per session, tool call, subagent
    launch and context compaction; Codex, Gemini CLI and Cursor send `prompt_count` and `subagent_count` only);
  - Codex usage limits, when its logs report them: `five_hour_percent` and `seven_day_percent` readings with their
    reset time, and `five_hour_limit_hit_count` / `seven_day_limit_hit_count` when a window reaches 100%;
  - `agentId` on observations from a Claude Code subagent: the subagent's short ID from its transcript, so parallel
    subagents of one session can be told apart. Only sent to a server that announces support for it.
- **Task summaries** (`POST /api/observer/tasks`), one per finished agent task: the provider, a source key
  (`collector`, a Claude Cowork workspace ID, or a random ID for an additional account folder), the agent's own CLI
  version, the task and session IDs, the model family and release label, start and end time, duration, status, token
  totals, the number of model calls, and how many tool calls, subagent calls and compactions the task had, counted by
  kind (read, write, test, web, other).
- **When you link the app** (`POST /api/pairings`, `POST /api/pairings/exchange`): your computer's name (so you can tell
  your devices apart on the web app) and the list of agents found on it.
- **Heartbeats** (`POST /api/installations/heartbeat`, every 30 s while an agent works and every 60 s otherwise, or as
  the server asks): the revision of the hero art the client already has, a random ID for this running app (so friends'
  visits only go to an app that is open), and the visits it has played.
- **The live channel** (`GET /api/live`): an open connection that tells the client when something changed on the web.
- **Hero actions** (`/api/companion/evolve`, `/level-up`, `/friends`, `/kiss`): the character ID and level, the old and
  new level, and a friend's handle and emote when you send your hero to visit.
- **Update checks** (`GET /api/clients/desktop/update`, every 6 hours): the platform, the architecture and the current
  version. Update downloads come from this repo's GitHub releases and carry no token.
- **Disconnecting** (`DELETE /api/installations/self`): `clashofmeat uninstall` and `clashofmeat connect --force`
  revoke this computer's token.

What it never sends: prompts, answers, code, file names or paths, tool inputs or outputs, project names, account
folder paths, and anything from before you linked the app.

The installation token is stored with the address of the app that issued it and is never sent anywhere else: if the
app address changes, the token is dropped and the app has to be linked again.

The app itself is downloaded from this repo's GitHub releases. The Mac install command checks the download against the
SHA-256 checksum in the release manifest and, when `minisign` is installed, against the release signature (see
[install/README.md](install/README.md)). Automatic updates must carry a valid signature.

## Build from source

See [docs/building.md](docs/building.md). In short, you need Rust, the Tauri CLI and Node.js, and
`scripts/check.sh` runs all the tests.

## Repo layout

| Path | What it holds |
|---|---|
| `crates/core` | Shared types: the provider interface, the local SQLite store, IDs, the file reader. |
| `crates/providers` | One module per agent: where its session files live and how to read token counts from them. |
| `crates/daemon` | The background process: watches the session files, keeps counts, syncs with the web app. |
| `crates/cli` | The `clashofmeat` command: install, connect, status, sync, uninstall, and the Cursor hook. |
| `apps/desktop` | The Tauri app: the hero overlay and the settings window (`ui/` is plain HTML, CSS and JS). |
| `install` | The macOS install script served by the web app. |
| `scripts` | Check, package and release scripts. |
| `fixtures` | Synthetic agent session files for the provider tests. They hold no real session data. |
| `docs` | The build guide and the design notes. |

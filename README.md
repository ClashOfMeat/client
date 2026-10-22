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

In client Settings, expand **Additional accounts** and add the local folder used by each Claude Code, Codex or Gemini CLI account. The normal account remains enabled. Use the account's `.claude` or `.codex` directory; for Gemini, use the home containing `.gemini` (the value of `GEMINI_CLI_HOME`). Up to eight extra homes per agent are watched simultaneously. Removing a home stops collecting it and preserves its queued counts. Account folder paths stay local; uploads use opaque profile identifiers.

## Privacy

The client reads agent session files, but it never uploads their content. It keeps a local database of counts and
syncs it to your account.

What it sends:

- **Token counts**, per response: input, output, cache read and cache write tokens, with the model name, the
  provider, a session ID and a timestamp.
- **Task summaries**, one per finished agent task: start and end time, duration, status, model family, token totals,
  and how many tool calls, subagent calls and compactions the task had, counted by kind (read, write, test, web,
  other).
- **When you link the app:** your computer's name (so you can tell your devices apart on the web app) and the list of
  agents found on it.
- **Heartbeats and updates:** the client version and the revision of the hero art it already has.
- **Hero events:** the old and new level when your hero levels up or evolves.

What it never sends: prompts, answers, code, file names or paths, tool inputs or outputs, and project names.

The client talks only to the Clash of Meat web app it was linked to, over HTTPS. The app itself is downloaded from
this repo's GitHub releases. The Mac install command checks the download against the SHA-256 checksum in the release
manifest, and automatic updates must carry a valid signature.

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

# Installer (macOS and Linux)

The web app serves `install.sh` after replacing its single `__CLASHOFMEAT_APP_URL__` and `__CLASHOFMEAT_VERSION__` placeholders with its own URL and the stable `x.y.z` release. A release copy can also be run with `sh install/install.sh --app https://example.com --version 1.2.3`; `CLASHOFMEAT_APP_URL` supplies the app URL when `--app` is absent. Remaining arguments go to `clashofmeat install`.

For version `x.y.z`, the installer reads `${CLASHOFMEAT_RELEASES_URL:-https://github.com/ClashOfMeat/client/releases}/download/desktop-vx.y.z/manifest.json`. The release contains:

```json
{
  "version": "x.y.z",
  "assets": {
    "aarch64-apple-darwin": {
      "file": "Clash.of.Meat_x.y.z_aarch64.app.tar.gz",
      "sha256": "<64 lowercase hex digits>",
      "size": 12345
    },
    "x86_64-apple-darwin": {
      "file": "Clash.of.Meat_x.y.z_x86_64.app.tar.gz",
      "sha256": "<64 lowercase hex digits>",
      "size": 12345
    },
    "x86_64-unknown-linux-musl": {
      "file": "clashofmeat_x.y.z_linux-x86_64.tar.gz",
      "sha256": "<64 lowercase hex digits>",
      "size": 12345
    },
    "aarch64-unknown-linux-musl": {
      "file": "clashofmeat_x.y.z_linux-aarch64.tar.gz",
      "sha256": "<64 lowercase hex digits>",
      "size": 12345
    }
  }
}
```

On macOS, `install.sh` reads only the two macOS entries (Linux uses each archive's `.sha256`, see below). `scripts/update-manifest.sh` writes this file during a release. Each tarball contains `Clash of Meat.app`. The installer checks the requested version, download size, SHA-256 and, when `minisign` is installed, the release signature before installing to `${CLASHOFMEAT_APPS_DIR:-$HOME/Applications}` and linking the CLI in `${CLASHOFMEAT_BIN_DIR:-$HOME/.local/bin}`. It then runs the app's `clashofmeat install` command, which sets up the background service and, when the computer isn't linked yet, waits for Enter, opens the app's link page and waits until the browser approves the computer (the daemon listens on `127.0.0.1` for that approval; from another device, the player types the code the terminal shows). Enter is read from `/dev/tty`, since stdin is the download under `curl | sh`; `--no-connect` skips linking, and `CLASHOFMEAT_NO_OPEN=1` keeps the browser closed. If the background service isn't reachable, the installer says to run `clashofmeat connect` later.

## What the installer checks

- The SHA-256 and size from `manifest.json`. They come from the same GitHub release as the archive, so they only prove
  the download is intact, not who built it.
- The archive's minisign signature (`<file>.sig`, the same base64 `.sig` the updater uses), checked against the updater
  public key built into `install.sh` (`UPDATER_KEY`, which must match `apps/desktop/src-tauri/updater.pub`). This needs
  `minisign` on `PATH` (`brew install minisign`). Without it the installer keeps the checksum check and prints a note.
- The whole script runs from `main`, called on its last line, so a download cut off mid-transfer runs nothing.

Someone who can replace the release's assets can also replace the `install.sh` in it, key included. The stronger
setup, for the web app to adopt:

1. Serve `install.sh` from the web app's own repo, not from the release, with the updater public key built in there.
   Then a hijacked GitHub release alone cannot change the key the script trusts.
2. Have the script refuse to install when it cannot check the signature, either by shipping a pinned verifier (for
   example a small `clashofmeat verify` bootstrap whose checksum is in the served script) or by having the web app
   check a signature over `manifest.json` before it serves the script.

Run the local release-server test and shell syntax check with:

```sh
node --test 'install/tests/*.test.mjs'
sh -n install/install.sh
```

## Linux

On Linux (`uname -s` is `Linux`) the installer takes the `install_linux` branch. It does not read `manifest.json`
(macOS's `plutil` is not there): it downloads `clashofmeat_<version>_linux-<x86_64|aarch64>.tar.gz` and its `.sha256`,
checks that the checksum file names that archive and matches it (`sha256sum`, or `shasum`), checks the `.sig` with
`minisign` when it is installed, then installs the `clashofmeat` binary in
`${XDG_DATA_HOME:-~/.local/share}/clashofmeat/bin` (where `clashofmeat update` expects it), links it from
`${CLASHOFMEAT_BIN_DIR:-$HOME/.local/bin}` and runs `clashofmeat install`, which sets up the systemd user service
(see [docs/building.md](../docs/building.md), section 7). `CLASHOFMEAT_TEST_UNAME=Linux` runs this branch on any Unix
for the installer tests.

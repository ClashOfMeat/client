# macOS desktop installer

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
    "x86_64-pc-windows-msvc": {
      "file": "Clash.of.Meat_x.y.z_x64-setup.exe",
      "sha256": "<64 lowercase hex digits>",
      "size": 12345
    }
  }
}
```

`install.sh` only reads the two macOS entries; Windows users download the `-setup.exe` directly (see [docs/building.md](../docs/building.md)). `scripts/update-manifest.sh` writes this file during a release. Each tarball contains `Clash of Meat.app`. The installer checks the requested version, download size, SHA-256 and, when `minisign` is installed, the release signature before installing to `${CLASHOFMEAT_APPS_DIR:-$HOME/Applications}` and linking the CLI in `${CLASHOFMEAT_BIN_DIR:-$HOME/.local/bin}`. It then runs the app's `clashofmeat install` command.

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

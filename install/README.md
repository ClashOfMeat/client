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

`install.sh` only reads the two macOS entries; Windows users download the `-setup.exe` directly (see [docs/building.md](../docs/building.md)). `scripts/update-manifest.sh` writes this file during a release. Each tarball contains `Clash of Meat.app`. The installer checks the requested version, download size, and SHA-256 before installing to `${CLASHOFMEAT_APPS_DIR:-$HOME/Applications}` and linking the CLI in `${CLASHOFMEAT_BIN_DIR:-$HOME/.local/bin}`. It then runs the app's `clashofmeat install` command.

Run the local release-server test and shell syntax check with:

```sh
node --test 'install/tests/*.test.mjs'
sh -n install/install.sh
```

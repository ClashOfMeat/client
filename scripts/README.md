# Desktop releases

## One-time updater key setup

Install Tauri CLI 2.12.2, then run `cargo tauri signer generate`. Store the generated private key and password as GitHub Actions secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Replace `apps/desktop/src-tauri/updater.pub` with the generated public key and commit that file. Never commit the private key.

## Cut a release

Run **Desktop release** from GitHub Actions with an `x.y.z` version. `min_version` is optional; use it only when older installed apps must update before continuing. Select `security` for a security update. The workflow tests, packages both Mac architectures, signs the updater archives in a job that checks out no repository code, then creates `desktop-v<version>`. The release publishes both archives and their `.sha256` and `.sig` files, plus `install.sh`; it uploads `manifest.json` and `update.json` last. A version tag cannot be reused.

After the release exists, stage it in the web app by setting `CLASHOFMEAT_DESKTOP_CANDIDATE_VERSION`, `CLASHOFMEAT_DESKTOP_CANDIDATE_MANIFEST` (the release's `update.json` URL), and `CLASHOFMEAT_DESKTOP_CANDIDATE_PERCENT` (0–100), then redeploy. To promote, set `CLASHOFMEAT_DESKTOP_STABLE_VERSION` and `CLASHOFMEAT_DESKTOP_STABLE_MANIFEST` to the same version and URL, clear the candidate variables, and redeploy. `CLASHOFMEAT_DESKTOP_MINIMUM` is the separate server-side minimum for normal client calls. See the web app's `docs/releasing.md` section “Desktop app” for rollout and recovery details.

## Local package

On an Apple Silicon Mac with Rust, Tauri CLI 2.12.2, and the macOS build tools installed:

```sh
scripts/package.sh 0.1.0 aarch64-apple-darwin
```

On an Intel Mac use `x86_64-apple-darwin`. Add `--out /path/to/dist` to choose an output directory. The script writes an ad-hoc signed `.app.tar.gz` and `.sha256`; the updater `.sig` is generated only by the isolated release signing job. Run `scripts/update-manifest.sh --self-test` to check manifest generation locally.

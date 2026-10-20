#!/usr/bin/env bash
# Build one macOS app and its installer archive. Updater signatures are made in CI's isolated sign job.
set -euo pipefail

die() { echo "package: $*" >&2; exit 1; }
[[ $# -eq 2 || $# -eq 4 ]] || die "usage: $0 <version> <target-triple> [--out DIR]"
version=$1
target=$2
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "version must be x.y.z"
case "$target" in
  aarch64-apple-darwin) arch=aarch64 ;;
  x86_64-apple-darwin) arch=x86_64 ;;
  *) die "unsupported target: $target" ;;
esac
root=$(cd "$(dirname "$0")/.." && pwd)
if [[ $# -eq 4 ]]; then
  [[ $3 == --out && -n $4 ]] || die "usage: $0 <version> <target-triple> [--out DIR]"
  out=$4
else
  out="$root/dist/$version"
fi
mkdir -p "$out"
out=$(cd "$out" && pwd)

cargo build --release -p clashofmeat-cli --target "$target"
sidecar_dir="$root/apps/desktop/src-tauri/binaries"
mkdir -p "$sidecar_dir"
cp "$root/target/$target/release/clashofmeat" "$sidecar_dir/clashofmeat-$target"
chmod 755 "$sidecar_dir/clashofmeat-$target"

# Keep signing secrets out of Tauri's build; only the separate sign job receives them.
(
  unset TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PATH TAURI_SIGNING_PRIVATE_KEY_PASSWORD
  cd "$root/apps/desktop/src-tauri"
  cargo tauri build --bundles app --target "$target" --config "{\"version\":\"$version\"}"
)
app="$root/apps/desktop/src-tauri/target/$target/release/bundle/macos/Clash of Meat.app"
[[ -x "$app/Contents/MacOS/clashofmeat" ]] || die "bundle lacks executable Contents/MacOS/clashofmeat: $app"
codesign --force --deep --sign - "$app"

name="Clash.of.Meat_${version}_${arch}.app.tar.gz"
COPYFILE_DISABLE=1 tar -czf "$out/$name" -C "$(dirname "$app")" "$(basename "$app")"
hash=$(shasum -a 256 "$out/$name" | cut -d ' ' -f 1)
printf '%s  %s\n' "$hash" "$name" >"$out/$name.sha256"
echo "package: $out/$name"

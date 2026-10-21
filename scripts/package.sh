#!/usr/bin/env bash
# Build one desktop installer (macOS .app.tar.gz or Windows NSIS -setup.exe), its .sha256 and, when a
# signing key is set, its updater .sig. See docs/building.md.
set -euo pipefail

die() { echo "package: $*" >&2; exit 1; }
usage="usage: $0 <version> <target-triple> [--out DIR] [--app URL]"
(( $# >= 2 )) || die "$usage"
version=$1
target=$2
shift 2
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "version must be x.y.z"
case "$target" in
  aarch64-apple-darwin) os=macos; arch=aarch64 ;;
  x86_64-apple-darwin) os=macos; arch=x86_64 ;;
  x86_64-pc-windows-msvc) os=windows; arch=x64 ;;
  *) die "unsupported target: $target" ;;
esac
root=$(cd "$(dirname "$0")/.." && pwd)
out="$root/dist/$version"
while (( $# )); do
  case "$1" in
    --out)
      [[ $# -ge 2 && -n $2 ]] || die "$usage"
      out=$2
      shift 2 ;;
    --app)
      [[ $# -ge 2 && -n $2 ]] || die "$usage"
      CLASHOFMEAT_DEFAULT_APP_URL=$2
      shift 2 ;;
    *) die "$usage" ;;
  esac
done
# The CLI compiles this in (option_env!) as the app URL used when `clashofmeat install` gets no --app.
if [[ -n ${CLASHOFMEAT_DEFAULT_APP_URL:-} ]]; then
  CLASHOFMEAT_DEFAULT_APP_URL=${CLASHOFMEAT_DEFAULT_APP_URL%/}
  [[ $CLASHOFMEAT_DEFAULT_APP_URL == https://?* || $CLASHOFMEAT_DEFAULT_APP_URL =~ ^http://(localhost|127\.0\.0\.1)(:[0-9]+)?$ ]] ||
    die "app URL must be https:// (http:// only for localhost or 127.0.0.1): $CLASHOFMEAT_DEFAULT_APP_URL"
  export CLASHOFMEAT_DEFAULT_APP_URL
  echo "package: default app URL $CLASHOFMEAT_DEFAULT_APP_URL"
else
  unset CLASHOFMEAT_DEFAULT_APP_URL
  if [[ $os == windows ]]; then
    echo "package: warning: no --app or CLASHOFMEAT_DEFAULT_APP_URL; the Windows installer runs 'clashofmeat install' without --app" >&2
  fi
fi

mkdir -p "$out"
out=$(cd "$out" && pwd)

# Decide before the long build whether this run can sign.
sign=false
if [[ -n ${TAURI_SIGNING_PRIVATE_KEY:-} ]]; then
  sign=true
elif [[ -n ${TAURI_SIGNING_PRIVATE_KEY_PATH:-} ]]; then
  [[ -s $TAURI_SIGNING_PRIVATE_KEY_PATH ]] || die "TAURI_SIGNING_PRIVATE_KEY_PATH is not a readable key: $TAURI_SIGNING_PRIVATE_KEY_PATH"
  sign=true
fi

if [[ $os == windows ]]; then
  command -v cargo-xwin >/dev/null || die "cargo-xwin is missing: cargo install --locked cargo-xwin"
  command -v makensis >/dev/null || die "makensis is missing: brew install nsis"
  # Homebrew keeps llvm (clang-cl, llvm-lib) and lld (lld-link) keg-only, so put both on PATH here.
  if command -v brew >/dev/null; then
    for formula in lld llvm; do
      prefix=$(brew --prefix "$formula" 2>/dev/null) && [[ -d $prefix/bin ]] && PATH="$prefix/bin:$PATH"
    done
    export PATH
  fi
  for tool in clang-cl llvm-lib lld-link; do
    command -v "$tool" >/dev/null || die "$tool is missing: brew install llvm lld"
  done
  cargo xwin build --release -p clashofmeat-cli --target "$target"
  sidecar_src="$root/target/$target/release/clashofmeat.exe"
  sidecar_name="clashofmeat-$target.exe"
else
  cargo build --release -p clashofmeat-cli --target "$target"
  sidecar_src="$root/target/$target/release/clashofmeat"
  sidecar_name="clashofmeat-$target"
fi
sidecar_dir="$root/apps/desktop/src-tauri/binaries"
mkdir -p "$sidecar_dir"
cp "$sidecar_src" "$sidecar_dir/$sidecar_name"
chmod 755 "$sidecar_dir/$sidecar_name"

# Keep signing keys out of Tauri's build; the finished artifact is signed below.
(
  unset TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PATH TAURI_SIGNING_PRIVATE_KEY_PASSWORD \
    TAURI_PRIVATE_KEY TAURI_PRIVATE_KEY_PATH TAURI_PRIVATE_KEY_PASSWORD
  cd "$root/apps/desktop/src-tauri"
  if [[ $os == windows ]]; then
    cargo tauri build --runner cargo-xwin --target "$target" --bundles nsis --config "{\"version\":\"$version\"}"
  else
    cargo tauri build --bundles app --target "$target" --config "{\"version\":\"$version\"}"
  fi
)

bundle="$root/apps/desktop/src-tauri/target/$target/release/bundle"
if [[ $os == windows ]]; then
  setup="$bundle/nsis/Clash of Meat_${version}_${arch}-setup.exe"
  [[ -s $setup ]] || die "missing NSIS installer: $setup"
  name="Clash.of.Meat_${version}_${arch}-setup.exe"
  cp "$setup" "$out/$name"
else
  app="$bundle/macos/Clash of Meat.app"
  [[ -x "$app/Contents/MacOS/clashofmeat" ]] || die "bundle lacks executable Contents/MacOS/clashofmeat: $app"
  codesign --force --deep --sign - "$app"
  name="Clash.of.Meat_${version}_${arch}.app.tar.gz"
  COPYFILE_DISABLE=1 tar -czf "$out/$name" -C "$(dirname "$app")" "$(basename "$app")"
fi
hash=$(shasum -a 256 "$out/$name" | cut -d ' ' -f 1)
printf '%s  %s\n' "$hash" "$name" >"$out/$name.sha256"

rm -f "$out/$name.sig"
if [[ $sign == true ]]; then
  cargo tauri signer sign --app-version "$version" "$out/$name" >/dev/null
  [[ -s "$out/$name.sig" ]] || die "signing produced no $out/$name.sig"
  echo "package: signed $out/$name.sig"
else
  echo "package: note: TAURI_SIGNING_PRIVATE_KEY_PATH / TAURI_SIGNING_PRIVATE_KEY unset; no updater .sig for $name"
fi
echo "package: $out/$name"

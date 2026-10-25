#!/bin/sh
# The web app substitutes the app address and stable version before serving this installer.
# A release copy can instead be run with: sh install.sh --app <url> --version <x.y.z>
set -eu

fail() { printf 'Clash of Meat: %s\n' "$1" >&2; exit 1; }
say() { printf 'Clash of Meat: %s\n' "$1"; }

# The updater's minisign public key (apps/desktop/src-tauri/updater.pub); release archives carry its .sig.
UPDATER_KEY='RWRP6ClwSA6d5cZZJmyTzyqamZG0SXQ/Y5SlJ5CNEVmv16lrKqQfcG0P'

# Everything runs from main, called on the last line, so a download cut off mid-transfer runs nothing.
main() {
  DEFAULT_APP='__CLASHOFMEAT_APP_URL__'
  DEFAULT_VERSION='__CLASHOFMEAT_VERSION__'
  APP="${CLASHOFMEAT_APP_URL:-}"
  VERSION=''
  case "$DEFAULT_APP" in __*) ;; *) [ -n "$APP" ] || APP="$DEFAULT_APP" ;; esac
  case "$DEFAULT_VERSION" in __*) ;; *) VERSION="$DEFAULT_VERSION" ;; esac

  # Rotate unrecognised arguments to the end, preserving their order for `clashofmeat install`.
  remaining=$#
  while [ "$remaining" -gt 0 ]; do
    argument=$1
    shift
    remaining=$((remaining - 1))
    case "$argument" in
      --app|--version)
        [ "$remaining" -gt 0 ] || fail "$argument needs a value"
        value=$1
        shift
        remaining=$((remaining - 1))
        [ -n "$value" ] || fail "$argument needs a value"
        if [ "$argument" = --app ]; then APP=$value; else VERSION=$value; fi
        ;;
      *) set -- "$@" "$argument" ;;
    esac
  done

  [ -n "$APP" ] || fail 'Use the command from your Clash of Meat setup page, or pass --app <url>.'
  case "$APP" in
    https://?*|http://localhost|http://localhost:|http://localhost/*|http://localhost:[0-9]*|\
    http://127.0.0.1|http://127.0.0.1/*|http://127.0.0.1:[0-9]*) ;;
    *) fail "--app must be an https:// address (http:// is allowed only for localhost or 127.0.0.1): $APP" ;;
  esac
  APP=${APP%/}
  [ -n "$VERSION" ] || fail 'A desktop release version is required; pass --version <x.y.z>.'
  printf '%s\n' "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' || fail 'the desktop version must be x.y.z'

  [ "$(uname -s)" = Darwin ] || fail 'the Clash of Meat desktop app is macOS-only for now'
  case "$(uname -m)" in
    arm64|aarch64) target=aarch64-apple-darwin ;;
    x86_64|amd64)
      target=x86_64-apple-darwin
      if [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || printf 0)" = 1 ]; then
        target=aarch64-apple-darwin
      fi
      ;;
    *) fail "no Clash of Meat desktop build for $(uname -m)" ;;
  esac

  command -v curl >/dev/null 2>&1 || fail 'curl is required'
  command -v shasum >/dev/null 2>&1 || fail 'shasum is required'
  fetch() { curl -fsSL --proto '=https,http' --proto-redir '=https' --retry 2 -o "$2" "$1"; }
  sha256() { shasum -a 256 "$1" | cut -d ' ' -f 1; }
  read_manifest() { /usr/bin/plutil -extract "$1" raw -o - "$2" 2>/dev/null; }

  RELEASES=${CLASHOFMEAT_RELEASES_URL:-https://github.com/ClashOfMeat/client/releases}
  RELEASES=${RELEASES%/}
  APPS_DIR=${CLASHOFMEAT_APPS_DIR:-$HOME/Applications}
  BIN_DIR=${CLASHOFMEAT_BIN_DIR:-$HOME/.local/bin}
  APP_PATH=$APPS_DIR/Clash\ of\ Meat.app
  LINK=$BIN_DIR/clashofmeat

  # Refuse this before modifying an existing installation.
  if [ -e "$LINK" ] && [ ! -L "$LINK" ]; then
    fail "$LINK is a regular file or directory; move it aside before installing"
  fi

  work=$(mktemp -d "${TMPDIR:-/tmp}/clashofmeat-install.XXXXXX") || fail 'could not create a temporary directory'
  stage=''
  cleanup() {
    status=$?
    trap - 0
    if [ -n "$stage" ] && [ -d "$stage" ]; then
      if [ -e "$stage/old" ] && [ ! -e "$APP_PATH" ]; then mv "$stage/old" "$APP_PATH"; fi
      rm -rf "$stage"
    fi
    rm -rf "$work"
    exit "$status"
  }
  trap 'cleanup' 0
  trap 'exit 1' 1 2 3 15

  base=$RELEASES/download/desktop-v$VERSION
  say "Downloading desktop release $VERSION for ${target}..."
  fetch "$base/manifest.json" "$work/manifest.json" || fail 'could not download the desktop release manifest'
  manifest_version=$(read_manifest version "$work/manifest.json") || fail 'the release manifest has no version'
  [ "$manifest_version" = "$VERSION" ] || fail "the release manifest version ($manifest_version) does not match $VERSION"
  asset="assets.$target"
  file=$(read_manifest "$asset.file" "$work/manifest.json") || fail "the release has no $target build"
  expected=$(read_manifest "$asset.sha256" "$work/manifest.json") || fail 'the release asset has no sha256'
  size=$(read_manifest "$asset.size" "$work/manifest.json") || fail 'the release asset has no size'
  [ "$file" = "Clash.of.Meat_${VERSION}_${target%%-apple-darwin}.app.tar.gz" ] || fail "unexpected release asset name: $file"
  case "$expected" in *[!0-9a-f]*|'') fail 'invalid sha256 in the release manifest' ;; esac
  [ "${#expected}" -eq 64 ] || fail 'invalid sha256 in the release manifest'
  case "$size" in *[!0-9]*|'') fail 'invalid size in the release manifest' ;; esac

  fetch "$base/$file" "$work/app.tar.gz" || fail "could not download $file"
  [ "$(wc -c < "$work/app.tar.gz" | tr -d '[:space:]')" = "$size" ] || fail 'download size does not match the release manifest; nothing was installed'
  [ "$(sha256 "$work/app.tar.gz")" = "$expected" ] || fail 'download sha256 does not match the release manifest; nothing was installed'
  # The checksum comes from the same release as the archive, so it only proves the download is intact. The
  # signature proves who built it: it must match the updater key, which only the release signer holds.
  if minisign=$(command -v minisign 2>/dev/null); then
    fetch "$base/$file.sig" "$work/app.sig" || fail "could not download $file.sig"
    base64 --decode < "$work/app.sig" > "$work/app.minisig" 2>/dev/null || fail 'the release signature is not valid base64'
    "$minisign" -V -q -P "$UPDATER_KEY" -m "$work/app.tar.gz" -x "$work/app.minisig" >/dev/null 2>&1 ||
      fail 'the download is not signed with the Clash of Meat release key; nothing was installed'
    say 'Download verified (SHA-256 and release signature).'
  else
    say 'Download verified (SHA-256 from the release manifest).'
    say 'Note: install minisign (brew install minisign) and run this again to also check the release signature.'
  fi

  mkdir "$work/unpacked"
  tar -xzf "$work/app.tar.gz" -C "$work/unpacked" || fail 'could not extract the desktop app'
  [ -x "$work/unpacked/Clash of Meat.app/Contents/MacOS/clashofmeat" ] || fail 'the release archive has no executable Clash of Meat app'
  mkdir -p "$APPS_DIR" "$BIN_DIR"
  stage=$(mktemp -d "$APPS_DIR/.clashofmeat-install.XXXXXX") || fail 'could not stage the desktop app'
  mv "$work/unpacked/Clash of Meat.app" "$stage/new" || fail 'could not stage the desktop app'
  if [ -e "$APP_PATH" ]; then mv "$APP_PATH" "$stage/old" || fail 'could not move the previous desktop app'; fi
  mv "$stage/new" "$APP_PATH" || fail 'could not install the desktop app'
  rm -rf "$stage"
  stage=''
  xattr -dr com.apple.quarantine "$APP_PATH" 2>/dev/null || true

  if [ -L "$LINK" ]; then rm "$LINK" || fail "could not replace $LINK"; fi
  ln -s "$APP_PATH/Contents/MacOS/clashofmeat" "$LINK" || fail "could not create $LINK"
  say "Installed desktop app $VERSION in $APP_PATH."
  case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) say "Run the CLI with $LINK (its directory is not on PATH)." ;;
  esac

  exec "$APP_PATH/Contents/MacOS/clashofmeat" install --app "$APP" "$@"
}

main "$@"

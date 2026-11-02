#!/bin/sh
# The web app substitutes the app address and stable version before serving this installer.
# A release copy can instead be run with: sh install.sh --app <url> --version <x.y.z>
set -eu

fail() { printf 'Clash of Meat: %s\n' "$1" >&2; exit 1; }
say() { printf 'Clash of Meat: %s\n' "$1"; }

# The updater's minisign public key (apps/desktop/src-tauri/updater.pub); release archives carry its .sig.
UPDATER_KEY='RWSC/TfveVpY2OTPXC0Xtm3TKi6XKYNZIc4H2Mu2JZ0GO3V3n+tTFSXp'

fetch() { curl -fsSL --proto '=https,http' --proto-redir '=https' --retry 2 -o "$2" "$1"; }

# check_signature <archive> <signature url> <file name> <checksum source> <minisign install hint>
# The checksum comes from the same release as the archive, so it only proves the download is intact. The
# signature proves who built it: it must match the updater key, which only the release signer holds.
check_signature() {
  if minisign=$(command -v minisign 2>/dev/null); then
    fetch "$2" "$work/release.sig" || fail "could not download $3.sig"
    base64 --decode < "$work/release.sig" > "$work/release.minisig" 2>/dev/null || fail 'the release signature is not valid base64'
    "$minisign" -V -q -P "$UPDATER_KEY" -m "$1" -x "$work/release.minisig" >/dev/null 2>&1 ||
      fail 'the download is not signed with the Clash of Meat release key; nothing was installed'
    say 'Download verified (SHA-256 and release signature).'
  else
    say "Download verified (SHA-256 from $4)."
    say "Note: install minisign ($5) and run this again to also check the release signature."
  fi
}

# Linux: the CLI alone (no desktop app yet). It goes to ${XDG_DATA_HOME:-~/.local/share}/clashofmeat/bin, where
# `clashofmeat update` expects it, linked from ~/.local/bin; `clashofmeat install` then sets up the systemd service.
install_linux() {
  case "$(uname -m)" in
    x86_64|amd64) arch=x86_64 ;;
    aarch64|arm64) arch=aarch64 ;;
    *) fail "no Clash of Meat Linux build for $(uname -m)" ;;
  esac
  command -v curl >/dev/null 2>&1 || fail 'curl is required'
  command -v tar >/dev/null 2>&1 || fail 'tar is required'
  if command -v sha256sum >/dev/null 2>&1; then
    sha256() { sha256sum "$1" | cut -d ' ' -f 1; }
  elif command -v shasum >/dev/null 2>&1; then
    sha256() { shasum -a 256 "$1" | cut -d ' ' -f 1; }
  else
    fail 'sha256sum is required'
  fi

  RELEASES=${CLASHOFMEAT_RELEASES_URL:-https://github.com/ClashOfMeat/client/releases}
  RELEASES=${RELEASES%/}
  DATA_DIR=${XDG_DATA_HOME:-}
  case "$DATA_DIR" in /*) ;; *) DATA_DIR=$HOME/.local/share ;; esac
  INSTALL_DIR=$DATA_DIR/clashofmeat/bin
  EXE=$INSTALL_DIR/clashofmeat
  BIN_DIR=${CLASHOFMEAT_BIN_DIR:-$HOME/.local/bin}
  LINK=$BIN_DIR/clashofmeat
  if [ -e "$LINK" ] && [ ! -L "$LINK" ]; then
    fail "$LINK is a regular file or directory; move it aside before installing"
  fi

  work=$(mktemp -d "${TMPDIR:-/tmp}/clashofmeat-install.XXXXXX") || fail 'could not create a temporary directory'
  staged=''
  cleanup_linux() {
    status=$?
    trap - 0
    if [ -n "$staged" ]; then rm -f "$staged"; fi
    rm -rf "$work"
    exit "$status"
  }
  trap 'cleanup_linux' 0
  trap 'exit 1' 1 2 3 15

  file="clashofmeat_${VERSION}_linux-${arch}.tar.gz"
  base=$RELEASES/download/desktop-v$VERSION
  say "Downloading Clash of Meat $VERSION for Linux ${arch}..."
  fetch "$base/$file" "$work/cli.tar.gz" || fail "could not download $file; release $VERSION may have no Linux build"
  fetch "$base/$file.sha256" "$work/cli.sha256" || fail "could not download $file.sha256"
  expected=$(sed -n "s/^\([0-9a-f]\{64\}\)  $file\$/\1/p" "$work/cli.sha256")
  [ -n "$expected" ] || fail "invalid checksum file for $file"
  [ "$(sha256 "$work/cli.tar.gz")" = "$expected" ] || fail 'download sha256 does not match the release checksum; nothing was installed'
  check_signature "$work/cli.tar.gz" "$base/$file.sig" "$file" 'the release checksum' 'apt install minisign, dnf install minisign or pacman -S minisign'

  mkdir "$work/unpacked"
  tar -xzf "$work/cli.tar.gz" -C "$work/unpacked" clashofmeat || fail 'could not extract the Clash of Meat CLI'
  [ -f "$work/unpacked/clashofmeat" ] && [ ! -L "$work/unpacked/clashofmeat" ] || fail 'the release archive has no clashofmeat binary'
  mkdir -p "$INSTALL_DIR" "$BIN_DIR"
  # Staged beside the target, so the final mv is an atomic rename; a running daemon keeps its old copy.
  staged=$INSTALL_DIR/.clashofmeat.new.$$
  cp "$work/unpacked/clashofmeat" "$staged" || fail "could not write to $INSTALL_DIR"
  chmod 755 "$staged"
  mv -f "$staged" "$EXE" || fail "could not install $EXE"
  staged=''

  if [ -L "$LINK" ]; then rm "$LINK" || fail "could not replace $LINK"; fi
  ln -s "$EXE" "$LINK" || fail "could not create $LINK"
  say "Installed Clash of Meat $VERSION in $EXE."
  case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) say "Run the CLI with $LINK (its directory is not on PATH)." ;;
  esac

  exec "$EXE" install --app "$APP" "$@"
}

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

  if [ "${CLASHOFMEAT_TEST_UNAME:-$(uname -s)}" = Linux ]; then install_linux "$@"; fi
  [ "$(uname -s)" = Darwin ] || fail "Clash of Meat runs on macOS and Linux; $(uname -s) is not supported yet"
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
  check_signature "$work/app.tar.gz" "$base/$file.sig" "$file" 'the release manifest' 'brew install minisign'

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

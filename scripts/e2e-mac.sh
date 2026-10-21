#!/usr/bin/env bash
# Local end-to-end test on this Mac: build 0.1.0 and 0.1.1 under a test identifier, sign them with a throwaway key,
# serve them as a release, install 0.1.0 with install/install.sh, start its daemon, get a pairing link from the web app,
# then let the app detect, download, verify and install 0.1.1. A bad signature must be refused.
# Nothing touches an installed Clash of Meat: everything lives in one temp dir, which is removed at the end.
# See docs/building.md, "Local end-to-end test".
set -euo pipefail

die() { echo "e2e: $*" >&2; exit 1; }
note() { echo "e2e: $*"; }
usage="usage: $0 [--app URL] [--port PORT] [--keep]"
root=$(cd "$(dirname "$0")/.." && pwd)
app_url=http://localhost:3110
port=3150
keep=false
while (( $# )); do
  case "$1" in
    --app) [[ $# -ge 2 && -n $2 ]] || die "$usage"; app_url=${2%/}; shift 2 ;;
    --port) [[ $# -ge 2 && $2 =~ ^[0-9]+$ ]] || die "$usage"; port=$2; shift 2 ;;
    --keep) keep=true; shift ;;
    *) die "$usage" ;;
  esac
done
[[ $(uname -s) == Darwin ]] || die "macOS only"
case "$(uname -m)" in arm64) arch=aarch64 ;; x86_64) arch=x86_64 ;; *) die "unsupported CPU $(uname -m)" ;; esac
identifier=com.clashofmeat.e2e
real_home=$HOME
! lsof -nP -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1 || die "port $port is in use; pass --port"
curl -s -o /dev/null --max-time 10 "$app_url/" || die "the web app does not answer at $app_url"

# A short path: the daemon's Unix socket lives in it and macOS caps socket paths at 104 bytes.
work=$(mktemp -d "${TMPDIR:-/tmp}/come2e.XXXXXX")
work=$(cd "$work" && pwd -P)
pids=()
cleanup() {
  local status=$?
  for pid in ${pids[@]+"${pids[@]}"}; do kill "$pid" 2>/dev/null || true; done
  # The app relaunched by the update runs under launchd, not under this script.
  pkill -f "$work/Applications/" 2>/dev/null || true
  # WebKit keeps its data by bundle identifier under the real ~/Library whatever HOME is.
  rm -rf "$real_home/Library/Caches/$identifier" "$real_home/Library/WebKit/$identifier" \
    "$real_home/Library/Application Support/$identifier" "$real_home/Library/Saved Application State/$identifier.savedState"
  if [[ $keep == true ]]; then note "kept $work"; else rm -rf "$work"; fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 1' INT TERM

# Wait for a command to succeed, up to a number of seconds.
until_ok() {
  local seconds=$1 deadline
  shift
  deadline=$(( $(date +%s) + seconds ))
  until "$@" >/dev/null 2>&1; do
    (( $(date +%s) < deadline )) || return 1
    sleep 0.5
  done
}

# Swift helper: list or press a button of a running app through the accessibility API.
cat >"$work/press.swift" <<'EOF'
import ApplicationServices
import Foundation
let pid = pid_t(CommandLine.arguments[1])!
let wanted = CommandLine.arguments[2]
let seconds = Double(CommandLine.arguments[3])!
guard AXIsProcessTrusted() else {
  print("this terminal needs Accessibility access (System Settings > Privacy & Security > Accessibility)")
  exit(2)
}
let app = AXUIElementCreateApplication(pid)
// Ask WebKit to expose the page to accessibility.
AXUIElementSetAttributeValue(app, "AXManualAccessibility" as CFString, kCFBooleanTrue)
func attr(_ e: AXUIElement, _ name: String) -> AnyObject? {
  var v: AnyObject?
  return AXUIElementCopyAttributeValue(e, name as CFString, &v) == .success ? v : nil
}
func find(_ e: AXUIElement, _ depth: Int) -> AXUIElement? {
  if depth > 30 { return nil }
  let texts = ["AXTitle", "AXDescription"].compactMap { attr(e, $0) as? String }
  if attr(e, "AXRole") as? String == "AXButton" && texts.contains(wanted) { return e }
  for child in (attr(e, "AXChildren") as? [AXUIElement]) ?? [] {
    if let hit = find(child, depth + 1) { return hit }
  }
  return nil
}
let deadline = Date().addingTimeInterval(seconds)
while Date() < deadline {
  if let button = find(app, 0) {
    exit(AXUIElementPerformAction(button, kAXPressAction as CFString) == .success ? 0 : 1)
  }
  Thread.sleep(forTimeInterval: 0.5)
}
print("no \"\(wanted)\" button within \(Int(seconds))s")
exit(1)
EOF

note "work dir $work"
note "throwaway updater key"
(cd "$root/apps/desktop/src-tauri" && cargo tauri signer generate --ci -w "$work/e2e.key" >/dev/null 2>&1)
releases="$work/www/releases/download"
base="http://localhost:$port/releases/download"
(
  export TAURI_SIGNING_PRIVATE_KEY_PATH="$work/e2e.key" TAURI_SIGNING_PRIVATE_KEY_PASSWORD=
  CLASHOFMEAT_UPDATE_PUBKEY=$(cat "$work/e2e.key.pub")
  export CLASHOFMEAT_UPDATE_PUBKEY
  # Test builds only: the local server is plain http, which the updater refuses unless this is set.
  config="{\"identifier\":\"$identifier\",\"plugins\":{\"updater\":{\"dangerousInsecureTransportProtocol\":true}}}"
  for version in 0.1.0 0.1.1; do
    for target in aarch64-apple-darwin x86_64-apple-darwin; do
      note "build $version $target"
      "$root/scripts/package.sh" "$version" "$target" --out "$releases/desktop-v$version" --app "$app_url" \
        --tauri-config "$config" >"$work/build.log" 2>&1 || { tail -30 "$work/build.log" >&2; exit 1; }
    done
    "$root/scripts/update-manifest.sh" "$releases/desktop-v$version" "$version" "$base/desktop-v$version/" --mac-only >/dev/null
    cp "$root/install/install.sh" "$releases/desktop-v$version/install.sh"
  done
)

note "serve $base"
python3 -I -m http.server "$port" --bind 127.0.0.1 --directory "$work/www" >"$work/server.log" 2>&1 </dev/null &
pids+=($!)
until_ok 20 curl -fs "$base/desktop-v0.1.0/manifest.json" || die "the release server did not start"

export HOME="$work/home" CLASHOFMEAT_HOME="$work/h" CLASHOFMEAT_APPS_DIR="$work/Applications" \
  CLASHOFMEAT_BIN_DIR="$work/bin" CLASHOFMEAT_LAUNCH_AGENTS_DIR="$work/LaunchAgents" \
  CLASHOFMEAT_RELEASES_URL="http://localhost:$port/releases" CLASHOFMEAT_NO_OPEN=1
mkdir -p "$HOME" "$CLASHOFMEAT_HOME/logs"
bundle="$CLASHOFMEAT_APPS_DIR/Clash of Meat.app"
plist_version() { /usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$bundle/Contents/Info.plist"; }

note "install 0.1.0 with install.sh"
sh "$root/install/install.sh" --app "$app_url" --version 0.1.0 --no-launch-agent --no-connect >"$work/install.log" 2>&1 ||
  { cat "$work/install.log" >&2; die "install.sh failed"; }
[[ $(plist_version) == 0.1.0 ]] || die "installed app is not 0.1.0"
[[ -L $CLASHOFMEAT_BIN_DIR/clashofmeat ]] || die "install.sh did not link the CLI"
[[ ! -e $CLASHOFMEAT_LAUNCH_AGENTS_DIR ]] || die "a LaunchAgent was written despite --no-launch-agent"
note "PASS install: 0.1.0 in $bundle"

"$CLASHOFMEAT_BIN_DIR/clashofmeat" daemon >"$CLASHOFMEAT_HOME/logs/daemon.log" 2>&1 </dev/null &
pids+=($!)
until_ok 20 test -S "$CLASHOFMEAT_HOME/daemon.sock" || die "the daemon socket did not appear"
"$CLASHOFMEAT_BIN_DIR/clashofmeat" connect --no-open >"$work/connect.log" 2>&1 </dev/null &
connect=$!
pids+=("$connect")
until_ok 30 grep -q 'Your code is' "$work/connect.log" || { cat "$work/connect.log" >&2; die "no pairing link"; }
link=$(grep -Eo 'https?://[^ ]+' "$work/connect.log" | head -1)
note "PASS pairing link issued: $link (not confirmed: that needs a signed-in browser)"
kill "$connect" 2>/dev/null || true

# Update. A "minimum" the app is below makes it open Settings by itself, with "Update and restart" once the update
# is downloaded and verified.
python3 -c 'import json, sys; d = json.load(open(sys.argv[1])); d["minimum"] = "0.1.1"; json.dump(d, open(sys.argv[2], "w"))' \
  "$releases/desktop-v0.1.1/update.json" "$releases/desktop-v0.1.1/update-required.json"
executable="$bundle/Contents/MacOS/clashofmeat-desktop"
CLASHOFMEAT_UPDATE_URL="$base/desktop-v0.1.1/update-required.json" "$executable" >"$work/app.log" 2>&1 </dev/null &
app=$!
pids+=("$app")
until_ok 60 grep -q "GET /releases/download/desktop-v0.1.1/Clash.of.Meat_0.1.1_${arch}.app.tar.gz" "$work/server.log" ||
  { cat "$work/app.log" >&2; die "0.1.0 did not download 0.1.1"; }
note "PASS update detected and downloaded"
swift "$work/press.swift" "$app" "Update and restart" 60 || { cat "$work/app.log" >&2; die "could not press Update and restart"; }
note "PASS update verified (Settings shows Update and restart); pressed it"
until_ok 60 sh -c "! kill -0 $app" || die "the app did not quit to install the update"
[[ $(plist_version) == 0.1.1 ]] || { cat "$work/app.log" >&2; die "the installed app is not 0.1.1 after the update"; }
until_ok 30 pgrep -f "$executable" || die "the updated app did not restart"
note "PASS update installed: $(plist_version), relaunched as pid $(pgrep -f "$executable" | head -1)"
pkill -f "$executable" || true
until_ok 20 sh -c "! pgrep -f '$executable'" || true

# A release whose signature does not match its archive must be refused.
python3 -c '
import json, sys
d = json.load(open(sys.argv[1]))
d["version"] = "0.1.2"
d["platforms"]["darwin-aarch64"]["signature"], d["platforms"]["darwin-x86_64"]["signature"] = (
    d["platforms"]["darwin-x86_64"]["signature"], d["platforms"]["darwin-aarch64"]["signature"])
json.dump(d, open(sys.argv[2], "w"))' "$releases/desktop-v0.1.1/update.json" "$releases/desktop-v0.1.1/update-bad.json"
CLASHOFMEAT_UPDATE_URL="$base/desktop-v0.1.1/update-bad.json" "$executable" >"$work/app-bad.log" 2>&1 </dev/null &
pids+=($!)
until_ok 60 grep -q 'signature verification failed' "$work/app-bad.log" || { cat "$work/app-bad.log" >&2; die "a bad signature was not refused"; }
[[ $(plist_version) == 0.1.1 ]] || die "the bad update changed the installed app"
note "PASS bad signature refused"
note "all passed"

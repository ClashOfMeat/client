#!/usr/bin/env bash
# Check, package, sign and publish one desktop release (macOS arm64 + x86_64, Windows x64) as GitHub release
# desktop-v<version>. Everything runs on this Mac; see docs/building.md.
set -euo pipefail

die() { echo "release: $*" >&2; exit 1; }
note() { echo "release: $*"; }
usage="usage: $0 <version> --app <url> [--min-version x.y.z] [--security] [--dry-run] [--out DIR]"
repo=PromptWar/clashofmeat-client
targets=(aarch64-apple-darwin x86_64-apple-darwin x86_64-pc-windows-msvc)
placeholder=REPLACE_WITH_TAURI_UPDATER_PUBLIC_KEY

(( $# >= 1 )) || die "$usage"
version=$1
shift
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "version must be x.y.z"
root=$(cd "$(dirname "$0")/.." && pwd)
app=
min_version=
security=false
dry_run=false
dist="$root/dist/$version"
while (( $# )); do
  case "$1" in
    --app) [[ $# -ge 2 && -n $2 ]] || die "$usage"; app=$2; shift 2 ;;
    --min-version) [[ $# -ge 2 && -n $2 ]] || die "$usage"; min_version=$2; shift 2 ;;
    --security) security=true; shift ;;
    --dry-run) dry_run=true; shift ;;
    --out) [[ $# -ge 2 && -n $2 ]] || die "$usage"; dist=$2; shift 2 ;;
    *) die "$usage" ;;
  esac
done
[[ -n $app ]] || die "--app <url> is required: the Windows installer runs 'clashofmeat install' without --app"
app=${app%/}
[[ $app == https://?* ]] || die "--app must be an https:// URL for a release: $app"
if [[ -n $min_version ]]; then
  [[ $min_version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "--min-version must be x.y.z"
fi
tag="desktop-v$version"
# A real release stops on any problem below; a dry run reports it and carries on.
problem() { if [[ $dry_run == true ]]; then echo "release: dry run: $*" >&2; else die "$*"; fi; }

cd "$root"

# Preflight: everything that can fail without building.
pubkey=$(python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))["plugins"]["updater"]["pubkey"])' \
  apps/desktop/src-tauri/tauri.conf.json)
if [[ -z $pubkey || $pubkey == "$placeholder" ]]; then
  problem "set plugins.updater.pubkey in apps/desktop/src-tauri/tauri.conf.json to the updater public key (docs/building.md)"
fi
if [[ -f apps/desktop/src-tauri/updater.pub ]] && [[ "$(tr -d '[:space:]' <apps/desktop/src-tauri/updater.pub)" != "$(printf '%s' "$pubkey" | tr -d '[:space:]')" ]]; then
  echo "release: warning: apps/desktop/src-tauri/updater.pub differs from the pubkey in tauri.conf.json; the app uses tauri.conf.json" >&2
fi

if [[ -z ${TAURI_SIGNING_PRIVATE_KEY:-} && -z ${TAURI_SIGNING_PRIVATE_KEY_PATH:-} ]]; then
  problem "set TAURI_SIGNING_PRIVATE_KEY_PATH (e.g. ~/.tauri/clashofmeat.key) so the updater artifacts get signed"
elif [[ -z ${TAURI_SIGNING_PRIVATE_KEY_PASSWORD+set} && -t 0 ]]; then
  # Ask once instead of once per artifact. An empty answer means the key has no password.
  read -rsp "release: updater key password: " TAURI_SIGNING_PRIVATE_KEY_PASSWORD
  echo
  export TAURI_SIGNING_PRIVATE_KEY_PASSWORD
fi

[[ -z "$(git status --porcelain --untracked-files=no)" ]] || problem "tracked files have uncommitted changes; release from a clean commit"
sha=$(git rev-parse HEAD)
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then problem "tag $tag already exists locally"; fi
have_gh=false
if command -v gh >/dev/null && gh auth status >/dev/null 2>&1; then have_gh=true; fi
[[ $have_gh == true || $dry_run == true ]] || die "gh is missing or not logged in: brew install gh && gh auth login"
if [[ $have_gh == true ]]; then
  matches=$(gh api "repos/$repo/git/matching-refs/tags/$tag" --jq "map(select(.ref == \"refs/tags/$tag\")) | length") ||
    die "could not list the tags of $repo"
  [[ $matches == 0 ]] || die "tag $tag already exists on $repo; versions are never reused"
  if gh release view "$tag" --repo "$repo" >/dev/null 2>&1; then die "release $tag already exists on $repo"; fi
  # gh creates the tag on GitHub at this commit, so the commit must already be pushed.
  gh api "repos/$repo/commits/$sha" --silent 2>/dev/null || problem "commit $sha is not on $repo; push it first"
elif refs=$(GIT_TERMINAL_PROMPT=0 git ls-remote --tags "https://github.com/$repo.git" "refs/tags/$tag" 2>/dev/null); then
  [[ -z $refs ]] || die "tag $tag already exists on $repo; versions are never reused"
else
  problem "could not check $repo for an existing $tag (gh not logged in)"
fi

note "checks"
scripts/check.sh

mkdir -p "$dist"
dist=$(cd "$dist" && pwd)
for target in "${targets[@]}"; do
  note "package $target"
  scripts/package.sh "$version" "$target" --out "$dist" --app "$app"
done

mac=("$dist/Clash.of.Meat_${version}_aarch64.app.tar.gz" "$dist/Clash.of.Meat_${version}_x86_64.app.tar.gz")
win="$dist/Clash.of.Meat_${version}_x64-setup.exe"
if [[ -s ${mac[0]}.sig && -s ${mac[1]}.sig && -s $win.sig ]]; then
  manifest_args=()
  if [[ -n $min_version ]]; then manifest_args+=(--min-version "$min_version"); fi
  if [[ $security == true ]]; then manifest_args+=(--security); fi
  scripts/update-manifest.sh "$dist" "$version" "https://github.com/$repo/releases/download/$tag/" ${manifest_args[@]+"${manifest_args[@]}"}
else
  problem "artifacts are unsigned, so manifest.json and update.json were not written"
fi

if [[ $dry_run == true ]]; then
  note "dry run: would run gh release create $tag --repo $repo --target $sha with:"
  printf '  %s\n' "${mac[@]}" "${mac[@]/%/.sha256}" "${mac[@]/%/.sig}" "$win" "$win.sha256" "$win.sig" "install/install.sh#install.sh"
  note "dry run: then gh release upload $tag $dist/manifest.json $dist/update.json"
  exit 0
fi

# Assets first, then the two pointers, so nobody reads a manifest whose files are not there yet.
gh release create "$tag" \
  "${mac[@]}" "${mac[@]/%/.sha256}" "${mac[@]/%/.sig}" \
  "$win" "$win.sha256" "$win.sig" \
  install/install.sh#install.sh \
  --repo "$repo" --title "$tag" --notes "Clash of Meat desktop $version" --target "$sha"
gh release upload "$tag" "$dist/manifest.json" "$dist/update.json" --repo "$repo"
note "published https://github.com/$repo/releases/tag/$tag"

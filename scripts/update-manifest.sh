#!/usr/bin/env bash
# Validate the two signed updater archives and write both release pointers.
set -euo pipefail

die() { echo "update-manifest: $*" >&2; exit 1; }
root=$(cd "$(dirname "$0")/.." && pwd)

if [[ ${1:-} == --self-test && $# -eq 1 ]]; then
  fixture=$(mktemp -d /tmp/clashofmeat-manifest.XXXXXX)
  trap 'rm -rf "$fixture"' EXIT
  python3 -c '
import base64, hashlib, pathlib, sys
out = pathlib.Path(sys.argv[1])
for arch in ("aarch64", "x86_64"):
    name = f"Clash.of.Meat_0.0.1_{arch}.app.tar.gz"
    data = ("fake " + arch).encode()
    (out / name).write_bytes(data)
    (out / (name + ".sha256")).write_text(hashlib.sha256(data).hexdigest() + "  " + name + "\n")
    comment = "untrusted comment: test\nAAAA\ntrusted comment: timestamp:1\tfile:" + name + "\tversion:0.0.1\nBBBB\n"
    (out / (name + ".sig")).write_text(base64.b64encode(comment.encode()).decode() + "\n")
' "$fixture"
  "$0" "$fixture" 0.0.1 https://example.test/releases/download/desktop-v0.0.1/ --min-version 0.0.1 --security
  python3 -c '
import base64, json, pathlib, sys
out = pathlib.Path(sys.argv[1])
manifest = json.loads((out / "manifest.json").read_text())
update = json.loads((out / "update.json").read_text())
assert manifest["version"] == update["version"] == "0.0.1"
assert set(manifest["assets"]) == {"aarch64-apple-darwin", "x86_64-apple-darwin"}
assert set(update["platforms"]) == {"darwin-aarch64", "darwin-x86_64"}
assert update["min_version"] == "0.0.1" and update["security"] is True
for arch in ("aarch64", "x86_64"):
    asset = manifest["assets"][arch + "-apple-darwin"]
    assert asset["size"] == (out / asset["file"]).stat().st_size
    assert len(asset["sha256"]) == 64
    platform = update["platforms"]["darwin-" + arch]
    assert platform["url"].endswith("/" + asset["file"])
    assert platform["signature"] == (out / (asset["file"] + ".sig")).read_text().strip()
    assert b"trusted comment:" in base64.b64decode(platform["signature"])
' "$fixture"
  echo "update-manifest: self-test passed"
  exit 0
fi

[[ $# -ge 3 ]] || die "usage: $0 <dist-dir> <version> <base-url> [--min-version X] [--security]"
dist=$1
version=$2
base_url=$3
shift 3
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "version must be x.y.z"
[[ $base_url == https://* || $base_url == http://* ]] || die "base-url must be http(s)"
base_url="${base_url%/}/"
min_version=
security=false
while (( $# )); do
  case "$1" in
    --min-version)
      (( $# >= 2 )) || die "--min-version needs a value"
      min_version=$2
      shift 2 ;;
    --security) security=true; shift ;;
    *) die "unknown option: $1" ;;
  esac
done
[[ -d $dist ]] || die "no dist directory: $dist"
if [[ -n $min_version ]]; then
  [[ $min_version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "min version must be x.y.z"
fi

python3 -c '
import base64, datetime, hashlib, json, pathlib, re, sys

dist, version, base_url, min_version, security = sys.argv[1:]
out = pathlib.Path(dist)
if min_version and tuple(map(int, min_version.split("."))) > tuple(map(int, version.split("."))):
    raise SystemExit("update-manifest: min version exceeds release version")
assets = {}
platforms = {}
for arch in ("aarch64", "x86_64"):
    target = arch + "-apple-darwin"
    name = f"Clash.of.Meat_{version}_{arch}.app.tar.gz"
    file = out / name
    checksum_file = out / (name + ".sha256")
    signature_file = out / (name + ".sig")
    for required in (file, checksum_file, signature_file):
        if not required.is_file() or required.stat().st_size == 0:
            raise SystemExit(f"update-manifest: missing or empty {required}")
    checksum_line = checksum_file.read_text().strip()
    match = re.fullmatch(r"([0-9a-f]{64})  " + re.escape(name), checksum_line)
    if not match:
        raise SystemExit(f"update-manifest: invalid checksum file {checksum_file}")
    digest = hashlib.sha256(file.read_bytes()).hexdigest()
    if digest != match.group(1):
        raise SystemExit(f"update-manifest: checksum mismatch for {file}")
    signature = signature_file.read_text().strip()
    try:
        decoded = base64.b64decode(signature, validate=True).decode("utf-8")
    except (ValueError, UnicodeError) as error:
        raise SystemExit(f"update-manifest: invalid signature encoding in {signature_file}: {error}")
    trusted = next((line for line in decoded.splitlines() if line.startswith("trusted comment: ")), "")
    fields = dict(re.findall(r"(?:^|[\t ])(file|version):([^\t ]+)", trusted))
    if fields.get("file") != name or fields.get("version") != version:
        raise SystemExit(f"update-manifest: {signature_file} is not bound to {name} and version {version}")
    assets[target] = {"file": name, "sha256": digest, "size": file.stat().st_size}
    platforms["darwin-" + arch] = {"url": base_url + name, "signature": signature}

manifest = {"version": version, "assets": assets}
update = {
    "version": version,
    "notes": "Clash of Meat " + version,
    "pub_date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "security": security == "true",
    "platforms": platforms,
}
if min_version:
    update["min_version"] = min_version
for filename, document in (("manifest.json", manifest), ("update.json", update)):
    path = out / filename
    temporary = out / (filename + ".tmp")
    temporary.write_text(json.dumps(document, indent=2) + "\n")
    temporary.replace(path)
    print(f"update-manifest: {path}")
' "$dist" "$version" "$base_url" "$min_version" "$security"

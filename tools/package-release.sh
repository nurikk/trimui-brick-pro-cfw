#!/bin/sh
set -eu

usage() {
    printf '%s\n' 'usage: tools/package-release.sh --manifest FILE --payload FILE --out ABSOLUTE_EMPTY_DIR' >&2
    exit 64
}
[ "$#" -eq 6 ] || usage
[ "$1" = --manifest ] || usage
MANIFEST=$2
[ "$3" = --payload ] || usage
PAYLOAD=$4
[ "$5" = --out ] || usage
OUT=$6
case "$OUT" in /*) ;; *) usage ;; esac
[ -f "$MANIFEST" ] && [ -f "$PAYLOAD" ] && [ ! -L "$MANIFEST" ] && [ ! -L "$PAYLOAD" ] || {
    echo 'package-release: inputs must be regular non-symlink files' >&2
    exit 2
}
[ -d "$OUT" ] && [ -z "$(find "$OUT" -mindepth 1 -maxdepth 1 -print -quit)" ] || {
    echo 'package-release: output must be an existing empty directory' >&2
    exit 2
}
WORK=$(mktemp -d "${TMPDIR:-/tmp}/brickpro-release.XXXXXX")
trap 'rm -rf "$WORK"' EXIT HUP INT TERM
RELEASE=$(
    python3 - "$MANIFEST" "$PAYLOAD" "$WORK" <<'PY'
import hashlib, json, pathlib, re, shutil, sys
manifest, payload, work = map(pathlib.Path, sys.argv[1:])
data = json.loads(manifest.read_text(encoding="utf-8"))
expected = {"$schema", "manifestVersion", "deviceId", "releaseId", "artifactUrl", "artifactName", "stockFirmware", "userspaceAbi", "dataSchema", "payloadType", "payloadSize", "payloadSha256"}
if set(data) != expected or manifest.read_bytes() != (json.dumps(data, indent=2, sort_keys=True) + "\n").encode():
    raise SystemExit("manifest is not closed canonical v1")
if data["deviceId"] != "TG4040" or data["payloadType"] != "squashfs-userspace":
    raise SystemExit("package is not a TG4040 userspace package")
if not re.fullmatch(r"[a-z0-9](?:[a-z0-9.-]{0,46}[a-z0-9])?", data["releaseId"]):
    raise SystemExit("release ID is outside bounds")
def valid_artifact_url(value):
    if len(value) > 2048 or not value.startswith("https://"):
        return False
    rest = value[8:]
    if "/" not in rest:
        return False
    authority, path = rest.split("/", 1)
    host, separator, port = authority.partition(":")
    return (
        host and host[0].isalnum() and host[-1].isalnum()
        and all(char.isascii() and (char.isalnum() or char in ".-:") for char in authority)
        and (not separator or port.isdigit())
        and path and all(not char.isspace() and ord(char) >= 32 and ord(char) != 127 and char not in "?#\\" for char in path)
    )
def valid_artifact_name(value):
    return bool(re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,127}", value))
if not valid_artifact_url(data["artifactUrl"]):
    raise SystemExit("artifact URL must be an ordinary HTTPS URL")
if not valid_artifact_name(data["artifactName"]):
    raise SystemExit("artifact name must be a safe plain filename")
if payload.name != data["artifactName"]:
    raise SystemExit("payload filename does not match artifact name")
raw = payload.read_bytes()
if data["payloadSize"] != len(raw) or data["payloadSha256"] != hashlib.sha256(raw).hexdigest() or raw[:4] != b"hsqs":
    raise SystemExit("payload size, SHA-256, or SquashFS header is invalid")
out = work / data["releaseId"]
out.mkdir()
shutil.copyfile(manifest, out / "manifest.json")
shutil.copyfile(payload, out / data["artifactName"])
(out / "interface.json").write_text(json.dumps({"schema": "brickpro-update-interface/v1", "actions": ["boot-current", "boot-previous", "discard-unactivated-staging"]}, indent=2, sort_keys=True) + "\n")
print(data["releaseId"])
PY
)
tar -C "$WORK" --format=ustar --sort=name --mtime='UTC 1970-01-01' -cf "$OUT/update-$RELEASE.tar" "$RELEASE"
sha256sum "$OUT/update-$RELEASE.tar"

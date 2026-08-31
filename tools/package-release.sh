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
expected = {"$schema", "manifestVersion", "deviceId", "targetSku", "hardwareRevision", "sourceRelease", "releaseId", "artifactUrl", "artifactName", "stockFirmware", "userspaceAbi", "dataSchema", "payloadType", "payloadSize", "payloadSha256", "requiredFreeBytes", "userDataManifest"}
if set(data) != expected or manifest.read_bytes() != (json.dumps(data, indent=2, sort_keys=True) + "\n").encode():
    raise SystemExit("manifest is not closed canonical v1")
if data["$schema"] != "https://trimui.invalid/schemas/update-manifest-v1.schema.json" or data["manifestVersion"] != 1 or data["userspaceAbi"] != "tg4040-userspace-v1" or data["dataSchema"] != {"min": 1, "max": 1}:
    raise SystemExit("manifest schema, ABI, or data schema is unsupported")
if data["deviceId"] != "tg4040" or data["targetSku"] != "TG4040" or data["hardwareRevision"] != "synthetic-v1" or data["payloadType"] != "squashfs-userspace":
    raise SystemExit("package is not an exact TG4040 synthetic-v1 userspace package")
for field in ("sourceRelease", "releaseId"):
    if not re.fullmatch(r"[a-z0-9](?:[a-z0-9.-]{0,46}[a-z0-9])?", data[field]):
        raise SystemExit(f"{field} is outside bounds")
if data["sourceRelease"] == data["releaseId"]:
    raise SystemExit("source and target releases must differ")
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
if data["requiredFreeBytes"] != len(raw) * 3:
    raise SystemExit("required free bytes must equal the three-copy peak")
approved = [
    ("saves", "data/saves"),
    ("credentials", "data/credentials"),
    ("achievements", "data/achievements"),
    ("mappings", "data/mappings"),
    ("fn-led-settings", "data/settings/fn-led"),
    ("service-settings", "data/settings/services"),
]
user_data = data["userDataManifest"]
if set(user_data) != {"format", "schemaVersion", "entries"} or user_data["format"] != "update-user-data-manifest" or user_data["schemaVersion"] != 1:
    raise SystemExit("approved user-data manifest is invalid")
expected_entries = [
    {"class": cls, "migration": "shared-data-copy-on-write", "path": path, "sourceSchema": 1, "targetSchema": 1}
    for cls, path in approved
]
if user_data["entries"] != expected_entries:
    raise SystemExit("approved user-data entries are invalid")
out = work / data["releaseId"]
out.mkdir()
shutil.copyfile(manifest, out / "manifest.json")
shutil.copyfile(payload, out / data["artifactName"])
(out / "interface.json").write_text(json.dumps({"schema": "update-interface/v1", "actions": ["install-online", "install-sideload", "boot-current", "boot-previous", "discard-unactivated-staging"]}, indent=2, sort_keys=True) + "\n")
print(data["releaseId"])
PY
)
tar -C "$WORK" --format=ustar --sort=name --mtime='UTC 1970-01-01' -cf "$OUT/update-$RELEASE.tar" "$RELEASE"
sha256sum "$OUT/update-$RELEASE.tar"

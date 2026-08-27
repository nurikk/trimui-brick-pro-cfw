#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd -P)
BIN_DIR=${1:-$ROOT/target/release}
DIAG=$BIN_DIR/brickpro-diagnostics
RECOVERY=$BIN_DIR/brick-recovery
[ -x "$DIAG" ] && [ -x "$RECOVERY" ] || exit 1

WORK=$(mktemp -d "${TMPDIR:-/tmp}/brickpro-diagnostics.XXXXXX")
trap 'rm -rf "$WORK"' EXIT HUP INT TERM
FIXTURE=$WORK/fixture
cp -a "$ROOT/fixtures/bootstrap/supported" "$FIXTURE"
DEST=$WORK/sd
mkdir "$DEST"

snapshot() {
    find "$FIXTURE/.brickpro/system" "$FIXTURE/.brickpro/data/update" "$FIXTURE/roms" -type f -print0 |
        sort -z | xargs -0 sha256sum
}
BEFORE=$(snapshot)

"$DIAG" --simulation-fixture-root "$FIXTURE" --present-safe-mode >"$WORK/report.json"
"$RECOVERY" --simulation-fixture-root "$FIXTURE" --select safe-mode >"$WORK/recovery.json"
PYTHONDONTWRITEBYTECODE=1 python3 - "$WORK/report.json" "$WORK/recovery.json" <<'PY'
import json
import sys
report = json.load(open(sys.argv[1], encoding="utf-8"))
recovery = json.load(open(sys.argv[2], encoding="utf-8"))
required = {"schema", "status", "mode", "activating", "firmware", "targetSku", "ram", "battery", "temperature", "storage", "slots", "activeCore", "lastCrash", "policy"}
assert set(report) == required
assert report["targetSku"] == {"status": "verified", "value": "TG4040"}
assert report["activating"] is False and report["mode"] == "safe-mode"
assert report["policy"] == {
    "theme": "built-in", "display": "conservative", "input": "conservative",
    "network": "disabled", "thirdPartyThemes": "disabled",
    "backgroundIndexing": "disabled", "automaticGameLaunch": "disabled",
    "firmwareMutation": "not-permitted", "romMutation": "not-permitted",
    "saveMutation": "not-permitted", "updaterRecordMutation": "not-permitted",
    "rawStorageMutation": "not-permitted", "emmcMutation": "not-permitted",
}
assert recovery["safeModePresentation"] == report
PY

"$DIAG" --simulation-fixture-root "$FIXTURE" --persist-crash >"$WORK/persist.json"
"$DIAG" --simulation-fixture-root "$FIXTURE" --present-safe-mode >"$WORK/persisted-report.json"
python3 - "$WORK/persisted-report.json" <<'PY'
import json
import sys
report = json.load(open(sys.argv[1], encoding="utf-8"))
assert report["lastCrash"]["status"] == "available"
assert report["lastCrash"]["record"]["id"] == "crash-002"
PY
"$DIAG" --simulation-fixture-root "$FIXTURE" --export-support-bundle "$DEST" >"$WORK/export.json"
PYTHONDONTWRITEBYTECODE=1 python3 - "$DEST" "$WORK/export.json" <<'PY'
import hashlib
import json
import sys
import tarfile
from pathlib import Path

dest = Path(sys.argv[1])
result = json.load(open(sys.argv[2], encoding="utf-8"))
assert result["bundle"] == "trimui-support-bundle-v1"
archive = dest / result["bundle"] / result["archive"]
sidecar = dest / result["bundle"] / (result["archive"] + ".sha256")
assert archive.is_file() and sidecar.is_file()
assert hashlib.sha256(archive.read_bytes()).hexdigest() == result["checksum"]
assert sidecar.read_text() == result["checksum"] + "  " + archive.name + "\n"
with tarfile.open(archive, "r:") as bundle:
    names = bundle.getnames()
    assert names == ["support-report.json", "metadata.json"]
    for member in bundle.getmembers():
        assert member.isfile() and member.name in names and member.name == member.name.strip()
        assert "/" not in member.name and ".." not in member.name
        content = bundle.extractfile(member).read()
        if member.name.endswith(".json"):
            json.loads(content)
        assert b"hunter2" not in content and b"rom-secret" not in content
metadata = json.loads(tarfile.open(archive).extractfile("metadata.json").read())
assert set(metadata) == {"schema", "bundleVersion", "source", "targetSku", "redactions"}
assert metadata["schema"] == "brickpro-support-bundle-metadata/v1"
PY

# Publication is one directory rename: collisions leave the complete pair intact.
ARCHIVE_SHA=$(sha256sum "$DEST/trimui-support-bundle-v1/trimui-support-bundle-v1.tar")
if "$DIAG" --simulation-fixture-root "$FIXTURE" --export-support-bundle "$DEST" >/dev/null 2>&1; then exit 1; fi
[ "$ARCHIVE_SHA" = "$(sha256sum "$DEST/trimui-support-bundle-v1/trimui-support-bundle-v1.tar")" ]
[ ! -e "$DEST/trimui-support-bundle-v1.tar" ]
[ ! -e "$DEST/trimui-support-bundle-v1.tar.sha256" ]
[ -z "$(find "$DEST" -maxdepth 1 -name '.support-bundle-v1-stage-*' -print -quit)" ]

[ "$BEFORE" = "$(snapshot)" ]

# A malformed persisted crash is represented as unavailable, never exported raw.
printf '%s' '{malformed' >"$FIXTURE/.brickpro/data/diagnostics/last-crash.json"
mkdir "$WORK/corrupt-sd"
"$DIAG" --simulation-fixture-root "$FIXTURE" --export-support-bundle "$WORK/corrupt-sd" >"$WORK/corrupt-export.json"
python3 - "$WORK/corrupt-sd/trimui-support-bundle-v1/trimui-support-bundle-v1.tar" <<'PY'
import json
import sys
import tarfile
with tarfile.open(sys.argv[1], "r") as bundle:
    report = json.loads(bundle.extractfile("support-report.json").read())
assert report["lastCrash"] == {"status": "unavailable", "reason": "crash-record-invalid"}
PY

# Forbidden values are rejected and never reach an archive.
SECRET=$WORK/secret
cp -a "$ROOT/fixtures/bootstrap/supported" "$SECRET"
python3 - "$SECRET/diagnostics.json" <<'PY'
import json
import sys
path = sys.argv[1]
data = json.load(open(path, encoding="utf-8"))
data["firmware"]["build"] = "TOKEN=never-export"
open(path, "w", encoding="utf-8").write(json.dumps(data))
PY
mkdir "$WORK/secret-sd"
if "$DIAG" --simulation-fixture-root "$SECRET" --export-support-bundle "$WORK/secret-sd" >/dev/null 2>&1; then exit 1; fi
[ ! -e "$WORK/secret-sd/trimui-support-bundle-v1.tar" ]

# Malformed crash input is rejected without creating a persisted record.
BAD=$WORK/bad
cp -a "$ROOT/fixtures/bootstrap/supported" "$BAD"
mkdir -p "$BAD/.brickpro/data/diagnostics"
printf '%s' '{"schema":"brickpro-synthetic-crash/v1","id":"hunter2"}' >"$BAD/.brickpro/data/diagnostics/crash-input.json"
if "$DIAG" --simulation-fixture-root "$BAD" --persist-crash >/dev/null 2>&1; then exit 1; fi
[ ! -e "$BAD/.brickpro/data/diagnostics/last-crash.json" ]

# Symlinked input, unsafe destination, and wrong/non-fixture roots all deny.
ln -s "$FIXTURE/.brickpro/data/diagnostics/crash-input.json" "$BAD/.brickpro/data/diagnostics/crash-input-link.json"
mv "$BAD/.brickpro/data/diagnostics/crash-input-link.json" "$BAD/.brickpro/data/diagnostics/crash-input.json"
if "$DIAG" --simulation-fixture-root "$BAD" --persist-crash >/dev/null 2>&1; then exit 1; fi
ln -s "$DEST" "$WORK/sd-link"
if "$DIAG" --simulation-fixture-root "$FIXTURE" --export-support-bundle "$WORK/sd-link" >/dev/null 2>&1; then exit 1; fi
mkdir "$WORK/empty"
if "$DIAG" --simulation-fixture-root "$ROOT/fixtures/bootstrap/wrong-model" --export-support-bundle "$WORK/empty" >/dev/null 2>&1; then exit 1; fi
if "$DIAG" --simulation-fixture-root "$ROOT" --export-support-bundle "$WORK/empty" >/dev/null 2>&1; then exit 1; fi
if "$DIAG" --simulation-fixture-root "$FIXTURE" --export-support-bundle "$WORK/empty/.." >/dev/null 2>&1; then exit 1; fi
mkdir "$WORK/parent"
ln -s "$DEST" "$WORK/parent/sd"
if "$DIAG" --simulation-fixture-root "$FIXTURE" --export-support-bundle "$WORK/parent/sd" >/dev/null 2>&1; then exit 1; fi

printf '%s\n' 'diagnostics/recovery journey: PASS'

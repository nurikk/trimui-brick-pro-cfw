#!/bin/sh
set -eu

ROOT=$(
    CDPATH=
    cd -- "$(dirname -- "$0")/../../.." && pwd -P
)
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
required = {"schema", "status", "mode", "activating", "firmware", "targetSku", "ram", "battery", "temperature", "storage", "slots", "activeCore", "lastCrash", "audio", "healthChecks", "policy"}
assert set(report) == required
assert [check["id"] for check in report["healthChecks"]] == ["build-sku", "storage", "battery-power", "input", "display", "audio", "wifi", "last-failed-stage"]
assert {check["status"] for check in report["healthChecks"]} == {"pass", "warn", "unavailable"}
assert report["healthChecks"][-1]["status"] == "unavailable"
assert report["targetSku"] == {"status": "verified", "value": "TG4040"}
assert report["audio"] == {"status": "available", "activeSink": "speaker", "sampleRateHz": 48000, "underrunCount": 0, "speakerAmpEnabled": False}
assert report["activating"] is False and report["mode"] == "safe-mode"
assert report["policy"] == {
    "theme": "built-in", "display": "conservative", "input": "conservative",
    "network": "disabled", "thirdPartyThemes": "disabled", "thirdPartyModules": "disabled",
    "networkAutoStart": "disabled", "backgroundIndexing": "disabled", "automaticGameLaunch": "disabled",
    "autoResume": "disabled", "saves": "read-only", "diagnostics": "read-only",
    "firmwareMutation": "not-permitted", "romMutation": "not-permitted",
    "saveMutation": "not-permitted", "updaterRecordMutation": "not-permitted",
    "rawStorageMutation": "not-permitted", "emmcMutation": "not-permitted",
}
assert recovery["safeModePresentation"] == report
PY

# Diagnostics reads the route manager state, not a duplicated diagnostics fixture value.
LIVE=$WORK/live-audio
cp -a "$FIXTURE" "$LIVE"
python3 - "$LIVE/.brickpro/data/audio-routing/state.json" <<'PY'
import json
import sys
path = sys.argv[1]
state = json.load(open(path, encoding="utf-8"))
state["route"].update({"available": ["speaker", "jack"], "currentSink": "jack", "requestedSink": "jack", "sampleRateHz": 44100, "underrunCount": 3})
open(path, "w", encoding="utf-8").write(json.dumps(state))
PY
"$DIAG" --simulation-fixture-root "$LIVE" --present-safe-mode >"$WORK/live-audio-report.json"
python3 - "$WORK/live-audio-report.json" <<'PY'
import json
import sys
assert json.load(open(sys.argv[1], encoding="utf-8"))["audio"] == {"status": "available", "activeSink": "jack", "sampleRateHz": 44100, "underrunCount": 3, "speakerAmpEnabled": False}
PY

# Invalid persisted rates fail closed and leave the state intact.
INVALID_AUDIO=$WORK/invalid-audio
cp -a "$FIXTURE" "$INVALID_AUDIO"
python3 - "$INVALID_AUDIO/.brickpro/data/audio-routing/state.json" <<'PY'
import json
import sys
path = sys.argv[1]
state = json.load(open(path, encoding="utf-8"))
state["route"]["sampleRateHz"] = 192001
open(path, "w", encoding="utf-8").write(json.dumps(state))
PY
INVALID_AUDIO_SHA=$(sha256sum "$INVALID_AUDIO/.brickpro/data/audio-routing/state.json")
"$DIAG" --simulation-fixture-root "$INVALID_AUDIO" --present-safe-mode >"$WORK/invalid-audio-report.json"
python3 - "$WORK/invalid-audio-report.json" <<'PY'
import json
import sys
assert json.load(open(sys.argv[1], encoding="utf-8"))["audio"] == {"status": "unavailable", "reason": "audio-state-invalid"}
PY
[ "$INVALID_AUDIO_SHA" = "$(sha256sum "$INVALID_AUDIO/.brickpro/data/audio-routing/state.json")" ]

"$DIAG" --simulation-fixture-root "$FIXTURE" --persist-crash >"$WORK/persist.json"
"$DIAG" --simulation-fixture-root "$FIXTURE" --present-safe-mode >"$WORK/persisted-report.json"
python3 - "$WORK/persisted-report.json" <<'PY'
import json
import sys
report = json.load(open(sys.argv[1], encoding="utf-8"))
assert report["lastCrash"]["status"] == "available"
assert report["lastCrash"]["record"]["id"] == "crash-002"
PY
"$DIAG" --simulation-fixture-root "$FIXTURE" --preview-support-bundle >"$WORK/preview.json"
CHECKSUM=$(
    PYTHONDONTWRITEBYTECODE=1 python3 - "$WORK/preview.json" <<'PY'
import json
import sys
preview = json.load(open(sys.argv[1], encoding="utf-8"))
assert preview["schema"] == "brickpro-support-bundle-preview/v1"
assert preview["includedFields"] == ["support-report.json", "metadata.json"]
assert 0 < preview["bytes"] <= 65536
print(preview["checksum"])
PY
)
if "$DIAG" --simulation-fixture-root "$FIXTURE" --export-support-bundle "$DEST" >/dev/null 2>&1; then exit 1; fi
"$DIAG" --simulation-fixture-root "$FIXTURE" --export-support-bundle "$DEST" --confirm-preview "$CHECKSUM" >"$WORK/export.json"
PYTHONDONTWRITEBYTECODE=1 python3 - "$DEST" "$WORK/export.json" "$WORK/preview.json" <<'PY'
import hashlib
import json
import sys
import tarfile
from pathlib import Path

dest = Path(sys.argv[1])
result = json.load(open(sys.argv[2], encoding="utf-8"))
preview = json.load(open(sys.argv[3], encoding="utf-8"))
assert result["bundle"] == "trimui-support-bundle-v1"
assert result["checksum"] == preview["checksum"]
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
if "$DIAG" --simulation-fixture-root "$FIXTURE" --export-support-bundle "$DEST" --confirm-preview "$CHECKSUM" >/dev/null 2>&1; then exit 1; fi
[ "$ARCHIVE_SHA" = "$(sha256sum "$DEST/trimui-support-bundle-v1/trimui-support-bundle-v1.tar")" ]
[ ! -e "$DEST/trimui-support-bundle-v1.tar" ]
[ ! -e "$DEST/trimui-support-bundle-v1.tar.sha256" ]
[ -z "$(find "$DEST" -maxdepth 1 -name '.support-bundle-v1-stage-*' -print -quit)" ]

[ "$BEFORE" = "$(snapshot)" ]

# A malformed persisted crash is represented as unavailable, never exported raw.
printf '%s' '{malformed' >"$FIXTURE/.brickpro/data/diagnostics/last-crash.json"
mkdir "$WORK/corrupt-sd"
CORRUPT_CHECKSUM=$("$DIAG" --simulation-fixture-root "$FIXTURE" --preview-support-bundle | python3 -c 'import json,sys; print(json.load(sys.stdin)["checksum"])')
"$DIAG" --simulation-fixture-root "$FIXTURE" --export-support-bundle "$WORK/corrupt-sd" --confirm-preview "$CORRUPT_CHECKSUM" >"$WORK/corrupt-export.json"
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
if "$DIAG" --simulation-fixture-root "$SECRET" --preview-support-bundle >/dev/null 2>&1; then exit 1; fi
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

# Safe-mode requests are consumed once, while cancellation and each narrow action leave ROMs/saves intact.
RECOVERY_ROOT=$WORK/recovery
cp -a "$ROOT/fixtures/bootstrap/supported" "$RECOVERY_ROOT"
printf '%s' 'private-rom-filename' >"$RECOVERY_ROOT/roms/keep-me"
mkdir -p "$RECOVERY_ROOT/.brickpro/data/saves"
printf '%s' 'private-save' >"$RECOVERY_ROOT/.brickpro/data/saves/keep-me"
RECOVERY_BEFORE=$(find "$RECOVERY_ROOT/roms" "$RECOVERY_ROOT/.brickpro/data/saves" -type f -print0 | sort -z | xargs -0 sha256sum)
"$RECOVERY" --simulation-fixture-root "$RECOVERY_ROOT" --schedule-safe-mode >"$WORK/scheduled.json"
"$RECOVERY" --simulation-fixture-root "$RECOVERY_ROOT" >"$WORK/consumed.json"
"$RECOVERY" --simulation-fixture-root "$RECOVERY_ROOT" --cancel >"$WORK/cancelled.json"
for action in reset-ui-profile disable-last-module-or-theme choose-internal-storage retry-index previous-update-slot; do
    "$RECOVERY" --simulation-fixture-root "$RECOVERY_ROOT" --apply-action "$action" >"$WORK/action-$action.json"
done
PYTHONDONTWRITEBYTECODE=1 python3 - "$RECOVERY_ROOT" "$WORK" <<'PY'
import json
import sys
from pathlib import Path
root, work = map(Path, sys.argv[1:])
scheduled = json.loads((work / "scheduled.json").read_text())
consumed = json.loads((work / "consumed.json").read_text())
cancelled = json.loads((work / "cancelled.json").read_text())
assert scheduled["selected"] == "safe-mode" and scheduled["selectionSource"] == "next-boot-request"
assert consumed["selected"] == "safe-mode" and consumed["selectionSource"] == "next-boot-marker"
assert cancelled["cancelled"] is True and cancelled["selected"] is None
for action in scheduled["actions"]:
    result = json.loads((work / f"action-{action}.json").read_text())
    assert result["appliedAction"] == action and result["safeModePresentation"]["policy"]["saves"] == "read-only"
assert (root / ".brickpro/data/recovery-next-boot").read_text() == "previous-userspace-release\n"
PY
[ "$RECOVERY_BEFORE" = "$(find "$RECOVERY_ROOT/roms" "$RECOVERY_ROOT/.brickpro/data/saves" -type f -print0 | sort -z | xargs -0 sha256sum)" ]

# Recovery requests refuse symlinked parents rather than reaching save data.
LINKED_RECOVERY=$WORK/linked-recovery
cp -a "$ROOT/fixtures/bootstrap/supported" "$LINKED_RECOVERY"
mkdir -p "$LINKED_RECOVERY/.brickpro/data/saves"
ln -s saves "$LINKED_RECOVERY/.brickpro/data/recovery"
if "$RECOVERY" --simulation-fixture-root "$LINKED_RECOVERY" --apply-action reset-ui-profile >/dev/null 2>&1; then exit 1; fi
[ ! -e "$LINKED_RECOVERY/.brickpro/data/saves/reset-ui-profile" ]


# The regular boot handoff consumes a pending safe-mode request instead of starting the supervisor.
BOOT_SAFE=$WORK/boot-safe
cp -a "$ROOT/fixtures/bootstrap/supported" "$BOOT_SAFE"
"$RECOVERY" --simulation-fixture-root "$BOOT_SAFE" --schedule-safe-mode >/dev/null
BRICKPRO_SIMULATION_PROBE=$BIN_DIR/bootstrap-probe \
BRICKPRO_SIMULATION_RECOVERY=$RECOVERY \
BRICKPRO_SIMULATION_SUPERVISOR=$ROOT/tools/sim/journeys/supervisor-record.sh \
sh "$ROOT/bootstrap/boot.sh" --simulation-fixture-root "$BOOT_SAFE" >"$WORK/boot-safe.json"
PYTHONDONTWRITEBYTECODE=1 python3 - "$WORK/boot-safe.json" "$BOOT_SAFE" <<'PY'
import json
import sys
from pathlib import Path
result = json.loads(Path(sys.argv[1]).read_text())
assert result["selected"] == "safe-mode" and result["reason"] == "safe-mode-requested", result
assert not (Path(sys.argv[2]) / ".brickpro/data/recovery-next-boot").exists()
PY


# Theme, module, index, SD2, and update faults can each schedule and consume one safe-mode boot.
for check in build-sku storage battery-power input display; do
    CASE=$WORK/fault-$check
    cp -a "$ROOT/fixtures/bootstrap/supported" "$CASE"
    python3 - "$CASE/diagnostics.json" "$check" <<'PY'
import json
import sys
path, check = sys.argv[1:]
data = json.load(open(path, encoding="utf-8"))
for item in data["healthChecks"]:
    if item["id"] == check:
        item.update(status="fail", detail="Recovery action required", nextStep="Enter safe mode")
open(path, "w", encoding="utf-8").write(json.dumps(data))
PY
    "$RECOVERY" --simulation-fixture-root "$CASE" --schedule-safe-mode >/dev/null
    "$RECOVERY" --simulation-fixture-root "$CASE" >"$WORK/fault-$check.json"
done
PYTHONDONTWRITEBYTECODE=1 python3 - "$WORK" <<'PY'
import json
import sys
from pathlib import Path
work = Path(sys.argv[1])
for path in work.glob("fault-*.json"):
    result = json.loads(path.read_text())
    assert result["selected"] == "safe-mode" and result["reason"] == "safe-mode-requested", result
PY

printf '%s\n' 'diagnostics/recovery journey: PASS'

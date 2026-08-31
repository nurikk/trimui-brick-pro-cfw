#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd -P)
BIN_DIR=${1:-$ROOT/target/release}
PROBE=$BIN_DIR/bootstrap-probe
RECOVERY=$BIN_DIR/brick-recovery
[ -x "$PROBE" ] && [ -x "$RECOVERY" ] || exit 1

WORK=$(mktemp -d "${TMPDIR:-/tmp}/brickpro-bootstrap.XXXXXX")
trap 'rm -rf "$WORK"' EXIT HUP INT TERM

assert_probe() {
    fixture=$1
    expected_code=$2
    expected_reason=$3
    output=$WORK/probe-$fixture.json
    if "$PROBE" --simulation-fixture-root "$ROOT/fixtures/bootstrap/$fixture" >"$output"; then
        code=0
    else
        code=$?
    fi
    [ "$code" -eq "$expected_code" ]
    PYTHONDONTWRITEBYTECODE=1 python3 - "$output" "$expected_reason" <<'PY'
import json
import sys
value = json.loads(open(sys.argv[1], encoding="utf-8").read())
assert value["reason"] == sys.argv[2], value
assert value["status"] == ("compatible" if sys.argv[2] == "compatible" else "recovery")
assert value["handoffEligible"] == (sys.argv[2] == "compatible")
PY
}

assert_probe supported 0 compatible
for case in \
    'wrong-model target-sku-mismatch' \
    'unsupported-firmware firmware-unsupported' \
    'missing-framebuffer framebuffer-missing' \
    'invalid-framebuffer framebuffer-invalid' \
    'missing-input input-missing' \
    'unsupported-storage storage-unsupported' \
    'missing-storage storage-missing' \
    'no-real-fingerprint real-fingerprint-not-approved'; do
    set -- $case
    assert_probe "$1" 1 "$2"
done

check_recovery() {
    input=$1
    expected_code=$2
    expected_reason=$3
    expected_selection=$4
    case_root=$WORK/recovery-fixture-$(basename "$input")
    rm -rf "$case_root"
    cp -a "$input" "$case_root"
    output=$WORK/recovery-$(basename "$input").json
    if "$RECOVERY" --simulation-fixture-root "$case_root" ${5-} >"$output"; then
        code=0
    else
        code=$?
    fi
    [ "$code" -eq "$expected_code" ]
    PYTHONDONTWRITEBYTECODE=1 python3 - "$output" "$expected_reason" "$expected_selection" <<'PY'
import json
import sys
value = json.loads(open(sys.argv[1], encoding="utf-8").read())
assert value["reason"] == sys.argv[2], value
assert value["status"] == "recovery", value
assert value["choices"] == [
    "previous-userspace-release", "safe-mode", "stock-passthrough"
], value
assert value["selected"] == (None if sys.argv[3] == "none" else sys.argv[3]), value
assert len(value["choices"]) == 3
PY
}

check_recovery "$ROOT/fixtures/bootstrap/unsupported-firmware" 1 firmware-unsupported none
check_recovery "$ROOT/fixtures/bootstrap/recovery-chord" 0 firmware-unsupported safe-mode
check_recovery "$ROOT/fixtures/bootstrap/recovery-next-boot" 0 firmware-unsupported previous-userspace-release
for choice in previous-userspace-release safe-mode stock-passthrough; do
    check_recovery "$ROOT/fixtures/bootstrap/unsupported-firmware" 0 firmware-unsupported "$choice" "--select $choice"
done

cp -R "$ROOT/fixtures/bootstrap/supported" "$WORK/boot-supported"
BRICKPRO_SIMULATION_PROBE=$PROBE \
    BRICKPRO_SIMULATION_RECOVERY=$RECOVERY \
    BRICKPRO_SIMULATION_SUPERVISOR=$ROOT/tools/sim/journeys/supervisor-record.sh \
    sh "$ROOT/bootstrap/boot.sh" --simulation-fixture-root "$WORK/boot-supported" >"$WORK/boot-output.json"
PYTHONDONTWRITEBYTECODE=1 python3 - "$WORK/boot-supported/.brickpro/data/update/boot-context.json" "$WORK/boot-supported/.brickpro/data/update/boot-context.sha256" "$WORK/boot-supported/.brickpro/data/update/supervisor-handoff.json" "$WORK/boot-supported/.brickpro/data/update/supervisor-handoff.sha256" <<'PY'
import json
import sys
context = json.loads(open(sys.argv[1], encoding="utf-8").read())
assert open(sys.argv[2], encoding="utf-8").read() == "f195fe5e16fb911f990359ab4dfc5bfa961373bd97f6d9bce5b6177ff56cc05d\n"
handoff = json.loads(open(sys.argv[3], encoding="utf-8").read())
assert open(sys.argv[4], encoding="utf-8").read() == "b18f3e3b3f3a65d5b9d90ecf61001a224993c5a4af18423224d61e54748299ab\n"
assert context == {
    "schema": "brickpro-boot-context/v1",
    "targetSku": "TG4040",
    "mode": "synthetic",
    "selectedRelease": "previous-userspace-release",
    "probe": "compatible",
    "handoffEligible": True,
}
assert handoff == {
    "schema": "brickpro-supervisor-handoff/v1",
    "mode": "synthetic",
    "handoff": "accepted",
    "activating": False,
}
PY

cp -R "$ROOT/fixtures/bootstrap/unsupported-firmware" "$WORK/boot-recovery"
if BRICKPRO_SIMULATION_PROBE=$PROBE \
    BRICKPRO_SIMULATION_RECOVERY=$RECOVERY \
    BRICKPRO_SIMULATION_SUPERVISOR=$ROOT/tools/sim/journeys/supervisor-record.sh \
    sh "$ROOT/bootstrap/boot.sh" --simulation-fixture-root "$WORK/boot-recovery" >"$WORK/boot-recovery.json"; then
    exit 1
else
    code=$?
fi
[ "$code" -eq 1 ]
PYTHONDONTWRITEBYTECODE=1 python3 - "$WORK/boot-recovery.json" <<'PY'
import json
value = json.loads(open(__import__('sys').argv[1], encoding="utf-8").read())
assert value["reason"] == "firmware-unsupported"
assert value["choices"] == [
    "previous-userspace-release", "safe-mode", "stock-passthrough"
]
PY

printf '%s\n' 'bootstrap/recovery journey: PASS'

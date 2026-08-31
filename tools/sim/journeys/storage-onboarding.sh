#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd -P)
BIN=${1:-$ROOT/target/release/storage-layout}
[ -x "$BIN" ] || exit 1
WORK=$(mktemp -d "${TMPDIR:-/tmp}/brickpro-storage-onboarding.XXXXXX")
trap 'rm -rf "$WORK"' EXIT HUP INT TERM

prepare() {
    name=$1
    mkdir "$WORK/$name"
    cp -a "$ROOT/fixtures/storage/onboarding/bundle" "$WORK/$name/bundle"
    mkdir "$WORK/$name/sd1"
    [ "$name" = single ] || mkdir "$WORK/$name/sd2"
}

prepare ready
"$BIN" simulate-onboard --root "$WORK/ready" --inventory "$ROOT/fixtures/storage/onboarding/ready.json" >"$WORK/ready.json"
python3 - "$WORK/ready.json" <<'PY'
import json, sys
value = json.load(open(sys.argv[1], encoding="utf-8"))
assert value["status"] == "ready" and value["mode"] == "two-card", value
assert value["usbMode"] == "mtp", value
assert value["sd2Uuid"] == "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee", value
PY
[ ! -e "$WORK/ready/sd1/.brickpro-format-verify" ]
[ ! -e "$WORK/ready/sd1/roms" ]
[ -d "$WORK/ready/sd2/roms/BIOS" ]
"$BIN" validate --layout "$WORK/ready/sd1/data/meta/layout.json" --root "$WORK/ready/sd1"
"$BIN" simulate-migrate --root "$WORK/ready/sd1" --to latest
"$BIN" simulate-rollback --root "$WORK/ready/sd1"

# A missing, dirty, or unejected SD2 is visible recovery; it creates no SD1 shadow folders.
for scenario in missing dirty unejected; do
    prepare "$scenario"
    python3 - "$ROOT/fixtures/storage/onboarding/ready.json" "$WORK/$scenario/inventory.json" "$scenario" <<'PY'
import json, sys
value = json.load(open(sys.argv[1], encoding="utf-8"))
scenario = sys.argv[3]
if scenario == "missing":
    value["sd2"] = None
elif scenario == "dirty":
    value["sd2"]["dirty"] = True
else:
    value["usb"]["ejectAcknowledged"] = False
json.dump(value, open(sys.argv[2], "w", encoding="utf-8"))
PY
    "$BIN" simulate-onboard --root "$WORK/$scenario" --inventory "$WORK/$scenario/inventory.json" >"$WORK/$scenario.json"
    python3 - "$WORK/$scenario.json" <<'PY'
import json, sys
value = json.load(open(sys.argv[1], encoding="utf-8"))
assert value["status"] == "recovery", value
assert value["mode"] == "read-only", value
PY
    [ ! -e "$WORK/$scenario/sd1/data" ]
    [ ! -e "$WORK/$scenario/sd1/roms" ]
done

# Reject aliases, ancestor symlinks, and semantic metadata corruption before any writes.
for scenario in alias symlink bundle-symlink corrupt; do
    prepare "$scenario"
    python3 - "$ROOT/fixtures/storage/onboarding/ready.json" "$WORK/$scenario/inventory.json" "$scenario" <<'PY'
import json, sys
value = json.load(open(sys.argv[1], encoding="utf-8"))
scenario = sys.argv[3]
if scenario == "alias":
    value["sd2"]["root"] = "sd1"
elif scenario == "symlink":
    value["sd1"]["root"] = "escape/link/sd1"
elif scenario == "bundle-symlink":
    value["bundle"]["root"] = "escape/link/bundle"
elif scenario == "corrupt":
    layout = {
        "$schema": "https://example.invalid/trimui-storage-v1.schema.json",
        "format": "brickpro-storage-layout", "schemaVersion": 1,
        "installationUuid": value["sd1"]["uuid"], "activeDataVersion": 0,
        "completedMigrations": [], "filesystem": value["sd1"]["filesystem"],
        "migrationDescriptor": "data/meta/migrations/storage-v1-to-v2.json",
        "sd2Uuid": value["sd2"]["uuid"],
    }
    import os
    os.makedirs(os.path.join(os.path.dirname(sys.argv[2]), "sd1", "data", "meta"), exist_ok=True)
    json.dump(layout, open(os.path.join(os.path.dirname(sys.argv[2]), "sd1", "data", "meta", "layout.json"), "w", encoding="utf-8"))
json.dump(value, open(sys.argv[2], "w", encoding="utf-8"))
PY
    if [ "$scenario" = symlink ]; then
        mkdir -p "$WORK/$scenario/escape/outside/sd1"
        ln -s outside "$WORK/$scenario/escape/link"
    elif [ "$scenario" = bundle-symlink ]; then
        mkdir -p "$WORK/$scenario/escape/outside"
        cp -a "$WORK/$scenario/bundle" "$WORK/$scenario/escape/outside/bundle"
        ln -s outside "$WORK/$scenario/escape/link"
    fi
    "$BIN" simulate-onboard --root "$WORK/$scenario" --inventory "$WORK/$scenario/inventory.json" >"$WORK/$scenario.json"
    python3 - "$WORK/$scenario.json" <<'PY'
import json, sys
value = json.load(open(sys.argv[1], encoding="utf-8"))
assert value["status"] == "recovery" and value["mode"] == "read-only", value
PY
    [ ! -e "$WORK/$scenario/sd1/roms" ]
    [ ! -e "$WORK/$scenario/sd2/roms" ]
    if [ "$scenario" = corrupt ]; then
        [ ! -e "$WORK/$scenario/sd1/data/config" ]
    else
        [ ! -e "$WORK/$scenario/sd1/data" ]
    fi
done

# FAT32 permits normal files but rejects >4 GiB; case collisions are rejected at the shared layout boundary.
prepare single
python3 - "$ROOT/fixtures/storage/onboarding/ready.json" "$WORK/single/inventory.json" <<'PY'
import json, sys
value = json.load(open(sys.argv[1], encoding="utf-8"))
value["twoCardRequested"] = False
value["sd2"] = None
value["usb"]["requested"] = False
for card in (value["sd1"],):
    card["filesystem"]["kind"] = "fat32"
    card["filesystem"]["maxFileBytes"] = 4294967295
    card["verificationFileBytes"] = 4096
json.dump(value, open(sys.argv[2], "w", encoding="utf-8"))
PY
"$BIN" simulate-onboard --root "$WORK/single" --inventory "$WORK/single/inventory.json" \
    --format-device 11111111-2222-3333-4444-555555555555 \
    --confirm-device 11111111-2222-3333-4444-555555555555 --confirm-format >"$WORK/single.json"
printf x >"$WORK/single/sd1/roms/copied"
mv "$WORK/single/sd1/roms/copied" "$WORK/single/sd1/roms/renamed"
rm "$WORK/single/sd1/roms/renamed"
mkdir "$WORK/single/sd1/roms/Case" "$WORK/single/sd1/roms/case"
if "$BIN" validate --layout "$WORK/single/sd1/data/meta/layout.json" --root "$WORK/single/sd1" >/dev/null 2>&1; then exit 1; fi

# Opt-in: observe the temporary exFAT verification file exceed FAT32's limit.
if [ "${BRICKPRO_LARGE_FILE_CHECK:-0}" = 1 ]; then
    prepare large
    python3 - "$WORK/large/sd1/.brickpro-format-verify" "$WORK/large/max-bytes" <<'PY' &
import os, sys, time
path, result = sys.argv[1:]
maximum = 0
while not os.path.exists(path):
    time.sleep(0.001)
while os.path.exists(path):
    maximum = max(maximum, os.path.getsize(path))
    time.sleep(0.001)
open(result, "w", encoding="utf-8").write(str(maximum))
PY
    watcher=$!
    "$BIN" simulate-onboard --root "$WORK/large" --inventory "$ROOT/fixtures/storage/onboarding/ready.json" \
        --format-device 11111111-2222-3333-4444-555555555555 \
        --confirm-device 11111111-2222-3333-4444-555555555555 --confirm-format >"$WORK/large.json"
    wait "$watcher"
    [ "$(cat "$WORK/large/max-bytes")" -gt 4294967295 ]
fi

printf '%s\n' 'storage onboarding journey: PASS'

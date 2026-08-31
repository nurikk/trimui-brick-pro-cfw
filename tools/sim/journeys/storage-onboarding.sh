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
"$BIN" simulate-onboard --root "$WORK/ready" --inventory "$ROOT/fixtures/storage/onboarding/ready.json" \
    --format-device 11111111-2222-3333-4444-555555555555 \
    --confirm-device 11111111-2222-3333-4444-555555555555 --confirm-format >"$WORK/ready.json"
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
"$BIN" simulate-onboard --root "$WORK/single" --inventory "$WORK/single/inventory.json" >"$WORK/single.json"
printf x >"$WORK/single/sd1/roms/copied"
mv "$WORK/single/sd1/roms/copied" "$WORK/single/sd1/roms/renamed"
rm "$WORK/single/sd1/roms/renamed"
mkdir "$WORK/single/sd1/roms/Case" "$WORK/single/sd1/roms/case"
if "$BIN" validate --layout "$WORK/single/sd1/data/meta/layout.json" --root "$WORK/single/sd1" >/dev/null 2>&1; then exit 1; fi

printf '%s\n' 'storage onboarding journey: PASS'

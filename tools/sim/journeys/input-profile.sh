#!/bin/sh
set -eu

ROOT=$(cd -- "$(dirname -- "$0")/../../.." && pwd -P)
BIN_DIR=${1:-$ROOT/target/release}
BIN=$BIN_DIR/input-profile-fixtures
[ -x "$BIN" ] || { printf '%s\n' "input-profile journey: missing executable $BIN" >&2; exit 1; }

WORK=$(mktemp -d "${TMPDIR:-/tmp}/brickpro-input-profile.XXXXXX")
trap 'rm -rf "$WORK"' EXIT HUP INT TERM
"$BIN" --fixture-journey --fixture-root "$WORK" >"$WORK/output.txt"
grep -q 'input-profile-fixtures: .*passed' "$WORK/output.txt"
test -f "$WORK/hall-calibration.json"
mode=$(stat -c '%a' "$WORK/hall-calibration.json")
test "$mode" = 600
printf '%s\n' 'input-profile journey: PASS'

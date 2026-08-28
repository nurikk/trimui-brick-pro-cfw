#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd -P)
export TRIMUI_DOCKER_NAMESPACE=${TRIMUI_DOCKER_NAMESPACE:-t99fa6fea-alphabet}
RUN=${1:-$(mktemp -d /tmp/trimui-alphabet-sim.XXXXXX)}
OWN_RUN=0
[ "$#" -gt 0 ] || OWN_RUN=1
mkdir -p "$RUN"
trap '[ "$OWN_RUN" -eq 1 ] && rm -rf "$RUN"' EXIT HUP INT TERM

simctl() { "$ROOT/scripts/simctl" --socket "$RUN/control.sock" "$@"; }
button() {
    simctl button --button "$1" --action press >/dev/null
    simctl button --button "$1" --action release >/dev/null
}
action() {
    name=$1
    shift
    simctl action --action "$name" "$@"
}
state() { simctl state; }

TRIMUI_DOCKER_NAMESPACE=t99fa6fea-alphabet \
    "$ROOT/scripts/sim" run --backend=dummy --catalog sim/fixtures/catalog-alphabet.json \
    --profile sim/device/tg4040-alphabet.json --run-dir "$RUN" --wait-ready 30 --detach
trap '"$ROOT/scripts/sim" stop --run-dir "$RUN" >/dev/null 2>&1 || true; [ "$OWN_RUN" -eq 1 ] && rm -rf "$RUN"' EXIT HUP INT TERM
simctl wait-ready --timeout 30 >/dev/null
simctl presentation --action home >/dev/null
button start
button down

expect_target() {
    expected=$1
    response=$(action "$2")
    python3 - "$expected" "$response" <<'PY'
import json, sys
expected, raw = sys.argv[1:]
data = json.loads(raw)['result']
assert data['presentation']['groupJump']['target'] == expected, data
assert data['selectedContentId'] == next(
    row['id'] for row in data['presentation']['gameRows'] if row['selected']
)
PY
}
expect_same() {
    before=$1
    response=$(action "$2")
    python3 - "$before" "$response" <<'PY'
import json, sys
before, raw = sys.argv[1:]
data = json.loads(raw)['result']
assert data['selectedContentId'] == before, data
assert data['presentation']['groupJump']['target'] is None, data
PY
}

# The generated catalog has intentional empty letters and both normalized/non-Latin groups.
expect_target '#' jump-next-group
expect_target 'A' jump-next-group
expect_target 'B' jump-next-group
expect_target 'C' jump-next-group
expect_target 'E' jump-next-group
expect_target 'Z' jump-next-group
expect_target 'Β' jump-next-group
expect_target 'Ж' jump-next-group
last=$(state | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["selectedContentId"])')
expect_same "$last" jump-next-group
# Release is accepted but cannot cause a second semantic jump.
release=$(action jump-next-group --phase release)
python3 - "$last" "$release" <<'PY'
import json, sys
assert json.loads(sys.argv[2])['result']['selectedContentId'] == sys.argv[1]
PY
expect_target 'Β' jump-previous-group
button r1
raw_next=$(state | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["presentation"]["groupJump"]["target"])')
test "$raw_next" = Ж

# D-pad still moves one item, while filters/details/back/rebuilt menus retain the content ID.
before=$(state | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["selectedContentId"])')
button down
after=$(state | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["selectedContentId"])')
test "$before" != "$after"
for view in favorites search media-details; do simctl presentation --action "$view" >/dev/null; done
button secondary
simctl presentation --action games >/dev/null
stable=$(state | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["selectedContentId"])')
test "$stable" = "$after"

# Walk to the first group and verify the other bounded edge.
for _ in 1 2 3 4 5 6 7 8; do action jump-previous-group >/dev/null; done
first=$(state | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["selectedContentId"])')
edge=$(action jump-previous-group)
python3 - "$first" "$edge" <<'PY'
import json, sys
data=json.loads(sys.argv[2])['result']
assert data['selectedContentId'] == sys.argv[1]
assert data['presentation']['groupJump']['target'] is None
PY

"$ROOT/scripts/sim" stop --run-dir "$RUN"
python3 - "$RUN" "$ROOT/sim/contracts/control.schema.json" <<'PY'
import json, sys
from pathlib import Path
run, schema_path = map(Path, sys.argv[1:])
try:
    import jsonschema
except ImportError:
    jsonschema = None
state = json.loads((run / 'route-selection.json').read_text())
assert state['lane'] == 'host-native userspace simulator'
index = json.loads((run / 'data' / 'rom-index.json').read_text())
catalog = json.loads(Path('sim/fixtures/catalog-alphabet.json').read_text())
assert len(catalog['entries']) == 1403
assert len(index['entries']) == 1402
lines = [json.loads(line) for line in (run / 'logs' / 'launcher.jsonl').read_text().splitlines()]
frames = [line for line in lines if line['event'] == 'input_to_frame']
assert frames and max(line['latencyUs'] for line in frames) < 1_000_000
semantic = [line for line in lines if line['event'] == 'control' and line.get('semanticAction') in ('jump-next-group', 'jump-previous-group')]
assert semantic and {line['semanticAction'] for line in semantic} == {'jump-next-group', 'jump-previous-group'}
if jsonschema:
    jsonschema.Draft202012Validator(json.loads(schema_path.read_text())).check_schema(json.loads(schema_path.read_text()))
print(f'alphabet-group-jumps journey: PASS (groups, semantic API, edges, filters, rebuild, D-pad, {len(index["entries"])} entries) {run}')
PY

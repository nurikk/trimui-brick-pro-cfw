#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd -P)
. "$ROOT/scripts/docker-worktree.sh"
DOCKER_NAMESPACE=$(trimui_docker_namespace "$ROOT")
WORK=${1:-$(mktemp -d "${TMPDIR:-/tmp}/brickpro-performance-profiles.XXXXXX")}
RUN1=$WORK/smoke
RUN2=$WORK/matrix
RUNS=
mkdir -p "$RUN1" "$RUN2"

cleanup() {
    for run in $RUNS; do
        timeout 20 "$ROOT/scripts/sim" stop --run-dir "$run" >/dev/null 2>&1 || true
    done
}
trap cleanup EXIT HUP INT TERM

ctl() {
    name=$(printf '%s' "$1" | cksum | awk -v ns="$DOCKER_NAMESPACE" '{print "trimui-sim-" ns "-" $1}')
    timeout 10 docker exec -i --user 10001:10001 "$name" python3 - /evidence/control.sock "$2" "$3" <<'PY'
import json, socket, sys
request = {"version": "sim-control/v1", "id": "power-journey", "command": sys.argv[2], "args": json.loads(sys.argv[3])}
with socket.socket(socket.AF_UNIX) as stream:
    stream.settimeout(5)
    stream.connect(sys.argv[1])
    stream.sendall(json.dumps(request, separators=(",", ":")).encode() + b"\n")
    stream.shutdown(socket.SHUT_WR)
    response = b""
    while chunk := stream.recv(65536):
        response += chunk
value = json.loads(response)
print(json.dumps(value, separators=(",", ":")))
raise SystemExit(0 if value.get("ok") else 2)
PY
}
start() {
    run=$1
    RUNS="$RUNS $run"
    timeout 90 "$ROOT/scripts/sim" run --backend=dummy \
        --profile sim/device/tg4040-alphabet.json --run-dir "$run" --wait-ready 30 --detach
    ctl "$run" presentation '{"action":"home"}' >/dev/null
}
button() { ctl "$1" button "{\"button\":\"$2\",\"action\":\"press\"}" >/dev/null; }
state() { ctl "$1" state '{}'; }
profile() {
    state "$1" | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["power"][sys.argv[1]])' "$2"
}
assert_policy() {
    run=$1 requested=$2 effective=$3 context=$4
    state "$run" >"$run/power-state.json"
    python3 - "$run/power-state.json" "$requested" "$effective" "$context" <<'PY'
import json, sys
power = json.load(open(sys.argv[1], encoding="utf-8"))["result"]["power"]
assert power["requestedProfile"] == sys.argv[2], power
assert power["effectiveProfile"] == sys.argv[3], power
assert power["context"] == sys.argv[4], power
assert power["hardwareVerified"] is False
assert power["realDeviceOperations"] == "denied"
assert power["globalDefault"] == "balanced"
assert power["throttlingEnabled"] is True
assert power["thermalLimitC"] == 75
assert power["policy"]["display"] == {"width": 1024, "height": 768, "refreshHz": 60, "mode": "1024x768@60"}
PY
}
launch() {
    run=$1 id=$2
    button "$run" menu
    case "$id" in
        nebula-nes)
            button "$run" primary; button "$run" down; button "$run" primary ;;
        mirror-ps1)
            button "$run" primary; button "$run" down; button "$run" down; button "$run" primary ;;
        signal-workshop)
            for _ in 1 2 3 4 5 6 7; do button "$run" down; done
            button "$run" primary; button "$run" down ;;
        orbit-garden)
            for _ in 1 2 3 4 5 6 7; do button "$run" down; done
            button "$run" primary; button "$run" primary; button "$run" primary ;;
        *) echo "unknown generated content $id" >&2; exit 1 ;;
    esac
    [ "$(state "$run" | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["activeSession"]["contentId"])')" = "$id" ]
}

printf '%s\n' 'performance profiles: representative smoke first; 2 roots total; worst-case <6 minutes (60 controls x 5s + startup)'
start "$RUN1"
assert_policy "$RUN1" eco eco launcher
launch "$RUN1" signal-workshop
assert_policy "$RUN1" eco eco game
ctl "$RUN1" adapter '{"action":"complete","status":0,"value":0}' >/dev/null
assert_policy "$RUN1" eco eco launcher

start "$RUN2"
launch "$RUN2" nebula-nes
assert_policy "$RUN2" eco eco game
[ "$(profile "$RUN2" temporaryGamePolicy)" = False ]
ctl "$RUN2" power '{"operation":"override","profile":"performance"}' >/dev/null
assert_policy "$RUN2" performance performance game
[ "$(profile "$RUN2" temporaryGamePolicy)" = True ]
ctl "$RUN2" lifecycle '{"operation":"suspend","timeoutMs":5000}' >/dev/null
assert_policy "$RUN2" eco eco suspend
ctl "$RUN2" lifecycle '{"operation":"resume","timeoutMs":5000,"wakeSource":"user"}' >/dev/null
assert_policy "$RUN2" eco eco launcher
ctl "$RUN2" adapter '{"action":"crash","status":1,"value":0}' >/dev/null
assert_policy "$RUN2" eco eco launcher

launch "$RUN2" mirror-ps1
assert_policy "$RUN2" balanced balanced game
ctl "$RUN2" adapter '{"action":"complete","status":0,"value":0}' >/dev/null

launch "$RUN2" orbit-garden
assert_policy "$RUN2" performance performance game
ctl "$RUN2" power '{"operation":"temperature","temperatureC":76}' >/dev/null
assert_policy "$RUN2" performance eco game
[ "$(profile "$RUN2" effectiveSource)" = thermal-limit ]
ctl "$RUN2" power '{"operation":"temperature","temperatureC":69}' >/dev/null
assert_policy "$RUN2" performance performance game
button "$RUN2" menu
for _ in 1 2 3 4 5; do button "$RUN2" down; done
button "$RUN2" primary
button "$RUN2" primary
assert_policy "$RUN2" eco eco launcher
[ "$(profile "$RUN2" safeModeReset)" = True ]
[ "$(state "$RUN2" | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["route"])')" = library ]

python3 - "$ROOT/fixtures/power-policy/benchmark-matrix.json" <<'PY'
import json, sys
matrix = json.load(open(sys.argv[1], encoding="utf-8"))
assert matrix["targetSku"] == "TG4040"
assert matrix["lane"] == "host-native userspace simulator"
assert matrix["hardwareVerified"] is False
assert matrix["conditions"]["refreshHz"] == 60
assert {row["kind"] for row in matrix["rows"]} >= {"emulator", "portmaster", "thermal-degrade"}
for row in matrix["rows"]:
    assert all(key in row for key in ("fps", "p99FrameMs", "temperatureC", "powerW")), row
PY

cleanup
RUNS=
printf '%s\n' "performance profiles journey: PASS ($WORK)"

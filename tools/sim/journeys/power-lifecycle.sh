#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd -P)
if [ "$#" -gt 1 ]; then
    echo "usage: $0 [EVIDENCE_ROOT]" >&2
    exit 2
fi
if [ "$#" -eq 1 ]; then
    WORK=$1
    OWN_WORK=0
    mkdir -p "$WORK"
else
    WORK=$(mktemp -d "${TMPDIR:-/tmp}/brickpro-power-lifecycle.XXXXXX")
    OWN_WORK=1
fi
trap 'for run in "$WORK"/*; do [ -d "$run" ] || continue; "$ROOT/scripts/sim" stop --run-dir "$run" >/dev/null 2>&1 || true; done; [ "$OWN_WORK" -eq 1 ] && rm -rf "$WORK"' EXIT HUP INT TERM

start() {
    run=$1
    mkdir -p "$run"
    "$ROOT/scripts/sim" run --backend=dummy --run-dir "$run" --wait-ready 30 --detach
}

stop() {
    "$ROOT/scripts/sim" stop --run-dir "$1" --wait-ready 30 >/dev/null
}

ctl() {
    run=$1
    shift
    "$ROOT/scripts/simctl" --socket "$run/control.sock" "$@"
}
state_file() {
    ctl "$1" state | python3 -c 'import json, sys; print(json.dumps(json.load(sys.stdin)["result"]))'
}
expect_failure() {
    run=$1
    shift
    if ctl "$run" "$@" >/dev/null; then
        echo "expected failure: $*" >&2
        exit 1
    fi
}

launch() {
    run=$1
    ctl "$run" button --button start --action press >/dev/null
    ctl "$run" button --button down --action press >/dev/null
    ctl "$run" button --button primary --action press >/dev/null
}

assert_state() {
    python3 - "$@" <<'PY'
import json
import sys

path, mode = sys.argv[1:]
state = json.load(open(path, encoding="utf-8"))
lifecycle = state["lifecycle"]
hardware = state["hardware"]
platform = state["platformState"]
if mode == "suspended":
    assert state["activeSession"] is not None
    assert lifecycle["phase"] == "suspended"
    assert lifecycle["launchesAllowed"] is False
    assert lifecycle["backgroundAllowed"] is False
    assert hardware["suspend"] == {"state": "suspended", "result": "success"}
    saved = lifecycle["savedState"]
    assert saved is not None
    assert saved["radios"]["wifi"] == {"enabled": True, "connected": True}
    assert saved["audio"]["enabled"] is True
    assert saved["audio"]["active"] is False
    assert saved["input"]["pressed"] == []
    assert platform["audio"]["enabled"] is False
    assert platform["input"]["pressed"] == []
    assert platform["radios"]["wifi"] == {"enabled": False, "connected": False}
    events = [entry["event"] for entry in lifecycle["journal"]]
    order = ["checkpoint-complete", "quiesce-audio", "quiesce-input", "quiesce-radios", "suspend-complete"]
    positions = [events.index(event) for event in order]
    assert positions == sorted(positions), events
elif mode == "awake":
    assert lifecycle["phase"] == "awake"
    assert lifecycle["launchesAllowed"] is True
    assert lifecycle["backgroundAllowed"] is True
    assert hardware["suspend"] == {"state": "active", "result": "none"}
else:
    raise AssertionError(mode)
PY
}

# Ordinary suspend/resume, typed snapshot, ordering, gates, and re-entrancy.
NORMAL=$WORK/normal
start "$NORMAL"
ctl "$NORMAL" hardware set radio.enabled=true radio.connected=true >/dev/null
launch "$NORMAL"
state_file "$NORMAL" >"$NORMAL/before.json"
python3 - "$NORMAL/before.json" <<'PY'
import json, sys
state = json.load(open(sys.argv[1], encoding="utf-8"))
assert state["activeSession"] is not None
assert state["lifecycle"]["phase"] == "awake"
PY
ctl "$NORMAL" lifecycle suspend --timeout 5 >"$NORMAL/suspend.json"
state_file "$NORMAL" >"$NORMAL/suspended.json"
assert_state "$NORMAL/suspended.json" suspended
[ "$(python3 - "$NORMAL/suspended.json" <<'PY'
import json, sys
state = json.load(open(sys.argv[1], encoding="utf-8"))
print(state["lifecycle"]["marker"]["checkpointGeneration"])
PY
)" -ge 1 ]
expect_failure "$NORMAL" lifecycle suspend --timeout 5
expect_failure "$NORMAL" button --button primary --action press
ctl "$NORMAL" lifecycle resume --timeout 5 >"$NORMAL/resume.json"
state_file "$NORMAL" >"$NORMAL/resumed.json"
assert_state "$NORMAL/resumed.json" awake
python3 - "$NORMAL/suspended.json" "$NORMAL/resumed.json" <<'PY'
import json, sys
suspended = json.load(open(sys.argv[1], encoding="utf-8"))
resumed = json.load(open(sys.argv[2], encoding="utf-8"))
saved = suspended["lifecycle"]["savedState"]
current = resumed["platformState"]
for domain in ("audio", "input", "radios"):
    assert current[domain] == saved[domain], (domain, current[domain], saved[domain])
events = [entry["event"] for entry in resumed["lifecycle"]["journal"]]
order = ["resume-active", "restore-radios", "restore-input", "restore-audio", "resume-complete"]
positions = [events.index(event) for event in order]
assert positions == sorted(positions), events
PY
expect_failure "$NORMAL" lifecycle resume --timeout 5
stop "$NORMAL"

# Checkpoint failure preserves the prior generation and never quiesces.
CHECKPOINT=$WORK/checkpoint
start "$CHECKPOINT"
launch "$CHECKPOINT"
ctl "$CHECKPOINT" autosave --reason periodic >/dev/null
before=$(python3 - "$CHECKPOINT/data/resume/current.json" <<'PY'
import json, sys
print(json.load(open(sys.argv[1], encoding="utf-8"))["generation"])
PY
)
ctl "$CHECKPOINT" fault set checkpoint-fail >/dev/null
expect_failure "$CHECKPOINT" lifecycle suspend --timeout 5
state_file "$CHECKPOINT" >"$CHECKPOINT/state.json"
python3 - "$CHECKPOINT/state.json" "$CHECKPOINT/data/resume/current.json" "$before" <<'PY'
import json, sys
state = json.load(open(sys.argv[1], encoding="utf-8"))
current = json.load(open(sys.argv[2], encoding="utf-8"))
assert state["lifecycle"]["phase"] == "recovery"
assert state["hardware"]["suspend"]["state"] == "active"
assert state["lifecycle"]["marker"]["reason"] == "checkpoint-failed"
assert current["generation"] == int(sys.argv[3])
assert not any(entry["event"] == "quiesce-audio" for entry in state["lifecycle"]["journal"])
PY
stop "$CHECKPOINT"

# Quiesce failure rolls back in reverse without claiming suspension.
QUIESCE=$WORK/quiesce
start "$QUIESCE"
launch "$QUIESCE"
ctl "$QUIESCE" fault set quiesce-audio-fail >/dev/null
expect_failure "$QUIESCE" lifecycle suspend --timeout 5
state_file "$QUIESCE" >"$QUIESCE/state.json"
python3 - "$QUIESCE/state.json" <<'PY'
import json, sys
state = json.load(open(sys.argv[1], encoding="utf-8"))
assert state["lifecycle"]["phase"] == "awake"
assert state["hardware"]["suspend"]["state"] == "active"
assert state["lifecycle"]["marker"]["reason"] == "audio-failed-rolled-back"
assert state["lifecycle"]["savedState"] is None
assert "rollback-complete" in [entry["event"] for entry in state["lifecycle"]["journal"]]
PY
stop "$QUIESCE"

# HAL loss fails closed before checkpoint or quiescence.
HAL=$WORK/hal
start "$HAL"
ctl "$HAL" fault set hal-loss >/dev/null
expect_failure "$HAL" lifecycle suspend --timeout 5
state_file "$HAL" >"$HAL/state.json"
python3 - "$HAL/state.json" <<'PY'
import json, sys
state = json.load(open(sys.argv[1], encoding="utf-8"))
assert state["lifecycle"]["phase"] == "recovery"
assert state["hardware"]["suspend"]["state"] == "active"
assert state["lifecycle"]["launchesAllowed"] is False
assert state["lifecycle"]["backgroundAllowed"] is False
PY
stop "$HAL"

# Resume failure leaves the saved checkpoint and a pending marker.
RESUME_FAIL=$WORK/resume-fail
start "$RESUME_FAIL"
launch "$RESUME_FAIL"
ctl "$RESUME_FAIL" lifecycle suspend --timeout 5 >/dev/null
ctl "$RESUME_FAIL" fault set resume-audio-fail >/dev/null
expect_failure "$RESUME_FAIL" lifecycle resume --timeout 5
state_file "$RESUME_FAIL" >"$RESUME_FAIL/state.json"
python3 - "$RESUME_FAIL/state.json" <<'PY'
import json, sys
state = json.load(open(sys.argv[1], encoding="utf-8"))
assert state["lifecycle"]["phase"] == "recovery"
assert state["lifecycle"]["marker"]["reason"] == "resume-audio-failed"
assert state["lifecycle"]["savedState"] is not None
assert state["hardware"]["suspend"]["state"] == "active"
PY
stop "$RESUME_FAIL"

# A fresh simulator with the pending marker remains conservatively gated.
COLD=$WORK/cold
mkdir -p "$COLD"
cp "$RESUME_FAIL/lifecycle-marker.json" "$COLD/lifecycle-marker.json"
start "$COLD"
state_file "$COLD" >"$COLD/state.json"
python3 - "$COLD/state.json" <<'PY'
import json, sys
state = json.load(open(sys.argv[1], encoding="utf-8"))
lifecycle = state["lifecycle"]
assert lifecycle["phase"] == "recovery"
assert lifecycle["marker"]["reason"].startswith("cold-recovery:")
assert lifecycle["launchesAllowed"] is False
assert lifecycle["backgroundAllowed"] is False
assert "cold-recovery" in [entry["event"] for entry in lifecycle["journal"]]
PY
expect_failure "$COLD" button --button primary --action press
stop "$COLD"

# Exercise the same semantic flow through the headed backend when available.
HEADED=$WORK/headed
mkdir -p "$HEADED"
if "$ROOT/scripts/sim" run --backend=x11 --run-dir "$HEADED" --wait-ready 30 --detach; then
    launch "$HEADED"
    ctl "$HEADED" lifecycle suspend --timeout 5 >/dev/null
    state_file "$HEADED" >"$HEADED/suspended.json"
    python3 - "$HEADED/suspended.json" <<'PY'
import json, sys
state = json.load(open(sys.argv[1], encoding="utf-8"))
assert state["lifecycle"]["phase"] == "suspended"
assert state["hardware"]["suspend"]["state"] == "suspended"
assert state["lifecycle"]["launchesAllowed"] is False
assert state["lifecycle"]["backgroundAllowed"] is False
PY
    ctl "$HEADED" screenshot --name headed-suspended >/dev/null
    ctl "$HEADED" lifecycle resume --timeout 5 >/dev/null
    state_file "$HEADED" >"$HEADED/resumed.json"
    python3 - "$HEADED/resumed.json" <<'PY'
import json, sys
state = json.load(open(sys.argv[1], encoding="utf-8"))
assert state["lifecycle"]["phase"] == "awake"
assert state["hardware"]["suspend"]["state"] == "active"
assert state["lifecycle"]["launchesAllowed"] is True
assert state["lifecycle"]["backgroundAllowed"] is True
PY
    ctl "$HEADED" screenshot --name headed-resumed >/dev/null
    stop "$HEADED"
    echo "power lifecycle journey: headed X11 evidence PASS ($HEADED)"
else
    status=$?
    echo "power lifecycle journey: headed X11 unavailable (scripts/sim status $status); dummy semantic evidence remained green" >&2
fi

printf '%s\n' "power lifecycle journey: PASS (ordering, typed restore, checkpoint/recovery, gates, marker cold recovery) $WORK"

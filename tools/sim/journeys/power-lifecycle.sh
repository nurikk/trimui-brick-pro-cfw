#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd -P)
if [ "$#" -gt 1 ]; then
    echo "usage: $0 [EVIDENCE_ROOT]" >&2
    exit 2
fi
if [ "$#" -eq 1 ]; then
    WORK=$1
    mkdir -p "$WORK"
else
    WORK=$(mktemp -d "${TMPDIR:-/tmp}/brickpro-power-lifecycle.XXXXXX")
fi
RUNS=
cleanup() {
    for run in $RUNS; do
        "$ROOT/scripts/sim" stop --run-dir "$run" >/dev/null 2>&1 || true
    done
}
trap cleanup EXIT HUP INT TERM

start() {
    run=$1
    mkdir -p "$run"
    RUNS="$RUNS $run"
    "$ROOT/scripts/sim" run --backend=dummy --run-dir "$run" --wait-ready 30 --detach
}
ctl() {
    run=$1
    shift
    "$ROOT/scripts/simctl" --socket "$run/control.sock" "$@"
}
start_session() {
    run=$1
    ctl "$run" button --button menu --action press >/dev/null
    ctl "$run" button --button primary --action press >/dev/null
    ctl "$run" button --button down --action press >/dev/null
    ctl "$run" button --button primary --action press >/dev/null
}
state_file() {
    ctl "$1" state | python3 -c 'import json,sys; print(json.dumps(json.load(sys.stdin)["result"]))'
}
expect_failure() {
    run=$1
    shift
    if ctl "$run" "$@" >/dev/null; then
        echo "expected failure: $*" >&2
        exit 1
    fi
}
assert_orderly() {
    python3 - "$1" "$2" <<'PY'
import json, sys
state = json.load(open(sys.argv[1], encoding="utf-8"))
lifecycle = state["lifecycle"]
assert lifecycle["phase"] == "orderly-shutdown", lifecycle
assert lifecycle["shutdownRequest"]["reason"] == sys.argv[2], lifecycle
assert lifecycle["launchesAllowed"] is False
assert lifecycle["backgroundAllowed"] is False
assert lifecycle["armedDeadline"] is None
assert state["activeSession"] is None
PY
}
assert_suspended() {
    python3 - "$1" <<'PY'
import json, sys
state = json.load(open(sys.argv[1], encoding="utf-8"))
lifecycle = state["lifecycle"]
assert lifecycle["phase"] == "suspended"
assert lifecycle["armedDeadline"]["durationMinutes"] == 5
assert lifecycle["armedDeadline"]["monotonicDeadlineMs"] > state["clock"]["monotonicMs"]
assert lifecycle["armedDeadline"]["bootTimeDeadlineMs"] > state["clock"]["wallClockMs"]
assert state["hardware"]["suspend"] == {"state": "suspended", "result": "success"}
platform = state["platformState"]
assert platform["audio"]["enabled"] is False and platform["audio"]["active"] is False
assert platform["input"]["pressed"] == []
assert platform["rumble"]["active"] is False
assert platform["usb"]["role"] == "None"
assert platform["leds"]["on"] is False
assert platform["radios"]["wifi"]["enabled"] is False
assert [event["event"] for event in lifecycle["journal"]][-10:] == [
    "checkpoint-complete", "alarm-cleared", "deadline-armed", "quiesce-audio",
    "quiesce-input", "quiesce-rumble", "quiesce-usb", "quiesce-leds",
    "quiesce-radios", "suspend-complete",
]
assert [event["event"] for event in lifecycle["journal"]][-1] == "suspend-complete"
PY
}

# Manual wake at 4:59 clears the first deadline and restores the checkpoint.
MANUAL=$WORK/manual-459
start "$MANUAL"
start_session "$MANUAL"
ctl "$MANUAL" lifecycle suspend --timeout 5 >/dev/null
ctl "$MANUAL" clock advance --milliseconds 298856 >/dev/null
state_file "$MANUAL" >"$MANUAL/at-459.json"
assert_suspended "$MANUAL/at-459.json"
ctl "$MANUAL" lifecycle resume --timeout 5 --source user >/dev/null
state_file "$MANUAL" >"$MANUAL/manual-wake.json"
python3 - "$MANUAL/manual-wake.json" <<'PY'
import json, sys
state = json.load(open(sys.argv[1]))
lifecycle = state["lifecycle"]
assert lifecycle["phase"] == "awake"
assert [event["event"] for event in lifecycle["journal"]][-9:] == [
    "resumed-by-user", "resume-active", "restore-radios", "restore-leds",
    "restore-usb", "restore-rumble", "restore-input", "restore-audio",
    "resume-complete",
]
platform = state["platformState"]
assert platform["input"]["pressed"] == []
assert platform["audio"]["enabled"] is True
PY
ctl "$MANUAL" lifecycle suspend --timeout 5 >/dev/null
state_file "$MANUAL" >"$MANUAL/fresh-suspend.json"
assert_suspended "$MANUAL/fresh-suspend.json"
ctl "$MANUAL" clock advance --milliseconds 300000 >/dev/null
state_file "$MANUAL" >"$MANUAL/deadline.json"
python3 - "$MANUAL/deadline.json" <<'PY'
import json, sys
state = json.load(open(sys.argv[1]))
assert state["lifecycle"]["phase"] == "orderly-shutdown"
assert state["lifecycle"]["wakeSource"] == "deadline"
assert state["lifecycle"]["shutdownRequest"]["status"] == "terminal"
PY
ctl "$MANUAL" lifecycle shutdown --timeout 5 >/dev/null 2>&1 || true

# Checkpoint failure is bounded and never reaches suspend.
CHECKPOINT=$WORK/checkpoint-failure
start "$CHECKPOINT"
ctl "$CHECKPOINT" fault set checkpoint-fail >/dev/null
expect_failure "$CHECKPOINT" lifecycle suspend --timeout 5
state_file "$CHECKPOINT" >"$CHECKPOINT/state.json"
assert_orderly "$CHECKPOINT/state.json" checkpoint-failure

# Arming, verification, and both crash points fail closed without entering suspended.
for case in arm-failure verify-failure crash-before-suspend crash-with-armed-journal; do
    run=$WORK/$case
    start "$run"
    fault=arm-fail
    reason=arm-failure
    [ "$case" = verify-failure ] && fault=verify-fail && reason=verify-failure
    [ "$case" = crash-before-suspend ] && fault=crash-before-suspend && reason=crash-before-suspend
    [ "$case" = crash-with-armed-journal ] && fault=crash-armed-journal && reason=crash-with-armed-journal
    ctl "$run" fault set "$fault" >/dev/null
    expect_failure "$run" lifecycle suspend --timeout 5
    state_file "$run" >"$run/state.json"
    assert_orderly "$run/state.json" "$reason"
done

# Cold recovery consumes a checksummed marker and never revives the session.
COLD=$WORK/cold-recovery
mkdir -p "$COLD"
start "$COLD"
"$ROOT/scripts/sim" stop --run-dir "$COLD"
rm -rf "$COLD/logs" "$COLD/screenshots" "$COLD/checkpoints" "$COLD/readiness.json" "$COLD/route-selection.json" "$COLD/launch.json" "$COLD/launch-request.json" "$COLD/session.json" "$COLD/exit-status.json"
for artifact in lifecycle-marker.json lifecycle-marker.checksum lifecycle-journal.json lifecycle-journal.checksum; do
    cp "$WORK/crash-with-armed-journal/data/$artifact" "$COLD/data/"
done
start "$COLD"
state_file "$COLD" >"$COLD/state.json"
python3 - "$COLD/state.json" <<'PY'
import json, sys
state = json.load(open(sys.argv[1]))
lifecycle = state["lifecycle"]
assert lifecycle["phase"] == "recovery"
assert lifecycle["launchesAllowed"] is False
assert lifecycle["shutdownRequest"]["reason"] == "cold-recovery"
assert state["activeSession"] is None
PY

# A foreign/stale alarm is ignored and the next sleep gets a fresh token.
STALE=$WORK/stale-alarm
start "$STALE"
ctl "$STALE" lifecycle suspend --timeout 5 >/dev/null
ctl "$STALE" lifecycle resume --timeout 5 --source stale-alarm >/dev/null
state_file "$STALE" >"$STALE/state.json"
python3 - "$STALE/state.json" <<'PY'
import json, sys
state = json.load(open(sys.argv[1]))
assert state["lifecycle"]["phase"] == "awake"
assert state["lifecycle"]["wakeSource"] == "stale-alarm"
assert state["lifecycle"]["wakeReason"] == "stale-alarm-ignored"
assert state["lifecycle"]["shutdownRequest"] is None
PY
ctl "$STALE" lifecycle suspend --timeout 5 >/dev/null

# A cancellation racing the deadline wins before the exact boundary.
CANCEL=$WORK/cancellation-race
start "$CANCEL"
ctl "$CANCEL" lifecycle suspend --timeout 5 >/dev/null
ctl "$CANCEL" clock advance --milliseconds 299855 >/dev/null
ctl "$CANCEL" lifecycle resume --timeout 5 --source user >/dev/null
ctl "$CANCEL" clock advance --milliseconds 1 >/dev/null
state_file "$CANCEL" >"$CANCEL/state.json"
python3 - "$CANCEL/state.json" <<'PY'
import json, sys
state = json.load(open(sys.argv[1]))
assert state["lifecycle"]["phase"] == "awake"
assert state["lifecycle"]["shutdownRequest"] is None
PY

# A forward boot-time/RTC jump is battery-safe: it reaches the persisted deadline immediately.
CLOCK=$WORK/clock-jump
start "$CLOCK"
ctl "$CLOCK" lifecycle suspend --timeout 5 >/dev/null
ctl "$CLOCK" clock jump --minutes 60 >"$CLOCK/jump.json"
python3 - "$CLOCK/jump.json" <<'PY'
import json, sys
state = json.load(open(sys.argv[1]))["result"]
lifecycle = state["lifecycle"]
assert lifecycle["phase"] == "orderly-shutdown"
assert lifecycle["wakeSource"] == "deadline"
assert lifecycle["shutdownRequest"]["reason"] == "deadline"
assert state["clock"]["wallClockMs"] == 3600000
PY

# Alarm-clear failures fail closed through typed orderly shutdown on both wake paths.
for case in user-alarm-clear-failure deadline-alarm-clear-failure; do
    run=$WORK/$case
    start "$run"
    ctl "$run" lifecycle suspend --timeout 5 >/dev/null
    ctl "$run" fault set clear-fail >/dev/null
    if [ "$case" = user-alarm-clear-failure ]; then
        expect_failure "$run" lifecycle resume --timeout 5 --source user
    else
        ctl "$run" clock advance --milliseconds 300000 >/dev/null
    fi
    state_file "$run" >"$run/state.json"
    assert_orderly "$run/state.json" alarm-clear-failure
done

# Low battery requests typed orderly shutdown; external power changes remain typed observations.
POWER=$WORK/power-events
start "$POWER"
start_session "$POWER"
ctl "$POWER" hardware set battery.externalPower=true >/dev/null
ctl "$POWER" hardware set battery.externalPower=false >/dev/null
ctl "$POWER" hardware set battery.percent=5 >"$POWER/low-battery.json"
python3 - "$POWER/low-battery.json" <<'PY'
import json, sys
state = json.load(open(sys.argv[1]))["result"]
assert state["hardware"]["externalPower"] is False
assert state["lifecycle"]["shutdownRequest"]["reason"] == "low-battery"
assert state["lifecycle"]["phase"] == "orderly-shutdown"
root = sys.argv[1].removesuffix("low-battery.json")
pointer = json.load(open(root + "data/resume/current.json"))
assert pointer["generation"] >= 1
record = json.load(open(root + f"data/resume/generations/generation-{pointer['generation']}/record.json"))
assert record["reason"] == "low-battery"
PY

# Shutdown retry is bounded and succeeds only after the fault is cleared.
RETRY=$WORK/shutdown-retry
start "$RETRY"
ctl "$RETRY" lifecycle suspend --timeout 5 >/dev/null
ctl "$RETRY" fault set shutdown-fail >/dev/null
ctl "$RETRY" clock advance --milliseconds 300000 >/dev/null
state_file "$RETRY" >"$RETRY/retry-pending.json"
python3 - "$RETRY/retry-pending.json" <<'PY'
import json, sys
state = json.load(open(sys.argv[1]))
request = state["lifecycle"]["shutdownRequest"]
assert state["lifecycle"]["phase"] == "orderly-shutdown"
assert request["status"] == "pending" and request["attempts"] == 1
PY
ctl "$RETRY" fault clear shutdown-fail >/dev/null
ctl "$RETRY" lifecycle shutdown --timeout 5 >/dev/null
state_file "$RETRY" >"$RETRY/retry-terminal.json"
python3 - "$RETRY/retry-terminal.json" <<'PY'
import json, sys
request = json.load(open(sys.argv[1]))["lifecycle"]["shutdownRequest"]
assert request["status"] == "terminal" and request["attempts"] == 2
PY

# HAL loss and repeated manual/deadline cycles remain gated and deterministic.
HAL=$WORK/hal-loss
start "$HAL"
ctl "$HAL" fault set hal-loss >/dev/null
expect_failure "$HAL" lifecycle suspend --timeout 5
state_file "$HAL" >"$HAL/state.json"
assert_orderly "$HAL/state.json" hal-loss

REPEAT=$WORK/repeated-cycles
start "$REPEAT"
ctl "$REPEAT" lifecycle suspend --timeout 5 >/dev/null
ctl "$REPEAT" lifecycle resume --timeout 5 --source user >/dev/null
ctl "$REPEAT" lifecycle suspend --timeout 5 >/dev/null
ctl "$REPEAT" clock advance --milliseconds 300000 >/dev/null
state_file "$REPEAT" >"$REPEAT/state.json"
assert_orderly "$REPEAT/state.json" deadline

# Repeat the semantic deadline path through the headed X11 lane when available.
HEADED=$WORK/headed-x11
mkdir -p "$HEADED"
if "$ROOT/scripts/sim" run --backend=x11 --run-dir "$HEADED" --wait-ready 30 --detach; then
    ctl "$HEADED" lifecycle suspend --timeout 5 >/dev/null
    ctl "$HEADED" clock advance --milliseconds 300000 >/dev/null
    state_file "$HEADED" >"$HEADED/state.json"
    assert_orderly "$HEADED/state.json" deadline
    ctl "$HEADED" screenshot --name headed-deadline >/dev/null
    "$ROOT/scripts/sim" stop --run-dir "$HEADED"
    printf '%s\n' "power lifecycle journey: headed X11 semantic evidence PASS ($HEADED)"
else
    status=$?
    printf '%s\n' "power lifecycle journey: headed X11 unavailable (scripts/sim status $status); dummy semantic evidence remained green" >&2
fi

printf '%s\n' "power lifecycle journey: PASS (4:59 cancel, deadline/RTC-jump shutdown, arm/crash/stale/alarm-clear/clock/power/HAL/retry/repeat coverage) $WORK"

#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd -P)
NAMESPACE=t867a27e7-smoke

fail() {
    printf '%s\n' "simulator-smoke: $*" >&2
    exit 1
}

usage() {
    printf '%s\n' "usage: $0 --out ABSOLUTE_EMPTY_DIR" >&2
    exit 2
}

if [ "$#" -eq 2 ] && [ "$1" = "--out" ]; then
    OUT=$2
elif [ "$#" -eq 1 ]; then
    case "$1" in
    --out=*) OUT=${1#--out=} ;;
    *) usage ;;
    esac
else
    usage
fi
case "$OUT" in /*) ;; *) fail "--out must be an absolute path" ;; esac
[ -d "$OUT" ] || fail "--out must be an existing directory"
OUT=$(CDPATH= cd -- "$OUT" && pwd -P) || fail "cannot resolve --out"
[ "$OUT" != / ] || fail "--out cannot be /"
case "$OUT" in
"$ROOT" | "$ROOT"/*) fail "--out must be outside the repository" ;;
esac
[ -z "$(find "$OUT" -mindepth 1 -maxdepth 1 -print -quit)" ] || fail "--out must be empty"

command -v docker >/dev/null 2>&1 || fail "docker is required"
command -v python3 >/dev/null 2>&1 || fail "python3 is required"
. "$ROOT/scripts/docker-worktree.sh"
export TRIMUI_DOCKER_NAMESPACE=$NAMESPACE
[ "$(trimui_docker_namespace "$ROOT")" = "$NAMESPACE" ] || fail "namespace override failed"

scoped_container_count() {
    docker ps -aq \
        --filter 'label=org.trimui-brick-pro-cfw.simulator=host-native' \
        --filter "label=org.trimui-brick-pro-cfw.worktree=$NAMESPACE" |
        awk 'NF { count += 1 } END { print count + 0 }'
}

[ "$(scoped_container_count)" -eq 0 ] || fail "namespace already owns simulator containers"

RUN1=$OUT/run-1
RUN2=$OUT/run-2
mkdir -p "$RUN1" "$RUN2"
[ ${#RUN1} -le 87 ] && [ ${#RUN2} -le 87 ] || fail "run path is too long for control.sock"

cleanup() {
    status=$?
    trap - EXIT HUP INT TERM
    for run in "$RUN1" "$RUN2"; do
        if [ ! -e "$run/control.sock" ] && grep -q '"cleanShutdown":true' "$run/exit-status.json" 2>/dev/null; then
            continue
        fi
        if [ -d "$run" ]; then
            "$ROOT/scripts/sim" stop --backend=dummy --run-dir "$run" --wait-ready 30 \
                >"$run/cleanup.log" 2>&1 || true
        fi
    done
    [ "$(scoped_container_count)" -eq 0 ] || status=1
    exit "$status"
}
trap cleanup EXIT HUP INT TERM

if ! "$ROOT/scripts/sim" build >"$OUT/build.log" 2>&1; then
    fail "namespace-scoped simulator build failed"
fi
IMAGE=trimui-brick-pro-cfw-simulator:$NAMESPACE
IMAGE_NAMESPACE=$(docker image inspect "$IMAGE" --format '{{ index .Config.Labels "org.trimui-brick-pro-cfw.worktree" }}') ||
    fail "built simulator image is unavailable"
[ "$IMAGE_NAMESPACE" = "$NAMESPACE" ] || fail "simulator image has the wrong namespace"

ctl() {
    run=$1
    shift
    "$ROOT/scripts/simctl" --socket "$run/control.sock" "$@"
}

ctl_save() {
    run=$1
    output=$2
    shift 2
    ctl "$run" "$@" >"$output" 2>&1
}

ctl_expect_failure() {
    run=$1
    output=$2
    shift 2
    if ctl "$run" "$@" >"$output" 2>&1; then
        fail "expected control failure: $*"
    fi
}

start_run() {
    run=$1
    mkdir -p "$run/commands" "$run/cases"
    "$ROOT/scripts/sim" run --backend=dummy --run-dir "$run" --wait-ready 30 --detach \
        >"$run/commands/sim-run.log" 2>&1
    ctl_save "$run" "$run/commands/wait-ready.response.json" wait-ready --timeout 30
    ctl_save "$run" "$run/commands/initial-state.response.json" state
    ctl_save "$run" "$run/commands/initial-session-complete.response.json" adapter complete --status 0 --value 0
    ctl_save "$run" "$run/commands/initial-session-completed-state.response.json" state
}

launch_active() {
    run=$1
    case_root=$2
    id=$3
    moves=$4
    mkdir -p "$case_root"
    ctl_save "$run" "$case_root/button-start.response.json" button --button start --action press
    ctl_save "$run" "$case_root/button-systems-to-games.response.json" button --button down --action press
    direction=down
    count=$moves
    case "$moves" in
    -*)
        direction=up
        count=${moves#-}
        ;;
    esac
    i=0
    while [ "$i" -lt "$count" ]; do
        ctl_save "$run" "$case_root/button-select-$i.response.json" button --button "$direction" --action press
        i=$((i + 1))
    done
    ctl_save "$run" "$case_root/launch.response.json" button --button primary --action press
    cp "$run/launch-request.json" "$case_root/launch-request.json"
    cp "$run/launch.json" "$case_root/launch.json"
    ctl_save "$run" "$case_root/active-state.response.json" state
    ctl_save "$run" "$case_root/screenshot.response.json" screenshot --name "launch-$id"
    ctl_save "$run" "$case_root/checkpoint.response.json" checkpoint --name "launch-$id"
}

complete_active() {
    run=$1
    case_root=$2
    ctl_save "$run" "$case_root/adapter-complete.response.json" adapter complete --status 0 --value 0
    cp "$run/session.json" "$case_root/session.json"
    ctl_save "$run" "$case_root/completed-state.response.json" state
}

launch_complete() {
    run=$1
    id=$2
    moves=$3
    case_root=$run/cases/$id
    launch_active "$run" "$case_root" "$id" "$moves"
    complete_active "$run" "$case_root"
}

lifecycle_success() {
    run=$1
    case_root=$run/cases/nebula-nes/lifecycle
    mkdir -p "$case_root"
    ctl_save "$run" "$case_root/suspend.response.json" lifecycle suspend --timeout 5
    ctl_save "$run" "$case_root/suspended-state.response.json" state
    ctl_save "$run" "$case_root/suspended-checkpoint.response.json" checkpoint --name lifecycle-suspended
    ctl_save "$run" "$case_root/suspended-screenshot.response.json" screenshot --name lifecycle-suspended
    ctl_save "$run" "$case_root/resume.response.json" lifecycle resume --timeout 5
    ctl_save "$run" "$case_root/resumed-state.response.json" state
    ctl_save "$run" "$case_root/resumed-checkpoint.response.json" checkpoint --name lifecycle-resumed
}

package_fixture() {
    run=$1
    if ! docker exec --user 10001:10001 "$run_container" /usr/local/bin/package-manager \
        demo /src/fixtures/packages >"$run/commands/package-manager.log" 2>&1; then
        fail "package-manager demo failed"
    fi
}

run_smoke() {
    run=$1
    run_container=$(printf '%s' "$run" | cksum | awk -v docker_ns="$NAMESPACE" '{print "trimui-sim-" docker_ns "-" $1}')
    start_run "$run"
    package_fixture "$run"

    launch_active "$run" "$run/cases/nebula-nes" nebula-nes -2
    lifecycle_success "$run"
    complete_active "$run" "$run/cases/nebula-nes"
    launch_complete "$run" mirror-ps1 1
    launch_complete "$run" orbit-garden 1
    launch_complete "$run" signal-workshop 1

    resume_root=$run/cases/resume
    mkdir -p "$resume_root/accepted" "$resume_root/rejected"
    ctl_save "$run" "$resume_root/accepted/decision.response.json" resume --content-id orbit-garden --decision resume
    ctl_save "$run" "$resume_root/accepted/active-state.response.json" state
    ctl_save "$run" "$resume_root/accepted/autosave.response.json" autosave --reason periodic
    ctl_save "$run" "$resume_root/accepted/checkpoint.response.json" checkpoint --name resume-accepted
    ctl_save "$run" "$resume_root/accepted/screenshot.response.json" screenshot --name resume-accepted
    ctl_save "$run" "$resume_root/accepted/adapter-complete.response.json" adapter complete --status 0 --value 0
    cp "$run/session.json" "$resume_root/accepted/session.json"
    ctl_save "$run" "$resume_root/accepted/completed-state.response.json" state

    ctl_save "$run" "$resume_root/rejected/decision.response.json" \
        resume --content-id nebula-nes --decision resume --runner-version 2.0.0
    ctl_save "$run" "$resume_root/rejected/state.response.json" state

    negative=$run/cases/lifecycle-negative
    launch_active "$run" "$negative" signal-workshop 0
    ctl_save "$run" "$negative/fault-set.response.json" fault set checkpoint-fail
    ctl_expect_failure "$run" "$negative/suspend.response.json" lifecycle suspend --timeout 5
    ctl_save "$run" "$negative/recovery-state.response.json" state
    ctl_save "$run" "$negative/recovery-checkpoint.response.json" checkpoint --name lifecycle-negative
    ctl_save "$run" "$negative/recovery-screenshot.response.json" screenshot --name lifecycle-negative
    ctl_expect_failure "$run" "$negative/adapter-gated.response.json" adapter complete --status 0 --value 0
    ctl_save "$run" "$negative/fault-clear.response.json" fault clear checkpoint-fail

    "$ROOT/scripts/sim" stop --backend=dummy --run-dir "$run" --wait-ready 30 \
        >"$run/commands/sim-stop.log" 2>&1
    [ ! -e "$run/control.sock" ] || fail "control socket survived stop for $run"
}

run_smoke "$RUN1"
run_smoke "$RUN2"

python3 - "$OUT" "$ROOT/sim/contracts/control.schema.json" \
    "$ROOT/schemas/launch-request-v1.schema.json" "$ROOT/schemas/launcher-presentation-v1.schema.json" <<'PY'
import hashlib
import json
import pathlib
import sys

out, control_schema_path, request_schema_path, presentation_schema_path = map(pathlib.Path, sys.argv[1:])
try:
    import jsonschema
except ImportError as error:
    raise SystemExit(f"jsonschema is required: {error}")

CONTROL = json.loads(control_schema_path.read_text(encoding="utf-8"))
REQUEST = json.loads(request_schema_path.read_text(encoding="utf-8"))
PRESENTATION = json.loads(presentation_schema_path.read_text(encoding="utf-8"))
IDENTITIES = ("nebula-nes", "mirror-ps1", "orbit-garden", "signal-workshop")
FORBIDDEN = ("/srv/", "/src/", "secret-value", "credential-value", "private-corpus")


def load(path):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise SystemExit(f"invalid JSON: {path}: {error}")


def validate(path, schema):
    try:
        jsonschema.validate(load(path), schema)
    except jsonschema.exceptions.ValidationError as error:
        raise SystemExit(f"schema validation failed: {path}: {error.message}")


def response(path, ok=None):
    value = load(path)
    validate(path, CONTROL)
    if ok is not None and value.get("ok") is not ok:
        raise SystemExit(f"unexpected response status: {path}")
    return value


def relative(path):
    return path.relative_to(out).as_posix()


def normalize(value):
    if isinstance(value, dict):
        return {
            key: normalize(item)
            for key, item in sorted(value.items())
            if key not in {"runId", "sessionId", "atMs", "hostElapsedUs", "latencyUs", "timestampMs", "elapsedMs", "durationMs", "generation", "checkpointGeneration", "deadlineMs"}
        }
    if isinstance(value, list):
        return [normalize(item) for item in value]
    return value


def check_privacy(path):
    text = path.read_text(encoding="utf-8", errors="strict")
    for marker in FORBIDDEN:
        if marker in text:
            raise SystemExit(f"privacy marker in evidence: {relative(path)}")


def check_state(path, expected_route=None, expected_phase=None, launches_allowed=None):
    state = response(path, True)["result"]
    if state.get("schema") != "sim-state/v1":
        raise SystemExit(f"not semantic simulator state: {relative(path)}")
    if expected_route is not None and state.get("route") != expected_route:
        raise SystemExit(f"unexpected route in {relative(path)}")
    lifecycle = state["lifecycle"]
    if expected_phase is not None and lifecycle.get("phase") != expected_phase:
        raise SystemExit(f"unexpected lifecycle phase in {relative(path)}")
    if launches_allowed is not None and lifecycle.get("launchesAllowed") is not launches_allowed:
        raise SystemExit(f"unexpected launch gate in {relative(path)}")
    validate_presentation(path, state["presentation"])
    return state


def validate_presentation(path, presentation):
    try:
        jsonschema.validate(presentation, PRESENTATION)
    except jsonschema.exceptions.ValidationError as error:
        raise SystemExit(f"presentation schema failed in {relative(path)}: {error.message}")


def assert_session(case, identity):
    request_path = case / "launch-request.json"
    validate(request_path, REQUEST)
    request = load(request_path)
    if request["contentId"] != identity:
        raise SystemExit(f"wrong LaunchRequest identity in {relative(request_path)}")
    expected_kind = "libretro" if identity in {"nebula-nes", "mirror-ps1"} else "portmaster"
    if request["kind"] != expected_kind:
        raise SystemExit(f"wrong launch kind for {identity}")
    if expected_kind == "portmaster" and request["package"]["id"] != identity:
        raise SystemExit(f"wrong package identity for {identity}")
    session = load(case / "session.json")
    if session.get("state") != "completed":
        raise SystemExit(f"session did not complete for {identity}")
    completed = check_state(case / "completed-state.response.json", "library")
    result = completed["lastSessionResult"]
    if result["accepted"] is not True or result["reason"] != "success" or result["resumePublished"] is not True:
        raise SystemExit(f"session result did not complete normally for {identity}")
    for name in ("launch", "screenshot", "checkpoint", "adapter-complete"):
        response(case / f"{name}.response.json", True)
    for prefix in (f"launch-{identity}",):
        for directory in (case.parent.parent / "screenshots", case.parent.parent / "checkpoints"):
            artifact = directory / f"{prefix}.png"
            if not artifact.is_file() or artifact.stat().st_size == 0:
                raise SystemExit(f"missing screenshot/checkpoint PNG: {relative(artifact)}")


def validate_run(run_name):
    run = out / run_name
    command_dir = run / "commands"
    response(command_dir / "wait-ready.response.json", True)
    check_state(command_dir / "initial-state.response.json", "session")
    check_state(command_dir / "initial-session-completed-state.response.json", "library")
    if not (run / "logs/launcher.jsonl").is_file():
        raise SystemExit(f"missing launcher log: {relative(run / 'logs/launcher.jsonl')}")
    log_lines = [json.loads(line) for line in (run / "logs/launcher.jsonl").read_text(encoding="utf-8").splitlines()]
    events = [entry["event"] for entry in log_lines]
    if not all(event in events for event in ("ready", "first_frame", "clean_shutdown")):
        raise SystemExit(f"incomplete launcher lifecycle log: {relative(run / 'logs/launcher.jsonl')}")
    if [entry["route"] for entry in log_lines if entry["event"] == "route_selection"][:3] != ["library", "systems", "games"]:
        raise SystemExit(f"launcher route progression missing: {relative(run / 'logs/launcher.jsonl')}")
    package_log = run / "commands/package-manager.log"
    package_text = package_log.read_text(encoding="utf-8")
    for marker in (
        "PASS safe install promoted demo-theme 1.0.0",
        "PASS interrupted install leaves no activation",
    ):
        if marker not in package_text:
            raise SystemExit(f"missing package boundary: {marker}")

    cases = run / "cases"
    for identity in IDENTITIES:
        assert_session(cases / identity, identity)
    lifecycle = cases / "nebula-nes/lifecycle"
    response(lifecycle / "suspend.response.json", True)
    suspended = check_state(lifecycle / "suspended-state.response.json", "session", "suspended", False)
    if suspended["hardware"]["suspend"] != {"state": "suspended", "result": "success"}:
        raise SystemExit("successful suspend proof missing")
    response(lifecycle / "suspended-checkpoint.response.json", True)
    response(lifecycle / "suspended-screenshot.response.json", True)
    response(lifecycle / "resume.response.json", True)
    check_state(lifecycle / "resumed-state.response.json", "session", "awake", True)
    response(lifecycle / "resumed-checkpoint.response.json", True)

    accepted = cases / "resume/accepted"
    accepted_response = response(accepted / "decision.response.json", True)
    if accepted_response["result"].get("accepted") is not True:
        raise SystemExit("resume acceptance proof missing")
    check_state(accepted / "active-state.response.json", "session")
    autosave = response(accepted / "autosave.response.json", True)
    if autosave["result"]["reason"] != "periodic":
        raise SystemExit("autosave reason was not recorded")
    response(accepted / "checkpoint.response.json", True)
    response(accepted / "screenshot.response.json", True)
    response(accepted / "adapter-complete.response.json", True)
    if load(accepted / "session.json").get("state") != "completed":
        raise SystemExit("accepted resume session did not complete")
    check_state(accepted / "completed-state.response.json", "library")

    rejected = cases / "resume/rejected"
    rejected_response = response(rejected / "decision.response.json", True)
    if rejected_response["result"].get("accepted") is not False:
        raise SystemExit("resume rejection proof missing")
    check_state(rejected / "state.response.json", "library")

    negative = cases / "lifecycle-negative"
    response(negative / "fault-set.response.json", True)
    failed_suspend = response(negative / "suspend.response.json", False)
    if failed_suspend["error"]["code"] != "protocol_rejected":
        raise SystemExit("negative lifecycle case was not rejected")
    recovery = check_state(negative / "recovery-state.response.json", "session", "recovery", False)
    if recovery["lifecycle"]["marker"] is None:
        raise SystemExit("recovery marker missing")
    response(negative / "recovery-checkpoint.response.json", True)
    response(negative / "recovery-screenshot.response.json", True)
    gated = response(negative / "adapter-gated.response.json", False)
    if gated["error"]["code"] != "protocol_rejected":
        raise SystemExit("recovery did not gate session completion")
    response(negative / "fault-clear.response.json", True)

    exit_status = load(run / "exit-status.json")
    if exit_status.get("exitCode") != 0 or exit_status.get("cleanShutdown") is not True:
        raise SystemExit(f"dirty shutdown: {relative(run / 'exit-status.json')}")
    if (run / "control.sock").exists():
        raise SystemExit(f"control socket survived: {relative(run / 'control.sock')}")

    all_json = sorted(run.rglob("*.json"))
    for path in all_json:
        load(path)
        check_privacy(path)
    for path in (run / "logs/launcher.jsonl", package_log):
        check_privacy(path)
    event_fields = {"event", "route", "selection", "control", "action", "name", "operation", "phase", "status", "value", "reason"}
    normalized_log = [normalize({key: entry[key] for key in event_fields if key in entry}) for entry in log_lines]
    normalized = {
        "routes": [entry["route"] for entry in log_lines if entry["event"] == "route_selection"][:3],
        "launcherIdentities": list(IDENTITIES),
        "sessionCompletions": 4,
        "packageBoundaries": ["safe-install", "interrupted-install"],
        "lifecycle": ["suspend-success", "resume-success", "checkpoint-failure-recovery"],
        "resume": ["accepted", "runner-mismatch-rejected"],
        "eventLog": normalized_log,
    }
    return {
        "id": run_name,
        "root": run_name,
        "outcome": "pass",
        "coverage": {
            "launcherSemanticLaunch": {"count": 4, "identities": list(IDENTITIES), "normalSessionCompletions": 4, "outcome": "pass"},
            "packageInstall": {"count": 1, "facility": "package-manager demo", "outcome": "pass"},
            "packageRejectionRecovery": {"count": 1, "case": "corrupt-target-retry", "outcome": "pass"},
            "lifecycleSuspendResume": {"count": 1, "outcome": "pass"},
            "lifecycleNegativeRecovery": {"count": 1, "case": "checkpoint-failure-gated", "outcome": "pass"},
            "resumeAccepted": {"count": 1, "decision": "resume", "outcome": "pass"},
            "resumeRejected": {"count": 1, "case": "runner-version-mismatch", "outcome": "pass"},
        },
        "artifacts": {
            "launcherLog": relative(run / "logs/launcher.jsonl"),
            "packageOutput": relative(package_log),
            "launchRequests": [relative(cases / identity / "launch-request.json") for identity in IDENTITIES],
            "sessionResults": [relative(cases / identity / "session.json") for identity in IDENTITIES],
            "screenshots": sorted(relative(path) for path in (run / "screenshots").glob("*.png")),
            "checkpoints": sorted(relative(path) for path in (run / "checkpoints").glob("*.png")),
            "semanticStates": sorted(relative(path) for path in run.rglob("*.json") if "state" in path.stem or path.parent.name in {"screenshots", "checkpoints"}),
            "commandOutputs": sorted(relative(path) for path in run.rglob("*.response.json")) + sorted(relative(path) for path in (run / "commands").glob("*") if path.is_file()),
        },
        "cleanShutdown": {"exitStatus": relative(run / "exit-status.json"), "controlSocketPresent": False, "recorded": True},
        "normalized": normalized,
    }

records = [validate_run("run-1"), validate_run("run-2")]
if len(records) != 2:
    raise SystemExit("exactly two run records are required")

hashes = []
for record in records:
    canonical = json.dumps(record["normalized"], sort_keys=True, separators=(",", ":")).encode()
    hashes.append(hashlib.sha256(canonical).hexdigest())
if hashes[0] != hashes[1]:
    raise SystemExit("normalized deterministic coverage differs between runs")

summary = {
    "schema": "trimui-simulator-smoke/v1",
    "schemaVersion": 1,
    "ticket": "t_867a27e7",
    "namespace": "t867a27e7-smoke",
    "lane": "host-native userspace simulator",
    "backend": "dummy",
    "fixturePolicy": "checked-in generated fixtures only",
    "runs": records,
    "selectedCoverage": {
        "caseCountPerRun": 7,
        "runCount": 2,
        "identities": list(IDENTITIES),
        "excluded": ["headed-x11", "hardware", "non-public-inputs", "exhaustive-menu-graph"],
    },
    "determinism": {
        "algorithm": "sha256(canonical normalized coverage)",
        "normalizedComparison": "run-1 equals run-2",
        "runHashes": hashes,
        "match": True,
    },
    "cleanup": {
        "simulatorLabel": "org.trimui-brick-pro-cfw.simulator=host-native",
        "worktreeLabel": "org.trimui-brick-pro-cfw.worktree=t867a27e7-smoke",
        "finalScopedContainerCount": 0,
    },
    "privacy": {"sourcePaths": False, "privateContent": False, "secrets": False, "network": False},
}
summary_path = out / "summary.json"
summary_path.write_text(json.dumps(summary, sort_keys=True, indent=2) + "\n", encoding="utf-8")
load(summary_path)
if summary["cleanup"]["finalScopedContainerCount"] != 0:
    raise SystemExit("scoped simulator containers remain")
print(json.dumps({
    "schema": summary["schema"],
    "namespace": summary["namespace"],
    "runs": len(summary["runs"]),
    "launcherIdentitiesPerRun": 4,
    "selectedCoverageCasesPerRun": summary["selectedCoverage"]["caseCountPerRun"],
    "deterministic": summary["determinism"]["match"],
    "finalScopedContainerCount": summary["cleanup"]["finalScopedContainerCount"],
    "summary": "summary.json",
}, sort_keys=True))
PY

[ "$(scoped_container_count)" -eq 0 ] || fail "namespace cleanup left simulator containers"

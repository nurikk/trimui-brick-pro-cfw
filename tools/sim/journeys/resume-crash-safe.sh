#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd -P)
RUN=${1:-$(mktemp -d /tmp/trimui-resume-sim.XXXXXX)}
OWN_RUN=0
[ "$#" -gt 0 ] || OWN_RUN=1
trap '[ "$OWN_RUN" -eq 1 ] && rm -rf "$RUN"' EXIT HUP INT TERM

"$ROOT/scripts/sim" run --backend=dummy --run-dir "$RUN" --wait-ready 30 --detach
trap '"$ROOT/scripts/sim" stop --run-dir "$RUN" >/dev/null 2>&1 || true; [ "$OWN_RUN" -eq 1 ] && rm -rf "$RUN"' EXIT HUP INT TERM
simctl() { "$ROOT/scripts/simctl" --socket "$RUN/control.sock" "$@" >/dev/null; }
autosave_response() { "$ROOT/scripts/simctl" --socket "$RUN/control.sock" autosave "$@"; }
resume_response() { "$ROOT/scripts/simctl" --socket "$RUN/control.sock" resume "$@"; }
mkdir -p "$RUN/data/saves"
printf '%s' 'generated-save-v1' >"$RUN/data/saves/mirror-ps1.save"
state() { "$ROOT/scripts/simctl" --socket "$RUN/control.sock" state; }
button() { simctl button --button "$1" --action press; }

# Start a real broker session before exercising its lifecycle checkpoints.
resume_response --content-id signal-workshop --decision fresh-start >/dev/null
autosave_response --reason periodic >"$RUN/lifecycle-periodic.json"
autosave_response --reason pre-suspend >"$RUN/lifecycle-pre-suspend.json"
autosave_response --reason low-battery >"$RUN/lifecycle-low-battery.json"
simctl adapter complete --status 0 --value 0

# Complete one normal checkpoint for each generated demo through the broker.
for content_id in nebula-nes mirror-ps1 orbit-garden; do
    resume_response --content-id "$content_id" --decision fresh-start >/dev/null
    simctl adapter complete --status 0 --value 0
done

# A running process crash has no post-crash checkpoint.
resume_response --content-id signal-workshop --decision fresh-start >/dev/null
for fault in artifact metadata promotion pointer; do
    simctl autosave --reason periodic --fault "$fault" || true
done
"$ROOT/scripts/simctl" --socket "$RUN/control.sock" adapter crash --status 1 --value 0 >"$RUN/crash-result.json"
python3 - "$RUN/crash-result.json" <<'PY'
import json, sys
result = json.load(open(sys.argv[1], encoding="utf-8"))["result"]
assert not result["lastSessionResult"]["resumePublished"]
PY

# Capture the broker-backed Game Switcher projection without exposing state paths.
simctl screenshot --name game-switcher
state >"$RUN/game-switcher-state.json"
python3 - "$RUN" <<'PY'
import hashlib
import json
import os
import sys
run = sys.argv[1]
records = {}
allowed_reasons = {"periodic", "pre-suspend", "low-battery", "normal-exit"}
for name in os.listdir(os.path.join(run, "data", "resume", "generations")):
    directory = os.path.join(run, "data", "resume", "generations", name)
    with open(os.path.join(directory, "record.json"), encoding="utf-8") as stream:
        record = json.load(stream)
    if record["contentId"] in records:
        continue
    assert record["reason"] in allowed_reasons
    data = open(os.path.join(directory, record["sram"]["relative"]), "rb").read()
    sram_sha256 = hashlib.sha256(data).hexdigest()
    assert len(data) == record["sram"]["size"]
    assert sram_sha256 == record["sram"]["sha256"]
    records[record["contentId"]] = {
        "generation": record["generation"],
        "reason": record["reason"],
        "sramSha256": sram_sha256,
    }
with open(os.path.join(run, "resume-before-decisions.json"), "w", encoding="utf-8") as stream:
    json.dump({"records": records, "saveSha256": hashlib.sha256(open(os.path.join(run, "data", "saves", "mirror-ps1.save"), "rb").read()).hexdigest()}, stream)
PY
resume_response --content-id nebula-nes --decision resume --runner-version 2.0.0 >"$RUN/mismatched-runner.json"
resume_response --content-id mirror-ps1 --decision retained-matching-core --core-id generated-core --core-version 2.0.0 >"$RUN/mismatched-core.json"
simctl adapter crash --status 1 --value 0
simctl resume --content-id signal-workshop --decision cancel
resume_response --content-id orbit-garden --decision resume
simctl adapter crash --status 1 --value 0

"$ROOT/scripts/sim" stop --run-dir "$RUN"
python3 - "$RUN" <<'PY'
import hashlib
import json
import os
import sys

run = sys.argv[1]
resume = os.path.join(run, "data", "resume")
with open(os.path.join(resume, "current.json"), encoding="utf-8") as stream:
    current = json.load(stream)
assert set(current) == {"schema", "generation", "checksum"}
assert current["schema"] == "trimui-resume-current/v1"
assert len(current["checksum"]) == 64
current_generation = current["generation"]
for reason in ("periodic", "pre-suspend", "low-battery"):
    with open(os.path.join(run, f"lifecycle-{reason}.json"), encoding="utf-8") as stream:
        response = json.load(stream)
    assert response["ok"]
    assert response["result"]["reason"] == reason
generations = []
for name in os.listdir(os.path.join(resume, "generations")):
    if not name.startswith("generation-"):
        continue
    generation = int(name.removeprefix("generation-"))
    if generation > current_generation:
        continue
    directory = os.path.join(resume, "generations", name)
    with open(os.path.join(directory, "record.json"), encoding="utf-8") as stream:
        record = json.load(stream)
    assert set(record) == {"$schema", "format", "schemaVersion", "generation", "contentId", "contentSha256", "runner", "core", "capability", "reason", "timestampMs", "state", "sram", "screenshot"}
    assert record["generation"] == generation
    assert record["contentId"] in {"nebula-nes", "mirror-ps1", "orbit-garden", "signal-workshop"}
    assert not any(part in json.dumps(record) for part in ("/srv/", "../", "\\"))
    for artifact in (record["state"], record["sram"], record["screenshot"]):
        path = os.path.join(directory, artifact["relative"])
        data = open(path, "rb").read()
        assert len(data) == artifact["size"]
        assert hashlib.sha256(data).hexdigest() == artifact["sha256"]
    generations.append(record)
expected_content_ids = {"nebula-nes", "mirror-ps1", "orbit-garden", "signal-workshop"}
allowed_reasons = {"periodic", "pre-suspend", "low-battery", "normal-exit"}
assert all(record["contentId"] in expected_content_ids for record in generations)
assert {record["contentId"] for record in generations} == expected_content_ids
assert all(record["reason"] in allowed_reasons for record in generations)
assert current_generation in {record["generation"] for record in generations}
with open(os.path.join(run, "resume-before-decisions.json"), encoding="utf-8") as stream:
    before = json.load(stream)
assert hashlib.sha256(open(os.path.join(run, "data", "saves", "mirror-ps1.save"), "rb").read()).hexdigest() == before["saveSha256"]
assert set(before["records"]) == expected_content_ids
for content_id, details in before["records"].items():
    assert set(details) == {"generation", "reason", "sramSha256"}
    assert details["reason"] in allowed_reasons
with open(os.path.join(run, "mismatched-core.json"), encoding="utf-8") as stream:
    mismatch_core = json.load(stream)["result"]
assert mismatch_core["accepted"]
assert mismatch_core["availableChoices"] == ["retained-matching-core", "cold-start-sram", "fresh-start", "cancel"]
assert mismatch_core["effectiveCore"] == {"id": "generated-retained-core", "version": "1.0.0"}
with open(os.path.join(run, "mismatched-runner.json"), encoding="utf-8") as stream:
    mismatch_runner = json.load(stream)["result"]
assert not mismatch_runner["accepted"]
assert mismatch_runner["availableChoices"] == ["cold-start-sram", "fresh-start", "cancel"]
with open(os.path.join(run, "game-switcher-state.json"), encoding="utf-8") as stream:
    response = json.load(stream)
state = response["result"]
assert state["route"] == "library"
presentation = state["presentation"]
assert [entry["label"] for entry in presentation["resume"]] == ["Nebula Notes", "Mirror Museum", "Orbit Garden", "Signal Workshop"]
assert [entry["system"] for entry in presentation["resume"]] == ["nes", "ps1", "portmaster", "portmaster"]
assert [entry["status"] for entry in presentation["resume"]] == ["available"] * 4
assert [entry["choices"] for entry in presentation["resume"]] == [
    ["resume", "cold-start-sram", "fresh-start", "cancel"],
    ["resume", "cold-start-sram", "fresh-start", "cancel"],
    ["resume", "cold-start-sram", "fresh-start", "cancel"],
    ["resume", "cold-start-sram", "fresh-start", "cancel"],
]
assert all(entry["timestampMs"] > 0 for entry in presentation["resume"])
assert all(entry["screenshot"].startswith("resume-preview-") for entry in presentation["resume"])
assert all(path not in json.dumps(state) for path in ("/srv/", "data/resume", "data/saves", "data/states"))
print(f"resume crash-safe journey: PASS (lifecycle checkpoints, fault boundaries, crash recovery, four controller-visible demos) {run}")
PY

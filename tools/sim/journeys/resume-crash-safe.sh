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

# The generated host profile starts Signal Workshop. Exercise every lifecycle checkpoint.
autosave_response --reason periodic >"$RUN/lifecycle-periodic.json"
autosave_response --reason pre-suspend >"$RUN/lifecycle-pre-suspend.json"
autosave_response --reason low-battery >"$RUN/lifecycle-low-battery.json"
simctl adapter complete --status 0 --value 0

# Complete one normal checkpoint for each of the other legal generated demos.
button start
button down
button up
button up
button up
button primary
simctl adapter complete --status 0 --value 0
button start
button down
button down
button primary
simctl adapter complete --status 0 --value 0
button start
button down
button down
button primary
simctl adapter complete --status 0 --value 0
button start
button down
button down
button primary
simctl adapter complete --status 0 --value 0

# A running process crash has no post-crash checkpoint.
button start
button down
button down
button primary
before_crash=$(
    python3 - "$RUN" <<'PY'
import json, sys
with open(sys.argv[1] + "/data/resume/current.json", encoding="utf-8") as stream:
    print(json.load(stream)["generation"])
PY
)
for fault in artifact metadata promotion pointer; do
    simctl autosave --reason periodic --fault "$fault" || true
done
simctl adapter crash --status 1 --value 0
after_crash=$(
    python3 - "$RUN" <<'PY'
import json, sys
with open(sys.argv[1] + "/data/resume/current.json", encoding="utf-8") as stream:
    print(json.load(stream)["generation"])
PY
)
[ "$before_crash" = "$after_crash" ] || {
    echo "crash published a new checkpoint" >&2
    exit 1
}

# Return through Home -> Game Switcher using semantic controller buttons only.
for _ in 1 2 3 4 5 6 7; do button down; done
button primary
simctl screenshot --name game-switcher
state >"$RUN/game-switcher-state.json"
python3 - "$RUN" <<'PY'
import hashlib
import json
import os
import sys
run = sys.argv[1]
records = {}
for name in os.listdir(os.path.join(run, "data", "resume", "generations")):
    directory = os.path.join(run, "data", "resume", "generations", name)
    with open(os.path.join(directory, "record.json"), encoding="utf-8") as stream:
        record = json.load(stream)
    if record["contentId"] in records:
        continue
    data = open(os.path.join(directory, record["sram"]["relative"]), "rb").read()
    records[record["contentId"]] = {
        "generation": record["generation"],
        "sramSha256": hashlib.sha256(data).hexdigest(),
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
assert len(generations) == 4
assert {record["contentId"] for record in generations} == {"nebula-nes", "mirror-ps1", "orbit-garden", "signal-workshop"}
with open(os.path.join(run, "resume-before-decisions.json"), encoding="utf-8") as stream:
    before = json.load(stream)
assert hashlib.sha256(open(os.path.join(run, "data", "saves", "mirror-ps1.save"), "rb").read()).hexdigest() == before["saveSha256"]
details = before["records"]["mirror-ps1"]
directory = os.path.join(resume, "generations", f"generation-{details['generation']}")
with open(os.path.join(directory, "record.json"), encoding="utf-8") as stream:
    record = json.load(stream)
data = open(os.path.join(directory, record["sram"]["relative"]), "rb").read()
assert hashlib.sha256(data).hexdigest() == details["sramSha256"]
with open(os.path.join(run, "mismatched-core.json"), encoding="utf-8") as stream:
    mismatch_core = json.load(stream)["result"]
assert mismatch_core["accepted"]
assert mismatch_core["availableChoices"] == ["retained-matching-core", "cold-start-sram", "cancel"]
assert mismatch_core["effectiveCore"] == {"id": "generated-retained-core", "version": "1.0.0"}
with open(os.path.join(run, "mismatched-runner.json"), encoding="utf-8") as stream:
    mismatch_runner = json.load(stream)["result"]
assert not mismatch_runner["accepted"]
assert mismatch_runner["availableChoices"] == ["cold-start-sram", "cancel"]
with open(os.path.join(run, "game-switcher-state.json"), encoding="utf-8") as stream:
    response = json.load(stream)
state = response["result"]
assert state["route"] == "game-switcher"
presentation = state["presentation"]
assert presentation["route"] == "game-switcher"
assert [entry["label"] for entry in presentation["resume"]] == ["Nebula Notes", "Mirror Museum", "Orbit Garden", "Signal Workshop"]
assert [entry["choices"] for entry in presentation["resume"]] == [
    ["resume", "cold-start-sram", "cancel"],
    ["resume", "cold-start-sram", "cancel"],
    ["resume", "cold-start-sram", "cancel"],
    ["resume", "cold-start-sram", "cancel"],
]
assert all(path not in json.dumps(state) for path in ("/srv/", "data/resume", "data/saves", "data/states"))
print(f"resume crash-safe journey: PASS (lifecycle checkpoints, fault boundaries, crash recovery, four controller-visible demos) {run}")
PY

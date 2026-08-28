#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd -P)
RUN=${1:-$(mktemp -d /tmp/trimui-presentation-sim.XXXXXX)}
OWN_RUN=0
[ "$#" -gt 0 ] || OWN_RUN=1
mkdir -p "$RUN"
trap '[ "$OWN_RUN" -eq 1 ] && rm -rf "$RUN"' EXIT HUP INT TERM

"$ROOT/scripts/sim" run --backend=dummy --profile sim/device/tg4040-alphabet.json --run-dir "$RUN" --wait-ready 30 --detach
trap '"$ROOT/scripts/sim" stop --run-dir "$RUN" >/dev/null 2>&1 || true; [ "$OWN_RUN" -eq 1 ] && rm -rf "$RUN"' EXIT HUP INT TERM
simctl() {
    "$ROOT/scripts/simctl" --socket "$RUN/control.sock" "$@"
}
simctl wait-ready --timeout 30 >/dev/null

# Reset through the explicit fixture hook, then drive the catalog with typed buttons.
simctl presentation --action home >/dev/null
button() {
    simctl button --button "$1" --action press >/dev/null
    simctl button --button "$1" --action release >/dev/null
}
screenshot() {
    simctl screenshot --name "$1" >/dev/null
}
button primary
screenshot controller-systems
button primary
screenshot controller-games
button down
button r1
screenshot controller-game-selection

# The existing bounded controllers are exercised with buttons after safe fixture setup.
simctl presentation --action settings >/dev/null
button primary
screenshot controller-settings-form
simctl presentation --action wifi-scan >/dev/null
button down
screenshot controller-wifi

# These states have no bounded controller transition in the simulator fixture.
for action in favorites search modal scraper-progress scraper-ambiguity fallback; do
    simctl presentation --action "$action" >/dev/null
    screenshot "$action"
done

$ROOT/scripts/sim stop --run-dir "$RUN"
python3 - "$RUN" "$ROOT/sim/contracts/control.schema.json" "$ROOT/schemas/launcher-presentation-v1.schema.json" <<'PY'
import glob
import json
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import zlib

run, control_schema_path, presentation_schema_path = sys.argv[1:]
try:
    import jsonschema
except ImportError:
    jsonschema = None

validator = None if jsonschema else shutil.which("jsonschema")
if jsonschema is None and validator is None:
    raise SystemExit("schema validation unavailable: Python jsonschema module and jsonschema CLI are both missing")


def validate(instance, schema_path):
    if jsonschema is not None:
        with open(schema_path, encoding="utf-8") as stream:
            jsonschema.validate(instance, json.load(stream))
        return
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as stream:
        json.dump(instance, stream)
        instance_path = stream.name
    try:
        subprocess.run([validator, "-i", instance_path, schema_path], check=True)
    finally:
        os.unlink(instance_path)


semantic_paths = sorted(glob.glob(os.path.join(run, "screenshots", "*.json")))
png_paths = sorted(glob.glob(os.path.join(run, "screenshots", "*.png")))
if len(semantic_paths) != 12 or len(png_paths) != len(semantic_paths):
    raise SystemExit(
        f"expected 12 paired semantic/PNG screenshots, got {len(semantic_paths)}/{len(png_paths)}"
    )
by_name = {os.path.splitext(os.path.basename(path))[0]: path for path in semantic_paths}
for path in semantic_paths:
    with open(path, encoding="utf-8") as stream:
        data = json.load(stream)
    validate(data, control_schema_path)
    screen = data["presentation"]
    validate(screen, presentation_schema_path)
    assert screen["schema"] == "launcher-presentation/v1"
    assert screen["identity"] == "Artbook"
    assert len(screen["regions"]) == 9
    assert screen["affordances"]["clock"]
    assert 0 <= screen["affordances"]["batteryPercent"] <= 100
    raw = json.dumps(data, sort_keys=True)
    assert "/srv/" not in raw and "secret-value" not in raw and "credential-value" not in raw

assert json.load(open(by_name["controller-systems"]))["presentation"]["route"] == "systems"
assert json.load(open(by_name["controller-games"]))["presentation"]["route"] == "games"
games = json.load(open(by_name["controller-games"]))["presentation"]
assert games["splash"] == "ready"
selected = json.load(open(by_name["controller-game-selection"]))["presentation"]
assert games["selectedLabel"] != selected["selectedLabel"]
assert json.load(open(by_name["controller-settings-form"]))["presentation"]["settings"]["surface"] == "form"
assert json.load(open(by_name["controller-wifi"]))["presentation"]["route"] == "wifi-scan"

splash_paths = sorted(glob.glob(os.path.join(run, "screenshots", "screen-*.json")))
if len(splash_paths) != 1:
    raise SystemExit(f"expected one startup splash artifact, got {len(splash_paths)}")
with open(splash_paths[0], encoding="utf-8") as stream:
    splash = json.load(stream)["presentation"]
assert splash["splash"] == "artbook-generated-splash"
assert splash["route"] == "home"
def first_pixel(path):
    data = open(path, "rb").read()
    offset = 8
    image_data = bytearray()
    while offset < len(data):
        length = struct.unpack(">I", data[offset : offset + 4])[0]
        kind = data[offset + 4 : offset + 8]
        chunk = data[offset + 8 : offset + 8 + length]
        offset += 12 + length
        if kind == b"IDAT":
            image_data.extend(chunk)
        if kind == b"IEND":
            break
    decoded = zlib.decompress(image_data)
    assert decoded[0] in range(5)
    return tuple(decoded[1:5])

splash_png_path = splash_paths[0].replace(".json", ".png")
with open(splash_png_path, "rb") as stream:
    splash_png = stream.read()
with open(by_name["controller-systems"].replace(".json", ".png"), "rb") as stream:
    systems_png = stream.read()
assert first_pixel(splash_png_path) == (16, 19, 28, 255)
assert splash["palette"]["background"] == [16, 19, 28, 255]
assert splash_png != systems_png
fallback_json = json.load(open(by_name["fallback"]))["presentation"]
assert fallback_json["splash"] == "artbook-generated-fallback"
assert fallback_json["themeFallback"] is not None
assert fallback_json["modal"] is None
with open(by_name["fallback"].replace(".json", ".png"), "rb") as stream:
    assert stream.read() != systems_png

for path in png_paths + [splash_paths[0].replace(".json", ".png")]:
    assert os.path.getsize(path) > 0
    with open(path, "rb") as stream:
        assert stream.read(8) == b"\x89PNG\r\n\x1a\n"
        length = struct.unpack(">I", stream.read(4))[0]
        assert stream.read(4) == b"IHDR" and length >= 8
        width, height = struct.unpack(">II", stream.read(8))
        assert (width, height) == (1024, 768)

kind = "Python jsonschema module" if jsonschema is not None else "jsonschema CLI"
print(f"presentation-screens journey: PASS (12 paired PNG/JSON artifacts; {kind} validated control + presentation schemas) {run}")
print("controller evidence: systems -> games -> changed game selection; settings form; Wi-Fi interaction")
print("direct fixture evidence: favorites, search, modal, scraper progress/ambiguity, fallback")
PY

#!/usr/bin/env python3
import argparse
import collections
import hashlib
import json
import os
import pathlib
import shutil
import struct
import subprocess
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parents[3]
GRAPH = ROOT / "sim/routes/controller-routes.json"
SIM = ROOT / "scripts/sim"
PROFILE = "sim/device/tg4040-alphabet.json"

CONTROL_TIMEOUT = 15
START_TIMEOUT = 45
ROUTE_TIMEOUT = 60
PASS_TIMEOUT = 600
CONTROLLER_ROUTE_COUNT = 66
SMOKE_ROUTES = [
    "home-systems",
    "settings-display",
    "diagnostics-safe-mode",
    "portmaster-catalog",
    "platform-nebula-restored",
]
RUN_SPECIFIC_KEYS = {
    "runId",
    "atMs",
    "sequence",
    "eventSequence",
    "timestampMs",
    "containerId",
    "containerName",
    "runDir",
    "outputDir",
    "artifactPath",
    "png",
    "state",
    "checkpoint",
    "artifact",
    "screenshotPath",
    "screenshotFilename",
    "screenshot",
    "output",
}

REQUIRED_INSPECTIONS = {
    "games-search-keyboard": "Editable query:",
    "settings-display": "Current value:",
    "games-details": "Rating:",
    "theme-garden-preview": "Live 4:3 preview",
}


def run(command, *, capture=False, check=True, timeout=CONTROL_TIMEOUT):
    try:
        return subprocess.run(
            [str(part) for part in command],
            check=check,
            text=True,
            stdout=subprocess.PIPE if capture else None,
            stderr=subprocess.PIPE if capture else None,
            timeout=timeout,
        )
    except subprocess.TimeoutExpired as error:
        rendered = " ".join(str(part) for part in command)
        raise RuntimeError(f"command timed out after {timeout}s: {rendered}") from error


def json_file(path):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise RuntimeError(f"invalid JSON: {path}") from error


def response(text):
    try:
        return json.loads(text)
    except json.JSONDecodeError as error:
        raise RuntimeError("simulator returned malformed JSON") from error


def load_graph():
    graph = json_file(GRAPH)
    routes = graph.get("routes")
    if graph.get("schema") != "controller-route-graph/v2" or not isinstance(routes, list):
        raise RuntimeError(f"invalid controller route graph: {GRAPH}")
    route_ids = [route.get("id") for route in routes]
    expected = graph.get("expectedCount")
    if (
        not isinstance(expected, int)
        or expected < 1
        or len(route_ids) != expected
        or len(set(route_ids)) != expected
        or any(not route.get("buttons") for route in routes)
    ):
        raise RuntimeError("controller route graph has invalid button journeys")
    return graph


def paths(graph=None):
    graph = graph or load_graph()
    routes = {route["id"]: route for route in graph["routes"]}
    built = {}

    def build(route_id, stack=()):
        if route_id in built:
            return built[route_id]
        if route_id in stack or route_id not in routes:
            raise RuntimeError(f"invalid route parent chain at {route_id}")
        route = routes[route_id]
        parent = route.get("from", "fresh-home")
        prefix = [] if parent == "fresh-home" else build(parent, (*stack, route_id))
        built[route_id] = [*prefix, *route["buttons"]]
        return built[route_id]

    for route_id in routes:
        build(route_id)
    return built


def container_for(run_dir):
    ids = run(
        [
            "docker",
            "ps",
            "-q",
            "--filter",
            "label=org.trimui-brick-pro-cfw.simulator=host-native",
        ],
        capture=True,
    ).stdout.split()
    records = response(run(["docker", "inspect", *ids], capture=True).stdout) if ids else []
    matches = [
        record["Id"]
        for record in records
        if any(
            mount["Destination"] == "/evidence"
            and pathlib.Path(mount["Source"]).resolve() == run_dir.resolve()
            for mount in record["Mounts"]
        )
    ]
    if len(matches) != 1:
        raise RuntimeError("simulator container identity is ambiguous or missing")
    return matches[0]


def call(container, *args):
    command = [
        "docker",
        "exec",
        "--user",
        "10001:10001",
        container,
        "/usr/local/bin/sim-control",
        "--socket",
        "/evidence/control.sock",
        *args,
    ]
    result = response(run(command, capture=True, timeout=CONTROL_TIMEOUT).stdout)
    if not result.get("ok"):
        raise RuntimeError(f"simulator rejected {' '.join(args)}: {result.get('error', result)}")
    return result["result"]


def event_visits(run_dir):
    path = run_dir / "logs/launcher.jsonl"
    try:
        events = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()]
    except (OSError, json.JSONDecodeError) as error:
        raise RuntimeError(f"invalid launcher event log: {path}") from error
    return [event["routeId"] for event in events if event.get("event") == "controller_route_visit"]


def button(container, run_dir, name):
    before = event_visits(run_dir)
    call(container, "button", "--button", name, "--action", "press")
    state = call(container, "button", "--button", name, "--action", "release")
    emitted = event_visits(run_dir)[len(before) :]
    if len(emitted) > 1:
        raise RuntimeError(f"{name} emitted more than one controller_route_visit: {emitted}")
    actual = state["controllerRoute"]["currentId"]
    if emitted and emitted[0] != actual:
        raise RuntimeError(f"{name} visited {emitted[0]} but returned {actual}")
    return state


def png_dimensions(path):
    with path.open("rb") as stream:
        header = stream.read(24)
    if header[:8] != b"\x89PNG\r\n\x1a\n" or header[12:16] != b"IHDR":
        raise RuntimeError(f"invalid PNG checkpoint: {path}")
    return struct.unpack(">II", header[16:24])



def sha256_file(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def decoded_frame_hash(path, at_seconds=None):
    command = ["ffmpeg", "-v", "error"]
    if at_seconds is not None:
        command.extend(["-ss", str(at_seconds)])
    command.extend(["-i", path, "-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
    try:
        result = subprocess.run(command, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=CONTROL_TIMEOUT)
    except (OSError, subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
        raise RuntimeError(f"cannot decode inspected frame: {path}") from error
    if len(result.stdout) != 1024 * 768 * 4:
        raise RuntimeError(f"decoded frame is not 1024x768 RGBA: {path}")
    return hashlib.sha256(result.stdout).hexdigest()


def recording_path(recording, relative):
    if not isinstance(relative, str) or pathlib.PurePosixPath(relative).is_absolute():
        raise RuntimeError(f"recording artifact path is not bundle-relative: {relative!r}")
    path = (recording / relative).resolve()
    try:
        path.relative_to(recording.resolve())
    except ValueError as error:
        raise RuntimeError(f"recording artifact escapes bundle: {relative}") from error
    return path


def copy_log(run_dir, recording, root):
    source = run_dir / "logs/launcher.jsonl"
    try:
        records = [json.loads(line) for line in source.read_text(encoding="utf-8").splitlines()]
    except (OSError, json.JSONDecodeError) as error:
        raise RuntimeError(f"invalid chronological launcher log: {source}") from error
    sequence = -1
    for record in records:
        if not isinstance(record, dict) or not isinstance(record.get("sequence"), int) or not isinstance(record.get("atMs"), int):
            raise RuntimeError(f"launcher log lacks chronology: {source}")
        if record["sequence"] <= sequence or record["atMs"] < 0:
            raise RuntimeError(f"launcher log is not chronological: {source}")
        sequence = record["sequence"]
    relative = f"logs/{root}/launcher.jsonl"
    destination = recording_path(recording, relative)
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, destination)
    return relative


def encode_video(recording, pngs):
    video = recording / "action-recording.mp4"
    with tempfile.NamedTemporaryFile("w", dir=recording, suffix=".ffconcat", delete=False, encoding="utf-8") as stream:
        listing = pathlib.Path(stream.name)
        for png in pngs:
            if "'" in str(png):
                raise RuntimeError(f"unsupported quote in recording path: {png}")
            stream.write(f"file '{png.resolve()}'\nduration 1\n")
        stream.write(f"file '{pngs[-1].resolve()}'\n")
    try:
        subprocess.run(
            ["ffmpeg", "-v", "error", "-y", "-f", "concat", "-safe", "0", "-i", listing, "-r", "2", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-movflags", "+faststart", video],
            check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=180,
        )
    except (OSError, subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
        raise RuntimeError("cannot encode continuous H.264/yuv420p action recording") from error
    finally:
        listing.unlink(missing_ok=True)
    try:
        probe = json.loads(subprocess.run(
            ["ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries", "stream=codec_name,pix_fmt,width,height", "-of", "json", video],
            check=True, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=CONTROL_TIMEOUT,
        ).stdout)
        stream = probe["streams"][0]
        subprocess.run(["ffmpeg", "-v", "error", "-i", video, "-f", "null", "-"], check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=180)
    except (OSError, KeyError, IndexError, TypeError, json.JSONDecodeError, subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
        raise RuntimeError("cannot decode continuous action recording") from error
    if stream != {"codec_name": "h264", "pix_fmt": "yuv420p", "width": 1024, "height": 768}:
        raise RuntimeError(f"action recording has wrong stream format: {stream}")
    return {
        "path": video.name,
        "sha256": sha256_file(video),
        "codec": "h264",
        "pixelFormat": "yuv420p",
        "dimensions": {"width": 1024, "height": 768},
        "inspectedFrames": [
            {"position": position, "atSeconds": second, "decodedFrameSha256": decoded_frame_hash(video, second)}
            for position, second in (("first", 0), ("middle", len(pngs) // 2), ("last", len(pngs) - 1))
        ],
    }


def validate_action_manifest(recording, manifest, expected):
    checkpoints = manifest.get("checkpoints")
    if manifest.get("schema") != "trimui-action-recording/v1" or not isinstance(checkpoints, list) or len(checkpoints) != len(expected):
        raise RuntimeError("action manifest checkpoint count is invalid")
    timestamp = -1
    for checkpoint, (root, ordinal, route_id) in zip(checkpoints, expected):
        if checkpoint.get("freshRoot") != root or checkpoint.get("ordinal") != ordinal or checkpoint.get("routeId") != route_id:
            raise RuntimeError(f"action manifest route identity mismatch: {checkpoint}")
        if not isinstance(checkpoint.get("recordedAtMs"), int) or checkpoint["recordedAtMs"] <= timestamp:
            raise RuntimeError("action manifest timestamps are not strictly chronological")
        timestamp = checkpoint["recordedAtMs"]
        artifact = checkpoint.get("artifact", {})
        png, state = recording_path(recording, artifact.get("png")), recording_path(recording, artifact.get("state"))
        if checkpoint.get("dimensions") != {"width": 1024, "height": 768} or not png.is_file() or not state.is_file() or png_dimensions(png) != (1024, 768):
            raise RuntimeError(f"action manifest artifact is missing or malformed: {artifact}")
        if sha256_file(png) != checkpoint.get("pngSha256") or decoded_frame_hash(png) != checkpoint.get("inspectedFrameSha256"):
            raise RuntimeError(f"action manifest artifact hash mismatch: {artifact}")
        screen = json_file(state).get("presentation")
        if not isinstance(screen, dict) or not isinstance(screen.get("menu"), list):
            raise RuntimeError(f"action manifest semantic pair is invalid: {artifact}")
    return len(checkpoints)


def write_action_recording(out, route_ids, first, second):
    recording = out / "recording"
    recording.mkdir()
    expected, checkpoints, pngs = [], [], []
    start_ms = time.time_ns() // 1_000_000
    for run_number, result in enumerate((first, second), 1):
        root = f"run-{run_number}"
        copy_log(out / root, recording, root)
        for checkpoint in result["checkpoints"]:
            ordinal, route_id = checkpoint["ordinal"], checkpoint["routeId"]
            expected.append((root, ordinal, route_id))
            stem = f"{ordinal:02d}-{route_id}"
            png_relative, state_relative = f"screenshots/{root}/{stem}.png", f"screenshots/{root}/{stem}.json"
            source_png, source_state = out / root / checkpoint["png"], out / root / checkpoint["state"]
            png, state = recording_path(recording, png_relative), recording_path(recording, state_relative)
            png.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source_png, png)
            shutil.copyfile(source_state, state)
            pngs.append(png)
            checkpoints.append({
                "freshRoot": root,
                "ordinal": ordinal,
                "routeId": route_id,
                "recordedAtMs": start_ms + len(checkpoints),
                "artifact": {"png": png_relative, "state": state_relative},
                "dimensions": {"width": 1024, "height": 768},
                "pngSha256": sha256_file(png),
                "inspectedFrameSha256": decoded_frame_hash(png),
            })
    video = encode_video(recording, pngs)
    route_inspections = {}
    for route_id, expected_label in REQUIRED_INSPECTIONS.items():
        checkpoint = next((item for item in checkpoints if item["freshRoot"] == "run-1" and item["routeId"] == route_id), None)
        if checkpoint is None:
            raise RuntimeError(f"required inspected route is absent: {route_id}")
        screen = json_file(recording_path(recording, checkpoint["artifact"]["state"])).get("presentation", {})
        if not any(expected_label in str(item.get("label")) for item in screen.get("menu", []) if isinstance(item, dict)):
            raise RuntimeError(f"required inspected surface is absent: {route_id}")
        route_inspections[route_id] = {
            "png": checkpoint["artifact"]["png"],
            "state": checkpoint["artifact"]["state"],
            "expectedLabel": expected_label,
            "decodedFrameSha256": checkpoint["inspectedFrameSha256"],
        }
    evidence = {}
    for name, route_id in {
        "package": "portmaster-install",
        "session": "platform-nebula-restored",
        "protectedData": "portmaster-uninstall-protected-data",
    }.items():
        pairs = [
            item["artifact"]
            for item in checkpoints
            if item["routeId"] == route_id and item["ordinal"] > len(SMOKE_ROUTES)
        ]
        if len(pairs) != 2:
            raise RuntimeError(f"required {name} evidence is absent: {route_id}")
        evidence[name] = {"routeId": route_id, "artifacts": pairs}
    manifest = {
        "schema": "trimui-action-recording/v1",
        "checkpointCount": len(checkpoints),
        "routeCount": len(route_ids),
        "checkpoints": checkpoints,
        "logs": [f"logs/run-{number}/launcher.jsonl" for number in (1, 2)],
        "evidence": evidence,
        "inspections": {"routes": route_inspections, "video": video},
    }
    manifest_path = recording / "action-manifest.json"
    manifest_path.write_text(json.dumps(manifest, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    validate_action_manifest(recording, manifest, expected)
    return manifest_path, len(checkpoints)


def start(run_dir, backend, display):
    command = [
        SIM,
        "run",
        f"--backend={backend}",
        "--profile",
        PROFILE,
        "--run-dir",
        run_dir,
        "--wait-ready",
        "60",
        "--detach",
    ]
    if display:
        command.extend(["--display", display])
    run(command, timeout=START_TIMEOUT)
    container = container_for(run_dir)
    call(container, "wait-ready", "--timeout", "30")
    return container


def stop(run_dir, backend, display):
    command = [SIM, "stop", f"--backend={backend}", "--run-dir", run_dir]
    if display:
        command.extend(["--display", display])
    run(command, timeout=START_TIMEOUT)


def clean_shutdown_recorded(run_dir):
    try:
        return '"event":"clean_shutdown"' in (run_dir / "logs/launcher.jsonl").read_text(encoding="utf-8")
    except OSError:
        return False


def normalize(value):
    """Drop run-specific metadata while retaining semantic order and presentation data."""
    if isinstance(value, dict):
        return {
            key: normalize(item)
            for key, item in value.items()
            if key not in RUN_SPECIFIC_KEYS
        }
    if isinstance(value, list):
        return [normalize(item) for item in value]
    return value


def validate_visits(visits, route_ids):
    duplicates = sorted(route for route, count in collections.Counter(visits).items() if count > 1)
    unexpected = sorted(set(visits) - set(route_ids))
    missing = sorted(set(route_ids) - set(visits))
    if duplicates or unexpected or missing or visits != route_ids:
        raise RuntimeError(
            "route coverage mismatch: "
            f"duplicates={duplicates}, unexpected={unexpected}, missing={missing}, visited={visits}"
        )


def wait_for_clean_exit_status(run_dir):
    deadline = time.monotonic() + 30
    status_path = run_dir / "exit-status.json"
    while time.monotonic() < deadline:
        if status_path.exists():
            try:
                status = json.loads(status_path.read_text(encoding="utf-8"))
            except (OSError, json.JSONDecodeError):
                time.sleep(0.05)
                continue
            if status.get("exitCode") != 0 or not status.get("cleanShutdown"):
                raise RuntimeError(f"simulator wrote an unclean exit status: {status}")
            return status
        time.sleep(0.05)
    raise RuntimeError(f"simulator did not write clean exit-status.json within 30s: {run_dir}")


def cleanup(container, run_dir, backend, display, shutdown_requested=False):
    errors = []
    clean_exit = False
    status_path = run_dir / "exit-status.json"
    if status_path.exists():
        try:
            status = json_file(status_path)
            clean_exit = status.get("exitCode") == 0 and bool(status.get("cleanShutdown"))
        except RuntimeError:
            clean_exit = False
    clean_exit = clean_exit or shutdown_requested or clean_shutdown_recorded(run_dir)
    try:
        if container and shutdown_requested:
            run(["docker", "stop", "--time", "10", container], capture=True, check=False, timeout=START_TIMEOUT)
        if container and (run_dir / "control.sock").exists() and not clean_exit:
            stop(run_dir, backend, display)
    except Exception as error:
        errors.append(f"stop failed: {error}")
    if container:
        try:
            run(["docker", "rm", "-f", container], capture=True, check=False, timeout=CONTROL_TIMEOUT)
        except Exception as error:
            errors.append(f"container cleanup failed: {error}")
    try:
        (run_dir / "control.sock").unlink(missing_ok=True)
    except OSError as error:
        errors.append(f"simulator control socket cleanup failed: {error}")
    return errors


def validate_focus_flow(container, run_dir):
    state = call(container, "state")
    if state["focus"] != {
        "entries": ["nebula-nes", "mirror-ps1"],
        "defaultHome": True,
        "kidSafe": True,
        "missing": 0,
    } or state["controllerRoute"]["currentId"] != "focus-home":
        raise RuntimeError(f"Focus admin flow did not produce the restricted ordered home: {state['focus']!r}")
    state = button(container, run_dir, "primary")
    if state["activeSession"] is None:
        raise RuntimeError("Focus launch did not create a session")
    button(container, run_dir, "menu")
    state = button(container, run_dir, "primary")
    if state["activeSession"] is None:
        raise RuntimeError("kid Continue did not keep the Focus session")
    button(container, run_dir, "menu")
    button(container, run_dir, "down")
    button(container, run_dir, "down")
    state = button(container, run_dir, "primary")
    if state["activeSession"] is not None or state["controllerRoute"]["currentId"] != "focus-home":
        raise RuntimeError("kid Exit did not return to Focus home")
    for control in ("menu", "select"):
        state = button(container, run_dir, control)
        if state["controllerRoute"]["currentId"] != "focus-home" or not state["focus"]["kidSafe"]:
            raise RuntimeError(f"restricted shortcut escaped Focus home: {control}")
    for control in ("start", "select", "start", "select"):
        state = button(container, run_dir, control)
    if state["controllerRoute"]["currentId"] is not None or state["focus"]["kidSafe"]:
        raise RuntimeError("parent gesture did not restore the full Home")



def one_pass(run_dir, route_ids, backend, display, smoke_routes=SMOKE_ROUTES):
    run_dir.mkdir(parents=True)
    route_paths = paths()
    container = None
    shutdown_requested = False
    visits, screenshots, screenshot_states, checkpoints = [], {}, {}, []
    pass_deadline = time.monotonic() + PASS_TIMEOUT
    failure = None
    cleanup_errors = []
    try:
        container = start(run_dir, backend, display)
        sequence = [(route_id, False) for route_id in smoke_routes]
        sequence.extend((route_id, True) for route_id in route_ids)
        for ordinal, (route_id, record) in enumerate(sequence, 1):
            route_deadline = min(time.monotonic() + ROUTE_TIMEOUT, pass_deadline)
            state = button(container, run_dir, "menu")
            if state["controllerRoute"]["currentId"] == "game-quick-menu":
                state = button(container, run_dir, "menu")
            if state["controllerRoute"]["currentId"] is not None:
                raise RuntimeError(f"{route_id} could not reset to Home before traversal")
            controls = route_paths[route_id]
            for control_index, control in enumerate(controls, 1):
                if time.monotonic() >= route_deadline:
                    raise RuntimeError(
                        f"{route_id} timed out before control {control_index}/{len(controls)} ({control}); "
                        f"pass output: {run_dir}"
                    )
                try:
                    state = button(container, run_dir, control)
                except Exception as error:
                    raise RuntimeError(
                        f"{route_id} failed at control {control_index}/{len(controls)} ({control}); "
                        f"inspect {run_dir}/logs/launcher.jsonl: {error}"
                    ) from error
            if state is None:
                raise RuntimeError(f"{route_id} had no controller state")
            controller = state.get("controllerRoute", {})
            presentation = state.get("presentation", {})
            if (
                controller.get("navigatorVisible", True)
                or controller.get("expectedCount") != CONTROLLER_ROUTE_COUNT
                or controller.get("currentId") != route_id
            ):
                raise RuntimeError(
                    f"{route_id} produced controller state {controller!r} after {len(controls)} controls"
                )
            if not isinstance(presentation, dict) or not presentation.get("route"):
                raise RuntimeError(f"{route_id} returned no semantic presentation record")
            artifact = call(container, "screenshot", "--name", f"route-{ordinal:02d}-{route_id}")
            png = run_dir / artifact["png"]
            if png_dimensions(png) != (1024, 768):
                raise RuntimeError(f"checkpoint is not 1024x768 for {route_id}: {png}")
            state = json_file(run_dir / artifact["state"])
            screen = state.get("presentation")
            if not isinstance(screen, dict) or not screen.get("menu"):
                raise RuntimeError(f"{route_id} screenshot has no semantic product presentation")
            labels = [item.get("label") for item in screen["menu"] if isinstance(item, dict)]
            if route_id in labels or any("Controller route navigator" in str(label) for label in labels):
                raise RuntimeError(f"{route_id} rendered route metadata instead of product rows")
            checkpoints.append({"ordinal": ordinal, "routeId": route_id, "png": artifact["png"], "state": artifact["state"]})
            if record:
                screenshots[route_id] = artifact["png"]
                screenshot_states[route_id] = normalize(screen)
                visits.append(route_id)
            if time.monotonic() >= pass_deadline:
                raise RuntimeError(f"whole pass timed out after route {route_id}; output: {run_dir}")
        validate_visits(visits, route_ids)
        if "focus-home" in route_ids:
            validate_focus_flow(container, run_dir)
        home_state = button(container, run_dir, "menu")
        if home_state["controllerRoute"]["currentId"] in {"focus-home", "focus-recovery", "kid-quick-menu"}:
            for control in ("start", "select", "start", "select"):
                home_state = button(container, run_dir, control)
            home_state = button(container, run_dir, "menu")
        if home_state["controllerRoute"]["currentId"] == "game-quick-menu":
            home_state = button(container, run_dir, "menu")
        if home_state["controllerRoute"]["currentId"] is not None:
            raise RuntimeError("could not return to Home before shutdown verification")
        shutdown_path = route_paths["shutdown-confirm"]
        if not shutdown_path or shutdown_path[-1] != "primary":
            raise RuntimeError("shutdown route must end with primary confirmation")
        for control in shutdown_path[:-1]:
            button(container, run_dir, control)
        cancelled = button(container, run_dir, "secondary")
        if cancelled["controllerRoute"]["currentId"] is not None or (run_dir / "exit-status.json").exists():
            raise RuntimeError(
                "shutdown cancel did not leave the simulator alive at Home: "
                f"state={cancelled.get('controllerRoute')!r}, exitStatus={(run_dir / 'exit-status.json').exists()}"
            )
        home_state = button(container, run_dir, "menu")
        if home_state["controllerRoute"]["currentId"] in {"focus-home", "focus-recovery", "kid-quick-menu"}:
            for control in ("start", "select", "start", "select"):
                home_state = button(container, run_dir, control)
            home_state = button(container, run_dir, "menu")
        if home_state["controllerRoute"]["currentId"] == "game-quick-menu":
            home_state = button(container, run_dir, "menu")
        if home_state["controllerRoute"]["currentId"] is not None:
            raise RuntimeError("could not reset Home selection before final shutdown verification")
        for control in shutdown_path[:-1]:
            button(container, run_dir, control)
        button(container, run_dir, "primary")
        shutdown_requested = True
    except Exception as error:
        failure = error
    finally:
        cleanup_errors = cleanup(container, run_dir, backend, display, shutdown_requested)
    if not failure:
        try:
            wait_for_clean_exit_status(run_dir)
        except RuntimeError as error:
            cleanup_errors.append(str(error))
    if failure:
        if cleanup_errors:
            raise RuntimeError(f"{failure}; cleanup also failed: {'; '.join(cleanup_errors)}") from failure
        raise failure
    if cleanup_errors:
        raise RuntimeError("; ".join(cleanup_errors))
    return {
        "visitedIds": visits,
        "eventVisits": event_visits(run_dir),
        "screenshots": screenshots,
        "screenshotStates": screenshot_states,
        "checkpoints": checkpoints,
        "semantic": {
            "visitedIds": visits,
            "eventVisits": event_visits(run_dir),
            "screenshots": screenshot_states,
        },
        "passed": visits == route_ids,
    }


def full_coverage(out, route_ids, backend, display):
    first = one_pass(out / "run-1", route_ids, backend, display, SMOKE_ROUTES)
    second = one_pass(out / "run-2", route_ids, backend, display, SMOKE_ROUTES)
    return {
        "schema": "controller-route-determinism/v5",
        "expectedIds": route_ids,
        "visitedIds": first["visitedIds"],
        "semanticRun1": first["semantic"],
        "semanticRun2": second["semantic"],
        "simulatorStarts": 2,
        "cleanups": 2,
        "passes": [first, second],
        "passed": first["semantic"] == second["semantic"] and first["passed"] and second["passed"],
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", required=True, type=pathlib.Path)
    parser.add_argument("--backend", choices=["dummy", "x11", "host-x11"], default="dummy")
    parser.add_argument("--display")
    parser.add_argument(
        "--smoke-only",
        action="store_true",
        help="run only the representative fail-first smoke subset; do not launch exhaustive coverage",
    )
    args = parser.parse_args()
    if args.out.exists():
        raise SystemExit("--out must not already exist")
    if args.backend == "host-x11" and not (args.display or os.environ.get("DISPLAY")):
        raise SystemExit("host-x11 requires --display or DISPLAY")
    graph = load_graph()
    route_ids = [route["id"] for route in graph["routes"]]
    smoke_routes = [] if args.smoke_only else SMOKE_ROUTES
    target_routes = SMOKE_ROUTES if args.smoke_only else route_ids
    args.out.mkdir(parents=True)
    if args.smoke_only:
        first = one_pass(args.out / "run-1", target_routes, args.backend, args.display, smoke_routes)
        result = {
            "schema": "controller-route-smoke/v1",
            "expectedIds": target_routes,
            "visitedIds": first["visitedIds"],
            "passed": first["passed"],
            "simulatorStarts": 1,
            "cleanups": 1,
        }
    else:
        result = full_coverage(args.out, route_ids, args.backend, args.display)
        manifest_path, checkpoint_count = write_action_recording(args.out, route_ids, *result.pop("passes"))
        result["actionRecording"] = {"manifest": manifest_path.relative_to(args.out).as_posix(), "checkpointCount": checkpoint_count}
    (args.out / "result.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if not result["passed"]:
        raise SystemExit("controller route coverage failed")
    print(f"controller route coverage: PASS ({len(target_routes)} routes) {args.out}")


if __name__ == "__main__":
    main()

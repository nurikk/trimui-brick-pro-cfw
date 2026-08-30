#!/usr/bin/env python3
import argparse
import collections
import json
import os
import pathlib
import struct
import subprocess
import time

ROOT = pathlib.Path(__file__).resolve().parents[3]
GRAPH = ROOT / "sim/routes/controller-routes.json"
SIM = ROOT / "scripts/sim"
PROFILE = "sim/device/tg4040-alphabet.json"

CONTROL_TIMEOUT = 15
START_TIMEOUT = 45
ROUTE_TIMEOUT = 60
PASS_TIMEOUT = 600
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
    "output",
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
    if (
        graph.get("expectedCount") != 64
        or len(route_ids) != 64
        or len(set(route_ids)) != 64
        or any(not route.get("buttons") for route in routes)
    ):
        raise RuntimeError("controller route graph must contain 64 unique button journeys")
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


def one_pass(run_dir, route_ids, backend, display, smoke_routes=SMOKE_ROUTES):
    run_dir.mkdir(parents=True)
    route_paths = paths()
    container = None
    shutdown_requested = False
    visits, screenshots, screenshot_states = [], {}, {}
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
                or controller.get("expectedCount") != 64
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
            if record:
                screenshots[route_id] = artifact["png"]
                screenshot_states[route_id] = normalize(screen)
                visits.append(route_id)
            if time.monotonic() >= pass_deadline:
                raise RuntimeError(f"whole pass timed out after route {route_id}; output: {run_dir}")
        validate_visits(visits, route_ids)
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
    (args.out / "result.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if not result["passed"]:
        raise SystemExit("controller route coverage failed")
    print(f"controller route coverage: PASS ({len(target_routes)} routes) {args.out}")


if __name__ == "__main__":
    main()

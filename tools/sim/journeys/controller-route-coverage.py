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
MANIFEST = pathlib.Path("/tmp/t_d5ce181e/authoritative-route-manifest.json")
SIM = ROOT / "scripts/sim"
PROFILE = "sim/device/tg4040-alphabet.json"


CONTROL_TIMEOUT = 15
START_TIMEOUT = 45
ROUTE_TIMEOUT = 60
PASS_TIMEOUT = 600


def run(command, *, capture=False, check=True, timeout=CONTROL_TIMEOUT):
    try:
        return subprocess.run(
            [str(part) for part in command],
            check=check,
            text=True,
            stdout=subprocess.PIPE if capture else None,
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
    records = (
        response(run(["docker", "inspect", *ids], capture=True).stdout) if ids else []
    )
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
        raise RuntimeError("simulator container identity is ambiguous")
    return matches[0]


def call(container, *args):
    result = response(
        run(
            [
                "docker",
                "exec",
                "--user",
                "10001:10001",
                container,
                "/usr/local/bin/sim-control",
                "--socket",
                "/evidence/control.sock",
                *args,
            ],
            capture=True,
            timeout=CONTROL_TIMEOUT,
        ).stdout
    )
    if not result.get("ok"):
        raise RuntimeError(f"simulator rejected {' '.join(args)}")
    return result["result"]


def event_visits(run_dir):
    path = run_dir / "logs/launcher.jsonl"
    try:
        events = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()]
    except (OSError, json.JSONDecodeError) as error:
        raise RuntimeError("invalid launcher event log") from error
    return [event["routeId"] for event in events if event["event"] == "controller_route_visit"]


def button(container, run_dir, name):
    before = event_visits(run_dir)
    call(container, "button", "--button", name, "--action", "press")
    state = call(container, "button", "--button", name, "--action", "release")
    emitted = event_visits(run_dir)[len(before):]
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


def home(index):
    return ["menu", *(["down"] * index), "primary"]


def paths():
    settings = home(4)
    system = [*settings, "primary"]
    base = {
        "home-systems": home(0),
        "games-long-list": home(1),
        "home-game-list": [*home(0), "primary"],
        "games-details": [*home(0), "primary", "right"],
        "games-favorite-toggle": [*home(0), "primary", "right", "select"],
        "games-favorites": home(2),
        "games-search-keyboard": home(3),
        "games-search-results": [*home(3), "start"],
        "settings-root": settings,
        "settings-display": [*settings, "primary"],
        "settings-input": [*settings, "primary", "down"],
        "settings-audio": [*settings, "primary", "down", "down"],
        "settings-power": [*settings, "primary", *(["down"] * 3)],
        "settings-library": [*settings, "primary", *(["down"] * 4)],
        "settings-scraper": [*settings, "primary", *(["down"] * 5)],
        "settings-theme": [*settings, "primary", *(["down"] * 6)],
        "settings-system": [*settings, "primary", *(["down"] * 7)],
        "settings-confirm-apply-cancel": [*settings, "primary", "right"],
        "settings-validation": [*settings, "primary", "down", "primary"],
        "wifi-scan": [*system, *(["down"] * 7), "primary"],
        "wifi-open-confirmation": [
            *system,
            *(["down"] * 7),
            "primary",
            "down",
            "down",
            "down",
        ],
        "wifi-secure-password": [*system, *(["down"] * 7), "primary", "primary"],
        "wifi-hidden": [*system, *(["down"] * 7), "primary", "down", "down"],
        "wifi-manual-ssid": [
            *system,
            *(["down"] * 7),
            "primary",
            "down",
            "down",
            "primary",
        ],
        "wifi-saved-network": [*system, *(["down"] * 7), "primary", "down"],
        "wifi-forget-confirmation": [
            *system,
            *(["down"] * 7),
            "primary",
            "down",
            "primary",
        ],
        "wifi-forgotten": [
            *system,
            *(["down"] * 7),
            "primary",
            "down",
            "primary",
            "primary",
        ],
        "wifi-connect-progress": [
            *system,
            *(["down"] * 7),
            "primary",
            "primary",
            "primary",
        ],
        "wifi-retry-error": [
            *system,
            *(["down"] * 7),
            "primary",
            "primary",
            "primary",
            "primary",
        ],
        "theme-garden-catalog": [*settings, "primary", *(["down"] * 6), "primary"],
        "theme-garden-preview": [
            *settings,
            "primary",
            *(["down"] * 6),
            "primary",
            "primary",
        ],
        "theme-garden-install": [
            *settings,
            "primary",
            *(["down"] * 6),
            "primary",
            "primary",
            "primary",
        ],
        "theme-garden-update": [
            *settings,
            "primary",
            *(["down"] * 6),
            "primary",
            "primary",
            "primary",
            "primary",
        ],
        "theme-garden-remove": [
            *settings,
            "primary",
            *(["down"] * 6),
            "primary",
            "primary",
            "primary",
            "primary",
            "primary",
        ],
        "theme-garden-fallback": [
            *settings,
            "primary",
            *(["down"] * 6),
            "primary",
            "primary",
            "down",
        ],
        "scraper-settings": [*settings, "primary", *(["down"] * 5), "primary"],
        "scraper-game": [*settings, "primary", *(["down"] * 5), "primary", "primary"],
        "scraper-queue": [
            *settings,
            "primary",
            *(["down"] * 5),
            "primary",
            "primary",
            "primary",
        ],
        "scraper-progress": [
            *settings,
            "primary",
            *(["down"] * 5),
            "primary",
            "primary",
            "primary",
            "primary",
        ],
        "scraper-paused": [
            *settings,
            "primary",
            *(["down"] * 5),
            "primary",
            "primary",
            "primary",
            "primary",
            "select",
        ],
        "scraper-ambiguity": [
            *settings,
            "primary",
            *(["down"] * 5),
            "primary",
            "primary",
            "primary",
            "primary",
            "right",
        ],
        "scraper-success": [
            *settings,
            "primary",
            *(["down"] * 5),
            "primary",
            "primary",
            "primary",
            "primary",
            "start",
        ],
        "scraper-failure": [
            *settings,
            "primary",
            *(["down"] * 5),
            "primary",
            "primary",
            "primary",
            "primary",
            "primary",
        ],
        "diagnostics": home(5),
        "diagnostics-safe-mode": [*home(5), "primary"],
        "updater-available": [*home(5), *(["down"] * 2), "primary"],
        "updater-rollback": [*home(5), *(["down"] * 3), "primary"],
        "faults-storage-full": [*home(5), *(["down"] * 4), "primary"],
        "faults-low-battery": [*home(5), *(["down"] * 5)],
        "shutdown-confirm": [*home(5), *(["down"] * 5), "primary"],
        "game-switcher-list": home(6),
        "game-switcher-resume": [*home(6), "primary"],
        "game-switcher-restoration": [*home(6), "primary", "primary"],
        "platform-nebula-launch": [*home(0), "down", "primary"],
        "platform-mirror-launch": [*home(0), "down", "down", "primary"],
        "portmaster-catalog": home(7),
        "portmaster-install": [*home(7), "primary"],
        "portmaster-launch-orbit": [*home(7), "primary", "primary"],
        "portmaster-launch-signal": [*home(7), "down"],
        "portmaster-uninstall-protected-data": [*home(7), "select"],
    }
    # These require a real session/checkpoint; they deliberately do not use synthetic route actions.
    base["game-switcher-autosave"] = [*base["platform-nebula-launch"], "right", "select"]
    base["game-switcher-exit"] = [*base["platform-nebula-launch"], "right", "secondary"]
    base["platform-nebula-restored"] = [
        *base["game-switcher-autosave"],
        "primary",
        "menu",
        *home(6),
        "primary",
        "primary",
        "primary",
    ]
    base["platform-mirror-restored"] = [
        *base["platform-mirror-launch"],
        "secondary",
        "menu",
        *home(6),
        "primary",
        "primary",
        "primary",
    ]
    return base


SMOKE_ROUTES = [
    "home-systems",
    "settings-display",
    "diagnostics-safe-mode",
    "portmaster-catalog",
    "game-switcher-autosave",
]
RUN_SPECIFIC_KEYS = {"runId", "atMs", "sequence", "eventSequence", "timestampMs"}


def wait_for_exit(container, run_dir):
    deadline = time.monotonic() + CONTROL_TIMEOUT
    while time.monotonic() < deadline:
        status_path = run_dir / "exit-status.json"
        if status_path.exists() and not (run_dir / "control.sock").exists():
            status = json_file(status_path)
            clean_shutdown = status.get("cleanShutdown")
            if (
                status.get("exitCode") != 0
                or type(clean_shutdown) != bool
                or not clean_shutdown
            ):
                raise RuntimeError(f"simulator {container} wrote an unclean exit status: {status}")
            return
        time.sleep(0.05)
    raise RuntimeError(
        f"simulator {container} did not remove control.sock and write clean exit-status.json"
    )


def normalize(value):
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
    duplicates = sorted(
        route for route, count in collections.Counter(visits).items() if count > 1
    )
    unexpected = sorted(set(visits) - set(route_ids))
    missing = sorted(set(route_ids) - set(visits))
    if duplicates or unexpected or missing or visits != route_ids:
        raise RuntimeError(
            f"route coverage mismatch: duplicates={duplicates}, unexpected={unexpected}, "
            f"missing={missing}, visited={visits}"
        )


def one_pass(run_dir, route_ids, backend, display, smoke_routes=SMOKE_ROUTES):
    run_dir.mkdir(parents=True)
    container = None
    visits, screenshots, screenshot_states = [], {}, {}
    pass_deadline = time.monotonic() + PASS_TIMEOUT
    try:
        container = start(run_dir, backend, display)
        sequence = [(route_id, False) for route_id in smoke_routes]
        sequence.extend((route_id, True) for route_id in route_ids)
        smoke_count = len(smoke_routes)
        for ordinal, (route_id, record) in enumerate(sequence, 1):
            route_deadline = min(time.monotonic() + ROUTE_TIMEOUT, pass_deadline)
            state = None
            controls = paths()[route_id]
            for control_index, control in enumerate(controls, 1):
                if time.monotonic() >= route_deadline:
                    raise RuntimeError(
                        f"{route_id} timed out before control {control_index}/{len(controls)} ({control})"
                    )
                try:
                    state = button(container, run_dir, control)
                except Exception as error:
                    raise RuntimeError(
                        f"{route_id} failed at control {control_index}/{len(controls)} ({control}): {error}"
                    ) from error
            if state is None:
                raise RuntimeError(f"{route_id} had no controller state")
            actual = state["controllerRoute"]["currentId"]
            presentation_route = state["presentation"]["route"]
            if actual != route_id or presentation_route != route_id:
                raise RuntimeError(
                    f"{route_id} produced {actual}/{presentation_route} after {len(controls)} controls"
                )
            artifact = call(
                container, "screenshot", "--name", f"route-{ordinal:02d}-{route_id}"
            )
            png = run_dir / artifact["png"]
            if png_dimensions(png) != (1024, 768):
                raise RuntimeError(f"checkpoint is not 1024x768: {route_id}")
            state = json_file(run_dir / artifact["state"])
            screen = state["presentation"]
            labels = [item["label"] for item in screen["menu"]]
            if (
                not labels
                or route_id in labels
                or any("Controller route navigator" in label for label in labels)
            ):
                raise RuntimeError(
                    f"{route_id} rendered route metadata instead of product rows"
                )
            if record:
                screenshots[route_id] = artifact["png"]
                screenshot_states[route_id] = normalize(screen)
                visits.append(actual)
            elif ordinal == smoke_count:
                print(f"smoke subset passed in {run_dir}")
            if time.monotonic() >= pass_deadline:
                raise RuntimeError(f"whole pass timed out after route {route_id}")
        validate_visits(visits, route_ids)
        unexpected_events = sorted(set(event_visits(run_dir)) - set(route_ids))
        if unexpected_events:
            raise RuntimeError(f"unexpected controller route events: {unexpected_events}")
        for control in paths()["shutdown-confirm"]:
            button(container, run_dir, control)
        cancelled = button(container, run_dir, "secondary")
        if (
            cancelled["controllerRoute"]["currentId"] is not None
            or (run_dir / "exit-status.json").exists()
        ):
            raise RuntimeError("shutdown cancel did not leave the simulator alive at Home")
        for control in paths()["shutdown-confirm"]:
            button(container, run_dir, control)
        button(container, run_dir, "primary")
        wait_for_exit(container, run_dir)
    finally:
        try:
            if container and (run_dir / "control.sock").exists():
                stop(run_dir, backend, display)
        finally:
            if container:
                run(["docker", "rm", "-f", container], check=False)
            if (run_dir / "control.sock").exists():
                raise RuntimeError("simulator control socket remains after cleanup")
            scoped = run(
                [
                    "docker",
                    "ps",
                    "-q",
                    "--filter",
                    "label=org.trimui-brick-pro-cfw.simulator=host-native",
                ],
                capture=True,
            ).stdout.split()
            records = (
                response(run(["docker", "inspect", *scoped], capture=True).stdout)
                if scoped
                else []
            )
            if any(
                any(
                    mount["Destination"] == "/evidence"
                    and pathlib.Path(mount["Source"])
                    .resolve()
                    .is_relative_to(run_dir.parent.resolve())
                    for mount in record["Mounts"]
                )
                for record in records
            ):
                raise RuntimeError("scoped simulator container remains after cleanup")
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


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", required=True, type=pathlib.Path)
    parser.add_argument(
        "--backend", choices=["dummy", "x11", "host-x11"], default="dummy"
    )
    parser.add_argument("--display")
    args = parser.parse_args()
    if args.out.exists():
        raise SystemExit("--out must not already exist")
    if args.backend == "host-x11" and not (args.display or os.environ.get("DISPLAY")):
        raise SystemExit("host-x11 requires --display or DISPLAY")
    manifest = json_file(MANIFEST)
    route_ids = manifest["orderedRouteIds"]
    if (
        manifest.get("expectedCount") != 64
        or len(route_ids) != 64
        or len(set(route_ids)) != 64
        or set(route_ids) != set(paths())
    ):
        raise SystemExit("authoritative controller manifest/path set is invalid")
    args.out.mkdir(parents=True)
    first = one_pass(args.out / "run-1", route_ids, args.backend, args.display)
    second = one_pass(args.out / "run-2", route_ids, args.backend, args.display)
    result = {
        "schema": "controller-route-determinism/v4",
        "expectedIds": route_ids,
        "visitedIds": first["visitedIds"],
        "duplicateIds": [
            route
            for route, count in collections.Counter(first["visitedIds"]).items()
            if count > 1
        ],
        "eventVisits": first["eventVisits"],
        "semanticRun1": first["semantic"],
        "semanticRun2": second["semantic"],
        "passed": first["semantic"] == second["semantic"] and first["passed"] and second["passed"],
    }
    (args.out / "result.json").write_text(
        json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    if not result["passed"]:
        raise SystemExit("controller route coverage failed")
    print(
        f"controller route coverage: PASS ({len(route_ids)} independent product journeys) {args.out}"
    )


if __name__ == "__main__":
    main()

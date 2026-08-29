#!/usr/bin/env python3
import argparse
import collections
import hashlib
import json
import os
import pathlib
import struct
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[3]
GRAPH = ROOT / "sim/routes/controller-routes.json"
SIM = ROOT / "scripts/sim"


def run(command, *, capture=False):
    return subprocess.run(
        [str(part) for part in command],
        check=True,
        text=True,
        stdout=subprocess.PIPE if capture else None,
    )


def container_for(run_dir):
    ids = run(
        ["docker", "ps", "-q", "--filter", "label=org.trimui-brick-pro-cfw.simulator=host-native"],
        capture=True,
    ).stdout.split()
    if not ids:
        raise RuntimeError("simulator container is missing")
    records = json.loads(run(["docker", "inspect", *ids], capture=True).stdout)
    expected = run_dir.resolve()
    matches = [
        record["Id"]
        for record in records
        if any(
            mount["Destination"] == "/evidence"
            and pathlib.Path(mount["Source"]).resolve() == expected
            for mount in record["Mounts"]
        )
    ]
    if len(matches) != 1:
        raise RuntimeError("simulator container identity is ambiguous")
    return matches[0]


def call(container, *args):
    completed = run(
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
    )
    response = json.loads(completed.stdout)
    if not response.get("ok"):
        raise RuntimeError(f"simulator rejected {' '.join(args)}")
    return response["result"]


def button(container, name):
    return call(container, "button", "--button", name, "--action", "press")


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
        "--run-dir",
        run_dir,
        "--wait-ready",
        "60",
        "--detach",
    ]
    if display:
        command.extend(["--display", display])
    run(command)
    container = container_for(run_dir)
    call(container, "wait-ready", "--timeout", "30")
    return container


def stop(run_dir, backend, display):
    command = [SIM, "stop", f"--backend={backend}", "--run-dir", run_dir]
    if display:
        command.extend(["--display", display])
    run(command)


def one_pass(run_dir, graph, backend, display):
    run_dir.mkdir(parents=True)
    action_log = run_dir / "controller-actions.jsonl"
    visited = []
    screenshots = {}
    presentation_routes = {}
    started = False
    try:
        container = start(run_dir, backend, display)
        started = True
        with action_log.open("w", encoding="utf-8") as stream:
            for index, route in enumerate(graph["routes"]):
                button(container, "menu")
                if index:
                    button(container, "down")
                state = button(container, "primary")
                route_id = route["id"]
                controller = state["controllerRoute"]
                if controller["navigatorVisible"] or controller["currentId"] != route_id:
                    raise RuntimeError(f"controller did not visit {route_id}")
                actual_presentation_route = state["presentation"]["route"]
                expected_presentation_route = route["expectedPresentationRoute"]
                if actual_presentation_route != expected_presentation_route:
                    raise RuntimeError(
                        f"controller route {route_id} rendered {actual_presentation_route}, "
                        f"expected {expected_presentation_route}"
                    )
                checkpoint = f"route-{index + 1:02d}-{route_id}"
                artifact = call(container, "screenshot", "--name", checkpoint)
                png = run_dir / artifact["png"]
                if png_dimensions(png) != (1024, 768):
                    raise RuntimeError(f"checkpoint is not 1024x768: {route_id}")
                screenshots[route_id] = artifact["png"]
                presentation_routes[route_id] = actual_presentation_route
                visited.append(route_id)
                if route["action"] != "save-vault-confirm":
                    exited = button(container, "secondary")
                    if exited["controllerRoute"]["currentId"] is not None:
                        raise RuntimeError(f"controller back/cancel did not exit {route_id}")
                stream.write(
                    json.dumps(
                        {
                            "sequence": index,
                            "buttons": [
                                "menu",
                                *(["down"] if index else []),
                                "primary",
                                *([] if route["action"] == "save-vault-confirm" else ["secondary"]),
                            ],
                            "routeId": route_id,
                            "checkpoint": artifact["png"],
                        },
                        sort_keys=True,
                    )
                    + "\n"
                )
        expected = [route["id"] for route in graph["routes"]]
        counts = collections.Counter(visited)
        duplicates = sorted(route_id for route_id, count in counts.items() if count > 1)
        unexpected = sorted(set(visited) - set(expected))
        missing = sorted(set(expected) - set(visited))
        coverage = {
            "schema": "controller-route-coverage/v1",
            "lane": "host-native-simulator",
            "input": "controller-buttons-only",
            "graphSha256": hashlib.sha256(GRAPH.read_bytes()).hexdigest(),
            "expectedCount": len(expected),
            "visitedCount": len(visited),
            "expectedIds": expected,
            "visitedIds": visited,
            "duplicateIds": duplicates,
            "unexpectedIds": unexpected,
            "missingIds": missing,
            "screenshots": screenshots,
            "presentationRoutes": presentation_routes,
            "passed": not duplicates and not unexpected and not missing,
        }
        (run_dir / "coverage.json").write_text(
            json.dumps(coverage, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        if not coverage["passed"]:
            raise RuntimeError("controller route coverage has gaps")
    finally:
        if started:
            stop(run_dir, backend, display)
    if not (run_dir / "exit-status.json").exists():
        raise RuntimeError("clean shutdown evidence is missing")
    status = json.loads((run_dir / "exit-status.json").read_text(encoding="utf-8"))
    if status.get("cleanShutdown") is not True or status.get("exitCode") != 0:
        raise RuntimeError("simulator did not shut down cleanly")
    return coverage


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
    graph = json.loads(GRAPH.read_text(encoding="utf-8"))
    if graph.get("schema") != "controller-route-graph/v1":
        raise SystemExit("invalid controller route graph schema")
    args.out.mkdir(parents=True)
    first = one_pass(args.out / "run-1", graph, args.backend, args.display)
    second = one_pass(args.out / "run-2", graph, args.backend, args.display)
    equal = first == second
    result = {
        "schema": "controller-route-determinism/v1",
        "lane": "host-native-simulator",
        "backend": args.backend,
        "semanticResultsEqual": equal,
        "expectedCount": first["expectedCount"],
        "missingIds": first["missingIds"],
        "duplicateIds": first["duplicateIds"],
        "unexpectedIds": first["unexpectedIds"],
        "cleanShutdowns": 2,
        "passed": equal and first["passed"] and second["passed"],
    }
    (args.out / "result.json").write_text(
        json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    if not result["passed"]:
        raise SystemExit("controller route determinism failed")
    print(
        f"controller route coverage: PASS ({first['expectedCount']} routes, two equal fresh roots, clean shutdowns) {args.out}"
    )


if __name__ == "__main__":
    main()

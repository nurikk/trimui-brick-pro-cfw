#!/usr/bin/env python3
import importlib.util
import tempfile
import unittest
from pathlib import Path
from unittest import mock


MODULE_PATH = Path(__file__).with_name("controller-route-coverage.py")
SPEC = importlib.util.spec_from_file_location("controller_route_coverage", MODULE_PATH)
if SPEC is None or SPEC.loader is None:
    raise ImportError(f"cannot load {MODULE_PATH}")
coverage = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(coverage)


class ControllerCoverageTests(unittest.TestCase):
    def fake_pass(self, route_ids, smoke_routes=()):
        paths = {route_id: ["primary"] for route_id in route_ids}
        paths.update({route_id: ["primary"] for route_id in smoke_routes})
        paths["shutdown-confirm"] = ["primary", "primary"]
        calls = []
        primary_targets = iter([*smoke_routes, *route_ids, "shutdown-confirm", "shutdown-confirm", "shutdown-confirm"])

        def button(_container, _run_dir, name):
            calls.append(name)
            if name == "menu" or name == "secondary":
                return {"controllerRoute": {"navigatorVisible": False, "expectedCount": 64, "currentId": None}}
            return {
                "controllerRoute": {"navigatorVisible": False, "expectedCount": 64, "currentId": next(primary_targets)},
                "presentation": {"route": "product"},
            }

        return paths, calls, button

    def test_two_pass_orchestration_starts_and_cleans_each_root_once(self):
        starts = mock.Mock(side_effect=["run-1-container", "run-2-container"])
        cleanups = mock.Mock(return_value=[])
        route_ids = ["route-a"]
        with tempfile.TemporaryDirectory() as temp:
            paths, _calls, button1 = self.fake_pass(route_ids)
            _paths, _calls, button2 = self.fake_pass(route_ids)
            with mock.patch.object(coverage, "SMOKE_ROUTES", []), mock.patch.object(
                coverage, "paths", return_value=paths
            ), mock.patch.object(
                coverage, "start", starts
            ), mock.patch.object(coverage, "cleanup", cleanups), mock.patch.object(
                coverage, "button", side_effect=lambda container, run_dir, name: (
                    button1(container, run_dir, name) if run_dir.name == "run-1" else button2(container, run_dir, name)
                )
            ), mock.patch.object(coverage, "call", return_value={"png": "screen.png", "state": "screen.json"}), mock.patch.object(
                coverage, "png_dimensions", return_value=(1024, 768)
            ), mock.patch.object(
                coverage, "json_file", side_effect=lambda path: (
                    {"exitCode": 0, "cleanShutdown": True}
                    if path.name == "exit-status.json"
                    else {"presentation": {"menu": [{"label": "Product"}]}}
                )
            ), mock.patch.object(coverage, "event_visits", return_value=[]), mock.patch.object(
                coverage, "wait_for_clean_exit_status"
            ):
                coverage.full_coverage(Path(temp), route_ids, "dummy", None)
        self.assertEqual(starts.call_count, 2)
        self.assertEqual(cleanups.call_count, 2)

    def test_routes_share_one_pass_without_per_route_restart(self):
        starts = mock.Mock(return_value="container")
        cleanups = mock.Mock(return_value=[])
        route_ids = ["route-a", "route-b"]
        with tempfile.TemporaryDirectory() as temp:
            paths, calls, button = self.fake_pass(route_ids)
            with mock.patch.object(coverage, "paths", return_value=paths), mock.patch.object(
                coverage, "start", starts
            ), mock.patch.object(coverage, "cleanup", cleanups), mock.patch.object(
                coverage, "button", side_effect=button
            ), mock.patch.object(coverage, "call", return_value={"png": "screen.png", "state": "screen.json"}), mock.patch.object(
                coverage, "png_dimensions", return_value=(1024, 768)
            ), mock.patch.object(
                coverage, "json_file", side_effect=lambda path: (
                    {"exitCode": 0, "cleanShutdown": True}
                    if path.name == "exit-status.json"
                    else {"presentation": {"menu": [{"label": "Product"}]}}
                )
            ), mock.patch.object(coverage, "event_visits", return_value=[]), mock.patch.object(
                coverage, "wait_for_clean_exit_status"
            ):
                coverage.one_pass(Path(temp) / "run", route_ids, "dummy", None, ())
        self.assertEqual(starts.call_count, 1)
        self.assertEqual(calls.count("menu"), 4)
        self.assertEqual(cleanups.call_count, 1)

    def test_normalization_ignores_artifacts_but_keeps_order_and_presentation(self):
        first = {
            "visitedIds": ["home-systems", "settings-display"],
            "screenshots": {"home-systems": {"png": "run-1/screenshots/one.png", "route": "systems"}},
            "runId": "run-one",
            "timestampMs": 10,
        }
        second = {
            "visitedIds": ["home-systems", "settings-display"],
            "screenshots": {"home-systems": {"png": "run-2/screenshots/other.png", "route": "systems"}},
            "runId": "run-two",
            "timestampMs": 99,
        }
        self.assertEqual(coverage.normalize(first), coverage.normalize(second))
        changed = dict(second)
        changed["visitedIds"] = ["settings-display", "home-systems"]
        self.assertNotEqual(coverage.normalize(first), coverage.normalize(changed))
        changed = dict(second)
        changed["screenshots"] = {"home-systems": {"png": "run-2/other.png", "route": "games"}}
        self.assertNotEqual(coverage.normalize(first), coverage.normalize(changed))

    def test_smoke_failure_prevents_exhaustive_routes(self):
        calls = []
        paths = {"smoke": ["primary"], "exhaustive": ["primary"], "shutdown-confirm": ["primary"]}

        def button(_container, _run_dir, name):
            calls.append(name)
            if name == "primary":
                raise RuntimeError("smoke failed")
            return {"controllerRoute": {"navigatorVisible": False, "expectedCount": 64, "currentId": None}}

        with tempfile.TemporaryDirectory() as temp:
            with mock.patch.object(coverage, "paths", return_value=paths), mock.patch.object(
                coverage, "start", return_value="container"
            ), mock.patch.object(coverage, "cleanup", return_value=[]), mock.patch.object(
                coverage, "button", side_effect=button
            ):
                with self.assertRaisesRegex(RuntimeError, "smoke failed"):
                    coverage.one_pass(Path(temp) / "run", ["exhaustive"], "dummy", None, ["smoke"])
        self.assertEqual(calls, ["menu", "primary"])


if __name__ == "__main__":
    unittest.main()

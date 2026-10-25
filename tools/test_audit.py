"""Failure seams in the disposable native audit harness; no compositor required."""

import importlib.util
import json
import os
from pathlib import Path
import signal
import sys
import tempfile
import time
import unittest


SPEC = importlib.util.spec_from_file_location("audit", Path(__file__).with_name("audit.py"))
audit = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(audit)


@unittest.skipUnless(sys.platform.startswith("linux"), "Linux session harness")
class AuditTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.output = Path(self.directory.name)
        self.processes = audit.Processes(self.output, 30)

    def tearDown(self):
        self.processes.cleanup()
        self.directory.cleanup()

    def test_timeout_reaps_owned_child_and_records_failure(self):
        child = self.processes.spawn(
            "hung", [sys.executable, "-c", "import time; time.sleep(60)"],
            os.environ.copy(), timeout=0.05,
        )
        with self.assertRaisesRegex(audit.AuditFailure, "deadline"):
            self.processes.finish(child)
        self.processes.cleanup()
        self.assertIsNotNone(child["process"].poll())
        events = [json.loads(line) for line in self.processes.events.read_text().splitlines()]
        self.assertEqual(events[0]["event"], "spawn")
        self.assertEqual(events[-1]["event"], "reaped")

    def test_exited_leader_does_not_leave_descendant_running(self):
        pid_path = self.output / "descendant.pid"
        source = (
            "import pathlib, subprocess, sys; "
            "child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)']); "
            "pathlib.Path(sys.argv[1]).write_text(str(child.pid))"
        )
        self.processes.run("leader", [sys.executable, "-c", source, pid_path], os.environ.copy())
        leader = self.processes.children[-1]["process"]
        self.assertIsNone(leader.returncode)
        self.assertEqual(Path(f"/proc/{leader.pid}/stat").read_text().split()[2], "Z")
        descendant = int(pid_path.read_text())
        self.processes.cleanup()
        deadline = time.monotonic() + 2
        while time.monotonic() < deadline:
            stat = Path(f"/proc/{descendant}/stat")
            if not stat.exists() or stat.read_text().split()[2] == "Z":
                break
            time.sleep(0.01)
        else:
            os.kill(descendant, signal.SIGKILL)
            self.fail("owned descendant survived cleanup after its leader exited")

    def test_nonzero_commands_are_not_success(self):
        with self.assertRaisesRegex(audit.AuditFailure, "returned 7"):
            self.processes.run("failure", [sys.executable, "-c", "raise SystemExit(7)"],
                               os.environ.copy())
        result = self.processes.run("expected-failure", [sys.executable, "-c", "raise SystemExit(1)"],
                                    os.environ.copy(), expected=1)
        self.assertEqual(result, "")

    def test_completion_requires_one_structured_verdict_without_failure(self):
        for log in ("", "AUDIT_COMPLETE {}\nAUDIT_COMPLETE {}", "AUDIT_COMPLETE []",
                    "AUDIT_COMPLETE {}\nAUDIT_FAILED: rejected"):
            with self.assertRaises(audit.AuditFailure):
                audit.completion(log, "workflow")
        with self.assertRaises(ValueError):
            audit.completion("AUDIT_COMPLETE broken", "workflow")
        self.assertEqual(audit.completion('AUDIT_COMPLETE {"busy":false}', "workflow"),
                         {"busy": False})

    def test_state_paths_are_private_and_environment_is_not_mutated(self):
        before = os.environ.copy()
        args = audit.parse_args(["--compositor", "sway", "--output-dir", str(self.output)])
        session = audit.Audit(args, self.output, self.processes)
        self.assertEqual(session.environment.get("HOME"), before.get("HOME"))
        for key in ("XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME"):
            self.assertTrue(Path(session.environment[key]).is_relative_to(self.output))
        for key in ("NIRI_SOCKET", "SWAYSOCK", "HYPRLAND_INSTANCE_SIGNATURE",
                    "DBUS_SESSION_BUS_ADDRESS"):
            self.assertNotIn(key, session.environment)
        self.assertEqual(os.environ, before)

    def test_overall_deadline_blocks_new_processes(self):
        self.processes.deadline = time.monotonic() - 1
        with self.assertRaisesRegex(audit.AuditFailure, "overall audit deadline"):
            self.processes.spawn("late", [sys.executable, "-c", "pass"], os.environ.copy())
        self.assertFalse(self.processes.children)

    def test_atomic_configuration_replaces_complete_json(self):
        path = self.output / "config.json"
        audit.atomic_json(path, {"outputs": {"one": 1}})
        audit.atomic_json(path, {"outputs": {}})
        self.assertEqual(json.loads(path.read_text()), {"outputs": {}})
        self.assertFalse(path.with_suffix(".json.pending").exists())

    def test_screenshot_verdict_requires_a_real_png(self):
        path = self.output / "frame.png"
        path.write_bytes(b"not an image")
        with self.assertRaisesRegex(audit.AuditFailure, "invalid PNG"):
            audit.verify_png(path)
        with self.assertRaisesRegex(audit.AuditFailure, "missing screenshot"):
            audit.verify_png(self.output / "missing.png")


if __name__ == "__main__":
    unittest.main()

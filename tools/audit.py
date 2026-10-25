#!/usr/bin/env python3
"""Exercise native Wisp workflows in an owned, disposable Linux desktop."""

import argparse
import copy
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time


ROOT = Path(__file__).resolve().parents[1]


class AuditFailure(RuntimeError):
    pass


def require(condition, message):
    if not condition:
        raise AuditFailure(message)


def atomic_json(path, value):
    temporary = path.with_suffix(path.suffix + ".pending")
    with temporary.open("w") as stream:
        json.dump(value, stream, indent=2)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())
    temporary.replace(path)


class Processes:
    """Every child has an owned process group, including short-lived commands."""

    def __init__(self, output, timeout):
        self.output = output
        self.timeout = timeout
        self.deadline = time.monotonic() + timeout
        self.children = []
        self.events = output / "events.jsonl"

    def record(self, event, **values):
        with self.events.open("a") as stream:
            stream.write(json.dumps({"event": event, "time": time.time(), **values}) + "\n")

    def check_deadline(self):
        require(time.monotonic() < self.deadline, "overall audit deadline exceeded")

    def spawn(self, name, args, env, timeout=150):
        self.check_deadline()
        args = [str(argument) for argument in args]
        log = self.output / (name + ".log")
        with log.open("wb") as stream:
            process = subprocess.Popen(
                args, cwd=ROOT, env=env, stdout=stream, stderr=subprocess.STDOUT,
                start_new_session=True,
            )
        child = {"process": process, "name": name, "log": log,
                 "deadline": min(self.deadline, time.monotonic() + timeout)}
        self.children.append(child)
        self.record("spawn", name=name, args=args, pid=process.pid, timeout=timeout)
        return child

    def alive(self, child):
        self.check_deadline()
        require(time.monotonic() < child["deadline"], f'{child["name"]} exceeded its deadline')
        require(self.exit_status(child) is None,
                f'{child["name"]} exited early; see {child["log"]}')

    def exit_status(self, child):
        process = child["process"]
        if process.returncode is not None:
            return process.returncode
        # Retaining the zombie reserves its PID/group ID until descendant teardown.
        status = os.waitid(os.P_PID, process.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
        if status is None:
            return None
        return status.si_status if status.si_code == os.CLD_EXITED else -status.si_status

    def wait_for(self, name, predicate, timeout=20, child=None):
        deadline = min(self.deadline, time.monotonic() + timeout)
        while time.monotonic() < deadline:
            if child:
                self.alive(child)
            value = predicate()
            if value:
                self.record("checkpoint", name=name)
                return value
            time.sleep(0.05)
        raise AuditFailure(f"timeout waiting for {name}")

    def finish(self, child, expected=0):
        process = child["process"]
        deadline = min(self.deadline, child["deadline"])
        while (code := self.exit_status(child)) is None:
            require(time.monotonic() < deadline, f'{child["name"]} exceeded its deadline')
            time.sleep(0.02)
        self.record("exit", name=child["name"], pid=process.pid, code=code, expected=expected)
        require(code == expected,
                f'{child["name"]} returned {code}, expected {expected}; see {child["log"]}')
        return child["log"].read_text(errors="replace")

    def run(self, name, args, env, timeout=60, expected=0):
        return self.finish(self.spawn(name, args, env, timeout), expected)

    def cleanup(self):
        # A leader may exit before its descendants, so inspect every owned group.
        for child in reversed(self.children):
            process = child["process"]
            if process.returncode is not None:
                continue
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline and any(
            self.exit_status(child) is None for child in self.children
        ):
            time.sleep(0.05)
        for child in reversed(self.children):
            process = child["process"]
            if process.returncode is not None:
                continue
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait(timeout=5)
            self.record("reaped", name=child["name"], pid=process.pid,
                        code=process.returncode)


def command(name, **values):
    return {"command": name, **values}


def assertion(**state):
    return command("assert", state=state)


def wait(milliseconds=1800):
    return command("wait", milliseconds=milliseconds)


def ready(**state):
    state.setdefault("busy", False)
    return command("wait_for", state=state, timeout_ms=10000)


def screenshot(path):
    return command("screenshot", path=str(path))


def verify_png(path):
    require(path.is_file(), f"missing screenshot: {path}")
    with path.open("rb") as stream:
        header = stream.read(24)
    require(header[:8] == b"\x89PNG\r\n\x1a\n" and len(header) == 24,
            f"invalid PNG: {path}")
    dimensions = [int.from_bytes(header[index:index + 4], "big") for index in (16, 20)]
    require(all(size >= 32 for size in dimensions), f"unexpected screenshot size: {dimensions}")
    return dimensions


def completion(log, name):
    require("AUDIT_FAILED" not in log, f"{name}: audit failure in log")
    completions = [line.removeprefix("AUDIT_COMPLETE ") for line in log.splitlines()
                   if line.startswith("AUDIT_COMPLETE ")]
    require(len(completions) == 1, f"{name}: missing unique AUDIT_COMPLETE verdict")
    state = json.loads(completions[0])
    require(isinstance(state, dict), f"{name}: completion state must be an object")
    return state


class Audit:
    def __init__(self, args, output, processes):
        self.args = args
        self.output = output
        self.processes = processes
        self.binaries = {name: args.bin_dir / name for name in
                         ("wisp", "wispd", "wispc", "wisp-preview")}
        self.environment = os.environ.copy()
        for key in ("NIRI_SOCKET", "SWAYSOCK", "HYPRLAND_INSTANCE_SIGNATURE",
                    "DBUS_SESSION_BUS_ADDRESS", "XDG_ACTIVATION_TOKEN", "DESKTOP_STARTUP_ID"):
            self.environment.pop(key, None)
        for key, directory in (("XDG_DATA_HOME", "data"),
                               ("XDG_CONFIG_HOME", "config"), ("XDG_CACHE_HOME", "cache"),
                               ("XDG_STATE_HOME", "state")):
            path = output / "session" / directory
            path.mkdir(parents=True)
            self.environment[key] = str(path)
        self.environment["WINIT_UNIX_BACKEND"] = "wayland"
        self.environment.setdefault("WGPU_BACKEND", "vulkan")
        self.runtime_alias = None

    def binary(self, name):
        path = self.binaries[name]
        require(path.is_file() and os.access(path, os.X_OK),
                f"missing executable {path}; build the workspace or pass --build")
        return path

    def start_desktop(self):
        for executable in (self.args.compositor, "wtype", "grim"):
            require(shutil.which(executable), f"required executable not found: {executable}")
        runtime = self.output / "session" / "runtime"
        runtime.mkdir(mode=0o700)
        # Unix socket names have a small fixed limit, even in deeply nested worktrees.
        self.runtime_alias = Path(tempfile.mkdtemp(prefix="wisp-audit-"))
        (self.runtime_alias / "runtime").symlink_to(runtime, target_is_directory=True)
        runtime_path = self.runtime_alias / "runtime"
        compositor_environment = self.environment.copy()
        compositor_environment["XDG_RUNTIME_DIR"] = str(runtime_path)
        if self.args.compositor == "niri":
            parent_display = os.environ.get("WAYLAND_DISPLAY")
            require(parent_display or os.environ.get("DISPLAY"),
                    "nested Niri needs an existing Wayland or X11 display")
            if parent_display:
                parent_socket = Path(parent_display)
                if not parent_socket.is_absolute():
                    parent_runtime = os.environ.get("XDG_RUNTIME_DIR")
                    require(parent_runtime, "parent Wayland display has no XDG_RUNTIME_DIR")
                    parent_socket = Path(parent_runtime) / parent_socket
                compositor_environment["WAYLAND_DISPLAY"] = str(parent_socket)
            config = self.output / "niri.kdl"
            config.write_text('hotkey-overlay { skip-at-startup; }\n'
                              'prefer-no-csd\n'
                              'layout { default-column-width { proportion 1.0; }; }\n')
            args = ["niri", "--config", config]
        else:
            compositor_environment.pop("WAYLAND_DISPLAY", None)
            compositor_environment.update(WLR_BACKENDS="headless", WLR_HEADLESS_OUTPUTS="1",
                                          WLR_LIBINPUT_NO_DEVICES="1", WLR_RENDERER="pixman")
            config = self.output / "sway.conf"
            config.write_text("output * resolution 1440x1000\n"
                              "seat seat0 fallback true\n"
                              "xwayland disable\n"
                              "default_border none\n"
                              "focus_follows_mouse no\n")
            args = ["sway", "--unsupported-gpu", "--config", config]
        self.desktop = self.processes.spawn("compositor", args, compositor_environment,
                                            self.args.timeout)

        def find_socket():
            for path in runtime.glob("wayland-*"):
                if path.is_socket():
                    return path.name
            return None

        display = self.processes.wait_for("disposable Wayland socket", find_socket,
                                          child=self.desktop)
        self.environment["XDG_RUNTIME_DIR"] = str(runtime_path)
        self.environment["WAYLAND_DISPLAY"] = display
        self.environment.pop("DISPLAY", None)
        socket_pattern, variable = (("niri*.sock", "NIRI_SOCKET")
                                    if self.args.compositor == "niri"
                                    else ("sway-ipc*.sock", "SWAYSOCK"))
        ipc = self.processes.wait_for(
            "disposable compositor IPC", lambda: next(runtime.glob(socket_pattern), None),
            child=self.desktop,
        )
        self.environment[variable] = str(runtime_path / ipc.name)
        monitors = json.loads(self.processes.run(
            "outputs", [self.binary("wispd"), "--list-outputs", "--json"], self.environment,
        ))
        require(monitors, "disposable compositor has no outputs")
        atomic_json(self.output / "outputs.json", monitors)
        self.monitor = monitors[0]["id"]

    def close(self):
        if self.runtime_alias:
            (self.runtime_alias / "runtime").unlink(missing_ok=True)
            self.runtime_alias.rmdir()

    def app(self, name, args, commands, keys=None):
        directory = self.output / name
        directory.mkdir()
        sequence = directory / "commands.json"
        atomic_json(sequence, commands + [command("finish")])
        child = self.processes.spawn(
            name, [self.binary("wisp"), "--data-dir", directory / "library",
                   *args, "--audit-commands", sequence], self.environment,
        )
        for checkpoint, key in keys or []:
            self.processes.wait_for(str(checkpoint), checkpoint.is_file, timeout=45, child=child)
            if self.args.compositor == "niri":
                windows = json.loads(self.processes.run(
                    name + "-windows-" + key, ["niri", "msg", "--json", "windows"],
                    self.environment, timeout=5,
                ))
                candidates = [window for window in windows if window.get("pid") == child["process"].pid]
                require(len(candidates) == 1, "could not identify the owned app window for keyboard input")
                self.processes.run(
                    name + "-focus-" + key,
                    ["niri", "msg", "action", "focus-window", "--id", str(candidates[0]["id"])],
                    self.environment, timeout=5,
                )
            else:
                self.processes.run(
                    name + "-focus-" + key,
                    ["swaymsg", f'[pid={child["process"].pid}]', "focus"],
                    self.environment, timeout=5,
                )
            self.processes.run(name + "-key-" + key, ["wtype", "-k", key],
                               self.environment, timeout=5)
        log = self.processes.finish(child)
        state = completion(log, name)
        self.processes.record("workflow", name=name, state=state)
        for step in commands:
            if step["command"] == "screenshot":
                self.processes.record("screenshot", path=step["path"],
                                      dimensions=verify_png(Path(step["path"])))

    def screenshots(self):
        for name, args, expected in (
            ("empty", [], 0),
            ("missing-designer", ["--create", self.output / "missing.png"], 1),
            ("missing-graph", ["--graph", self.output / "missing-project"], 1),
        ):
            path = self.output / (name + ".png")
            self.processes.run(
                name, [self.binary("wisp"), "--data-dir", self.output / (name + "-library"),
                       *args, "--screenshot", path], self.environment, expected=expected,
            )
            verify_png(path)
        paused = self.output / "paused.png"
        self.app("paused", ["--import", ROOT / "examples/graph-fused"], [
            ready(preview_ready=True), command("pause_preview"), assertion(paused=True), screenshot(paused),
        ])
        checkpoint = self.output / "combined-checkpoint.png"
        final = self.output / "combined-final.png"
        sequence = self.output / "combined-commands.json"
        atomic_json(sequence, [screenshot(checkpoint), command("finish")])
        log = self.processes.run(
            "combined-captures",
            [self.binary("wisp"), "--data-dir", self.output / "combined-library",
             "--screenshot", final, "--audit-commands", sequence],
            self.environment,
        )
        completion(log, "combined captures")
        verify_png(checkpoint)
        verify_png(final)

        failure = self.output / "terminal-error.png"
        atomic_json(sequence, [command("finish")])
        log = self.processes.run(
            "terminal-error",
            [self.binary("wisp"), "--data-dir", self.output / "terminal-error-library",
             "--create", self.output / "absent-terminal-image.png",
             "--screenshot", failure, "--audit-commands", sequence],
            self.environment, expected=1,
        )
        require("AUDIT_FAILED" in log and "AUDIT_COMPLETE" not in log,
                "terminal capture lost the startup failure verdict")
        verify_png(failure)

        atomic_json(sequence, [command("finish"), assertion(page="impossible"), command("finish")])
        log = self.processes.run(
            "early-finish",
            [self.binary("wisp"), "--data-dir", self.output / "early-finish-library",
             "--audit-commands", sequence], self.environment, expected=1,
        )
        require("exactly one finish at the end" in log and "AUDIT_COMPLETE" not in log,
                "an early finish bypassed later assertions")

        atomic_json(sequence, [command("wait_for", state={"page": "settings"}, timeout_ms=100),
                               command("finish")])
        log = self.processes.run(
            "state-wait-timeout",
            [self.binary("wisp"), "--data-dir", self.output / "state-wait-library",
             "--audit-commands", sequence], self.environment, expected=1,
        )
        require("AUDIT_FAILED" in log and "AUDIT_COMPLETE" not in log,
                "state wait timeout did not retain its failure verdict")

    def library(self):
        package = self.output / "library-export.wisp"
        self.app("library", ["--import", ROOT / "examples/graph-blur"], [
            ready(page="library", preview_ready=True, parameter_errors=0),
            command("library_float", id="brightness", value=1.5),
            command("library_parameter", id="tint", value="[0.4,0.8,1,1]"),
            assertion(parameter_errors=0),
            screenshot(self.output / "library-valid.png"),
            command("library_parameter", id="tint", value="[bad"),
            assertion(parameter_errors=1, preview_ready=True),
            screenshot(self.output / "library-invalid.png"),
            command("library_reset"), assertion(parameter_errors=0),
            screenshot(self.output / "library-reset.png"),
            command("library_export", path=str(package)),
            assertion(status_contains="Wallpaper exported"),
            command("library_export", path=str(package)),
            assertion(status_contains="could not commit application data"),
        ])
        self.processes.run("library-export-check", [self.binary("wisp-preview"), "--package",
                                                    package, "--check"], self.environment)
        normal = self.output / "keyboard-normal.png"
        focused = self.output / "keyboard-focus.png"
        restored = self.output / "keyboard-restored.png"
        self.app("keyboard", ["--graph", ROOT / "examples/graph-fused"], [
            ready(page="graph", focus=False, preview_ready=True), screenshot(normal), wait(6000),
            assertion(focus=True), screenshot(focused), wait(6000),
            assertion(focus=False), screenshot(restored),
        ], keys=[(normal, "F11"), (focused, "Escape")])

    def designer(self):
        project = self.output / "designer-project"
        package = self.output / "designer.wisp"
        self.app("designer", ["--create", ROOT / "examples/shader-basic/background.png"], [
            ready(page="designer", designer_valid=True, preview_ready=True),
            command("designer_name", value="Native audit design"),
            command("designer_author", value="Wisp audit"),
            command("designer_panel", value="effects"),
            command("designer_effect", effect="pulse", enabled=True),
            command("designer_value", id="brightness", value=1.4),
            ready(designer_dirty=True, designer_name="Native audit design", preview_ready=True),
            screenshot(self.output / "designer-edit.png"),
            command("designer_save", path=str(project)), assertion(designer_dirty=False),
            command("designer_open", path=str(project)),
            ready(designer_name="Native audit design", designer_valid=True, preview_ready=True),
            command("designer_open", path=str(self.output / "absent-image.png")),
            assertion(designer_name="Native audit design", designer_valid=True,
                      preview_ready=True, status_contains="Could not open design"),
            screenshot(self.output / "designer-invalid-open.png"),
            command("designer_export", path=str(package)),
        ])
        self.validate("designer", project, package)

    def layers(self):
        project = self.output / "layers-project"
        package = self.output / "layers.wisp"
        source = ROOT / "examples/layers-effects/assets/image1.png"
        mask = ROOT / "examples/layers-effects/assets/image2.png"
        self.app("layers", ["--create", ROOT / "examples/shader-basic/background.png"], [
            ready(preview_ready=True), command("layer_enable"), command("layer_select", index=0),
            command("layer_import", path=str(source), kind="layer"),
            command("layer_select", index=1),
            command("layer_import", path=str(mask), kind="mask", index=1),
            command("layer_import", path=str(mask), kind="displacement", index=1),
            command("layer_mask_flag", flag="composition", value=True),
            command("layer_mask_flag", flag="effect", value=True),
            command("layer_value", id="opacity", value=0.8),
            command("layer_expand", value=True),
            command("layer_paint", **{"from": [0.2, 0.2], "to": [0.8, 0.7]}),
            command("layer_stroke_end"), wait(2500),
            screenshot(self.output / "layers-painted.png"),
            command("layer_erase", value=True),
            command("layer_paint", **{"from": [0.3, 0.3], "to": [0.6, 0.5]}),
            command("layer_stroke_end"), wait(2500),
            screenshot(self.output / "layers-erased.png"),
            command("layer_erase", value=False), command("layer_expand", value=False),
            command("designer_save", path=str(project)),
            assertion(designer_valid=True, designer_dirty=False, paint_dirty=False),
            command("designer_open", path=str(project)),
            ready(designer_valid=True, preview_ready=True),
            command("designer_export", path=str(package)),
        ])
        self.validate("layers", project, package)
        layers = json.loads((project / "layers.json").read_text())["layers"]
        require(len(layers) == 2 and layers[1]["mask"] and layers[1]["displacement"],
                "saved layer project lost imported image/mask/displacement")

    def graph(self):
        project = self.output / "graph-project"
        package = self.output / "graph.wisp"
        self.app("graph", ["--graph", ROOT / "examples/graph-fused"], [
            command("graph_new"), ready(graph_valid=True, graph_nodes=2, preview_ready=True),
            command("graph_select", id="color"),
            command("graph_field_edit", id="value", value="[0.2,"),
            command("graph_field", id="value", value="[0.2,"),
            assertion(graph_valid=True, preview_ready=True, status_contains="Edit rejected"),
            screenshot(self.output / "graph-invalid-field.png"),
            command("graph_field", id="value", value="[0.2,0.7,0.9,1]"),
            ready(graph_valid=True, graph_diagnostic="", graph_dirty=True, preview_ready=True),
            command("graph_wire", id="output"),
            command("graph_socket", id="output", input="color"),
            assertion(graph_valid=True, status_contains="Edit rejected"),
            screenshot(self.output / "graph-invalid-wire.png"),
            command("graph_wire", id="color"),
            command("graph_socket", id="output", input="color"), ready(preview_ready=True),
            command("graph_move", id="color", position=[150, 230]),
            command("graph_save", path=str(project)), assertion(graph_dirty=False),
            command("graph_open", path=str(project)),
            ready(graph_valid=True, preview_ready=True, graph_nodes=2, graph_diagnostic=""),
            screenshot(self.output / "graph-reopened.png"),
            command("graph_export", path=str(package)),
        ])
        self.validate("graph", project, package)
        layout = json.loads((project / "layout.json").read_text())
        require(layout["positions"]["color"] == [150.0, 230.0], "graph move was not saved")

    def validate(self, name, project, package):
        self.processes.run(name + "-project-check", [self.binary("wispc"), project, "--check"],
                           self.environment)
        self.processes.run(name + "-package-check", [self.binary("wisp-preview"), "--package",
                                                   package, "--check"], self.environment)

    def daemon(self):
        directory = self.output / "daemon"
        directory.mkdir()
        config_path = directory / "assignments.json"
        status_path = directory / "status.json"
        stop_path = directory / "stop"
        package = self.output / "graph.wisp"
        config = {"outputs": {self.monitor: {"package": str(package), "fps": 30}},
                  "audio": {"enabled": False, "gain": 1.0}}
        atomic_json(config_path, config)
        child = self.processes.spawn(
            "daemon", [self.binary("wispd"), "--config", config_path,
                       "--status-file", status_path, "--stop-file", stop_path,
                       "--lock-file", directory / "lock"], self.environment,
            timeout=120,
        )

        def status_when(label, predicate):
            def read():
                try:
                    status = json.loads(status_path.read_text())
                except (FileNotFoundError, json.JSONDecodeError):
                    return None
                require(status.get("schema_version") == 1, "unsupported daemon status schema")
                require(status.get("pid") == child["process"].pid, "daemon status PID mismatch")
                return status if predicate(status) else None
            status = self.processes.wait_for(label, read, child=child)
            atomic_json(directory / (label + ".json"), status)
            return status

        assigned = status_when("assigned", lambda state: state["ready"] and state["outputs"]
                               and state["outputs"][0]["presented"] >= 2)
        output = assigned["outputs"][0]
        require(output["id"] == self.monitor and output["initialized"],
                "daemon did not initialize the assigned disposable output")
        identity = output["instance_id"]
        invalid = copy.deepcopy(config)
        invalid["outputs"][self.monitor]["package"] = str(directory / "missing.wisp")
        atomic_json(config_path, invalid)
        rejected = status_when("rejected", lambda state:
                               state["attempted_generation"] > assigned["attempted_generation"]
                               and state["rejection"])
        require(rejected["accepted_generation"] == assigned["accepted_generation"]
                and rejected["config"] == assigned["config"]
                and rejected["outputs"][0]["instance_id"] == identity
                and rejected["outputs"][0]["presented"] >= output["presented"],
                "rejected reload changed the accepted configuration or renderer")
        atomic_json(config_path, config)
        recovered = status_when("recovered", lambda state: state["ready"]
                                and state["accepted_generation"] > rejected["attempted_generation"]
                                and state["rejection"] is None)
        require(recovered["outputs"][0]["instance_id"] == identity,
                "recovery replaced the unchanged renderer")
        config["audio"]["gain"] = 0.5
        atomic_json(config_path, config)
        audio = status_when("audio-only", lambda state: state["ready"]
                            and state["accepted_generation"] > recovered["accepted_generation"])
        require(audio["outputs"][0]["instance_id"] == identity
                and audio["config"]["audio"]["gain"] == 0.5
                and audio["outputs"][0]["presented"] >= recovered["outputs"][0]["presented"],
                "audio-only reload failed to preserve renderer state")
        atomic_json(config_path, {"outputs": {}})
        cleared = status_when("cleared", lambda state: state["ready"] and not state["outputs"]
                              and state["accepted_generation"] > audio["accepted_generation"])
        atomic_json(config_path, config)
        reassigned = status_when("reassigned", lambda state: state["ready"] and state["outputs"]
                                 and state["accepted_generation"] > cleared["accepted_generation"]
                                 and state["outputs"][0]["presented"] >= 2)
        require(reassigned["outputs"][0]["instance_id"] != identity,
                "cleared renderer identity survived reassignment")
        self.processes.run("daemon-desktop", ["grim", self.output / "daemon-desktop.png"],
                           self.environment)
        verify_png(self.output / "daemon-desktop.png")
        stop_path.touch()
        self.processes.finish(child)
        stopped = json.loads(status_path.read_text())
        require(not stopped["running"] and not stopped["ready"] and not stop_path.exists(),
                "graceful daemon shutdown did not publish stopped state/remove stop file")
        atomic_json(directory / "stopped.json", stopped)
        self.processes.record("workflow", name="daemon", state=stopped)

    def daemon_integration(self):
        args = ["cargo", "test", "--locked", "-p", "wisp", "--test", "daemon",
                "--target-dir", self.args.bin_dir.parent]
        if self.args.bin_dir.name == "release":
            args.append("--release")
        args.extend(["--", "--ignored"])
        self.processes.run("daemon-integration", args, self.environment, timeout=180)


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, epilog=(
        "Run from an interactive desktop with --compositor niri, or use headless Sway in CI. "
        "Fresh state, screenshots, command files and JSON results live in a new run directory. "
        "No existing daemon, desktop assignment or user config is controlled."
    ))
    parser.add_argument("--compositor", required=True, choices=("niri", "sway"),
                        help="niri: nested desktop; sway: wlroots headless desktop")
    parser.add_argument("--output-dir", type=Path, required=True,
                        help="artifact parent; each invocation creates a fresh run-* child")
    parser.add_argument("--bin-dir", type=Path, default=ROOT / "target/debug",
                        help="directory containing prebuilt workspace binaries (default: target/debug)")
    parser.add_argument("--build", action="store_true", help="build workspace before the audit")
    parser.add_argument("--integration-tests", action="store_true",
                        help="run the Rust owned-daemon integration test inside the disposable desktop")
    parser.add_argument("--timeout", type=int, default=600,
                        help="overall deadline in seconds, including optional build (default: 600)")
    args = parser.parse_args(argv)
    if args.timeout < 30:
        parser.error("--timeout must be at least 30 seconds")
    args.bin_dir = args.bin_dir.resolve()
    if (args.build or args.integration_tests) and args.bin_dir.name not in ("debug", "release"):
        parser.error("--build/--integration-tests require --bin-dir to end in debug or release")
    return args


def main(argv=None):
    args = parse_args(argv)
    require(sys.platform.startswith("linux"), "this disposable compositor runner requires Linux")
    args.output_dir.mkdir(parents=True, exist_ok=True)
    output = Path(tempfile.mkdtemp(prefix="run-" + time.strftime("%Y%m%dT%H%M%S-") ,
                                  dir=args.output_dir.resolve()))
    print(f"Audit artifacts: {output}", flush=True)
    processes = Processes(output, args.timeout)
    audit = None
    result = {"schema_version": 1, "passed": False, "compositor": args.compositor,
              "artifacts": str(output)}
    started = time.monotonic()

    def interrupted(number, _frame):
        raise AuditFailure(f"interrupted by signal {number}")

    previous_handlers = {number: signal.signal(number, interrupted)
                         for number in (signal.SIGTERM, signal.SIGINT)}
    try:
        if args.build:
            build_args = ["cargo", "build", "--locked", "--workspace", "--target-dir", args.bin_dir.parent]
            if args.bin_dir.name == "release":
                build_args.append("--release")
            processes.run("build", build_args, os.environ.copy(), timeout=args.timeout)
        audit = Audit(args, output, processes)
        for name in audit.binaries:
            audit.binary(name)
        audit.start_desktop()
        for name in ("screenshots", "library", "designer", "layers", "graph", "daemon"):
            print(f"Running {name}…", flush=True)
            processes.alive(audit.desktop)
            getattr(audit, name)()
        if args.integration_tests:
            print("Running owned-daemon integration…", flush=True)
            processes.alive(audit.desktop)
            audit.daemon_integration()
        result["passed"] = True
    except (AuditFailure, OSError, ValueError, KeyError) as error:
        result["error"] = str(error)
        print(f"Audit failed: {error}", file=sys.stderr, flush=True)
    finally:
        # Ignore repeated interrupts only while reaping the groups this run owns.
        for number in previous_handlers:
            signal.signal(number, signal.SIG_IGN)
        try:
            try:
                processes.cleanup()
            finally:
                if audit:
                    audit.close()
            processes.record("cleanup_complete")
        except (OSError, subprocess.TimeoutExpired) as error:
            result["passed"] = False
            result["cleanup_error"] = str(error)
            print(f"Audit cleanup failed: {error}", file=sys.stderr, flush=True)
        finally:
            result["elapsed_seconds"] = round(time.monotonic() - started, 3)
            atomic_json(output / "result.json", result)
            for number, handler in previous_handlers.items():
                signal.signal(number, handler)
    print(json.dumps(result), flush=True)
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())

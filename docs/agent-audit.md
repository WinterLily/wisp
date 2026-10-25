# Native agent audit

Run the real application and independent wallpaper daemon inside an owned,
disposable Linux desktop:

```sh
cargo build --workspace
python3 tools/audit.py --compositor niri --integration-tests --output-dir agents/native-audit
```

`niri` starts a nested compositor on the caller's Wayland or X11 display. It does
not use `--session`, import environment into systemd, or control the caller's
compositor. Keep the nested window visible during keyboard verification. Python
3.9+, Niri, `wtype`, `grim`, the normal native build dependencies, and a working
Vulkan driver are required. Mesa software Vulkan is sufficient. The runner
defaults to `WGPU_BACKEND=vulkan`; set it explicitly to test another supported
backend. The native renderer still needs a GPU or software implementation.

Headless CI uses Sway with the wlroots headless backend and Pixman compositor:

```sh
python3 tools/audit.py --compositor sway --integration-tests --output-dir agents/ci-audit
```

Install `sway`, `grim`, `wtype`, Mesa Vulkan/EGL drivers and the project's native
build dependencies on Linux. `xvfb-run -a python3 tools/audit.py --compositor sway
--output-dir agents/ci-audit` can be used on environments that require an X11
display during graphics initialization; Sway itself still uses its disposable
headless backend. Do not run Sway as root. No display manager, user's Sway
configuration, or physical monitor is needed.

Pass `--build` to build first, `--bin-dir target/release` to use other binaries,
or `--timeout 900` to raise the overall deadline, including an optional build.
`--build --bin-dir target/release` builds the release profile. Without `--build`,
the runner uses the four existing binaries. Cargo is invoked only with `--build`
or `--integration-tests`.

Each invocation creates a fresh `run-*` directory below `--output-dir`. It
contains private XDG directories, per-workflow libraries, compositor config,
scripts, projects, packages, logs, PNG captures, `events.jsonl`, daemon status
checkpoints, and `result.json`. Existing output is never reused. The final JSON
and process exit code agree on success. Upload the whole run directory on CI
failure. `agents/` is ignored by Git.

All child commands use argument arrays and run in owned process groups. Every
command has a deadline; a 600-second overall deadline bounds the default run.
The runner reaps its app, daemon and compositor groups on success, assertion
failure, timeout, SIGINT or SIGTERM. A temporary short socket alias under `/tmp`
points into the run's private runtime directory and is removed during teardown.
As with any userspace runner, SIGKILL or a machine crash bypasses cleanup.
Exited leaders remain unreaped until group cleanup, reserving their PID/group
identities even when descendants outlive them. HOME is inherited; Wisp uses
explicit per-workflow data directories and private XDG configuration/state.
Existing daemons and desktop assignments are never controlled.

## Workflows and verdicts

The scripts feed `--audit-commands FILE` through the same UI update handlers as
interactive actions. They wait for asynchronous work, assert state and await
screenshots. A zero app exit code alone is insufficient: every script must emit
one `AUDIT_COMPLETE` JSON verdict and no `AUDIT_FAILED`. The runner exercises:

- Library float and color edits, invalid color input, error display and Reset.
- Native `wtype` F11 then Escape, with focus assertions and screenshots before,
  during and after focus mode.
- Designer name/author/effect/value edits, save, reopen and export. An invalid
  open must preserve the prior valid design and rendered preview.
- Layer source image, mask and displacement imports, opacity/mask edits,
  expanded painting and erasing, save/reopen and export.
- Graph invalid field and cyclic wire rejection, valid recovery, node movement,
  save/reopen and export, including persisted node position verification.
- Standalone screenshots without `--frames`, both empty and startup error
  screens. Startup errors must return exit code 1 and still write a PNG. A
  paused preview is also captured through the command queue.
- CPU validation of all saved projects with `wispc --check` and exported
  packages with `wisp-preview --check`.
- An independent owned `wispd` with atomic config reloads: initial presentation,
  missing-package rejection preserving accepted config/generation/output
  identity, recovery, audio-only gain reload preserving renderer identity,
  clear/reassign with a fresh renderer, and graceful stop. Versioned status
  snapshots verify initialized rendering and presentation; opening a window or
  detecting a running PID does not count as readiness. Audio remains disabled
  for this deterministic reload test.

Review the screenshots as visual evidence in addition to checking the structured
verdicts. This runner checks native workflows, not pixel-perfect UI appearance,
physical monitor hotplug, sustained performance, live audio capture, or actual
operating-system file-picker interaction. Paths supplied by the command queue
enter the same open/import/save/export implementations as dialog results.

For a focused run, pass `--audit-commands /absolute/commands.json` to `wisp`.
The JSON array uses snake_case command tags, for example:

```json
[
  {"command":"page","value":"graph"},
  {"command":"graph_new"},
  {"command":"assert","state":{"page":"graph","graph_valid":true}},
  {"command":"finish"}
]
```

Commands wait for pending compilation and captures. Scripts must contain exactly
one final `finish`; failed assertions emit `AUDIT_FAILED` and exit nonzero.
The runner's retained command files show edit, path, recovery and capture examples.
Standalone `--screenshot /absolute/capture.png` also works without `--frames`;
`--screenshot-delay-ms` defaults to 1000. Checkpoint captures can inspect recoverable
errors; a final capture retains unresolved startup/compilation failure verdicts.
Frame-limited success requires the current preview generation to render without
GPU errors, including when an older renderer remains visible after a rejected edit.

The focused harness failure tests need no compositor:

```sh
python3 -m unittest discover -s tools -p 'test_audit.py' -v
```

Pass `--integration-tests` to run the independent Rust daemon integration test
inside the runner's disposable desktop, using the same target directory and
profile as `--bin-dir`. CI enables this option. The test also needs built `wispd`;
do not invoke it against an everyday desktop:

```sh
cargo test -p wisp --test daemon -- --ignored
```

## Native Windows

The Python compositor runner is Linux-specific. Windows native output/daemon
and audio-loopback checks require a dedicated, quiet, interactive desktop runner
with Explorer running and an available audio output device. A GitHub-hosted
service session is insufficient for these desktop and loopback checks. Use the
repository's opt-in `windows_desktop` workflow dispatch on a self-hosted runner
labelled `Windows` and `wisp-desktop`; inspect the desktop evidence from that run.
Cross-compilation and CPU tests alone do not establish native Windows behavior.

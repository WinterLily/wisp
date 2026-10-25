# Phase 6: Wisp application

The `wisp` executable is an Iced 0.14 application with a local wallpaper library,
settings, selection, per-monitor assignment, import/export, parameter controls,
and an embedded live preview. The desktop daemon remains a separate process.

## Run

```sh
cargo build --workspace
cargo run -p wisp

# Import and select a package or compile an existing project.
cargo run -p wisp -- --import /path/to/wallpaper.wisp
cargo run -p wisp -- --import examples/graph-blur

# A separate test library; no window, display connection, or GPU is needed.
cargo run -p wisp -- --data-dir /tmp/wisp-library --import examples/graph-fused --check
```

Install `wisp` and `wispd` beside one another. Discovery and desktop startup invoke
that sibling executable directly, without a shell or PATH search. The GUI runs on
Wayland, X11, and Windows; desktop assignment requires Niri/Hyprland or Windows.
KDE desktop support remains Phase 10. A missing daemon or unsupported compositor
produces an error while library/import/preview remain available.

## Library and import

Use **Import** to choose a `.wisp` file or Wisp project directory, or enter a path.
Project import calls the Phase 5 compiler. Package, resource, image, shader, and
runtime-limit validation completes before installation. Loading/compiling imports,
loading selections, discovery, and assignment validation run on background workers.
GPU pipeline preparation occurs once per selection on the Iced rendering thread.

The library displays a thumbnail, name, author, source, tags, pass count, estimated
resource memory, and an installed content revision. Search matches names, authors,
and tags. Memory estimates exclude pipelines, driver overhead, and preview targets;
they are not measured GPU timings. Packages without previews still have selectable
text cards and a live preview.

Identical imports reuse one library entry. Different package contents, even with
the same manifest ID, are separate installed revisions. Originals are untouched.
**Export .wisp** exports the selected installed package to a new file. Per-monitor
parameter overrides are saved in assignments; they do not rewrite exported assets.
Image/Easy Mode creation is Phase 7, and WGSL editing/import workflows are later
authoring work. This phase imports compiled packages and existing graph projects.

## Live preview

The preview uses `wisp-runtime::Renderer` on Iced's own wgpu device. It executes the
same fragment/compute passes into an sRGB GPU texture. A small Iced shader widget
composites that texture within the panel, including clipping and display scaling.
There is no CPU pixel readback in the live loop and no second wallpaper renderer.
Texture/pipeline allocations are reused until selection or panel size changes.
Preview targets are capped at 4096 pixels per dimension and device limits.

Time, frame, resolution, and normalized pointer input work in the panel. Scalar,
boolean, color, and vector controls update the preview's existing parameter buffers.
Invalid values retain the previous valid value and disable Apply until corrected.
Reset restores the installed package's defaults, including after loading saved
monitor overrides. Invalid replacement shaders/resources preserve the prior preview.

The preview defaults to 30 FPS, configurable from 1–60 in persisted settings (common
rates appear in the UI). Pause keeps the last rendered texture. Leaving Library or
unfocusing the window stops animation ticks; resuming excludes time spent paused.
Parameter/size changes can redraw the paused preview. Audio is supplied by the [Phase 9 service](phase-9.md) when enabled.

Iced 0.14 hardcodes a two-bind-group device limit, while Wisp's compute ABI uses
four. [The vendored Iced dependency](../vendor/iced_wgpu/README.md) contains one source
change requesting four groups. Both wgpu default and downlevel limits support four.
No Wisp package ABI or shader logic changes. The runtime's new host-device constructor
retains resource/device checks and validation scopes; desktop constructors are intact.

## Assignments and daemon lifetime

Choose **All monitors** or a discovered monitor, set controls and FPS, then **Apply
wallpaper**. All monitors sets a default for current/future outputs and clears
individual overrides. A monitor assignment overrides that default. **Clear** on one
monitor disables its wallpaper even when a default exists; Clear on All monitors
removes all assignments. Selecting a monitor loads its saved wallpaper controls
when that package is in the library. Refresh discovers hotplug changes; saved
assignments for disconnected outputs survive.

The app atomically saves the shared `wisp-host::Config`, then starts `wispd` if no
managed daemon holds this library's lock. An existing daemon reloads config edits
once per second and retains prior renderers if preparation fails. Applying updates
restarts the wallpaper's animation/simulation, following existing host behavior.
Closing the app leaves the daemon running. Settings also offers explicit startup.
Clear unloads wallpapers but leaves the daemon available for future assignments.
Session autostart is not installed automatically.

This phase uses a file-based control channel, plus `wispd --list-outputs --json` for
structured discovery. Socket command IPC, renderer acknowledgements/performance
telemetry, pause/resume desktop policy, and crash supervision remain future work.
The UI distinguishes **saved** assignments from daemon process status; running does
not certify that a specific GPU replacement succeeded. Detailed errors are written
to `daemon.log`, and daemon exit is reported by the UI's status polling. Monitor
sizes are logical on Wayland and physical on Windows, with a separate scale field.

New daemon options:

```sh
wispd --list-outputs --json
wispd --config /path/assignments.json --lock-file /path/daemon.lock
```

`--lock-file` holds an OS advisory lock for the process lifetime. Both CLI and app
must use the same lock path to share a managed instance. Independently launched
unmanaged daemons are not detected. One GUI holds `app.lock` per library. Lock files
may remain on disk after exit; OS locks, not file existence or cached PIDs, determine
ownership. Windows uses the same mechanism and a console-free child process.

The backward-compatible config extension is:

```json
{
  "default": {"package": "/absolute/path/wallpaper.wisp", "fps": 60},
  "outputs": {},
  "disabled_outputs": ["DP-2"]
}
```

Disabled names cannot also have explicit assignments. Windows checks both stable
interface IDs and GDI aliases. Existing configs without this field remain valid.

## Persistence

The default root is the platform local-data directory (`$XDG_DATA_HOME/wisp` or
`~/.local/share/wisp` on Linux; local AppData on Windows). `--data-dir` overrides it.

```text
library.json          Version 1 catalog, source and import timestamp
packages/<sha256>.wisp Immutable validated portable packages
settings.json         Version 1 FPS and appearance settings
assignments.json      Shared daemon configuration, absolute installed paths
app.lock              GUI ownership lock
daemon.lock           Managed daemon ownership lock
daemon.log            Current managed daemon diagnostics
```

Catalog/config/settings writes use temporary files in the destination directory,
flush contents, and atomically replace the destination. Imports validate and write
the package before publishing a catalog entry; interrupted imports can leave an
unreferenced package file. Catalogs are bounded to 1 MiB and 4096 entries. Package
reads are bounded and hash checked. Changed/missing/corrupt installed objects are
reported without hiding healthy entries. Invalid saved settings fail explicitly
instead of being overwritten with defaults. Exports refuse existing destinations.
There is no library removal or orphan-collection UI in this phase.

## Verification

The [native agent audit](agent-audit.md) runs editing, recovery, keyboard and
independent-daemon workflows inside a disposable Niri or headless Sway desktop,
with bounded teardown and retained evidence.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings

WGPU_BACKEND=vulkan cargo test -p wisp-app --lib -- --ignored
WGPU_BACKEND=gl cargo test -p wisp-app --lib -- --ignored

# App window, real runtime, clean exit, and optional app-only screenshot.
cargo run -p wisp -- --data-dir /tmp/wisp-ui --import examples/graph-blur --frames 12 --screenshot /tmp/wisp-ui.png

# Disposable desktop only: starts an owned daemon and applies a wallpaper there.
cargo build --workspace
cargo test -p wisp --test daemon -- --ignored
```

CPU tests exercise durable imports/export/reload, duplicates, malformed imports,
changed installed objects, assignment precedence and per-monitor controls,
transactional rejection, settings bounds, locks, typed edits, default reset,
failed selection preservation, and CPU-only CLI operation. Two GPU tests compare
Iced preview pixels to the standalone runtime (tolerance 1/255), check panel bounds,
scrolled clipping, paused texture reuse, live parameters, multipass shaders, and
compute/storage bindings. The ignored desktop test checks structured IDs, managed
start idempotence, config reload, rendering, exit, and lock release.

Local Linux verification includes Vulkan/OpenGL GPU suites and Iced window smoke
tests for blur and compute packages, an inspected application screenshot, and the
desktop integration test under nested Niri. An X11/Xwayland application smoke test
also rendered twelve frames and exited cleanly. Windows all-target Clippy passes, but
native Windows application/dialog/desktop execution, DirectX 12 parity, physical
monitor soak tests, and Rust 1.88 execution remain unverified. CI coverage is added;
no remote CI run was initiated.

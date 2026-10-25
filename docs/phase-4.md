# Windows desktop host

Phase 4 adds `wisp-host-windows` and enables `wispd` on Windows. The host owns
monitor discovery, desktop attachment, presentation, and scheduling. It uses the
same `wisp-runtime`, package loader, shaders, parameters, and fragment/compute
passes as Linux. Shared assignment parsing lives in `wisp-host`.

The implementation has passed Windows GNU cross-compilation and Clippy on Linux.
Native Windows execution, DirectX 12 rendering, and Linux/Windows pixel parity
have **not** been verified locally. The automated and manual checks below are
provided to close those verification gaps.

## Build and run

Use Windows 10 version 1703 or newer / Windows 11, an interactive Explorer desktop,
a working wgpu-compatible graphics driver, and Rust 1.88 or newer with the Visual
Studio C++ build tools. Run one daemon per session, under the normal desktop user.

PowerShell, from the repository root:

```powershell
cargo build --release --workspace
cargo run -- --export wallpaper.wisp
cargo run -p wispd -- --list-outputs
cargo run -p wispd -- --package wallpaper.wisp --check
cargo run --release -p wispd -- --package wallpaper.wisp --fps 30
# Stop with Ctrl+C.

# Use the exact GDI names printed by --list-outputs.
cargo run --release -p wispd -- --assign '\\.\DISPLAY1=wallpaper.wisp' --assign '\\.\DISPLAY2=blur.wisp'
cargo run --release -p wispd -- --config assignments.json

# Short desktop presentation check.
$env:WGPU_BACKEND = 'dx12'
cargo run -p wispd -- --package wallpaper.wisp --frames 60
```

`--check` needs neither a GPU nor Explorer. `--list-outputs` enumerates displays
without initializing the GPU or requesting a wallpaper WorkerW. `--frames N`
exits once every currently active assigned monitor has presented at least N frames;
unmatched assignments wait for their monitor. Use an external timeout for
unattended tests. No autostart entry or system wallpaper setting is installed.

## Monitor assignment

`--list-outputs` prints each monitor's GDI name, description, physical pixel bounds,
primary flag, and monitor device interface ID. Assignments match the interface ID
first, then the GDI name, then `default`. Prefer the interface ID for persistence;
GDI names such as `\\.\DISPLAY1` can change after reconnecting hardware. Driver or
port changes can also change interface IDs. When Windows supplies no interface ID,
the GDI name is used as the fallback.

The JSON schema is shared with the [Wayland host](phase-3.md). Backslashes must be
escaped in JSON (PowerShell single-quoted CLI arguments do not need that escaping):

```json
{
  "default": {"package": "wallpapers/compute.wisp", "fps": 30},
  "outputs": {
    "\\\\.\\DISPLAY2": {
      "package": "wallpapers/compute.wisp",
      "fps": 60,
      "parameters": {"speed": {"type": "float", "value": 0.5}}
    }
  }
}
```

Package paths resolve relative to the config file. To use an interface ID, copy
its printed value and JSON-escape its backslashes, or generate the JSON with
PowerShell `ConvertTo-Json`. FPS accepts 1–240. Limits are 1 MiB of JSON, 64 explicit
assignments, and 16 active outputs. `{}` is valid and unloads all Wisp surfaces.

Config modification times and monitor topology are polled once per second.
Replacement windows stay hidden while all changed packages and GPU resources are
prepared. A failed preparation retains the previous visible assignments. Removing
an assignment falls back to `default`, or destroys that monitor's Wisp window.
Package-file edits alone do not trigger reload; save the config too. Successful
reloads restart animation and simulation state. Replacement preparation temporarily
holds both sets of resources.

Disconnects release output resources; reconnects restore assignments. Changes to
monitor position, resolution, rotation, and effective DPI recreate the affected
surface. Signed virtual-desktop coordinates support monitors left of or above the
primary. Each output uses a physical-size buffer, its own parameters and persistent
simulation state, and the shared GPU device. Multi-adapter presentation depends on
wgpu/driver support; an unsupported surface produces a diagnostic.

## Desktop attachment and rendering

The host requests Explorer's wallpaper layer with a bounded `0x052c` message.
For the traditional layout it creates child windows under the WorkerW behind the
icons. For the raised desktop layout it creates opaque layered children of Progman
and places them below `SHELLDLL_DefView`. This is an undocumented Explorer
convention and can change with Windows updates; unsupported layouts fail with a
diagnostic. The two layouts and raised-desktop requirements are described in
[Lively's desktop integration](https://github.com/rocksdanister/lively/blob/core-separation/src/Lively/Lively/Core/WinDesktopCore.cs).

Window creation uses the parent's DPI context, converting physical monitor bounds
into parent client coordinates. This avoids mismatched parent/child DPI awareness;
Microsoft documents the risks in [SetParent](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setparent).
Monitor rectangles come from [EnumDisplayMonitors](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-enumdisplaymonitors).

The windows are disabled, do not activate, are omitted from the taskbar, and return
transparent hit tests. Desktop icons and mouse interaction remain with Explorer.
Cursor position is sampled in physical desktop coordinates and normalized per
monitor, clamped to its edges. The host updates time, delta time, frame, resolution,
and parameters; audio is supplied by the [Phase 9 service](phase-9.md) when enabled.

Each output uses FIFO presentation and its FPS cap. Surface loss/outdated errors
trigger reconfiguration; timeouts retry on the next scheduled frame. GPU/device
failure exits with a diagnostic. A slow surface can delay other outputs because
they share a message/render thread. Explicit fullscreen, idle, session-lock, and
display-sleep scheduling policies remain future work.

Explorer/window loss clears stale surfaces and retries desktop discovery. The host
recreates wallpapers when Explorer returns. Ctrl+C and window close release GPU
surfaces before destroying owned child windows. An ownership token prevents a
recycled native handle from being destroyed accidentally. Explorer windows and
system wallpaper preferences are never destroyed or rewritten. Shutdown/logoff
messages are handled when delivered, but Windows may forcibly terminate a console
process. There is no device-loss restart supervisor or monitor spanning yet.

## Verification

CPU checks and cross-compilation (Linux):

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
rustup target add x86_64-pc-windows-gnu
cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings
```

Native Windows checks (not run in this implementation session):

```powershell
$env:WGPU_BACKEND = 'dx12'
cargo test -p wisp-runtime --test gpu -- --ignored
cargo test -p wisp-host-windows -- --ignored
cargo run -- --package wallpaper.wisp --frames 60
cargo run -p wispd -- --package wallpaper.wisp --frames 60
```

The ignored host test checks each monitor's native child bounds, parent, input and
activation styles, and cleanup while preserving Explorer. It requires an interactive
desktop. It does not prove visible GPU presentation. Manually verify on Windows:

1. Wallpaper behind icons, normal desktop clicks/context menus, and no focus theft.
2. Two monitors with distinct packages/parameters, mixed DPI, a negative origin,
   portrait rotation, resolution/scale changes, unplug/replug, and primary changes.
3. Invalid reload retaining both wallpapers; valid reload, unload, and reassignment.
4. Explorer restart restoring surfaces; Ctrl+C releasing them; sleep/wake and lock/unlock.
5. Traditional and raised Explorer desktop layouts, including Show Desktop.

CI now builds one set of version 1–4 `.wisp` fixtures, transfers those exact files
to Linux Vulkan and Windows DirectX 12 jobs, validates them with `wispd --check`,
and renders deterministic 321 × 181 snapshots at time 3. A comparison job allows
at most 3/255 channel error and mean error 0.25/255, including alpha. It also runs
the existing GPU suite on DirectX 12. CI has been configured but has not been run
remotely; hosted-runner GPU availability may require a Windows graphics runner.

Local Vulkan/OpenGL snapshots can be compared with the same tool:

```sh
cargo run -p wisp-runtime --example compare-snapshots -- linux.png windows.png
```

Local tests are recorded in [implementation progress](../wisp-implementation-progress.md).
Physical desktop testing and Windows parity remain required before declaring the
Phase 4 cross-platform success criterion verified.

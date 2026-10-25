# Niri and Hyprland desktop host

`wispd` runs independently of the preview application. The Linux-only
`wisp-host-wayland` crate owns Wayland discovery, output mapping, layer-shell
surfaces, presentation, and assignment reload. All wallpaper validation, resources,
parameters, and GPU pass execution remain in `wisp-runtime`. Assignment parsing
is shared with Windows through `wisp-host`.

## Start a wallpaper

Build with `cargo build --release --workspace`. Run within a Niri or Hyprland session:

```sh
# Discover connector names; no wallpaper or GPU device is required.
target/release/wispd --list-outputs

# Assign one package to every current and future monitor.
target/release/wispd --package /absolute/path/wallpaper.wisp --fps 30

# Assign individual outputs; unspecified outputs are left without a Wisp surface.
target/release/wispd --assign DP-1=/path/first.wisp --assign HDMI-A-1=/path/second.wisp

# Validate packages and parameter values without Wayland or a GPU.
target/release/wispd --config /path/assignments.json --check
```

`--package` provides a default; explicit `--assign` entries override it. Duplicate
output names are rejected. Assignments to disconnected monitors are retained and
applied when they connect. Matching uses the compositor's output name (typically
`DP-1`, `HDMI-A-1`, etc.). A make/model/global-ID fallback is printed if no name is
available; that fallback is not guaranteed stable across reconnects or sessions.

## Persistent assignments and live reload

Create a JSON file:

```json
{
  "default": {
    "package": "wallpapers/compute.wisp",
    "fps": 30,
    "parameters": {
      "speed": {"type": "float", "value": 1.0}
    }
  },
  "outputs": {
    "DP-1": {
      "package": "wallpapers/compute.wisp",
      "fps": 60,
      "parameters": {
        "speed": {"type": "float", "value": 0.5},
        "mirror": {"type": "float", "value": 1.0}
      }
    }
  }
}
```

Run `wispd --config /path/assignments.json`. Package paths in a config are relative
to that file's directory. Parameter values use the package format's tagged types.
`fps` defaults to 60 and accepts 1–240. `parameters` and `outputs` default to empty;
`default` may be omitted or null. The config is limited to 1 MiB and 64 explicit
assignments; the host supports up to 16 active output surfaces.

The daemon checks the file's modification time once per second. An edit reloads
all assignments and prepares replacement GPU resources before swapping renderers.
Malformed JSON, missing packages, invalid parameters, or pipeline errors leave the
previous assignment set running and print a diagnostic. Saving via atomic file
replacement avoids intermediate partial JSON. Editing a package alone does not
trigger reload; save the assignment file too. Successful reloads restart wallpaper
animation and simulation state. Briefly, both old and replacement resources exist.

Remove an output entry to fall back to `default`. With no default, that monitor's
Wisp surface is destroyed. Saving `{}` unloads every running wallpaper. Re-adding
an entry creates a new surface. Per-monitor parameters and simulations are isolated;
the GPU device is shared. The Phase 6 Iced application provides graphical assignment through this config; socket IPC remains future work.

## Desktop presentation

Each output gets a `wlr-layer-shell` **background** surface with all four anchors,
zero requested size, exclusive zone -1, and no keyboard interaction. An empty input
region lets mouse input reach the desktop. The compositor supplies logical size;
Wisp applies its integer buffer scale and validates physical dimensions against the
GPU limit. The compositor handles output rotation. Fractional output scales use
integer buffer scaling and compositor resampling; dedicated fractional-scale /
viewporter support is not implemented.

FIFO presentation, compositor frame callbacks, and a per-output FPS cap control
rendering. Each output may advance independently; a withheld callback stops new
frames on that output. This is not explicit session-lock, fullscreen, or DPMS policy.
Output removal and layer closure release their GPU/surface state. SIGINT/SIGTERM
release wallpaper surfaces and GPU resources before the Wayland connection closes.

Time, frame, resolution, and per-output parameters work on the desktop. Mouse
coordinates currently remain at the center: standard Wayland does not supply a
global cursor position to an input-transparent wallpaper. Interactive pointer input
continues to work in `wisp-preview`. Audio capture and FFT are available through the [Phase 9 service](phase-9.md). Monitor spanning and crash supervision are later work. UI assignment is available in Phase 6.

## Session startup

Use an absolute binary and config path. Add one of these to your compositor config
when you want Wisp to start automatically (the application does not edit it):

Niri `config.kdl`:

```kdl
spawn-at-startup "/absolute/path/wispd" "--config" "/absolute/path/assignments.json"
```

Hyprland `hyprland.conf`:

```ini
exec-once = /absolute/path/wispd --config /absolute/path/assignments.json
```

Run a single Wisp daemon for the session. Stop another wallpaper host before using
Wisp on the same outputs, so multiple background surfaces do not cover one another.

## Verification

```sh
# Must be run in the compositor session being tested.
WGPU_BACKEND=vulkan target/debug/wispd --package compute.wisp --frames 10
WGPU_BACKEND=gl target/debug/wispd --package compute.wisp --frames 10
```

`--frames N` exits after each currently assigned, connected output has presented N
frames. An unmatched assignment waits for its output; an obscured output may not
receive frame callbacks. Use an external timeout for unattended smoke tests.

Local verification used Niri 26.04 in a nested window and Hyprland with two
headless outputs. Vulkan and OpenGL both presented and exited cleanly. Hyprland
layer inspection confirmed namespace `wisp` at layer 0 with full monitor bounds.
Screenshot comparisons checked distinct per-monitor parameters, rejected config
reload preserving pixels, and valid reload changing pixels. Resize to 1280 × 720
at scale 2 produced a 640 × 360 logical layer and 1280 × 720 screenshot. Output
removal/recreation, unassignment/reassignment, and SIGTERM shutdown passed.

The installed Hyprland nested Wayland output encountered an initial-configure
protocol error in its backend; its headless outputs were used for verification.
Physical multi-monitor soak testing, fractional-scale fidelity, Windows rendering,
and cross-platform parity are still unverified.

The host follows Smithay's [layer-shell example](https://github.com/Smithay/client-toolkit/blob/v0.19.2/examples/simple_layer.rs)
and [wgpu surface integration](https://github.com/Smithay/client-toolkit/blob/v0.19.2/examples/wgpu.rs),
using the existing pinned toolkit version in this workspace.

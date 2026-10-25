# Wisp

A cross-platform GPU wallpaper compiler, runtime, and designer, built in Rust.
The architecture and roadmap are in [wisp-spec.md](wisp-spec.md).

**Implementation now extends through Phase 9.5, with native Windows verification pending.** The shared `wisp-runtime`
executes portable `.wisp` packages with multiple textures, typed parameters,
ordered fragment and compute passes, intermediate targets, and storage buffers.
`wisp-preview` renders in an ordinary window or to a PNG. `wispd` uses the
`wisp-host-wayland` adapter on Niri/Hyprland and `wisp-host-windows` on Windows,
with per-monitor assignment and automatic config reload.

`wispc` compiles typed graph projects to fused WGSL and ordered passes in portable
packages. `wisp` is the Iced library application, with imports, settings, per-monitor
assignment, parameter controls, and an embedded live preview using the same runtime.
Easy Designer creates dynamic wallpapers from images with presets, live controls,
editable projects, and portable export. Visual Graph adds a node canvas, typed wiring,
search, properties, compiler diagnostics, and editable graph projects. Audio capture/FFT,
mouse-driven emitters, persistent particle state, and fading feedback are implemented.
Layer/effect authoring and Workshop remain later milestones.
Windows host code passes cross-compilation; native desktop behavior and Windows
rendering parity remain unverified. See [implementation progress](wisp-implementation-progress.md) for
verified results and current limits.

Layers and effects are available in Easy Designer. Try
`cargo run -p wisp -- --create examples/layers-effects` for a transparent character,
painted hair mask, and rain. See [the Phase 9.5 guide](docs/phase-9.5.md) for layer
controls, masks, deformation, weather presets, and portable exports.

## Wallpaper library application

```sh
cargo build --workspace
cargo run -p wisp
cargo run -p wisp -- --import examples/graph-blur
```

Import a `.wisp` package or existing graph project, preview it, choose a monitor,
and apply it. Install `wisp` and `wispd` beside one another. Closing the app leaves
desktop wallpapers running. See [the application guide](docs/phase-6.md) for
settings, persistence, daemon management, limitations, and verification.

## Create a wallpaper

Open **Create**, choose an image, pick an effect preset, and adjust the live controls.
Save an editable project, install it in your library, or export a `.wisp` package.
Image imports have no fixed size cap. Projects preserve full-resolution originals;
preview and export prepare smaller playback copies when renderer limits require it.

```sh
cargo run -p wisp -- --create examples/shader-basic/background.png
```

See [the Easy Designer guide](docs/phase-7.md) for presets, saving and reopening
projects, export, and verification.

## Audio and interactivity

Open `examples/audio-reactive` in Visual Graph, then enable audio in Settings.
The project combines a mouse-driven particle emitter, audio response, and trails.

```sh
cargo run -p wisp -- --graph examples/audio-reactive
cargo run -p wispc -- examples/audio-reactive --output audio-reactive.wisp
cargo run -- --package audio-reactive.wisp --audio
cargo run -p wispd -- --package audio-reactive.wisp --audio
```

See [the Phase 9 guide](docs/phase-9.md) for capture sources, FFT normalization,
particle/feedback semantics, limits, snapshots, and verification. Linux builds need
ALSA headers (`libasound2-dev` on Debian/Ubuntu); capture uses PulseAudio or PipeWire's
PulseAudio compatibility server. Windows uses WASAPI loopback.

## Run

Install Rust 1.88 or newer and a working graphics driver. From the repository root:

```sh
cargo run --release
```

Move the pointer over the window to see parallax. Press Escape or close the window
to exit. The example shader and image are embedded, so the executable can run from
any directory.

```sh
# Preview your own image with the example effect.
cargo run --release -- --image /path/to/background.png --fps 30

# Load the editable example explicitly.
cargo run --release -- --shader examples/shader-basic/wallpaper.wgsl --image examples/shader-basic/background.png

# Validate without a display server or GPU.
cargo run -- --check

# Export and load a portable package (export requires a new destination).
cargo run -- --export wallpaper.wisp --name "My Wallpaper" --author "Your Name"
cargo run -- --package wallpaper.wisp --check
cargo run -- --package wallpaper.wisp

# Build and preview the two-image example with parameter overrides.
python3 examples/texture-blend/build-package.py blend.wisp
cargo run -- --package blend.wisp --param blend=0.6 --param 'tint=[0.5,1,0.8,1]'

# Build a three-pass blur with persistent intermediate targets.
python3 examples/multipass-blur/build-package.py blur.wisp
cargo run -- --package blur.wisp --param radius=4

# Render a PNG using the same runtime, without opening a window.
cargo run --release -- --output preview.png --width 1280 --height 720 --time 3

# Open a window, present ten frames, and exit.
cargo run -- --frames 10
```

Images may be PNG, JPEG, or WebP. Run `cargo run -- --help` for all options.
On Linux the window host supports Wayland and X11. It needs the corresponding
client libraries and keyboard support; Debian/Ubuntu build environments may need
`pkg-config libwayland-dev libxkbcommon-dev libx11-dev libxrandr-dev libxi-dev`.
A Vulkan driver is recommended (`mesa-vulkan-drivers` supports Mesa hardware and
software rendering). Windows uses the same renderer and shader via wgpu.
`WGPU_BACKEND=vulkan`, `gl`, or `dx12` can select a backend for diagnosis where supported.

## Desktop wallpaper on Niri and Hyprland

```sh
# Discover output names.
cargo run -p wispd -- --list-outputs

# Run one wallpaper on all current and future outputs.
cargo run --release -p wispd -- --package wallpaper.wisp

# Assign different wallpapers by output name.
cargo run --release -p wispd -- --assign DP-1=wallpaper.wisp --assign HDMI-A-1=blur.wisp

# Load persistent assignments and watch for edits.
cargo run --release -p wispd -- --config /path/to/assignments.json

# Build a procedural compute + storage-buffer example.
python3 examples/compute-gradient/build-package.py compute.wisp
cargo run -- --package compute.wisp --output compute.png --time 3
```

See [the Wayland host guide](docs/phase-3.md) for assignment JSON, startup commands,
reload behavior, scaling, testing, and desktop input limitations.

## Desktop wallpaper on Windows

```powershell
cargo run -p wispd -- --list-outputs
cargo run -p wispd -- --package wallpaper.wisp --check
cargo run --release -p wispd -- --package wallpaper.wisp --fps 30
cargo run --release -p wispd -- --config assignments.json
```

Stop with Ctrl+C. See [the Windows host guide](docs/phase-4.md) for monitor IDs,
PowerShell/JSON examples, Explorer attachment, recovery, and verification commands.

## Compile a graph project

```sh
# Type-check, lower, and validate without a GPU.
cargo run -p wispc -- examples/graph-fused --check

# Compile image/parallax/color/vignette nodes to one fragment pass.
cargo run -p wispc -- examples/graph-fused --output fused.wisp
cargo run -- --package fused.wisp --param brightness=1.3

# Compile a separable blur and downstream effects to three passes.
cargo run -p wispc -- examples/graph-blur --output graph-blur.wisp
cargo run -- --package graph-blur.wisp --param radius=4
```

See [the compiler guide](docs/phase-5.md) for the editable project schema, typed
node catalog, pass boundaries, limits, library API, and verification. Generated
packages contain runtime programs; graph nodes never execute in the daemon.

## Author a shader

Supply a WGSL file containing:

```wgsl
fn wallpaper(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(background, background_sampler, uv);
}
```

The runtime supplies entry points, globals, and the background texture bindings.
See [the Phase 1 interface](docs/phase-1.md) for coordinates, uniform layout, limits,
and the current scope. An example effect is in
[wallpaper.wgsl](examples/shader-basic/wallpaper.wgsl).

## Development

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Explicit GPU integration test (also works with a software Vulkan adapter).
cargo test -p wisp-runtime --test gpu -- --ignored
cargo test -p wisp-compiler --test gpu -- --ignored
```

The regular test suite runs without a GPU. The GPU suite verifies fullscreen
coverage, texture orientation, padded readback, time/mouse updates, multiple image
bindings, typed parameter layouts, live updates across passes, intermediate color
storage, pass dependencies, per-pass resolutions, compute dispatch, storage-buffer
sharing, and renderer isolation. CI is
configured to build and run CPU tests on Linux and Windows, with Linux GPU
tests for Vulkan and OpenGL. New CI jobs run DirectX 12 GPU tests and compare
Linux/Windows snapshots from identical version 1–4 packages and compiler examples; those jobs have not
yet been run remotely. Native Windows desktop testing remains pending.

See [the package format](docs/package-format.md) for the archive layout, program
schema, fragment/compute ABIs, versioning, and validation limits.

### Visual graph editor

Open **Graph** to create nodes, wire typed sockets, edit properties, and preview the
compiled wallpaper. Open existing projects with `cargo run -p wisp -- --graph examples/graph-blur`.
Editable drafts preserve graph layout and source assets; exported wallpapers use the
same runtime as desktop playback. See [the Phase 8 guide](docs/phase-8.md).

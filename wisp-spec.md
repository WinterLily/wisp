# Wisp

## Cross-Platform GPU Wallpaper Compiler, Runtime, and Designer

## 1. Overview

**Wisp** is a cross-platform GPU-accelerated dynamic wallpaper application for:

* Windows
* KDE Plasma
* Niri
* Hyprland

Wisp provides:

* A lightweight GPU-based wallpaper runtime.
* A visual wallpaper designer.
* Direct WGSL shader authoring for advanced users.
* Steam Workshop browsing, installation, subscription, and publishing.
* Import/export of portable Wisp wallpaper packages.
* Identical wallpaper packages across supported operating systems and desktop environments.
* Strong rendering parity between Windows and Linux.

Wisp is deliberately **not** a miniature game engine.

Wallpapers are authored as visual effect graphs or shaders, compiled ahead of time into an optimized GPU program, and executed by a minimal runtime.

The central design principle is:

> The authoring representation never executes at runtime.

The Wisp editor produces a graph. The Wisp compiler converts that graph into the smallest practical set of WGSL shaders and GPU passes. The Wisp runtime only executes the compiled program.

---

# 2. Goals

## 2.1 Primary Goals

Wisp MUST:

1. Run the same wallpaper package on Windows, KDE Plasma, Niri, and Hyprland.
2. Use `wgpu` as the common graphics abstraction.
3. Use WGSL as the canonical shader language.
4. Provide visually identical or near-identical rendering across supported platforms.
5. Support GPU-rendered dynamic wallpapers without requiring a general-purpose game engine.
6. Allow non-programmers to create wallpapers.
7. Allow advanced users to directly author GPU shaders.
8. Integrate with Steam Workshop.
9. Allow wallpapers to be imported without Steam.
10. Remain lightweight enough to run continuously.
11. Support multiple monitors.
12. Allow wallpapers to react to:

    * Time
    * Mouse position
    * Audio
    * User parameters
    * Resolution
13. Pause or reduce rendering activity when appropriate.

---

# 3. Non-Goals

The following are explicitly out of scope unless reconsidered later:

* GNOME support.
* macOS support.
* Android support.
* Running arbitrary native executables as wallpapers.
* Embedding arbitrary native plugins in wallpapers.
* Acting as a general-purpose game engine.
* Providing arbitrary filesystem access to wallpaper content.
* Providing arbitrary network access to wallpaper content.
* Supporting every feature available in Wallpaper Engine.
* Binary compatibility with Wallpaper Engine projects in the initial release.

Wallpaper Engine import may be investigated later as a migration feature.

---

# 4. Supported Platforms

## 4.1 Linux

Officially supported desktop environments and compositors:

* Niri
* Hyprland
* KDE Plasma

### Niri

Use Wayland layer-shell to provide a background-layer surface.

### Hyprland

Use Wayland layer-shell to provide a background-layer surface.

### KDE Plasma

Provide a native Plasma wallpaper integration.

The KDE integration SHOULD be an adapter around the same Rust Wisp runtime rather than implementing a second renderer.

---

## 4.2 Windows

Support current Windows versions capable of running the required `wgpu` backend.

The Windows implementation MUST use the same:

* Wisp wallpaper package.
* WGSL shaders.
* Runtime.
* Compiler.
* Effect implementations.

Only the desktop presentation layer should differ.

---

# 5. Technology Stack

Primary language:

```text
Rust
```

Core technologies:

| Component                 | Technology                     |
| ------------------------- | ------------------------------ |
| GPU abstraction           | `wgpu`                         |
| Shader language           | WGSL                           |
| Shader parsing/validation | Naga                           |
| Application UI            | Iced                           |
| Serialization             | Serde                          |
| Audio input               | CPAL                           |
| FFT/audio analysis        | rustfft                        |
| Steam integration         | steamworks-rs                  |
| Linux wallpaper host      | Wayland / layer-shell          |
| KDE integration           | Plasma wallpaper adapter       |
| Windows integration       | Native Windows desktop host    |
| IPC                       | Local platform-appropriate IPC |

No major renderer functionality should depend on Qt.

Qt/QML MAY be used as a thin KDE-specific presentation adapter where required by Plasma.

---

# 6. High-Level Architecture

```text
                         ┌──────────────────────────┐
                         │           Wisp           │
                         │           Iced           │
                         │                          │
                         │ Library                  │
                         │ Workshop                 │
                         │ Designer                 │
                         │ Import / Export          │
                         │ Settings                 │
                         └────────────┬─────────────┘
                                      │
                                      ▼
                         ┌──────────────────────────┐
                         │      Wisp Compiler       │
                         │                          │
                         │ Graph                    │
                         │  ↓                       │
                         │ Typed IR                 │
                         │  ↓                       │
                         │ Optimization             │
                         │  ↓                       │
                         │ WGSL + Pass Graph        │
                         └────────────┬─────────────┘
                                      │
                                      ▼
                                   `.wisp`
                                      │
                                      ▼
                         ┌──────────────────────────┐
                         │          wispd           │
                         │                          │
                         │ Minimal wgpu runtime     │
                         └────────────┬─────────────┘
                                      │
               ┌──────────────────────┼──────────────────────┐
               │                      │                      │
               ▼                      ▼                      ▼
        Wayland Host           Plasma Adapter          Windows Host
          │      │                   │                       │
          ▼      ▼                   ▼                       ▼
        Niri  Hyprland              KDE                   Windows
```

---

# 7. Process Model

The Wisp GUI and wallpaper renderer SHOULD be separate processes.

## 7.1 `wisp`

The primary user-facing application.

Responsible for:

* Local wallpaper library.
* Steam Workshop UI.
* Wallpaper editing.
* Wallpaper creation.
* Import/export.
* Settings.
* Wallpaper assignment.
* Publishing.
* Live previews.

The Wisp GUI does not need to remain open for wallpapers to continue running.

---

## 7.2 `wispd`

The lightweight Wisp wallpaper daemon.

Responsible for:

* Loading compiled Wisp wallpapers.
* Creating GPU resources.
* Rendering wallpapers.
* Managing monitor outputs.
* Mouse input state.
* Audio analysis.
* Frame scheduling.
* Performance policies.
* Wallpaper parameter changes.
* Desktop integration.

`wispd` MUST NOT contain editor logic.

---

# 8. Core Runtime Philosophy

The Wisp runtime MUST remain intentionally dumb.

It should understand:

* Resources.
* GPU buffers.
* Textures.
* Bindings.
* Render passes.
* Compute passes.
* Uniform values.
* Pass dependencies.
* Render targets.
* Presentation.

It should NOT understand concepts such as:

* "Bloom node."
* "Parallax node."
* "Image effect."
* "Particle editor."
* "Visual graph."
* "Timeline editor."

Those are Wisp compiler and editor concepts.

At runtime:

```text
Compiled Wisp program
       ↓
Update globals
       ↓
Execute pass 0
       ↓
Execute pass 1
       ↓
...
       ↓
Present
```

---

# 9. Wisp Program

A compiled wallpaper consists conceptually of:

```rust
struct WispProgram {
    version: ProgramVersion,
    resources: Vec<Resource>,
    passes: Vec<Pass>,
    parameters: Vec<Parameter>,
    metadata: ProgramMetadata,
}
```

Example pass model:

```rust
enum Pass {
    Fragment(FragmentPass),
    Compute(ComputePass),
}
```

A fragment pass describes:

* Shader.
* Inputs.
* Bindings.
* Output target.
* Resolution policy.
* Blend state.

A compute pass describes:

* Shader.
* Inputs.
* Outputs.
* Dispatch dimensions.

---

# 10. Wisp Package Format

Portable wallpapers use:

```text
*.wisp
```

A `.wisp` package MUST be platform-independent.

Example:

```text
cyberpunk-girl.wisp
├── manifest.json
├── program.json
├── shaders/
│   ├── pass_0.wgsl
│   ├── pass_1.wgsl
│   └── particles.wgsl
├── assets/
│   ├── background.webp
│   ├── character.webp
│   └── noise.webp
└── preview.webp
```

The package MAY internally be a ZIP-compatible archive or another documented archive format.

The package format SHOULD be documented so Wisp wallpapers can be generated by third-party tooling.

---

# 11. Manifest

Example:

```json
{
  "format_version": 1,
  "id": "example.cyberpunk-girl",
  "name": "Cyberpunk Girl",
  "author": "Example",
  "description": "Interactive neon wallpaper",
  "preview": "preview.webp",
  "program": "program.json",
  "minimum_wisp_version": "1.0",
  "gpu_profile": 1,
  "tags": [
    "interactive",
    "audio-responsive",
    "cyberpunk"
  ]
}
```

---

# 12. Standard Wisp Shader ABI

Every Wisp wallpaper receives a common global uniform interface.

Example:

```wgsl
struct Globals {
    resolution: vec2<f32>,
    mouse: vec2<f32>,

    time: f32,
    delta_time: f32,

    audio_bass: f32,
    audio_mid: f32,
    audio_treble: f32,
    audio_volume: f32,

    frame: u32,
};

@group(0) @binding(0)
var<uniform> globals: Globals;
```

The ABI SHOULD later support richer audio data such as:

```wgsl
audio_spectrum: array<f32, N>
```

Wallpaper-specific parameters are supplied separately.

Example:

```wgsl
struct Parameters {
    bloom_intensity: f32,
    movement_speed: f32,
    parallax_strength: f32,
};
```

---

# 13. Authoring Modes

Wisp MUST support multiple levels of authoring complexity.

## 13.1 Easy Mode

Designed for users with no shader or graphics-programming experience.

Example workflow:

```text
Create Wallpaper
      ↓
Select Image
      ↓
Add Effects
      ↓
Configure
      ↓
Preview
      ↓
Save / Publish
```

Example UI:

```text
Effects

☑ Parallax
☑ Mouse tracking
☑ Floating particles
☑ Audio response
☑ Bloom
☑ Chromatic aberration
☐ VHS distortion
```

Users never need to see shader code or nodes.

Internally, Easy Mode generates the same graph used by Wisp's advanced designer.

---

# 14. Graph Editor

Advanced visual authoring uses a node graph.

Example:

```text
                  ┌─────────┐
                  │  Mouse  │
                  └────┬────┘
                       │
                       ▼
┌──────────┐     ┌───────────┐
│  Image   ├────►│ Parallax  │
└──────────┘     └─────┬─────┘
                       │
                       ▼
                 ┌───────────┐
Audio Bass ─────►│ Distortion│
                 └─────┬─────┘
                       │
                       ▼
                  ┌─────────┐
                  │  Bloom  │
                  └────┬────┘
                       │
                       ▼
                  ┌─────────┐
                  │ Output  │
                  └─────────┘
```

Nodes MUST have typed inputs and outputs.

Example categories:

## Inputs

* Time.
* Delta time.
* Resolution.
* Mouse.
* Audio volume.
* Audio bass.
* Audio mids.
* Audio treble.
* FFT spectrum.
* User parameter.

## Assets

* Image.
* Texture.
* Noise texture.
* Generated texture.

## UV Operations

* Translate.
* Scale.
* Rotate.
* Parallax.
* Warp.
* Wave.
* Perspective.
* Kaleidoscope.

## Colour

* Brightness.
* Contrast.
* Saturation.
* Hue.
* Curves.
* Tint.
* Gradient.
* Vignette.

## Effects

* Chromatic aberration.
* Scanlines.
* Distortion.
* Pixelation.
* Bloom.
* Blur.
* Glitch.
* Noise.
* Sharpen.

## Composition

* Mix.
* Add.
* Multiply.
* Mask.
* Alpha blend.
* Screen.
* Overlay.

## Simulation

* Particles.
* Feedback.
* Persistent buffers.

---

# 15. Shader Mode

Advanced creators MAY write WGSL directly.

Example:

```wgsl
fn wallpaper(uv: vec2<f32>) -> vec4<f32> {
    let movement = sin(globals.time) * 0.01;

    let shifted_uv = uv + vec2<f32>(
        movement,
        globals.audio_bass * 0.01
    );

    return textureSample(
        background,
        background_sampler,
        shifted_uv
    );
}
```

Raw WGSL MUST pass:

* Parsing.
* Validation.
* Capability validation.
* Resource-limit validation.

before it can be executed by Wisp.

---

# 16. Wisp Compiler

The compiler converts authoring graphs into GPU programs.

```text
Visual Graph
     ↓
Typed Graph
     ↓
Wisp IR
     ↓
Optimization
     ↓
Pass Generation
     ↓
WGSL Generation
     ↓
Naga Validation
     ↓
Wisp Program
```

---

# 17. Fusable Nodes

Most operations SHOULD compile into functions within the same shader.

For example:

```text
Image
 ↓
Parallax
 ↓
Hue
 ↓
Contrast
 ↓
Vignette
 ↓
Audio Brightness
 ↓
Output
```

should ideally compile into:

```text
1 fragment shader
1 render pass
1 draw call
```

Example generated structure:

```wgsl
fn wallpaper(uv: vec2<f32>) -> vec4<f32> {
    let warped_uv = parallax(uv, globals.mouse);

    var color = textureSample(
        image,
        image_sampler,
        warped_uv
    );

    color = hue_shift(color, params.hue);
    color = adjust_contrast(color, params.contrast);
    color = vignette(color, uv);

    color.rgb *= 1.0 +
        globals.audio_bass *
        params.audio_response;

    return color;
}
```

---

# 18. Pass-Boundary Nodes

Some effects inherently require intermediate GPU state.

Examples:

* Gaussian blur.
* Bloom.
* Feedback.
* Persistent simulations.
* Compute workloads.
* Particle buffers.
* Temporal effects.
* Render-to-texture operations.

Example:

```text
Image
 ↓
Colour Adjustment
 ↓
Blur
 ↓
Bloom
 ↓
Vignette
 ↓
Output
```

might compile into:

```text
Pass 0:
Image + Colour Adjustment

Pass 1:
Horizontal Blur

Pass 2:
Vertical Blur

Pass 3:
Bloom + Vignette + Composite
```

The compiler SHOULD minimize these boundaries.

---

# 19. Compiler Optimizations

The Wisp compiler SHOULD eventually implement:

* Constant folding.
* Dead-node elimination.
* Dead-resource elimination.
* Shader fusion.
* Pass fusion.
* Uniform propagation.
* Common-expression elimination.
* Duplicate texture-sample elimination.
* Unused asset elimination.
* Render-target lifetime analysis.
* Render-target reuse.
* Resolution reduction for suitable intermediate effects.

Example:

```text
7 visual nodes
      ↓
Wisp compiler
      ↓
1 fragment pass
```

---

# 20. Resource Management

Resources SHOULD be allocated when loading a wallpaper rather than every frame.

Load process:

```text
Open .wisp
    ↓
Validate
    ↓
Load assets
    ↓
Create textures
    ↓
Create buffers
    ↓
Create pipelines
    ↓
Create bind groups
    ↓
Allocate intermediate targets
    ↓
Ready
```

The normal Wisp render loop SHOULD perform zero heap allocations where practical.

---

# 21. Render Loop

Conceptually:

```rust
while active {
    update_global_uniforms();

    for pass in program.passes() {
        execute(pass);
    }

    present();
}
```

The runtime should contain no generic object-update loop.

---

# 22. Performance Model

Wisp wallpapers are expected to run continuously.

Performance is therefore a primary product requirement.

Wisp SHOULD support:

* Configurable FPS.
* Per-monitor FPS.
* Automatic idle FPS reduction.
* Pause while monitors are asleep.
* Pause while the session is locked.
* Optional pause for fullscreen applications.
* Optional FPS reduction while fullscreen applications are running.
* Configurable GPU usage policies.

Example:

```text
Normal desktop        60 FPS
Idle desktop          30 FPS
Fullscreen app         1 FPS / paused
Display asleep         0 FPS
Session locked         0 FPS
```

---

# 23. Performance Information for Creators

The Wisp designer SHOULD display estimated or measured wallpaper cost.

Example:

```text
Performance

GPU frame time     0.31 ms
Passes             1
Draw calls         1
Textures           3
Estimated VRAM     42 MB
Resolution         3840 × 2160

Rating             Very Light
```

Warnings SHOULD be displayed for expensive designs.

Example:

```text
Warning

This effect allocates four full-resolution
intermediate textures.

Estimated additional VRAM at 4K: 126 MB.
```

---

# 24. Multiple Monitors

Wisp MUST allow users to:

* Use one wallpaper across every monitor.
* Assign different wallpapers to different monitors.
* Configure wallpaper parameters per monitor.
* Span a single wallpaper across monitors where technically appropriate.

Example:

```text
Monitor 1
2560×1440
Cyberpunk Girl

Monitor 2
1920×1080
Audio Spectrum
```

Monitor identity SHOULD remain stable across restarts where possible.

---

# 25. Wayland Integration

Niri and Hyprland use a shared Wisp Wayland host implementation.

```text
wispd
  ↓
wgpu
  ↓
Wayland surface
  ↓
layer-shell
  ↓
BACKGROUND layer
```

Compositor-specific code SHOULD be avoided unless required.

Niri and Hyprland SHOULD share almost all implementation code.

---

# 26. KDE Plasma Integration

KDE should receive a proper Plasma wallpaper integration.

Preferred architecture:

```text
wispd
   ↓
GPU-rendered output
   ↓
Linux GPU texture sharing
   ↓
thin Plasma adapter
   ↓
Plasma desktop
```

The Plasma adapter SHOULD NOT contain a second wallpaper renderer.

Its responsibility is presentation and Plasma integration only.

If zero-copy texture sharing is unavailable or unreliable, a fallback integration strategy MAY be provided.

The KDE renderer itself MUST remain the same `wisp-runtime` used by the other platforms.

---

# 27. Windows Integration

The Windows host is responsible only for attaching Wisp's rendering surface to the desktop.

```text
wispd
  ↓
wgpu
  ↓
Windows presentation surface
  ↓
Windows desktop
```

No Windows-specific shader implementation should exist.

---

# 28. Main Wisp Application UI

The primary Wisp application is built using Iced.

Major pages:

```text
Library
Workshop
Create
Designer
Import
Settings
```

Example:

```text
┌───────────────────────────────────────────────────────────────┐
│ Wisp                                                          │
│                                                               │
│ Library    Workshop    Create    Import              Settings │
├───────────────────────────────────────────────────────────────┤
│                                                               │
│ ┌────────────┐ ┌────────────┐ ┌────────────┐                  │
│ │            │ │            │ │            │                  │
│ │ Cyberpunk  │ │ Aquarium   │ │ Synthwave  │                  │
│ │   Girl     │ │            │ │            │                  │
│ └────────────┘ └────────────┘ └────────────┘                  │
│                                                               │
│ Monitor 1: Cyberpunk Girl                                     │
│ Monitor 2: Aquarium                                           │
│                                                               │
└───────────────────────────────────────────────────────────────┘
```

The Wisp UI SHOULD look like a polished consumer application rather than developer tooling.

---

# 29. Live Preview

The Wisp designer preview MUST use the real Wisp wallpaper runtime.

There MUST NOT be:

```text
Editor renderer
+
Desktop renderer
```

Instead:

```text
Wisp Designer
    ↓
wisp-runtime
    ↓
wgpu texture
    ↓
Iced preview widget
```

This guarantees that editor output matches the actual desktop wallpaper.

---

# 30. Hot Reloading

Editing a graph should trigger incremental recompilation where possible.

```text
Graph changed
      ↓
Determine affected shader/pass
      ↓
Generate WGSL
      ↓
Validate
      ↓
Create new pipeline
      ↓
Atomic pipeline swap
```

Compilation errors should leave the previous valid preview running.

---

# 31. Wisp Library

The local Wisp library stores:

* Installed wallpapers.
* Imported wallpapers.
* Workshop subscriptions.
* User-created wallpapers.
* Draft projects.

Each wallpaper should display:

* Preview.
* Name.
* Author.
* Source.
* Tags.
* GPU cost.
* Installed version.

---

# 32. Import

Supported initial import formats:

```text
.wisp
Wisp project directory
WGSL shader
image
```

Possible later import formats:

```text
ShaderToy-style shader
video
Wallpaper Engine project
```

Image import SHOULD immediately offer Easy Mode creation.

Example:

```text
Import image
    ↓
Create Wisp Wallpaper
    ↓
Add effects
    ↓
Save
```

---

# 33. Export

Creators MUST be able to export a wallpaper as:

```text
wallpaper-name.wisp
```

Exported packages MUST work without Steam.

This allows:

* Manual sharing.
* GitHub releases.
* Direct downloads.
* LAN distribution.
* Linux packaging.
* Offline installation.

Steam is a distribution mechanism, not a Wisp runtime requirement.

---

# 34. Steam Workshop

Wisp's Steam Workshop integration SHOULD support:

* Browse.
* Search.
* Tags.
* Subscribe.
* Unsubscribe.
* Download.
* Update.
* Preview images.
* Publishing.
* Updating published wallpapers.

Example user experience:

```text
Wisp Workshop
   ↓
Search "cyberpunk"
   ↓
Select wallpaper
   ↓
Subscribe
   ↓
Download
   ↓
Apply
```

---

# 35. Workshop Publishing

Publishing workflow:

```text
Create wallpaper
      ↓
Validate
      ↓
Generate preview
      ↓
Performance test
      ↓
Publish
```

Publisher UI:

```text
Title
Description
Tags
Preview image
Visibility
Change notes

[ Publish to Workshop ]
```

The Workshop item contains the portable `.wisp` package.

---

# 36. Wallpaper Parameters

Creators may expose configurable parameters.

Supported initial types:

* Float.
* Integer.
* Boolean.
* Colour.
* Enum.
* Slider.
* Vector.

Example declaration:

```text
Parallax Strength

Type: Float
Min: 0
Max: 2
Default: 0.5
```

Users see:

```text
Wallpaper Settings

Parallax       ━━━━━●━━━━ 0.5
Bloom          ━━━━━━━●━━ 0.8
Audio React    ☑
Theme          [ Neon ▼ ]
```

---

# 37. Audio Reactivity

`wispd` handles audio analysis centrally.

Pipeline:

```text
Audio output
    ↓
CPAL
    ↓
sample buffer
    ↓
FFT
    ↓
normalized bands
    ↓
Wisp shader globals
```

Initial shader values:

```text
volume
bass
mid
treble
```

Later:

```text
FFT bins
beat detection
spectral centroid
transients
```

Audio analysis should happen once regardless of the number of active wallpapers.

---

# 38. Input

Initial Wisp runtime inputs:

* Mouse position.
* Time.
* Delta time.
* Frame number.
* Resolution.
* Audio.
* User parameters.

Future inputs MAY include:

* Cursor velocity.
* Desktop focus state.
* Time of day.
* Battery state.
* System load.

Inputs exposing sensitive information should not be added casually.

---

# 39. Security Model

Workshop content is untrusted.

Wisp wallpapers MUST NOT receive:

* Native code execution.
* Shell execution.
* Filesystem access.
* Arbitrary process access.
* Arbitrary network access.
* Dynamic native libraries.

WGSL shaders MUST be validated before pipeline creation.

---

# 40. Resource Limits

`wispd` MUST enforce configurable hard limits.

Examples:

```text
Maximum texture dimensions
Maximum asset size
Maximum package size
Maximum number of textures
Maximum number of passes
Maximum storage-buffer size
Maximum compute dispatch
Maximum intermediate render targets
Maximum total estimated GPU memory
Maximum particle count
```

Extremely expensive wallpapers should be rejected or require explicit user approval.

---

# 41. Wisp GPU Profiles

Wisp should define its own portable GPU capability profiles.

Initial profile:

```text
Wisp GPU Profile 1
```

A wallpaper targeting Profile 1 MUST work on every officially supported platform satisfying Profile 1.

The profile should initially avoid optional GPU features such as:

* Vendor-specific extensions.
* Ray tracing.
* Mesh shaders.
* Uncommon texture formats.
* Platform-specific shader extensions.
* Features unavailable across the common `wgpu` backend set.

This enables the guarantee:

> If Wisp accepts a Profile 1 wallpaper, that wallpaper should run everywhere Wisp supports Profile 1.

---

# 42. Validation

Before installation or execution:

```text
.wisp package
     ↓
Manifest validation
     ↓
Path validation
     ↓
Asset validation
     ↓
Resource-limit validation
     ↓
WGSL parse
     ↓
Naga validation
     ↓
GPU-profile validation
     ↓
Program validation
     ↓
Ready
```

Malformed wallpapers MUST fail safely.

---

# 43. Crash Isolation

A bad wallpaper SHOULD NOT take down the Wisp GUI.

Because rendering occurs in `wispd`, crashes can be detected independently.

Wisp SHOULD:

1. Detect repeated renderer crashes.
2. Disable the offending wallpaper.
3. Restore a safe fallback.
4. Notify the user.

---

# 44. Frame Watchdog

`wispd` SHOULD monitor GPU frame time.

Example policy:

```text
Target frame time: 16.7 ms

Wallpaper repeatedly exceeds:
50 ms
    ↓
reduce FPS or warn

Wallpaper repeatedly exceeds:
250 ms
    ↓
pause wallpaper
```

Exact defaults should be determined experimentally.

---

# 45. Wisp Project Format

Editable Wisp projects SHOULD be distinct from distributable `.wisp` packages.

Example:

```text
my-wallpaper/
├── project.json
├── graph.json
├── assets/
│   ├── background.png
│   └── character.png
└── shaders/
    └── custom.wgsl
```

Compilation produces:

```text
my-wallpaper.wisp
```

The project contains authoring information.

The `.wisp` package contains runtime information.

---

# 46. Repository Layout

Suggested workspace:

```text
wisp/
├── Cargo.toml
│
├── crates/
│   ├── wisp-format/
│   ├── wisp-ir/
│   ├── wisp-compiler/
│   ├── wisp-runtime/
│   ├── wisp-audio/
│   ├── wisp-ipc/
│   ├── wisp-workshop/
│   │
│   ├── wisp-host-wayland/
│   ├── wisp-host-windows/
│   ├── wisp-host-plasma/
│   │
│   ├── wisp-designer/
│   └── wisp-app/
│
├── apps/
│   ├── wispd/
│   └── wisp/
│
├── integrations/
│   └── plasma/
│
├── examples/
│   ├── shader-basic/
│   ├── audio-reactive/
│   └── multipass-blur/
│
└── docs/
```

---

# 47. Crate Responsibilities

## `wisp-format`

Owns:

* `.wisp` manifest.
* Program serialization.
* Format versioning.
* Package loading.
* Package validation.

---

## `wisp-ir`

Owns:

* Typed visual graph IR.
* Node types.
* Graph type checking.
* Compiler-facing representation.

---

## `wisp-compiler`

Owns:

* Graph lowering.
* Optimization.
* Pass generation.
* WGSL generation.
* Naga validation.
* GPU-profile validation.

---

## `wisp-runtime`

Owns:

* `wgpu` device.
* Pipelines.
* Resources.
* Pass execution.
* Uniform updates.
* Render-target management.

It MUST contain no platform-specific wallpaper code.

---

## `wisp-audio`

Owns:

* Audio capture.
* FFT.
* Band normalization.
* Beat detection where implemented.

---

## `wisp-host-wayland`

Owns:

* Wayland surfaces.
* Layer-shell.
* Monitor mapping.
* Niri integration.
* Hyprland integration.

---

## `wisp-host-windows`

Owns:

* Windows desktop attachment.
* Windows monitor handling.
* Windows presentation surfaces.

---

## `wisp-host-plasma`

Owns:

* Communication with the Plasma adapter.
* GPU-frame sharing where required.

---

## `wisp-designer`

Owns:

* Node canvas.
* Easy Mode.
* Shader editor.
* Parameter editor.
* Live preview.
* Compiler integration.

---

## `wisp-workshop`

Owns:

* Steam API integration.
* Search.
* Subscriptions.
* Downloads.
* Publishing.
* Updating Workshop items.

---

## `wisp-app`

Owns:

* Iced application.
* Library.
* Settings.
* Workshop UI.
* Import/export.
* Monitor assignment.

---

# 48. IPC

The Wisp GUI communicates with `wispd`.

Example commands:

```text
ListMonitors
GetActiveWallpapers
SetWallpaper
UnloadWallpaper
SetParameter
Pause
Resume
SetFpsLimit
ReloadWallpaper
GetPerformanceStats
```

Example:

```rust
enum WispDaemonCommand {
    SetWallpaper {
        monitor: MonitorId,
        package: PathBuf,
    },

    SetParameter {
        monitor: MonitorId,
        parameter: String,
        value: ParameterValue,
    },

    Pause {
        monitor: Option<MonitorId>,
    },

    Resume {
        monitor: Option<MonitorId>,
    },
}
```

---

# 49. Versioning

The following should be versioned independently:

```text
Wisp package format version
Wisp program format version
Wisp shader ABI version
Wisp GPU profile version
Wisp project format version
```

The runtime SHOULD preserve backwards compatibility where practical.

---

# 50. Development Phases

## Phase 1 — Renderer Proof of Concept

Implement:

* Rust.
* `wgpu`.
* WGSL.
* Ordinary window.
* Fullscreen triangle.
* Time uniform.
* Mouse uniform.
* Texture loading.

Success criterion:

> The exact same Wisp wallpaper renders in a normal test window on Windows and Linux.

---

## Phase 2 — Portable Wisp Runtime

Implement:

* `.wisp`.
* Program definition.
* Resource loading.
* Multiple textures.
* User parameters.
* Multiple fragment passes.
* Compute passes.
* Validation.

---

## Phase 3 — Niri and Hyprland

Implement:

* Layer-shell.
* Niri.
* Hyprland.
* Multiple monitors.
* Wallpaper assignment.

Success criterion:

> Wisp can replace the desktop background on both Niri and Hyprland.

---

## Phase 4 — Windows

Implement:

* Windows desktop host.
* Multi-monitor handling.
* Shared runtime.

Success criterion:

> The same `.wisp` package runs on Windows and Linux.

---

## Phase 5 — Wisp Compiler

Implement:

* Typed visual graph.
* Fusable nodes.
* WGSL generation.
* Pass-boundary nodes.
* Shader fusion.
* Dead-node elimination.

---

## Phase 6 — Basic Wisp Application

Implement:

* Iced.
* Library.
* Settings.
* Wallpaper selection.
* Monitor assignment.
* Import.
* Live preview.

---

## Phase 7 — Easy Designer

Implement:

* Image import.
* Effect presets.
* Parameters.
* Preview.
* Export.

At this point a non-programmer should be able to create a dynamic Wisp wallpaper.

---

## Phase 8 — Visual Graph

Implement:

* Node canvas.
* Connections.
* Type checking.
* Node search.
* Properties.
* Compiler diagnostics.

---

## Phase 9 — Interactivity

Implement:

* Audio capture.
* FFT.
* Mouse-driven nodes.
* Persistent buffers.
* Particles.
* Feedback effects.

---

## Phase 9.5 — Layers and Effects

Build on the visual graph editor and Phase 9's interactivity, particle, and
feedback foundations to provide ready-to-use wallpaper effects.

Implement:

* An image layer panel with ordering, visibility, opacity, and independent
  position, scale, rotation, and pivot controls.
* Transparent image composition and alpha, add, multiply, screen, and overlay
  blending, with matching typed graph operations.
* Imported and paintable grayscale masks with feathering and inversion, usable
  for both layer composition and effect strength.
* Wave, warp, and displacement UV operations with configurable direction,
  amplitude, frequency, and animation speed where applicable.
* Masked hair and clothing motion presets: anchor regions remain still while
  selected regions sway or deform. Support separate transparent character parts
  and localized deformation of an image.
* Droplet, rain, and ripple presets, including configurable density, size,
  speed, distortion strength, and affected region where applicable. Use Phase 9's
  particle/feedback facilities where an effect requires persistent state.
* Easy Designer effect controls and equivalent graph nodes, with live preview
  through the shared runtime.
* Editable project persistence for layers, masks, and effect settings; portable
  package export containing the compiled shaders, passes, and required assets.
* Creator cost information for added passes and intermediate storage, with
  bounded defaults for particle counts, textures, and simulation workloads.

Layer and effect authoring MUST lower through the existing compiler architecture.
The runtime MUST continue to execute compiled resources and passes rather than
interpret layers, presets, or graph nodes.

Initial hair/clothing motion uses masked image deformation. Skeletal rigging,
strand-level hair physics, and automatic subject segmentation are later work.
Creators may supply prepared transparent parts and background images.

Success criteria:

* Without writing WGSL, a creator can layer a background and character, mask the
  character's hair, and animate it while keeping the face and anchor regions still.
* A creator can add and configure visible droplets, rain, and ripples using presets.
* Saved projects reopen with the same layers, masks, and effect settings.
* Exported packages reproduce the preview through the shared runtime and run as
  Niri wallpapers; deterministic rendering checks cover supported GPU backends
  and Windows/Linux parity under the existing platform verification requirements.

---

## Phase 10 — KDE Plasma

Implement:

* Plasma wallpaper adapter.
* GPU-frame sharing.
* Multi-monitor integration.
* Wallpaper settings integration where useful.

---

## Phase 11 — Steam Workshop

Implement:

* Steam integration.
* Search.
* Browse.
* Subscribe.
* Install.
* Update.
* Publish.
* Update published wallpaper.

---

## Phase 12 — Advanced Shader Authoring

Implement:

* WGSL editor.
* Syntax highlighting.
* Compiler diagnostics.
* Live reload.
* Shader templates.
* Shader import.

---

# 51. MVP Definition

The minimum compelling public release of Wisp SHOULD include:

* Windows.
* Niri.
* Hyprland.
* KDE Plasma.
* `wgpu`.
* Portable `.wisp` packages.
* Image wallpapers.
* WGSL effects.
* Time interaction.
* Mouse interaction.
* Audio response.
* Multiple monitors.
* Easy Mode designer.
* Visual graph editor.
* Raw WGSL editor.
* Local import/export.
* Steam Workshop download.
* Steam Workshop publishing.
* Performance controls.

---

# 52. Product Principles

## 52.1 Platform Parity

Wisp wallpaper content MUST NOT contain platform-specific implementations.

Platform-specific code belongs below the renderer.

```text
Wisp Wallpaper
      ↓
Wisp Compiler
      ↓
Wisp Runtime
      ↓
Host Adapter
```

Never:

```text
Wallpaper
 ├── windows_shader
 ├── kde_shader
 └── hyprland_shader
```

---

## 52.2 Compile Complexity Away

Do not make the Wisp runtime interpret designer abstractions.

Prefer:

```text
powerful designer
       ↓
smart compiler
       ↓
tiny runtime
```

rather than:

```text
simple designer
      ↓
complex runtime engine
```

---

## 52.3 GPU First

Effects should execute on the GPU whenever practical.

CPU work should primarily consist of:

* Updating uniforms.
* Audio analysis.
* IPC.
* Frame scheduling.
* GPU command submission.

---

## 52.4 Safe by Default

Workshop wallpapers are data and GPU programs, not applications.

They do not receive general operating-system capabilities.

---

## 52.5 Steam Is Optional

Steam Workshop is a first-class distribution mechanism but not a technical dependency for running Wisp.

`.wisp` files remain independently installable and shareable.

---

## 52.6 Creation Should Scale With Expertise

A casual user should be able to create:

```text
Image
+ Parallax
+ Audio
+ Bloom
= Wisp wallpaper
```

An advanced user should be able to create:

```text
Custom compute simulation
        ↓
Persistent storage buffer
        ↓
Procedural renderer
        ↓
Temporal feedback
        ↓
Custom WGSL composite
```

Both use the same Wisp compiler and runtime.

---

# 53. Cross-Platform Guarantee

Wisp should aim to make the following a core user expectation:

```text
Created on Windows
       ↓
Publish
       ↓
Runs on Niri
Runs on Hyprland
Runs on KDE
Runs on Windows
```

and equally:

```text
Created on Linux
       ↓
Publish
       ↓
Runs unchanged on Windows
```

Wallpaper authors should not need to know which desktop environment or operating system their users run.

A valid Wisp wallpaper targets Wisp, not Windows, KDE, Niri, or Hyprland.

---

# 54. Core Differentiator

Wisp should not primarily position itself as:

> Wallpaper Engine for Linux.

The stronger identity is:

> **Wisp is a cross-platform GPU wallpaper compiler, runtime, and designer.**

A Wisp wallpaper is effectively a small portable GPU program.

The designer may expose complex scenes and visual effects, but the compiler reduces them into the minimum practical set of GPU operations.

For a simple wallpaper:

```text
Image
 ↓
Parallax
 ↓
Colour grading
 ↓
Audio response
 ↓
Vignette
 ↓
Output
```

Wisp may compile the entire wallpaper into:

```text
1 shader
1 pass
1 fullscreen triangle
1 draw call
```

The author gets a powerful visual design environment.

The user gets a tiny runtime.

That separation is fundamental to Wisp.

---

# 55. Product Summary

Wisp consists of three major systems:

```text
┌───────────────────────────────┐
│         Wisp Designer         │
│                               │
│ Easy Mode                     │
│ Visual Graph                  │
│ WGSL                          │
└───────────────┬───────────────┘
                │
                ▼
┌───────────────────────────────┐
│         Wisp Compiler         │
│                               │
│ Graph → IR → Optimization     │
│ → WGSL → GPU Pass Program     │
└───────────────┬───────────────┘
                │
              .wisp
                │
                ▼
┌───────────────────────────────┐
│          Wisp Runtime         │
│                               │
│      wispd + wgpu             │
└───────────────┬───────────────┘
                │
       ┌────────┼────────┐
       ▼        ▼        ▼
   Wayland    Plasma   Windows
      │
   ┌──┴──┐
   ▼     ▼
 Niri Hyprland
```

The overall objective is simple:

> **Create once. Render everywhere. Keep the runtime tiny.**

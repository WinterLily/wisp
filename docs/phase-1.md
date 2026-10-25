# Phase 1: renderer proof of concept

## Boundaries

`crates/wisp-runtime` owns Naga validation, a wgpu device/queue, image uploads,
persistent bind groups and pipelines, uniform updates, and a fullscreen GPU pass.
The caller provides the target texture view. It has no dependency on winit, an
editor, a desktop host, or Steam.

`apps/wisp-preview` owns the ordinary window and surface, cursor events, resizing,
frame pacing, and CLI. The event loop uses winit's
[ApplicationHandler](https://docs.rs/winit/0.30.13/winit/application/trait.ApplicationHandler.html).
The renderer uses wgpu 27 and matching Naga 27, pinned by Cargo.lock. A shader is
validated before GPU resources are created; wgpu pipeline validation errors are
returned through error scopes.

The headless PNG path uses the same `Renderer::render` method as the window host.
It adds an offscreen target and a padded GPU readback buffer. Readback allocates
per snapshot; the live loop retains its textures, pipeline, buffers, and bindings.
wgpu still creates command encoders and performs internal frame bookkeeping.

The preview releases GPU resources and surfaces in the event loop's `exiting`
callback, before winit disconnects the Wayland display. Struct field order also
keeps the native window alive until GPU resources and its surface are dropped.
This matters even when rendering through Vulkan, since adapter discovery may
initialize EGL as well.

## Shader interface (ABI 1, provisional)

The author supplies `fn wallpaper(uv: vec2<f32>) -> vec4<f32>` plus helper functions.
The host prepends [abi.wgsl](../crates/wisp-runtime/src/abi.wgsl), which owns the
vertex and fragment entry points. Do not redeclare host bindings or entry points.
This interface is also used by the initial [versioned package format](package-format.md).
Shader ABI 2 adds image and parameter bindings as documented there; ABI 1 stays
unchanged. A broader compiled resource/pass model remains future work.

| Group | Binding | Resource |
| --- | --- | --- |
| 0 | 0 | `globals`, uniform buffer |
| 1 | 0 | `background`, filterable 2D sRGB image |
| 1 | 1 | `background_sampler`, linear filtering, clamp to edge |

UV and mouse positions are normalized to `[0, 1]`, with `(0, 0)` at the top left.
Mouse defaults to the center and retains its last value when leaving the window.
Resolution is in physical pixels. Images stretch to the viewport in the initial
shader; aspect-ratio fitting is left to shader authors for this milestone.

| Globals field | Byte offset | Meaning |
| --- | --- | --- |
| `resolution: vec2<f32>` | 0 | Render target dimensions |
| `mouse: vec2<f32>` | 8 | Normalized cursor position |
| `time: f32` | 16 | Elapsed animation seconds, excluding pauses |
| `delta_time: f32` | 20 | Seconds since the last frame |
| `audio_bass: f32` | 24 | Reserved, initially zero |
| `audio_mid: f32` | 28 | Reserved, initially zero |
| `audio_treble: f32` | 32 | Reserved, initially zero |
| `audio_volume: f32` | 36 | Reserved, initially zero |
| `frame: u32` | 40 | Frame index, wraps at u32 maximum |
| Host padding | 44 | Zero; total buffer size is 48 bytes |

Texture sampling decodes sRGB into linear RGB. Shaders return linear RGB; an 8-bit
sRGB output target performs the output conversion. Alpha is written without
blending. The preview uses FIFO presentation and a configurable frame-rate cap
(default 60). Zero-sized and reported occluded windows pause rendering. Compositor
support determines whether occlusion is reported; this is not desktop idle,
fullscreen, monitor sleep, or session-lock detection.

## ABI 1 limits

- Shader source: at most 1 MiB, parsed and validated by Naga with no optional capabilities.
- ABI 1 accepts only the three host resource bindings and two host entry points.
- Encoded images: at most 32 MiB, dimensions at most 4096 × 4096, with a 64 MiB decoder allocation limit. RGBA conversion may allocate an additional image buffer.
- Snapshots and initial window dimensions: 1–4096 pixels per axis; resized windows are limited by the device.
- No optional wgpu device features are requested. Limits start at wgpu's downlevel defaults, with adapter texture resolution limits.

These are initial local-preview limits, not a complete Wisp GPU Profile 1
implementation or untrusted Workshop package validator. Shader validation cannot
bound shader execution time. The initial package implementation adds archive and
metadata validation; configurable GPU resource budgets, process isolation,
watchdogs, and the complete GPU profile remain later work.

## Validation scope

CPU tests cover parsing and validation failures, forbidden resources, input limits,
and the host layout against Naga's parsed WGSL layout. The explicit GPU test checks
known corner colors from a 2×2 image, an unaligned 65-pixel readback width, invalid
snapshot dimensions, and changes from time and mouse uniforms.

Windows and Linux use the same source and assets. CI includes both targets, but
cross-platform visual parity requires running and comparing snapshots on actual
Windows and Linux drivers; it is not established by compilation alone.

Initial local validation passed on Linux: CPU tests, Clippy, Vulkan pixel
readback, a PNG snapshot, and window presentation/clean shutdown using Vulkan
and OpenGL. Windows execution has not yet been tested.

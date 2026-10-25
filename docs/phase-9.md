# Phase 9: Audio and persistent interactivity

Phase 9 adds host audio capture/FFT and graph authoring for particle emitters and
feedback. All effects compile ahead of time into the existing version 4 / ABI 3
program. The runtime still only executes resources, uniforms, and GPU passes.

## Try it

```sh
cargo build --workspace
cargo run -p wisp -- --graph examples/audio-reactive
cargo run -p wispc -- examples/audio-reactive --output audio-reactive.wisp
cargo run -- --package audio-reactive.wisp --audio
cargo run -p wispd -- --package audio-reactive.wisp --audio
```

In the app, open Settings, enable audio, select a source, set sensitivity, and Save
settings. Refresh devices populates the source picker. System output is the default
source. Audio starts disabled; enabling it affects the live preview and managed
daemon. Device selection and gain persist in `assignments.json`, so the daemon
reloads them without restarting or replacing wallpaper simulation state.

The example connects Mouse to a persistent emitter, audio bass to speed, volume
to brightness, and the particle image to a fading feedback effect. It has editable
size and decay parameters. Move the pointer within the preview. Save/open/export
uses the standard project workflow; there is no runtime graph interpreter.

## Capture and analysis

`wisp-audio` uses [CPAL 0.18.2](https://docs.rs/cpal/0.18.2/cpal/) and
[RustFFT 6.4](https://docs.rs/rustfft/6.4.1/rustfft/). Linux uses CPAL's native
PulseAudio backend, including PipeWire's PulseAudio compatibility server; the
selected output resolves to its monitor source. Windows uses WASAPI output
loopback. An explicitly selected input may be a microphone. Failure to find an
output monitor never falls back to a microphone.

Linux builds also need ALSA headers because CPAL includes its ALSA backend
(`libasound2-dev` on Debian/Ubuntu, `alsa-lib` on Arch). Capture needs a working
PulseAudio or `pipewire-pulse` server. No audio files are recorded by the service.

One service in `wispd` captures/analyzes once for all monitors. The app and standalone
preview each have their own service when enabled. Capture callbacks convert samples
into a bounded 16,384-frame queue with no callback allocation or blocking locks.
Overflow drops frames and increments a diagnostic counter. FFT and smoothing happen
on a worker, not the callback or render thread. Disabled services wait for a setting
change. Capture loss produces silence, reports a status, and retries every five
seconds; source/default-output changes are checked on the same interval. Changing
the source or gain resets analysis. Stale results expire after 250 ms.

The analyzer supports 8–192 kHz and one to eight channels. Each channel has DC
removed and a 4,096-sample Hann window; transforms run every 1,024 frames. Band
powers are combined across channels before taking their square root, so antiphase
stereo does not cancel. A sine at amplitude 0.5 settles near RMS 0.3536 at gain 1.
Bands are linear, gain-scaled and clamped to [0,1], with 25 ms attack and 200 ms
release. Full-window startup latency is about 85 ms at 48 kHz, in addition to
backend startup/buffering. Linux requests 512-frame delivery chunks.

| Global | Frequency range / meaning |
| --- | --- |
| `audio_bass` | 20–250 Hz |
| `audio_mid` | 250–4,000 Hz |
| `audio_treble` | 4,000–20,000 Hz, limited by Nyquist |
| `audio_volume` | Full-band, DC-rejected RMS |

These populate the existing globals without an ABI change. Rich spectrum bins,
beat detection, and automatic gain normalization are not part of this milestone.

```sh
# Lists stable CPAL IDs without opening a stream or a display.
cargo run -p wispd -- --list-audio-devices
# CPU validation never opens the audio device, even with audio enabled.
cargo run -p wispd -- --package audio-reactive.wisp --audio --check
# An explicit ID selects a source/output. Gain is bounded to 0..=10.
cargo run -p wispd -- --package audio-reactive.wisp --audio --audio-device 'PulseAudio:my-output.monitor' --audio-gain 2
# Bounded diagnostic meter; default or a device ID, followed by 1..=60 seconds.
cargo run -p wisp-audio --example meter -- default 8
```

Assignment JSON remains backward compatible:

```json
{
  "audio": { "enabled": true, "device": null, "gain": 1.0 },
  "default": { "package": "audio-reactive.wisp", "fps": 60 },
  "outputs": {}
}
```

## New graph operations

| Operation | Typed inputs | Properties / output |
| --- | --- | --- |
| `mouse_distance` | `uv`: Vector2 | Float distance from normalized mouse |
| `particles` | `position`: Vector2, `color`: Color, `speed`: Float, `size`: Float | `count`, `width`, `height`; Color image |
| `feedback` | `color`: Color, `decay`: Float | `width`, `height`; Color image |

Existing Mouse, Parallax, Translate, and arithmetic nodes remain available. Particle
inputs can use parameters, audio, mouse, or any type-correct upstream expressions.
Emitter position and speed are evaluated at deterministic per-particle UVs; color
and radius are evaluated per output pixel. Speed clamps to [0,2] normalized units
per second, radius to [0.001,0.25] of target height, and position to [0,1].

Particles lower to a compute pass that advances an f32 `vec4` state buffer and
writes a small position texture, followed by a bounded fragment image pass. Each
particle has position, age, and initialization state. It moves in a deterministic
radial direction, fades over a three-second lifetime, and respawns at the current
emitter after expiry or leaving the image. Simulation delta is capped at 0.1 s;
pauses do not fast-forward. State is independent per renderer, persists across
frames, runtime parameter updates, and presentation resize, and resets when loading a new
renderer. Desktop assignment changes can recreate a renderer; audio-only setting
changes preserve it. The sampled position texture is RGBA8, so displayed positions are
quantized to 1/255; authoritative simulation state stays f32.

Feedback materializes its input, then uses a compute pass with one persistent
f32 history element per pixel. Each invocation reads/writes only its own element:

```text
history = max(current_color, previous_history * decay^(delta_seconds * 60))
```

This produces temporal peak-hold trails without graph cycles or read/write texture
hazards. Decay clamps to [0,0.9999]. Zero delta retains existing history; zero decay
clears old history on an advancing frame. It is per-channel, including alpha, and
operates on linear color clamped to [0,1]. Intermediate/presentation textures retain
the runtime's existing RGBA8/sRGB behavior. Spatially warped texture feedback and
HDR accumulation remain future extensions.

Particles allow 1–128 particles, targets 1–1,024 in each dimension, and at most
16,777,216 particle/pixel evaluations per image. Defaults are 32 particles at
512×512. Feedback targets are also capped at 1,024. Existing limits additionally
bound storage buffers, total memory, passes, targets, and dispatch. Each effect
owns one storage buffer; the current format permits four buffers across a program.
The graph cost display includes target and persistent storage estimates. The example
uses five passes, four targets (about 3 MiB), and two buffers (about 4 MiB).

## Deterministic rendering and checks

```sh
# 120 warmup frames at 60 Hz, then one final snapshot frame. No live capture.
cargo run -- --package audio-reactive.wisp --output particles.png --width 640 --height 360 --warmup 120 --audio-bands 0.4,0.1,0.05,0.5
cargo test --workspace
WGPU_BACKEND=vulkan cargo test -p wisp-compiler --test gpu -- --ignored
WGPU_BACKEND=gl cargo test -p wisp-compiler --test gpu -- --ignored
WGPU_BACKEND=vulkan cargo test -p wisp-app --lib -- --ignored
WGPU_BACKEND=gl cargo test -p wisp-app --lib -- --ignored
# Requires pactl and paplay. Creates/removes its own null sink; no speaker playback.
cargo test -p wisp-audio --test capture -- --ignored
```

CPU tests cover band separation at 44.1/48/96 kHz, normalization, antiphase stereo,
silence, DC, invalid samples, gain, decay, stale input, settings checks, graph limits,
pruning, and project round trips. GPU tests check history against a CPU recurrence,
particle persistence/determinism/isolation, mouse movement, live controls, paused
redraws, and embedded-preview/export parity with synthetic audio analyzed by the
real FFT. Native capture tests check an isolated 100 Hz stereo source, return to
silence, zero dropped frames, and disabling the service.

## Platform limits

Wayland background surfaces remain input-transparent; standard Wayland does not
provide global cursor coordinates to them. Their mouse input stays centered.
Window/app previews have local pointer input, and Windows desktop hosts have global
cursor input. Persistent effects run on all these hosts regardless of pointer
availability. Default-device following is polling based, and server/device failure
recovery depends on CPAL returning from backend operations. Native Windows capture,
desktop rendering, and DirectX 12 parity still require execution on Windows.
Layer controls, masks, deformation, and rain/ripple presets are implemented in
[Phase 9.5](phase-9.5.md).

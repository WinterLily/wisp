# Wisp Implementation Progress

Last updated: 2026-09-06.

Source of requirements: [wisp-spec.md](wisp-spec.md), particularly Section 50,
Development Phases. This document records implemented behavior and verified
results separately from planned work.

## Current status

**Phase 1's renderer functionality is implemented and tested locally on Linux.**
The phase's cross-platform success criterion remains open until the same wallpaper
has been rendered and compared on Windows and Linux.

**Phase 2 and Phase 3 implementation is complete on Linux.** Portable packages now
support fragment and compute passes, textures, parameters, intermediate targets,
and storage buffers. `wispd` presents them through a shared Wayland layer-shell
host on Niri and Hyprland, with multiple monitors and live wallpaper assignment.

Local tests cover Vulkan/OpenGL, nested Niri, and Hyprland headless outputs.
Physical monitor soak testing and Windows/DirectX 12 parity remain unverified.
**Phase 4 implementation is now present.** `wisp-host-windows` provides native
Explorer desktop attachment and multi-monitor presentation through the shared
runtime. Windows GNU cross-compilation and Clippy passed on Linux. Native Windows
desktop operation and the Phase 4 Linux/Windows rendering success criterion remain
unverified; dedicated CI jobs and a native smoke test are now provided.
**Phase 5 implementation is complete.** `wisp-ir` checks typed authoring graphs;
`wisp-compiler` lowers live nodes into fused WGSL and explicit pass boundaries.
`wispc` compiles project directories into validated portable packages. The fused
example uses one pass and the blur example uses three.
**Phase 6 implementation is complete.** `wisp` / `wisp-app` provide an Iced library,
persistent settings, package/project import, export, selection, per-monitor
assignment, parameter controls, and a live GPU preview using `wisp-runtime`.
The daemon continues independently after the GUI closes. CPU tests, Vulkan/OpenGL
GPU and window checks, nested Niri assignment integration, and Windows GNU
cross-compilation passed. Native Windows execution remains unverified.
**Phase 7 implementation is complete.** Easy Designer imports images, generates
standard graphs from four effect presets, exposes live controls, previews through
`wisp-runtime`, saves/reopens editable projects, installs creations in the library,
and exports portable packages. CPU tests, Vulkan/OpenGL preview and window checks,
and Windows GNU cross-compilation passed. Audio service is implemented in Phase 9; Workshop remains later work.

**Phase 8 implementation is complete.** The Visual Graph page provides a node canvas,
typed connections, node search, properties, compiler diagnostics, and runtime previews.
It opens standard/Easy Designer projects, saves incomplete editable drafts with layout
and assets, and exports/installs compiled packages. All 68 CPU tests and 12 GPU tests
on each local Vulkan/OpenGL backend passed, as did Windows GNU cross-compilation.
Native Windows execution and the earlier physical-monitor/parity gaps remain open.

**Phase 9 implementation is complete.** `wisp-audio` captures system output through
CPAL, analyzes normalized FFT bands once per daemon, and shares them across monitors.
The app and window preview expose the same inputs. Visual Graph authors persistent
particle emitters, mouse-distance inputs, and temporal feedback, compiled into the
existing runtime format. CPU, Vulkan/OpenGL GPU/window, isolated Linux capture, nested
Niri, and Windows GNU cross-compilation checks passed. Native Windows audio/desktop
execution and earlier physical-monitor/parity gaps remain open.

**Phase 9.5 implementation is complete.** Easy Designer now supports ordered image
layers, independent transforms, transparent blend modes, imported/painted masks,
localized hair/clothing deformation, wave/warp/displacement, and droplet/rain/ripple
presets. Equivalent typed nodes lower through the existing compiler and shared
runtime. Editable sources round-trip and exported packages reproduce preview output.
CPU, Vulkan/OpenGL GPU/window, nested Niri, and Windows GNU checks passed. Native
Windows/DirectX 12 parity and earlier physical-monitor verification remain open.

## Implemented

### Workspace and architecture

- Rust workspace using edition 2024, with a declared minimum Rust version of 1.88.
- `wisp-format`: versioned metadata/program models and bounded portable ZIP loading/export.
- `wisp-ir`: versioned typed authoring graph, dependency checks, live topological order.
- `wisp-compiler`: graph lowering, shader fusion, pass generation, package validation.
- `wispc`: CPU-only graph project compilation/checking and portable package export.
- `wisp-app` / `wisp`: Iced application, persistent library/settings, import/export, assignment, live preview.
- `wisp-runtime`: platform-independent shader validation, GPU resources, and ordered pass execution.
- `wisp-audio`: host-owned CPAL capture, bounded sample transfer, FFT and normalized bands.
- `wisp-preview`: ordinary window host, input handling, frame scheduling, and CLI.
- `wisp-host`: cross-platform assignment configuration and daemon options.
- `wisp-host-wayland`: Linux layer-shell presentation, monitor mapping, config reload.
- `wisp-host-windows`: native Explorer attachment, monitor mapping, and presentation.
- `wispd`: independent wallpaper daemon and assignment CLI.
- wgpu 27 and Naga 27, with dependency versions recorded in `Cargo.lock`.
- No window-system, editor, or Steam dependency in the runtime crate.
- Window previews and headless snapshots execute the same runtime render method.

### Portable packages (initial Phase 2 subset)

- Independently checked package format, program format, shader ABI, and GPU profile
  versions, plus minimum application version compatibility.
- Strict Serde manifest and serialized image-resource/fragment-pass descriptions.
  Program version 1 retains exactly one image and one presentation pass. Version 2
  adds ordered image bindings and typed parameters; version 3 adds intermediate targets
  and multiple fragment passes; unsupported types, unknown
  fields, and invalid resource references fail safely.
- `.wisp` archives with Stored or Deflate compression, loaded into memory without
  filesystem extraction. Local headers are checked against central records.
- Configurable archive, entry-count, file, total uncompressed, metadata, and shader
  byte limits. Counts and header bounds are checked before per-file decoding.
- Portable path checks, duplicate/case-collision rejection, CRC/size checks, and
  rejection of symlinks, special files, overlapping entries, encryption, ZIP64,
  streamed data-descriptor entries, and unreferenced files.
- Runtime validation of packaged WGSL, background images, and optional previews
  before GPU creation. Packages execute through the existing shared renderer.
- `--package` works with window preview, `--check`, `--output`, and `--frames`.
- `--export` packages the embedded example or supplied shader/image with identifier,
  display name, and author options. It validates inputs and requires a new output
  file, preserving existing packages. The background doubles as the preview image.
- [docs/package-format.md](docs/package-format.md) documents the implemented schema,
  archive restrictions, validation split, limits, and third-party tooling.

### Multiple textures and parameters (Phase 2 continuation)

- Program/shader ABI pair `(2, 2)` alongside backward-compatible `(1, 1)` loading.
  Archive format and GPU profile remain version 1; mixed version pairs are rejected.
- Up to eight images in explicit pass binding order, independent of resource order.
  Duplicate IDs/bindings, missing references, and unbound resources are rejected.
- Up to 64 float, integer, boolean, linear RGBA color, or 2/3/4-component vector
  parameters, with finite-value checks and optional inclusive scalar bounds.
- Generated `params.pN` fields in 16-byte slots at group 2, binding 0. Integers stay
  i32, booleans use u32, and vector lanes retain f32 precision. Explicit padding
  preserves layout through Vulkan and OpenGL shader translation.
- Initial overrides through `Shader::set_parameter`; live updates through
  `Renderer::set_parameter` upload only the changed slot. Invalid updates leave
  the previous value intact; no pipeline rebuild is required.
- `Renderer::from_wallpaper` loads all bound images and initializes parameters.
  Each renderer owns its parameter state. Existing single-image construction works.
- Repeated `--param ID=VALUE` overrides for package checks, snapshots, and window
  previews; overrides never rewrite the package and are not persisted.
- All bound images are CPU-validated, with a fixed 256 MiB aggregate decoded RGBA
  budget and device texture/sampler limits checked before GPU allocation.
  Temporary decoding memory can exceed the aggregate budget by one image.
- [examples/texture-blend/build-package.py](examples/texture-blend/build-package.py)
  creates a portable two-image example with blend, tint, animation, speed, and
  offset controls using only Python's standard library.

### Intermediate targets and multiple fragment passes (Phase 2 continuation)

- Program version 3 with shader ABI 2, preserving `(1, 1)` and `(2, 2)` packages.
  Archive format and GPU profile remain version 1.
- Up to 16 ordered fragment passes and 15 fixed-size intermediate targets, with
  dimensions from 1 to 4096. Each pass may sample up to eight images/targets.
- Resource references define dependencies. Validation rejects reads before writes,
  cycles, self-sampling, duplicate bindings, multiple writers, unused resources,
  invalid targets, and presentation before the final pass. No authoring graph executes.
- Every referenced shader is size/UTF-8 checked during loading and export; all
  shaders are Naga-validated before GPU creation, with pass-index diagnostics.
- Images upload once per resource and may be shared across passes. Targets, bind
  groups, pipelines, and per-pass globals/parameter buffers are allocated at load.
- Intermediate targets use linear `rgba8unorm` (clamped and quantized); images and
  presentation remain sRGB. Each pass clears/replaces its output without blending.
- Each pass receives its own output resolution. Other globals match across passes.
  Targets retain their declared size when the host output is resized.
- Initial overrides through `Wallpaper::set_parameter` and live updates through
  `Renderer::set_parameter` affect every pass. Invalid updates preserve all values.
  Wallpaper topology is private; the CLI uses constructors and read-only accessors.
- Host-selected `RuntimeLimits` cap texture dimensions, combined image/target storage,
  pass count, and target count. Defaults are 4096 pixels, 256 MiB, 16 passes, and 15
  targets. Limits are checked during CPU package loading and before GPU allocation.
  Estimates exclude driver overhead, uniforms, pipelines, and host-owned output.
- [examples/multipass-blur/build-package.py](examples/multipass-blur/build-package.py)
  builds a three-pass horizontal/vertical blur and animated comparison, with radius
  and strength parameters, using Python's standard library.

### Compute passes and storage resources (Phase 2 completion)

- Program/ABI pair `(4, 3)`, retaining versions 1–3. Procedural packages can omit images.
- Ordered compute passes with up to eight sampled textures, one RGBA8 storage output,
  four storage buffers, and fixed 8 × 8 × 1 workgroups. The final fragment presents.
- Explicit dispatch and buffer declarations; single producers, read-before-write,
  self-sampling, duplicate/unknown bindings, and incompatible versions are checked.
- Storage buffers contain `vec4<f32>` elements, are zero-initialized by wgpu, and
  persist per renderer. Later compute passes can read an earlier pass's writes.
- Compute receives the same globals and typed parameter slots, with per-target
  resolution and initial/live parameter changes. No graph or editor logic executes.
- Dispatch capped at 1,048,576 groups and 65535 per dimension; workgroup storage at
  16 KiB. Each buffer is at most 64 MiB; default storage total is 128 MiB and combined
  image/target/buffer storage is 256 MiB. Runtime policies can lower these limits.
- CPU validation precedes GPU allocation; GPU compute capabilities, texture/storage
  binding counts, buffer sizes, and dispatch limits are checked before creation.
- [Compute gradient example](examples/compute-gradient/build-package.py), schema,
  ABI, and resource limits documented in [package format](docs/package-format.md).

### Wayland host and daemon (Phase 3)

- Shared Niri/Hyprland layer-shell implementation, with one background surface per
  assigned monitor, all-edge anchoring, exclusive zone -1, no keyboard focus, and
  an empty input region. All GPU work uses `wisp-runtime` and a shared device.
- `wispd --list-outputs`, default `--package`, repeated `--assign OUTPUT=PACKAGE`,
  per-output parameter/FPS configuration through JSON, and CPU-only `--check`.
- Config paths resolve relative to the config directory; output names identify
  assignments. Config modification triggers validated replacement preparation
  before swapping; invalid reloads preserve the previous running wallpapers.
- Monitor discovery, hotplug/removal, output sizing, integer buffer scaling,
  surface recovery, per-output frame callbacks, and FPS caps. Unassignment destroys
  the surface; reassignments and reconnects restore it.
- Bounded config input and output counts. SIGINT/SIGTERM exit cleanly, with GPU and
  surface cleanup before display disconnection. `--frames` supports smoke tests.
- [Wayland guide](docs/phase-3.md) includes config schema, startup commands,
  verification, fractional scaling and global-pointer limitations.

### Windows host (Phase 4 implementation)

- Native `wisp-host-windows` adapter, selected by `wispd` on Windows. Package,
  validation, shader, and render-pass code is shared with Linux without ABI changes.
- Traditional WorkerW and raised Explorer desktop layouts; disabled, nonactivating
  child windows below icons, with transparent hit testing and no taskbar presence.
- Monitor interface IDs, GDI-name aliases, primary flags, signed physical bounds,
  DPI-aware child placement, and per-monitor cursor normalization.
- Independent package/parameter/FPS assignments through the shared `wisp-host`
  config schema; bounded CPU-only checks also work on Windows. Empty configs work
  at startup. The Wayland crate reexports the shared types for compatibility.
- Prepared replacement surfaces and renderers before assignment commit; monitor
  polling handles removal, reconnection, geometry, rotation, and DPI changes.
- Shared GPU device, sRGB surfaces, FIFO scheduling, surface-error recovery, and
  Explorer restart recovery. Native ownership tokens protect against recycled
  handles; surfaces are released before owned HWNDs on normal/error shutdown.
- Windows-only ignored native test covers monitor child geometry, attachment,
  input styles, focus preservation, and destruction without touching Explorer.
- CI transfers the same version 1–4 package files to Linux Vulkan and Windows
  DirectX 12, renders deterministic snapshots, and compares pixels. A standalone
  `compare-snapshots` example reports maximum/mean errors and fails outside tolerance.
- [Windows host guide](docs/phase-4.md) includes setup, monitor IDs, JSON/PowerShell
  commands, Explorer layout details, native tests, and remaining limitations.

### Wisp compiler (Phase 5)

- Independently versioned project/graph format 1, with node-ID edges and typed
  constants, parameters, global inputs, UV/image/color/composition operations.
- Immutable checked graph with node-specific diagnostics for duplicate/missing
  IDs, cycles, invalid outputs, incompatible types, values, and target dimensions.
  Disconnected nodes are checked before dead-node elimination.
- Reachability removes dead nodes, unused parameters, and unused assets before
  loading asset files or generating WGSL. Missing dead assets do not block builds.
- Fusable nodes lower into one WGSL function per needed node per pass. The image,
  parallax, brightness, tint, and vignette example uses one shader/pass/draw.
- Separable Gaussian blur emits input and horizontal intermediate passes, then
  fuses vertical filtering with downstream effects. Explicit `materialize` nodes
  add render-to-texture boundaries. Shared boundaries are emitted once; nested
  boundaries follow topological dependency order.
- Deterministic package generation using existing program 2/3 and ABI 2; package
  and runtime validation, including Naga, completes without GPU creation.
  No graph/compiler dependency was added to the runtime or daemon.
- `wispc PROJECT --check` and `--output NEW.wisp`, bounded project/asset reads,
  contained portable paths, new-file-only export, and compilation reports.
- Two editable projects, CPU/CLI regression tests, and three GPU tests for fused
  shader equivalence, blur kernel pixels, materialized multi-image composition,
  parameter pruning, and live parameter updates.
- CI now includes compiler GPU tests and generated graph packages in Linux Vulkan /
  Windows DirectX 12 parity comparisons. [Compiler guide](docs/phase-5.md) documents
  schema, node semantics, pass generation, limits, and remaining optimizations.

### Basic Iced application (Phase 6)

- `wisp` binary with Library, Import, and Settings pages; search, thumbnail cards,
  metadata, source, tags, content revision, pass count, and resource-memory estimates.
- Validated `.wisp` and graph-project imports run on background workers. Immutable,
  SHA-256-addressed installed packages, duplicate import reuse, new-file-only export,
  and a bounded versioned catalog. Originals and package parameter defaults remain intact.
- Atomic catalog/settings/assignment writes, checked content hashes, invalid-item
  diagnostics, and per-library OS locks. Platform local-data paths and `--data-dir`.
- Real `wisp-runtime` rendering on Iced's wgpu device into an sRGB GPU texture,
  composited inside the preview widget without live CPU readback. Time, pointer,
  parameters, multiple passes, and compute/storage use existing package ABIs.
- Live scalar, boolean, color, and vector controls, bounded sliders, reset to package
  defaults, validation errors that preserve valid values, and per-monitor persistence.
- Configurable desktop/default and preview FPS, saved appearance, manual preview
  pause, and animation pausing while unfocused or outside Library. Paused previews
  retain textures; parameter and size changes can redraw them.
- Structured `wispd --list-outputs --json` discovery, stable monitor IDs, default and
  individual assignment, clear/reassign, and a backward-compatible `disabled_outputs`
  config field for clearing one monitor while retaining an all-monitor default.
- Atomic config control channel, sibling daemon startup without a shell, managed
  daemon locks and process-status polling. GUI exit leaves desktop rendering running;
  daemon diagnostics go to `daemon.log`. Socket IPC and render acknowledgements remain later work.
- Host-owned-device runtime constructor retains validation. Iced's hardcoded device
  request is patched from two to four bind groups in a documented vendored dependency,
  allowing existing compute packages to preview without changing their ABI.
- CPU-only `wisp --check`, startup `--import`, frame-limited window checks, and
  optional app-only PNG screenshots. CI includes CPU import, embedded GPU tests,
  and an Xvfb application window artifact. [Application guide](docs/phase-6.md).

### Easy Designer (Phase 7)

- Create page with PNG/JPEG/WebP image import, file dialogs and path entry, wallpaper
  name/author, four presets, independent effect toggles, and bounded scalar sliders.
- Clean image, Gentle motion, Dreamy focus, and Breathing light presets generate
  standard typed graphs using image, parallax, time/sine, brightness, contrast,
  vignette, and blur nodes. Simple designs use one pass; blur designs use three.
- Current controls become exported parameter defaults; unused controls are pruned.
  Live sliders update preview uniforms without recompilation. Topology changes
  compile on background workers; failed replacement preserves the valid preview.
- Real runtime preview with time/mouse input, pause/resume, pass count and intermediate
  storage estimates. Blur preserves source aspect ratio with a 1024-pixel long-edge
  cap, limiting its two intermediate targets to at most 8 MiB.
- Editable project saving copies the source image and writes versioned `designer.json`
  alongside standard `project.json` and `graph.json`. Reopening rejects external graph
  edits rather than silently discarding them. The same folder compiles with `wispc`.
- Independent library installation and new-file-only `.wisp` export recompile current
  settings and validate the result. Project saving requires a new folder; errors
  preserve the draft. Source-image hashes give wallpaper IDs stable across edits.
- `wisp --create IMAGE_OR_DRAFT`, image routing for `--import` and the import path UI,
  CPU-only creation checks, and frame-limited designer window/screenshot checks.
- Reproducible `easy-presets` example builds all four projects/packages. CI includes
  their Linux/Windows parity fixtures and an Easy Designer application screenshot.
  [Easy Designer guide](docs/phase-7.md) documents workflow, persistence, and limits.
- Authoring remains entirely in the application and existing compiler; no new runtime
  format, runtime effect interpretation, or daemon/compiler dependency was introduced.

### Visual Graph (Phase 8)

- New Graph page and `wisp --graph PROJECT` startup mode; CPU-only `--check` compiles
  without installing the draft. Standard compiler projects and Easy Designer projects
  open directly; the current Easy Mode creation can be copied into an independent graph.
- Iced canvas with draggable nodes, background/middle-button panning, cursor-centered
  zoom, fit-to-view, dependency arrangement, type labels, and colored connection curves.
  Click output/input sockets to connect or replace inputs; properties support disconnect,
  delete, and active output selection. All 25 existing compiler operations are searchable.
- Connection edits reject cycles, invalid endpoints, and known incompatible types before
  mutation. Every committed graph/property change passes through the canonical typed
  checker and compiler. Incomplete graphs remain editable and saveable; diagnostics
  retain node/input, asset, shader, and resource-limit context from validation.
- Property controls expose constants, parameter defaults/types/bounds, image selection,
  and intermediate dimensions. Explicit Set/Enter commits let users enter partial numbers
  and vectors. Image import uses bounded decoding and retains all available source assets.
- Background compilation with revision-checked results, pass/resource reports, and the
  existing runtime preview. Failed compilation preserves the previous preview; runtime
  GPU replacements use existing prepare-before-swap behavior. Layout edits do not compile.
- New-folder project saves contain standard project/graph files, optional versioned
  `layout.json`, and copied assets. Incomplete drafts and missing asset placeholders
  round-trip. Existing paths are contained and bounded; output filenames are generated.
  Missing unused assets retain the compiler's existing dead-resource behavior.
- Package export and library installation recompile current defaults. Runtime, daemon,
  and package ABI are unchanged; authoring/layout data never executes in the runtime.
- [Visual Graph guide](docs/phase-8.md), CPU/GPU regressions, CLI checks, and a CI
  application screenshot command. Incremental compilation, undo/redo, multi-selection,
  custom nodes, and shader editing remain future work.

### GPU runtime

- GPU adapter/device selection with no optional device features requested.
- One fullscreen triangle and draw call per fragment pass; ordered passes share image resources.
- Persistent textures, globals/parameter buffers, bind groups, sampler, and render pipeline.
- PNG, JPEG, and WebP image decoding and GPU texture upload.
- sRGB texture input and sRGB output, with shader operations in linear RGB.
- Provisional shader ABI 1: resolution, mouse, time, delta time, frame index, and
  audio fields initialized to silence and populated by the Phase 9 host service.
- Explicit 48-byte host uniform layout, checked against the WGSL layout.
- Offscreen RGBA rendering and GPU readback with row-padding handling.

### Shader input and validation

- Custom WGSL input defining `fn wallpaper(uv: vec2<f32>) -> vec4<f32>`.
- Runtime-supplied fullscreen entry points, global uniforms, texture, and sampler.
- Naga parsing and validation before GPU pipeline creation.
- Rejection of extra resource bindings and additional shader entry points.
- GPU pipeline validation/allocation errors returned through wgpu error scopes.
- Bounded input loading: 1 MiB shader source and 32 MiB encoded images.
- Image dimensions limited to 4096 × 4096, with a 64 MiB decoder allocation limit.
  RGBA conversion can allocate an additional image buffer.

### Preview application

- Ordinary window with resizing and surface-loss/outdated-surface recovery.
- Normalized mouse input with an initial position at the center.
- Animated time and frame uniforms.
- Configurable frame-rate cap, defaulting to 60 FPS, and FIFO presentation.
- Rendering pauses for zero-sized or reported occluded windows.
- Escape and window-close handling.
- GPU and surface teardown before the Wayland display connection closes.
- CLI options for shader, image, dimensions, FPS, validation, PNG output, snapshot
  time, and automatic exit after a specified number of presented frames.

### Examples, documentation, and automation

- Embedded example shader with time-driven distortion, mouse parallax, and vignette.
- Original gradient texture and a reproducible Python standard-library generator.
- [README.md](README.md) with build, run, and verification commands.
- [docs/phase-1.md](docs/phase-1.md) describing the provisional ABI and runtime limits.
- GitHub Actions configuration for Linux/Windows checks and a separate Linux GPU
  matrix for Vulkan and OpenGL. Remote CI execution has not been verified in this
  implementation session.

### Audio and persistent interactivity (Phase 9)

- CPAL 0.18.2 output capture: native PulseAudio/PipeWire monitors on Linux and
  WASAPI loopback on Windows, with explicit device selection and bounded gain.
  Audio is disabled by default and never falls back to a microphone implicitly.
- A single daemon audio service serves every output. Callback conversion uses a
  bounded nonblocking queue; a worker owns Hann-windowed FFT and channel-power
  normalization. Bass/mid/treble/volume populate the existing 48-byte globals ABI.
- One to eight channels, 8–192 kHz, 4096 samples/1024-frame hops, DC rejection,
  attack/release smoothing, stale-result silence, overflow diagnostics, configuration
  reload, source/default-output polling, retry and clean shutdown.
- App source picker, sensitivity, meters/status and saved settings; live preview
  audio, `wispd --audio` / device discovery, standalone preview capture, and
  deterministic snapshot band inputs plus bounded 60 Hz simulation warmup.
- Graph Mouse distance, Particles, and Feedback nodes with typed sockets/properties,
  editable project round trips, shared-runtime previews and portable exports.
  Particle and feedback operations each own persistent storage, execute as ordinary
  compute/fragment passes, and reset only with a new renderer.
- Compiler enforcement of particle counts, simulation target dimensions, bounded
  particle/pixel workload, storage/pass/memory budgets and dead-state pruning.
  Creator reports include persistent storage estimates. No authoring dependency
  was added to the runtime or daemon; existing packages remain compatible.
- [Audio Fireflies project](examples/audio-reactive) combines mouse, audio, particles
  and fading trails. [Phase 9 guide](docs/phase-9.md) documents semantics, controls,
  capture requirements, limits, checks, and platform caveats.

### Layers and effects (Phase 9.5)

- Easy Designer image stack with front/back ordering, selection, visibility, naming,
  removal, opacity, position, independent scale, rotation, pivot, and alpha/add/
  multiply/screen/overlay blending. Transparent images clip at transformed bounds;
  blend math uses straight-alpha source-over in linear RGB.
- Imported grayscale masks and a continuous paint/erase brush with a layer image
  overlay. New-black/fill-white, inversion, bounded feathering, and independent
  composition/effect routing. Interactive painted masks use a 1024×1024 raster; imported masks
  retain source dimensions until painted. Grayscale coverage compensates for the
  shared sRGB texture decoding, including mid-gray coverage and image alpha.
- Typed Transform, Mask, Layer, Blend, Wave, Warp, Displacement and Weather nodes,
  available in the Graph catalog with typed sockets and editable properties.
- Hair/Clothing presets generate masked Wave/Warp graphs. Their automatic black
  masks keep the image stationary until regions are painted. Prepared transparent
  parts and local image deformation use the same graph path. Direction, amplitude,
  frequency, speed, and displacement maps/strength are configurable.
- Droplet/Rain/Ripple presets with density, size, speed, distortion strength, visible
  highlights/refraction, and mask regions. Analytic periodic trajectories and rings
  use fixed nine-cell work and require no persistent state. Outputs can feed Phase
  9 Feedback; persistent effects still use the existing compute/storage facilities.
- Layer projects save `layers.json` alongside standard graph/project files, Easy
  settings, source images, and mask rasters. Opening, Graph conversion, `wispc`,
  library installation, and `.wisp` export share the same lowering. Hidden resources
  are pruned from packages but preserved in projects; external graph edits are
  detected. Existing Easy Designer drafts remain supported.
- Six-layer authoring limit, bounded asset count, shader control clamps, and
  existing portable node/texture/pass/memory limits. Cost reports include source
  images, passes, targets, and persistent storage; Easy Designer warns above four
  passes or 64 MiB of estimated resource storage.
- [Layer guide](docs/phase-9.5.md), [editable example](examples/layers-effects), and
  a generator for eight effect project/package fixtures. CI includes these packages
  in Linux/Windows parity checks and captures the layer designer window. No runtime
  or daemon effect interpretation, new package ABI, or platform-specific shader
  implementation was introduced.

## Verification completed

The following results were obtained locally on Linux during initial implementation:

| Check | Result |
| --- | --- |
| `cargo build --workspace` | Passed |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed |
| `cargo test --workspace` | Seven CPU tests passed; GPU test ignored by default |
| Explicit GPU integration test | Passed on Vulkan |
| `cargo run -- --check` | Example shader and image validated without requesting a GPU |
| Headless PNG snapshot | Rendered at 960 × 540, animation time 3 seconds; visually inspected |
| Vulkan window smoke test | Presented 60 frames and exited successfully |
| OpenGL window smoke test | Presented 10 frames and exited successfully |

CPU tests cover shader diagnostics, forbidden resources/entry points, source-size
limits, image decoding and dimension limits, and host/WGSL uniform layout.

The GPU integration test verifies known corner colors from a 2 × 2 texture,
fullscreen coverage and image orientation, an unaligned 65-pixel readback width,
snapshot dimension rejection, and pixel changes from time and mouse updates.

A native crash found during the initial Wayland window test was traced to GPU/EGL
cleanup occurring after display disconnection. Releasing window state in the
event loop's `exiting` callback and ordering resource destruction fixed the crash;
subsequent Vulkan and OpenGL smoke tests passed.

Windows compilation/execution, DirectX 12 rendering, cross-platform pixel parity,
and execution on the declared minimum Rust version have not been verified locally.

### Verification for the package implementation

Completed locally on Linux in the continuation session:

| Check | Result |
| --- | --- |
| `cargo build --workspace` | Passed |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed |
| `cargo test --workspace` | 19 CPU tests passed; GPU test ignored by default |
| Explicit Vulkan GPU test | Passed, including exact pixel equality after package export/reload |
| CLI export and package check | Passed without requesting a GPU |
| Independent Python Deflate repack | Accepted and rendered |
| Loose versus packaged PNG at 320 × 180, time 3 | Byte-identical |
| Packaged Vulkan window smoke test | Presented 10 frames and exited successfully |

New tests exercise malformed archives and metadata, all four version identifiers,
minimum application versions, required-file/reference checks, unsafe paths,
conflicting ZIP headers, declared and actual decompression limits, invalid package
shader/image/preview content, and CLI export/reload/overwrite protection.
The current Windows, DirectX 12, full Profile 1, and minimum-Rust-version
verification gaps remain open. No remote CI run was initiated.

### Verification for multiple images and parameters

Completed locally on Linux in this continuation:

| Check | Result |
| --- | --- |
| `cargo build --workspace` | Passed |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed |
| `cargo test --workspace` | 25 CPU tests passed; three GPU tests ignored by default |
| Explicit Vulkan GPU suite | All three tests passed |
| Explicit OpenGL GPU suite | All three tests passed |
| Version 2 example build and CLI check with overrides | Passed |
| Version 2 PNG snapshot with blend/tint overrides | Rendered at 640 × 360, time 3; visually inspected |
| Version 2 window smoke tests | Vulkan and OpenGL each presented ten frames and exited |

New coverage checks binding order, version compatibility, every image's content,
parameter types and bounds, invalid overrides, host/WGSL parameter offsets,
exact pixels for two-image blending, live updates, unchanged state after rejection,
and the maximum eight-image/64-parameter bindings. Existing version 1 archive and
rendering tests remain passing.

OpenGL initially exposed a parameter layout mismatch when the generated struct
relied on WGSL `@size(16)` alone. Explicit scalar padding fixed it; the mixed-type
and maximum-binding GPU tests now pass on both local backends. CI is configured to
exercise both backends using Mesa, but no remote CI run was initiated. Windows,
DirectX 12, cross-platform parity, and the minimum Rust version remain unverified.

### Verification for intermediate targets and multiple passes

Completed locally on Linux in this continuation:

| Check | Result |
| --- | --- |
| `cargo build --workspace` | Passed |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed |
| `cargo test --workspace` | 29 CPU tests passed; four GPU tests ignored by default |
| Explicit Vulkan GPU suite | All four tests passed |
| Explicit OpenGL GPU suite | All four tests passed |
| Version 3 builder and CLI validation with radius override | Passed |
| Three-pass PNG snapshot | Rendered at 640 × 360, time 3; visually inspected |
| Version 3 window smoke tests | Vulkan and OpenGL each presented ten frames and exited |

New coverage includes malformed dependencies, target dimensions, compatibility,
later-pass source limits at import/export, later-pass shader diagnostics, configurable
texture budgets, and CLI validation. GPU coverage checks chained targets and reuse of
an earlier output, independently sized pass uniforms, linear intermediate color,
initial/live overrides across passes, rejected updates, renderer isolation, and
reuse across different snapshot sizes. Existing version 1/2 tests remain passing.

Windows/DirectX 12, cross-platform pixel parity, and the minimum Rust version remain
unverified. No remote CI run was initiated; the existing GPU jobs include the new test.

### Verification for Phase 2 completion and Phase 3

Completed locally on Linux in this continuation:

| Check | Result |
| --- | --- |
| `cargo build --workspace` | Passed |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed |
| `cargo test --workspace` | 33 CPU tests passed; five GPU tests ignored by default |
| Explicit Vulkan/OpenGL GPU suites | All five tests passed on each backend |
| Version 4 builder, CLI check and snapshot | Passed; 640 × 360 image visually inspected |
| Niri nested wallpaper surface | Ten frames and clean exit on Vulkan and OpenGL |
| Hyprland two headless outputs | Ten frames on both outputs and clean exit on both backends |
| Hyprland layer inspection | `wisp` namespace at background layer 0, full output dimensions |
| Different per-output parameters | Screenshot output differed as configured |
| Invalid/valid config reload | Invalid package preserved pixels; valid parameter edit changed pixels |
| Resize and scale | 1280 × 720 pixels, scale 2, 640 × 360 logical layer; screenshot inspected |
| Output removal/recreation | Disconnected surface released and assignment restored on reconnect |
| Unassign/reassign via config | Surface removed and recreated as configured |
| SIGTERM | Clean exit with status 0 |

New CPU coverage includes compute dependency/version errors, dispatch and memory
limits, undeclared resource rejection, shader diagnostics, assignment precedence,
and scaled output bounds. GPU coverage checks compute texture/buffer dependencies,
empty bind groups, live parameters, rejected updates, exact output pixels, and
renderer isolation. Existing package versions and fragment tests remain passing.

Hyprland's nested Wayland backend encountered an initial-configure protocol error;
verification used its working headless outputs. This is not evidence of physical
monitor or long-running session behavior. Windows/DirectX 12, cross-platform pixel
parity, and execution on Rust 1.88 remain unverified. No remote CI was initiated.

### Verification for Phase 4 implementation

Completed locally on Linux:

| Check | Result |
| --- | --- |
| `cargo build --workspace` | Passed |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed |
| `cargo test --workspace` | 37 CPU tests passed; five GPU tests ignored by default |
| Windows GNU workspace/all-target Clippy | Passed, including Windows native code and ignored test compilation |
| Explicit Vulkan/OpenGL GPU suites | All five tests passed on each backend |
| Version 1–4 package builders and `wispd --check` | Passed |
| Version 1–4 snapshots on Vulkan/OpenGL | Rendered at 321 × 181, time 3; comparison passed |
| Snapshot comparison negative case | Different wallpapers rejected |

New CPU tests cover negative monitor origins, portrait dimensions, cursor clamping,
invalid/overflowing bounds, stable-ID/alias/default precedence, and the daemon's
CPU-only config path and argument validation. Native HWND tests are Windows-only
and ignored by default. The cross-compile uses `x86_64-pc-windows-gnu`; it type-checks
Windows code but does not link a Windows executable or prove desktop behavior.

No native Windows/DirectX 12 test or remote CI run was performed. Traditional and
raised Explorer layouts, mixed-DPI physical monitors, Explorer restart, native
input behavior, and Windows/Linux pixel parity still need execution on Windows.
The minimum Rust version and physical Linux monitor soak tests remain unverified.

### Verification for Phase 5

Completed locally on Linux in this continuation:

| Check | Result |
| --- | --- |
| `cargo build --workspace` | Passed |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed |
| `cargo test --workspace` | 46 CPU tests passed; eight GPU tests ignored by default |
| Windows GNU workspace/all-target Clippy | Passed, including compiler and CLI |
| Explicit Vulkan/OpenGL GPU suites | All eight tests passed on each backend |
| Fused and blur project compilation / daemon checks | Passed without GPU creation |
| Generated package snapshots at 321 × 181, time 3 | Both rendered and visually inspected |
| Vulkan/OpenGL snapshot comparisons | Fused: identical; blur: maximum error 1/255, mean 0.000125/255 |
| Fused/blur window smoke tests | Ten frames and clean exit for both packages on both backends |

The six new compiler CPU tests cover multiple invalid graph cases, every initial
fusable node category, dead parameters/assets, procedural output, shared/nested
boundaries, package determinism, malformed images, and configurable limits. Three
CLI tests cover checks/export, overwrite protection, invalid/oversized projects,
unsafe paths, and escaping symlinks. Three GPU tests verify handwritten-shader
pixel equivalence, an independently calculated Gaussian kernel with quantization,
materialized multi-image composition, pruned parameter slots, and live updates.

Generated packages use existing runtime formats; no runtime/host code changes were
needed. Native Windows/DirectX 12 execution and cross-platform parity remain
unverified locally. CI includes the compiler examples and tests, but no remote CI
run was initiated. Rust 1.88 execution and physical-monitor soak tests remain open.

### Verification for Phase 6

Completed locally on Linux:

| Check | Result |
| --- | --- |
| `cargo build --workspace` | Passed |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed |
| `cargo test --workspace` | 54 CPU tests passed; ten GPU tests and one desktop test ignored by default |
| Windows GNU workspace/all-target Clippy | Passed, including Iced, file dialogs, and native host |
| Explicit Vulkan/OpenGL GPU suites | All ten tests passed on each backend |
| CPU-only application import/check/reopen | Passed without display or GPU |
| Blur and compute Iced window smoke tests | Twelve frames and clean exit on Vulkan/OpenGL |
| Application PNG screenshot | Captured through Iced and visually inspected |
| X11 application smoke test | Twelve preview frames and clean exit through nested Xwayland |
| Nested Niri daemon integration | Passed on Vulkan/OpenGL: JSON IDs, lock/start idempotence, saved assignment reload, eight rendered frames, exit and lock release |

Eight new CPU tests cover library installation/deduplication/export/reopen,
rejected imports and assignments, changed package detection, per-monitor defaults,
clearing/reassignment, settings limits, GUI/daemon locks, typed parameter input,
reset after saved overrides, failed selection preservation, and CLI operation.
Existing host/CLI tests also cover disabled monitor aliases and invalid config/JSON options.
Two new GPU tests compare embedded preview pixels against the standalone runtime
within 1/255, test clipping/scrolling, paused texture reuse, live controls, multiple
passes, and four-group compute/storage bindings. Existing eight GPU tests remain passing.

The ignored desktop test runs only in a disposable desktop and cleans up its daemon.
The Iced source patch is limited to its requested bind-group count. Native Windows
UI/dialog/desktop behavior, DirectX 12 parity, physical monitor soak tests, and Rust
1.88 execution remain unverified. CI changes have not been run remotely.

### Verification for Phase 7

Completed locally on Linux:

| Check | Result |
| --- | --- |
| `cargo build --workspace` | Passed |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed |
| `cargo test --workspace` | 60 CPU tests passed; eleven GPU tests and one desktop test ignored by default |
| Windows GNU workspace/all-target Clippy | Passed, including designer code and tests |
| Explicit application Vulkan/OpenGL GPU tests | All three passed on each backend, including all four designer presets |
| All four preset project/package generation | Passed |
| Dreamy draft / image Create window smoke tests | Vulkan/OpenGL each presented twelve frames and exited cleanly |
| Designer PNG screenshot | Captured through Iced and visually inspected |

Six new CPU tests cover preset compilation and parameter pruning, editable project
round trips without original images, byte-identical compilation through `wispc`,
library/export defaults and deduplication, overwrite protection, malformed inputs,
unsafe asset paths, failed preview replacements, rejected live values, save status,
and CPU-only CLI image creation/import. The new GPU test compares every preset's
embedded preview after live edits against its exported defaults within 1/255, and
checks time and mouse response independently. Existing application GPU tests pass.

Native Windows UI/file dialogs/desktop execution, DirectX 12 parity, physical monitor
soak tests, and execution on Rust 1.88 remain unverified. CI was updated but not run
remotely. No desktop assignments were changed during Phase 7 verification.

### Verification for Phase 8

Completed locally on Linux:

| Check | Result |
| --- | --- |
| `cargo build --workspace` | Passed |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed |
| `cargo test --workspace` | 68 CPU tests passed; twelve GPU tests and one desktop test ignored by default |
| Windows GNU workspace/all-target Clippy | Passed, including canvas and graph tests |
| Explicit Vulkan/OpenGL GPU suites | All twelve tests passed on each backend |
| CPU-only graph CLI check | Three-pass graph validated without display/GPU; no installation |
| Graph window smoke tests | Vulkan blur/fused and OpenGL fused each rendered twelve frames and exited |
| Graph screenshots | Captured and visually inspected; Fit added after initial canvas review |

Eight new CPU tests cover graph/layout/source round trips, byte-identical `wispc`
compilation, incomplete draft repair, cycle/type rejection, every catalog operation's
sockets, Easy Mode conversion, multiple images, unsafe paths and oversized metadata,
overwrite protection, canvas interaction events, partial value entry, stale/failed
compilation results, save status, and the graph CLI. One new GPU test compares edited
one-pass and three-pass graph previews with separately exported packages within 1/255.
An initial test compared mismatched image dimensions; correcting the reference dimensions
resolved it. Existing compiler/runtime/application GPU suites remain passing.

A subsequent sixty-frame OpenGL window run timed out after compilation while the
window was behind a fullscreen application; it did not establish a longer window run.
The twelve-frame window checks and all headless GPU checks passed. Native Windows UI,
DirectX 12 parity, physical monitor soak tests, and Rust 1.88 execution remain unverified.
CI was updated but not run remotely. No desktop assignments were changed.

### Verification for Phase 9

Completed locally on Linux:

| Check | Result |
| --- | --- |
| `cargo build --workspace` | Passed |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed |
| `cargo test --workspace` | 74 CPU tests passed; 15 GPU, one desktop, and one native audio test ignored by default |
| Windows GNU workspace/all-target Clippy | Passed, including WASAPI capture and simulation code |
| Explicit Vulkan/OpenGL GPU suites | All 15 passed on each backend |
| Native Linux audio integration | Isolated PulseAudio/PipeWire null-sink test passed: 100 Hz antiphase stereo, expected RMS, release to silence, no drops, disable/shutdown |
| Default Linux output capture | Opened the output monitor; clean shutdown |
| Interactive graph CLI and portable package validation | Passed without display/GPU/capture |
| Interactive Iced window | Vulkan/OpenGL each rendered 24 frames and exited; Vulkan screenshot inspected |
| Deterministic simulation snapshots | 640×360, 120 warmup frames and fixed audio; Vulkan/OpenGL pixel-identical, visually inspected |
| Nested Niri daemon | New particle/feedback package with audio enabled presented 120 frames on Vulkan and OpenGL, then exited cleanly; audio-only enable/disable reload preserved renderer state |

Six new CPU tests cover FFT bands/rates/channel power, DC/silence/invalid samples,
normalization/release, stale audio, capture configuration, simulation limits and
pruning, editable graph round trips and types, and snapshot argument parsing.
Existing CLI tests additionally check audio settings and CPU-only validation.
Three new GPU tests cover exact feedback recurrence, floating history, pause/resize,
particle state/isolation/reload/mouse, live controls, and embedded-preview/export
parity with synthetic audio through the real FFT. The Linux native capture test is
opt-in and creates/removes its own null sink without speaker playback.

A snapshot argument check exposed comma-separated input counting; the parser was
corrected and a regression test added. An initial two-second default-capture meter
ended during backend startup; a longer run connected and the bounded capture test
passed. No desktop assignments or default audio routes were changed. Native Windows
WASAPI/desktop execution, DirectX 12 parity, physical-monitor soak tests, and Rust 1.88
execution remain unverified. CI was updated but not run remotely.

### Verification for Phase 9.5

Completed locally on Linux:

| Check | Result |
| --- | --- |
| `cargo build --workspace` | Passed |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed |
| `cargo test --workspace` | 78 CPU tests passed; 18 GPU, one desktop, and one native audio test ignored by default |
| Windows GNU workspace/all-target Clippy | Passed, including layer UI, graph operations, and tests |
| Vulkan/OpenGL GPU checks | New blend/transform, mask coverage, and all-effect preview/export tests passed on each backend; existing suites remain passing |
| Eight effect project/package round trips | Every effect compiled; reopened drafts and `wispc` produced identical package bytes |
| Deterministic Vulkan/OpenGL snapshots | All eight effect fixtures at 321×181, time 3: maximum channel difference ≤1/255 |
| Layer designer window | Vulkan/OpenGL each rendered 24 frames and exited cleanly; Vulkan screenshot visually inspected |
| Nested Niri wallpaper | Layered character/hair/rain package presented 120 frames on each Vulkan/OpenGL backend and exited cleanly |

Four new CPU tests cover all effects/blends, editable source retention, export
identity and overwrite protection, paint/erase/import, visibility pruning, invalid
controls and brush inputs, external graph edit detection, new sockets/types,
layer-to-persistent-feedback compilation, and canvas stroke/release behavior.
Three new GPU tests cover straight-alpha blending against a CPU reference,
transformed clipping, all-effect preview/export parity, animation with black
anchors, mask inversion, feathering, and perceptual grayscale coverage.

The layer controls and preview were visually inspected in the application window.
No existing desktop assignments were changed. Native Windows UI/desktop/DirectX 12 execution,
remote CI, physical-monitor soak tests, and Rust 1.88 execution remain unverified.

### Phase 9.5 follow-up: live painting and water rendering

- Corrected the painter's per-stroke source-handle replacement and encoded-image
  churn. The base image now uses a stable decoded RGBA handle; a retained 1024×1024
  mask updates only the stroke's bounds, with an immediate teal overlay. Painting
  continues while preview compilation is in progress. Changed masks are snapshotted
  at most once every two seconds, with encoding/compilation on a background worker.
  Save/export, layer switching, and Graph conversion preserve pending strokes.
- Added Expand mask editor / Back to layers. The expanded surface fills the working
  area, preserves source aspect ratio, and maps brush input through the fitted image
  rectangle. Brush radius remains circular; letterbox margins do not receive strokes.
- Replaced outline-style weather with curved water caps, water-ratio refraction,
  directional highlights, window/sky reflections, Fresnel edge reflection,
  randomized sliding rain drops, small beads, and wet trails. Refraction samples
  the composite through the selected layer, including lower layers underneath a
  transparent foreground, and clamps viewport edges. Work remains bounded to nine
  neighboring cells with no runtime effect interpreter.
- Layer source format 2 records the updated composition order. Old projects are
  checked against their original generated graph, then upgraded when opened; saving
  the upgraded project writes the new graph. Existing portable packages are unchanged
  until re-exported.
- Added regressions for stable painter textures, strokes during compilation,
  two-second throttling, persistence of pending strokes, expanded aspect/input
  mapping, legacy migration, and lower-layer refraction isolated from lighting.
  All 81 CPU tests pass; Vulkan/OpenGL GPU checks include the new lower-layer
  refraction test. Linux and Windows GNU all-target Clippy pass. Droplet/rain images
  and an instrumented expanded-editor screenshot with a painted stroke were visually
  inspected. The normal OpenGL application rendered 24 preview frames and exited;
  fixed-time droplet/rain snapshots on Vulkan and OpenGL differ by at most 1/255
  per channel. Native Windows execution and earlier physical-monitor verification
  remain open.

### Follow-up: unrestricted source-image imports

- Removed fixed file-byte, dimension, decoder-allocation, and aggregate source-byte
  caps from Easy Designer, layer/mask/displacement imports, Graph imports, saved
  project reopening, and `wispc` source loading. Regular-file, supported-format,
  path-containment, metadata, and asset-count validation remain in place.
- Projects retain original image/displacement bytes and full-resolution grayscale
  masks. Compilation prepares separate portable copies, preserving aspect ratio
  and alpha, with budgets accounting for shaders, intermediate textures, and
  persistent storage. Existing compatible images retain their encoded bytes.
  Editor and CLI reports identify resolution reductions. Runtime/package limits
  continue to apply to playback assets; local decoding depends on available RAM.
- Regression coverage imports a 5000-pixel, 33 MiB PNG through Easy, layers, masks,
  displacement, and Graph; saves/reopens sources; checks alpha; and validates
  identical app/CLI compilation and usable export. Additional checks cover wide
  JPEG/WebP exports, prepared paths, encoding preservation, and texture/byte budgets.
- All 88 CPU tests pass, as do all-target Clippy on Linux and Windows GNU and the
  workspace build. The eight application GPU tests pass on Vulkan and OpenGL.
  An actual 7680×4320 PNG was opened in the OpenGL application, rendered twelve
  preview frames, and captured for visual verification, including its source
  dimensions and playback-resize notice. No desktop assignments were changed.

## Roadmap status

| Phase | Scope from the spec | Status |
| --- | --- | --- |
| 1 | Renderer proof of concept | Implemented on Linux; Windows/parity verification pending |
| 2 | Portable Wisp runtime | Implemented; CPU and Vulkan/OpenGL verification passed |
| 3 | Niri and Hyprland desktop hosts | Implemented; Niri nested and Hyprland headless verification passed |
| 4 | Windows desktop host | Implemented; Windows cross-compilation passed; native execution/parity pending |
| 5 | Wisp compiler | Implemented; CPU and Vulkan/OpenGL verification passed |
| 6 | Basic Iced application | Implemented; CPU, Vulkan/OpenGL, window and nested Niri integration checks passed; native Windows pending |
| 7 | Easy Designer | Implemented; CPU, Vulkan/OpenGL preview/window checks and Windows cross-compilation passed |
| 8 | Visual graph editor | Implemented; CPU, Vulkan/OpenGL GPU/window checks and Windows cross-compilation passed |
| 9 | Audio and advanced interactivity | Implemented; CPU, Vulkan/OpenGL, native Linux audio, nested Niri and Windows cross-compilation checks passed |
| 9.5 | Layers and effects: masks, image deformation, hair/clothing motion, droplets, rain, and ripples | Implemented; CPU, Vulkan/OpenGL preview/export/window, nested Niri and Windows cross-compilation checks passed |
| 10 | KDE Plasma adapter | Not started |
| 11 | Steam Workshop | Not started |
| 12 | Advanced shader authoring | Not started; CLI WGSL loading exists, but no editor or hot reload |

## Current limitations

- Layers, blending, masks, image deformation, and weather presets are implemented.
  Easy Designer permits six layers and the shared runtime permits eight referenced
  image resources, including masks/maps. Interactive painted masks use a 1024×1024 raster and
  nine-tap feathering. Undo/history, skeletal rigging, strand-level hair physics,
  automatic subject segmentation, and spatial fluid/ripple simulation remain
  future work. Weather presets use bounded analytic animation.

- The initial compiler emits fragment programs from the documented node catalog.
  Constant folding, common-expression/sample elimination, automatic caching,
  target reuse, bloom, general-purpose compute authoring, and incremental compilation
  remain later work. `materialize` provides explicit caching. Blur radius controls
  fixed five-tap spacing; intermediate textures retain existing size/color limits.
  Package previews use the first image, not a rendered thumbnail. Procedural
  graphs receive a one-pixel background for the existing fragment ABI.

- Compute, persistent buffers, particle authoring and per-pixel temporal feedback are implemented.
  Spatial texture feedback and general-purpose simulation authoring remain future work. Intermediate targets have fixed dimensions
  and linear 8-bit storage; relative resolution policies,
  HDR and target reuse remain planned; layer blend modes compile into shaders. Parameter enum choices and
  slider/editor metadata remain planned.
- The versioned package/program format currently wraps the existing WGSL function
  interfaces. Ordered fragment/compute dependencies are implemented; additional
  buffer formats and generic compute output layouts remain future extensions.
- The application imports `.wisp`, compiled graph projects, and images through Easy
  Designer. WGSL editing, library removal, and Workshop remain later work. Easy Mode
  offers layer/mask/deformation/weather and image/parallax/time/brightness/contrast/vignette/blur presets;
  bloom, particle presets in Easy Mode, arbitrary parameter declarations, undo/history, and rendered
  thumbnails remain future work. Editable project saves require a new folder name. Installed revisions
  use content hashes because the package manifest has no wallpaper release-version field.
  Resource estimates are not GPU timings. The catalog loads eagerly and is bounded
  to 4096 entries; lazy loading and orphan collection remain future improvements.
- File-based assignment control is implemented; socket IPC, per-renderer acknowledgement,
  desktop pause/resume policy, automatic crash recovery, and monitor spanning remain
  future work. Unmanaged daemon instances are not detected by managed per-library locks.
  The GUI shows saved assignments and daemon process status; detailed load errors are in the log.
  Wayland desktop
  pointer globals stay centered; Windows samples physical cursor coordinates.
  Wayland fractional scales use integer buffers and compositor resampling.
- Windows desktop attachment uses undocumented Explorer conventions. Native behavior
  is unverified, including raised desktop layered-window presentation. A single GPU
  device serves all monitors; multi-adapter presentation depends on driver support.
  FIFO presentation on a slow output can delay others. Device-loss restart remains
  future work; Explorer loss is retried but does not recover a failed GPU device.
- Audio capture and FFT are implemented. Capture is opt-in; Linux requires PulseAudio
  or pipewire-pulse, and native Windows WASAPI behavior remains unverified. Source
  changes/recovery poll every five seconds. Rich spectrum/beat inputs remain future work.
- Particle state is f32; its sampled position texture is RGBA8. Particle targets/counts
  are bounded, radial emitters have a three-second lifetime, and feedback is per-pixel
  temporal peak hold. Spatial feedback, richer simulation behaviors, HDR accumulation,
  and general-purpose buffer editing remain future extensions.
- Images stretch to the viewport in the example shader; automatic aspect-ratio
  fitting is not implemented.
- Occlusion-based pausing depends on window-system reporting. Desktop idle,
  fullscreen applications, session locking, and display sleep are not detected.
- Package structure and encoded byte limits are validated. Full GPU Profile 1
  enforcement, complete GPU-memory accounting, crash isolation, and GPU watchdogs
  remain unimplemented. Configurable texture/buffer/combined-memory/pass/dispatch limits are available.
  Naga validation does not bound execution time.
- Persistent render resources are reused, but zero heap allocation per frame and
  long-running performance targets have not been measured or established.

## Next implementation steps

1. Execute the Windows desktop verification matrix in `docs/phase-4.md`, including
   physical mixed-DPI monitors, both Explorer layouts, restart, and input transparency.
2. Run the new DirectX 12 and Linux/Windows package-parity CI jobs; close Phase 1/4
   cross-platform verification and address any platform-specific findings.
3. Physical Niri/Hyprland multi-monitor soak tests, including sleep/wake and fractional
   scaling; add dedicated fractional-scale support if required by measured fidelity.
4. Execute native Windows WASAPI tests (system output, source switching, disconnect
   recovery) and the new persistent-simulation/FFT preview tests on DirectX 12.
   Relative targets, HDR, spatial texture feedback, watchdogs, and full GPU-cost
   accounting remain later runtime improvements.
5. Begin Phase 10 KDE integration when requested, using a thin Plasma adapter around
   the shared runtime. Phase 9.5 layer/effect implementation is complete; retain the
   new effect fixtures in native Windows parity and physical-monitor verification.

## Commands for continuing work

```sh
# Layers, paintable masks, localized motion, and weather.
cargo run -p wisp -- --create examples/layers-effects
cargo run -p wispc -- examples/layers-effects --output layers.wisp
cargo run -p wispd -- --package layers.wisp
cargo run -p wisp-app --example layer-effects -- /tmp/layer-examples

# Audio, mouse-driven particles and feedback.
cargo run -p wisp -- --graph examples/audio-reactive
cargo run -p wispc -- examples/audio-reactive --output audio-reactive.wisp
cargo run -- --package audio-reactive.wisp --audio
cargo run -p wispd -- --package audio-reactive.wisp --audio
cargo test -p wisp-audio --test capture -- --ignored

# Visual Graph: standard projects, typed wiring, and compiler diagnostics.
cargo run -p wisp -- --graph examples/graph-blur
cargo run -p wisp -- --graph examples/graph-fused --check

# Easy Designer: image import, saved drafts, and reproducible preset packages.
cargo run -p wisp -- --create examples/shader-basic/background.png
cargo run -p wisp -- --create /path/to/saved-project
cargo run -p wisp -- --create examples/shader-basic/background.png --check
cargo run -p wisp-app --example easy-presets -- /tmp/easy-presets

# Basic Iced application (build both app and sibling daemon).
cargo build --workspace
cargo run -p wisp
cargo run -p wisp -- --import examples/graph-blur
cargo run -p wisp -- --data-dir /tmp/wisp-library --import examples/graph-fused --check
cargo run -p wisp -- --data-dir /tmp/wisp-ui --import examples/graph-blur --frames 12 --screenshot /tmp/wisp-ui.png

# Compile typed graph projects to portable wallpapers.
cargo run -p wispc -- examples/graph-fused --check
cargo run -p wispc -- examples/graph-fused --output fused.wisp
cargo run -p wispc -- examples/graph-blur --output graph-blur.wisp
cargo run -- --package graph-blur.wisp --param radius=4

# Interactive example.
cargo run --release

# Validate source and image without a GPU.
cargo run -- --check

# Export and validate a portable package (destination must be new).
cargo run -- --export wallpaper.wisp --name "My Wallpaper" --author "Your Name"
cargo run -- --package wallpaper.wisp --check
cargo run -- --package wallpaper.wisp --frames 10

# Build a version 2 package and override its parameters.
python3 examples/texture-blend/build-package.py blend.wisp
cargo run -- --package blend.wisp --check --param blend=0.6
cargo run -- --package blend.wisp --param 'tint=[0.5,1,0.8,1]' --frames 10

# Build a version 3 package with two intermediate targets and three passes.
python3 examples/multipass-blur/build-package.py blur.wisp
cargo run -- --package blur.wisp --check --param radius=4
cargo run -- --package blur.wisp --frames 10

# Build a compute/storage wallpaper and assign it to the desktop.
python3 examples/compute-gradient/build-package.py compute.wisp
cargo run -- --package compute.wisp --check
cargo run -p wispd -- --list-outputs
cargo run -p wispd -- --package compute.wisp --frames 10
cargo run -p wispd -- --config assignments.json

# Windows desktop host (PowerShell); use names/IDs printed by --list-outputs.
# cargo run -p wispd -- --package compute.wisp --frames 60
# cargo test -p wisp-host-windows -- --ignored

# Type-check the Windows workspace from Linux.
cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings

# Deterministic inputs for a snapshot comparison.
cargo run --release -- --output preview.png --width 1280 --height 720 --time 3

# Window presentation and shutdown check.
cargo run -- --frames 60

# CPU and static checks.
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Explicit GPU verification; requires a working graphics adapter.
cargo test -p wisp-runtime --test gpu -- --ignored
cargo test -p wisp-compiler --test gpu -- --ignored
```

# Phase 5: Wisp compiler

`wispc` compiles editable graph projects into portable `.wisp` archives. The
runtime and daemon receive only resources, WGSL functions, typed parameter
slots, and ordered fragment passes. Neither depends on the authoring crates.

```sh
cargo run -p wispc -- examples/graph-fused --check
cargo run -p wispc -- examples/graph-fused --output fused.wisp
cargo run -- --package fused.wisp --param brightness=1.3

cargo run -p wispc -- examples/graph-blur --output blur.wisp
cargo run -- --package blur.wisp --param radius=4
cargo run -p wispd -- --package blur.wisp --check
cargo run -p wispd -- --package blur.wisp
```

Output destinations must be new files. Compilation, image decoding, Naga
validation, and runtime limit checks complete before the destination is created.
`--check` performs the same compilation without writing a file or requesting a GPU.
An invalid project exits with a diagnostic and does not replace existing packages.

## Editable project format 1

A project directory contains `project.json`, `graph.json`, and `assets/`.
It is distinct from a runtime package; graph and project files are not exported.

`project.json`:

```json
{
  "version": 1,
  "id": "example.wallpaper",
  "name": "My Wallpaper",
  "author": "Your Name",
  "description": "Optional description",
  "tags": ["image"],
  "assets": {
    "background": { "path": "assets/background.png" }
  }
}
```

`description`, `tags`, and `assets` may be omitted. Image paths are relative to
the project directory and must be under `assets/`, using forward slashes and a
lowercase PNG, JPEG (`jpg` or `jpeg`), or WebP extension. Absolute paths, parent
traversal, Windows drive paths, and symlinks escaping the directory are rejected.
Files must be regular files. The directory should not be modified concurrently
with compilation; canonical path checks are not a filesystem sandbox against
another process racing file replacement.

`graph.json`:

```json
{
  "version": 1,
  "parameters": [
    { "id": "gain", "default": { "type": "float", "value": 1.0 }, "min": 0, "max": 4 }
  ],
  "nodes": [
    { "id": "uv", "op": { "type": "uv" } },
    { "id": "image", "op": { "type": "image", "asset": "background", "uv": "uv" } },
    { "id": "gain", "op": { "type": "parameter", "parameter": "gain" } },
    { "id": "bright", "op": { "type": "brightness", "color": "image", "amount": "gain" } },
    { "id": "output", "op": { "type": "output", "color": "bright" } }
  ],
  "output": "output"
}
```

Node order does not need to follow dependency order. Input fields refer to node
IDs, not array indices. IDs contain 1–128 ASCII letters, digits, hyphens, periods,
or underscores. Every node has one typed output; the selected graph output must
be an `output` node and output nodes cannot feed other nodes. `parameters` may be
omitted. Parameter declarations use the existing [package schema](package-format.md).

Graph/project versions are checked independently of package/program/ABI versions.
Unknown node types, fields, parameters, missing edges, cycles, duplicate IDs,
non-finite constants, and incompatible input types fail compilation. All nodes
are checked, including disconnected nodes. After checking, nodes unreachable
from the selected output, unused parameters, and unused assets are removed.
Unused asset files are not opened and need not exist.

## Initial node catalog

Types are `Float`, `Integer`, `Boolean`, `Vector2`, `Vector3`, `Vector4`, and
`Color`. Color and Vector4 are distinct graph types even though both lower to
WGSL `vec4<f32>`. There are no implicit scalar/vector or color/vector conversions.

| Node `type` | Fields and input types | Output / behavior |
| --- | --- | --- |
| `constant` | `value`: typed parameter value | Value's type; finite values required |
| `parameter` | `parameter`: declaration ID | Declaration's type; retained parameters remain live uniforms |
| `uv` | None | Vector2; normalized current pass coordinates |
| `time`, `delta_time` | None | Float; seconds |
| `mouse`, `resolution` | None | Vector2; runtime mouse and current pass pixel resolution |
| `audio_bass`, `audio_mid`, `audio_treble`, `audio_volume` | None | Float; normalized host audio from the [Phase 9 service](phase-9.md) |
| `add`, `multiply` | `a`, `b`: same floating scalar/vector/color type | Component-wise operation |
| `sin` | `input`: Float | Float |
| `translate` | `uv`, `offset`: Vector2 | Vector2; UV plus offset |
| `parallax` | `uv`: Vector2, `strength`: Float | Vector2; UV plus `(mouse - 0.5) * strength` |
| `image` | `asset`: asset ID, `uv`: Vector2 | Color; linear RGB from the sRGB image |
| `brightness` | `color`: Color, `amount`: Float | Color; multiply RGB, preserve alpha |
| `contrast` | `color`: Color, `amount`: Float | Color; scale RGB around linear 0.5, preserve alpha |
| `tint` | `color`, `tint`: Color | Color; multiply RGBA |
| `vignette` | `color`: Color, `uv`: Vector2, `strength`: Float | Color; multiply RGB by clamped `1 - dot(uv - 0.5, uv - 0.5) * strength` |
| `mix` | `a`, `b`: Color, `factor`: Float | Color; linear interpolation (factor is not clamped) |
| `blur` | `color`: Color, `radius`: Float, `width`, `height`: integers | Color; separable Gaussian, two intermediate passes |
| `materialize` | `color`: Color, `width`, `height`: integers | Color; explicit render-to-texture boundary, one intermediate pass |
| `output` | `color`: Color | Final presentation |

Integer, boolean, and some vector constants/parameters are represented and type
checked for future node additions; the initial arithmetic/effect nodes only
consume the types listed above. The graph authoring API does not imply an editor
canvas or audio capture, which belong to later phases.

## Lowering and fusion

`wisp-ir` owns serialization and graph type checking. `Graph::check` returns an
immutable `TypedGraph` with output types and a topological order of live nodes.
`wisp-compiler::compile` accepts that graph, encoded assets, a manifest, and
package/runtime limits. `compile_project` adds bounded project directory loading.
Both return a validated `Package` plus a compilation report.

Fusable operations become WGSL helper functions in one shader. Function names
use compiler indices, and generated comments retain node IDs for diagnostics.
Each live node contributes one function per pass that needs it. Shader fusion
avoids allocating a render target or draw call for each ordinary effect.

The fused example's image, parallax, brightness, tint, and vignette execute in
one fragment pass and one fullscreen draw. Its disconnected image node is
removed, including its deliberately missing asset.

A blur produces:

1. The input color graph rendered at the blur's declared dimensions.
2. A horizontal five-tap Gaussian into another target.
3. A vertical Gaussian fused with downstream operations and final presentation.

The weights are `[1, 4, 6, 4, 1] / 16` on each axis. `radius` controls sample
spacing in intermediate pixels and is clamped to 0–64. It is not a variable tap
count or exact Gaussian standard deviation. Radius zero preserves sampling but
does not remove passes, because live parameter changes must preserve topology.
The runtime sampler clamps at texture edges.

A `materialize` node renders its input once to an intermediate target and lets
all consumers sample that target. Shared boundary nodes are produced once;
nested boundaries are emitted in dependency order. Downstream fusable effects
share the next required pass. Blur's vertical filter may be evaluated in multiple
consuming passes; materialize the blur explicitly when caching that result matters.

Intermediate targets use the runtime's fixed-size, linear `rgba8unorm` storage.
Each boundary clamps and quantizes colors. The final surface encodes sRGB.
Resizing the host does not change intermediate dimensions. UV and resolution
inputs in an upstream materialized branch refer to that intermediate pass;
other globals retain the frame's values.

Generated packages use program 2 / ABI 2 for a single pass and program 3 / ABI 2
for intermediate targets. No runtime format change is required. A procedural
graph with no image assets receives a compiler-owned one-pixel background because
the current fragment ABI requires an image binding. The first packaged image is
used as the manifest preview; generating a rendered thumbnail is future UI work.

## Limits and verification

Graphs contain at most 256 nodes. Local source images have no fixed byte or
dimension cap and stay intact in project folders. The compiler prepares resized
playback copies when necessary, preserving aspect ratio and alpha; its report
counts resolution reductions. Default metadata is bounded to 1 MiB per file,
packaged assets to 32 MiB per file and 64 MiB total package content. Existing
package and runtime limits also apply, including eight image assets, eight
sampled textures per pass, 64 parameter declarations, 16 passes, 15 targets,
4096-pixel target dimensions, and the default 256 MiB texture budget. The compiler
API permits stricter `PackageLimits` and `RuntimeLimits`; the CLI uses defaults.

The report includes live/dead node counts, passes, image assets, parameters,
intermediate target count, and intermediate bytes. Bytes count target storage,
not total GPU memory or measured frame time. Generated shaders are validated
with the shared runtime ABI and Naga's empty optional capability set. Existing
full GPU Profile 1 and watchdog limitations still apply.

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
WGPU_BACKEND=vulkan cargo test -p wisp-compiler --test gpu -- --ignored
WGPU_BACKEND=gl cargo test -p wisp-compiler --test gpu -- --ignored
```

CPU tests cover graph diagnostics, boundary dependencies, pruning, deterministic
archive round trips, limits, image validation, CLI export protection, and project
path containment. GPU tests compare the fused result with handwritten WGSL,
compare blur with a CPU kernel including intermediate quantization, and verify
materialized multi-image composition, parameter slot pruning, and live updates.
The same generated graph packages can be used for manual Linux Vulkan / Windows
DirectX 12 snapshot comparisons.

Later compiler work includes constant folding, common-expression/sample
elimination, automatic caching, render-target reuse, relative dimensions, HDR,
bloom and other effect libraries, compute/simulation authoring, and incremental
compilation. These are not required for the Phase 5 graph-to-program path.

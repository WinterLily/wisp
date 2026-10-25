# Wisp package format 1

Packages support up to eight image resources, 64 typed user parameters, 16 ordered
fragment/compute passes, 15 intermediate targets, and four storage buffers.
They execute through the same runtime in the preview window, PNG snapshots, and
Niri/Hyprland desktop host. Version 4 permits procedural packages with no images.

## Archive

A `.wisp` file is a single classic ZIP archive, with Stored or Deflate compression.
Metadata files live at the root; shaders live under `shaders/`, and image resources
under `assets/`. Only declared files are accepted. Empty directory entries are
allowed and count toward the archive entry limit; extraction is never performed.

```text
wallpaper.wisp
├── manifest.json
├── program.json
├── shaders/pass_0.wgsl
└── assets/background.png
```

Use forward-slash relative paths with ASCII letters, digits, dots, underscores, and
hyphens. Components must be nonempty, cannot be `.` or `..`, cannot end in a dot,
and cannot use Windows device names such as `CON` or `NUL`. Paths are limited to
240 bytes. Absolute paths, backslashes, drive prefixes, duplicate entries, and
case-insensitive name collisions are rejected on every platform. Display metadata
may contain Unicode.

ZIP64, multiple disks, encryption, self-extracting prefixes, streamed entries with
data descriptors, symlinks, and special files are unsupported. Local and central
headers must agree on names, flags, compression, CRC, and sizes. Entry data must
not overlap other entries or the central directory. Decompression checks CRC and
actual byte counts. ZIP readers that emit data descriptors must instead write to
a seekable output (such as a regular file) before sharing a package with Wisp.

## Manifest

```json
{
  "format_version": 1,
  "id": "example.shader-basic",
  "name": "Shader Basic",
  "author": "Example Author",
  "description": "Animated image with mouse parallax",
  "preview": "assets/background.png",
  "program": "program.json",
  "minimum_wisp_version": "0.1.0",
  "gpu_profile": 1,
  "tags": ["interactive"]
}
```

`preview` may be absent or null; if present, it must reference a decodable PNG,
JPEG, or WebP. It may reference the same file as the image resource. The CLI exporter
uses the original background as the preview; it does not render a thumbnail.

The program path is currently fixed to `program.json`. All other fields are
required. Unknown fields and duplicate JSON struct fields are rejected.
`minimum_wisp_version` is a full semantic version (`major.minor.patch`), checked
against the implementation version. Future required versions are rejected.

Identifiers are 1–128 ASCII letters, digits, dots, underscores, or hyphens. Display
name and author must be nonblank and at most 256 UTF-8 bytes each; description is
at most 4096 bytes. There may be at most 32 nonblank tags of at most 64 bytes each.

## Program

```json
{
  "version": 1,
  "shader_abi_version": 1,
  "resources": [
    {
      "type": "image",
      "id": "background",
      "path": "assets/background.png"
    }
  ],
  "passes": [
    {
      "type": "fragment",
      "shader": "shaders/pass_0.wgsl",
      "background": "background",
      "output": "presentation"
    }
  ]
}
```

Program version 1 requires exactly one image resource and one fragment pass. `background`
references the resource identifier, not its filename. Image extensions must be
`.png`, `.jpg`, `.jpeg`, or `.webp`; WGSL paths end in `.wgsl`.

The pass samples its image through `background` and `background_sampler` using
linear filtering and clamp-to-edge addressing. Output stretches to presentation
resolution, replaces every pixel without blending, and uses an 8-bit sRGB target.
The pass uses the existing fullscreen triangle entry points and supplies a UTF-8
WGSL function:

```wgsl
fn wallpaper(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(background, background_sampler, uv);
}
```

The [shader ABI](phase-1.md) defines coordinates, globals, and exact bindings.
A package stores the function body source, including any helper functions; it
must not redeclare the host ABI or add entry points or resource bindings.

Package format, program format, shader ABI, and GPU profile have independent
version fields. The archive format and GPU profile remain `1`. Accepted
program/shader ABI pairs are `(1, 1)`, `(2, 2)`, `(3, 2)`, and `(4, 3)`; other pairs are
rejected. Existing version 1 archives and loose shaders remain supported, and the
CLI `--export` still writes version 1. Version 1 rejects multiple resources,
nonempty `images`, and nonempty `parameters`. Omitted arrays default to empty.
Older Wisp builds reject version 2 explicitly rather than silently dropping its
bindings. These fields are distinct from application version `0.1.0`; this remains
a development format that may evolve with later milestones.

## Program version 2 and shader ABI 2

Version 2 adds ordered image bindings and parameter declarations:

```json
{
  "version": 2,
  "shader_abi_version": 2,
  "resources": [
    {"type": "image", "id": "base", "path": "assets/base.png"},
    {"type": "image", "id": "accent", "path": "assets/accent.png"}
  ],
  "parameters": [
    {"id": "blend", "default": {"type": "float", "value": 0.5}, "min": 0, "max": 1},
    {"id": "tint", "default": {"type": "color", "value": [1, 1, 1, 1]}}
  ],
  "passes": [
    {"type": "fragment", "shader": "shaders/pass.wgsl", "background": "base",
     "images": ["accent"], "output": "presentation"}
  ]
}
```

`background` is image slot 0; `images[0]` is slot 1, through `images[6]` at slot 7.
Resource declaration order does not affect binding order. Resource IDs must be
unique, every image must be bound exactly once, and unknown references fail.
Distinct resources may share an asset path; each binding still counts toward the
image and decoded-memory limits. The runtime validates every bound image.

| Binding | WGSL variable | Type |
| --- | --- | --- |
| Group 0, binding 0 | `globals` | Existing 48-byte Globals ABI |
| Group 1, binding 0 | `background` | `texture_2d<f32>` |
| Group 1, binding 1 | `background_sampler` | Filtering sampler |
| Group 1, binding `2*i` | `image_i` (i = 1–7) | `texture_2d<f32>` |
| Group 1, binding `2*i+1` | `image_i_sampler` | Filtering sampler |
| Group 2, binding 0 | `params` | Generated Parameters uniform struct |

Only declared image slots exist. The parameter binding exists only when parameters
are present. All textures are decoded as sRGB and use linear filtering with
clamp-to-edge addressing. The renderer allocates these resources once at load time.

Parameter IDs use the same syntax as resource IDs and must be unique within the
parameter list. IDs are used by hosts and the CLI. In WGSL, parameters use stable
positional fields `params.p0`, `params.p1`, etc., in declaration order. This avoids
injecting arbitrary identifiers or colliding with WGSL keywords.

| JSON default type | JSON value | WGSL field type |
| --- | --- | --- |
| `float` | Finite f32 number | `f32` |
| `integer` | Signed 32-bit integer | `i32` |
| `boolean` | `true` or `false` | `u32` (0 or 1) |
| `color` | Four numbers in 0–1 | `vec4<f32>` (linear RGBA) |
| `vector2` | Two finite f32 numbers | `vec2<f32>` |
| `vector3` | Three finite f32 numbers | `vec3<f32>` |
| `vector4` | Four finite f32 numbers | `vec4<f32>` |

`min` and `max` are optional inclusive bounds for floats and integers only.
Integer bounds must be integral and fit i32; float bounds must fit f32 and are
compared at f32 precision. Bounds must be finite and ordered. Defaults and overrides
must have the declared type and satisfy the bounds; vectors must have exactly the
specified number of components. Enum choices and editor slider metadata are not
yet implemented.

Each parameter occupies 16 bytes at offset `index * 16`; the buffer is exactly
`parameter_count * 16` bytes. Numeric lanes use little-endian 32-bit values, with
zero-filled unused lanes. WGSL fields have 16-byte alignment and explicit scalar
padding fields. The padding preserves the same layout through GLSL std140
translation; custom WGSL member sizes alone did not pass the OpenGL pixel tests.
Authors should use only `params.pN`, not the generated padding fields.

For the two parameters above, the shader can define:

```wgsl
fn wallpaper(uv: vec2<f32>) -> vec4<f32> {
    let base = textureSample(background, background_sampler, uv);
    let accent = textureSample(image_1, image_1_sampler, uv);
    return mix(base, accent, params.p0) * params.p1;
}
```

Hosts load with `Wallpaper::from_package` and create a renderer with
`Renderer::from_wallpaper`. `Wallpaper::set_parameter` changes initial values in
every pass before renderer creation. `Shader::set_parameter` remains available for
loose shader construction. `Renderer::set_parameter` validates and uploads one 16-byte
slot in each pass without rebuilding pipelines; invalid changes preserve the previous value.
Submit previously encoded frames before updating parameters, as with globals.
Each renderer owns independent parameter state. The CLI accepts repeated
`--param ID=VALUE` overrides with `--package`, including `--check`; values are JSON
numbers, booleans, or arrays. Later overrides of the same ID win. Overrides do not
modify the archive or persist across invocations.

## Program version 3: intermediate targets and ordered passes

Version 3 uses shader ABI 2. It adds a resource with fixed pixel dimensions and a
pass output referencing its ID:

```json
{
  "version": 3,
  "shader_abi_version": 2,
  "resources": [
    {"type": "image", "id": "source", "path": "assets/source.png"},
    {"type": "render_target", "id": "temporary", "width": 256, "height": 256}
  ],
  "passes": [
    {"type": "fragment", "shader": "shaders/first.wgsl", "background": "source",
     "output": {"target": "temporary"}},
    {"type": "fragment", "shader": "shaders/final.wgsl", "background": "temporary",
     "images": ["source"], "output": "presentation"}
  ]
}
```

Targets have no asset path. Dimensions are integers from 1 through 4096. A pass's
`background` and `images` fields can reference images or previously produced
targets, using the same texture/sampler slots as ABI 2. The first pass must bind
images, since no targets have been produced yet. There must be at least one image.

The pass array is the execution order; dependencies are implicit in resource
references. Each target has exactly one producer, must be read by a later pass,
and may be read by several later passes. Every image must be bound by at least one
pass. Duplicate bindings within a pass, unknown references, reads before writes,
cycles, self-sampling, and multiple producers are rejected. Only the last pass
writes presentation; all preceding passes write distinct declared targets. The
runtime does not sort passes or execute authoring graphs.

Image input remains sRGB. Intermediate storage is **linear `rgba8unorm`**, with
linear filtering and clamp-to-edge sampling. Shader results are clamped to 0–1 and
quantized to eight bits when stored; intermediate targets do not preserve HDR or
full float precision. Presentation converts linear shader output to sRGB. Each
pass clears and replaces its output with one fullscreen triangle; blending and
feedback are unsupported.

Each pass gets a separate globals buffer: `globals.resolution` is that pass's
output dimensions. Time, mouse, frame, delta time, and audio fields are
identical across passes; mouse remains normalized to the host viewport. Use
`textureDimensions` when an effect needs the input texture's pixel size. Parameters
have identical declarations and values in every pass. Initial CLI overrides and
live runtime updates apply to all passes.

Resources and bind groups are prepared once. Fixed intermediate sizes remain
unchanged when the window or snapshot size changes; the final pass renders at the
host's output size. Relative resolution policies, target reuse, HDR formats,
and texture feedback remain future work. Version 4 adds compute and storage buffers.

`Wallpaper` now keeps validated topology private. Use `Wallpaper::new(shader,
image)` for a loose single-image wallpaper, `images()` for images in resource
order, `shader()` for the first pass, `parameters()`, and `pass_count()`.
`Package::shader_source_at(index)` and `resource_image_bytes(id)` expose multipass
content. Legacy package getters still describe the first pass only. `image_bytes()` requires
a first-pass image and must not be used for image-free compute packages; use
`resource_image_bytes` or `image_bytes_at` (which returns `Option`) instead.

```sh
python3 examples/multipass-blur/build-package.py blur.wisp
cargo run -- --package blur.wisp --check --param radius=4
cargo run -- --package blur.wisp --frames 10
cargo run -- --package blur.wisp --output blur.png --time 3
```

This example performs horizontal blur, vertical blur, then a moving comparison
with the original image. `radius` controls the sample spacing and `strength` the
blurred side's blend. Its builder uses only Python's standard library and requires
a new destination file.

## Validation and limits

`wisp-format` validates archive structure, metadata, versions, paths, references,
and encoded byte limits without depending on wgpu. `wisp-runtime::Wallpaper`
then parses and validates WGSL with Naga and decodes all images and the optional
preview before GPU creation. GPU pipeline creation still uses wgpu validation and
allocation error scopes. Hosts should perform both validation stages before
accepting content for rendering.

Default `PackageLimits`:

| Limit | Default |
| --- | --- |
| Compressed archive | 64 MiB |
| Total uncompressed files | 64 MiB |
| Individual uncompressed file | 32 MiB |
| Each metadata JSON file | 1 MiB |
| WGSL source | 1 MiB |
| ZIP entries, including directories | 128 |

Archive entry counts and local header bounds are checked before constructing
per-file decoders. Declared sizes and actual decompressed bytes are bounded; JSON
size limits apply before deserialization. Hosts can pass different `PackageLimits`;
the preview CLI uses the defaults. Export reapplies the supplied limits.

The runtime also retains its 4096 × 4096 image and snapshot limit, 32 MiB encoded
image limit, and 64 MiB decoder allocation limit. RGBA conversion can allocate an
additional image buffer. Packages have fixed maxima of eight image resources, eight
texture bindings per pass (including targets), 64 parameters, 16 passes, and 15 targets.

Host-selected `RuntimeLimits` default to 4096 pixels per texture dimension, 256 MiB
combined image and intermediate texture storage, 16 passes, and 15 intermediate
targets. Use `Wallpaper::from_package_with_limits` for CPU validation and
`Renderer::from_wallpaper_with_limits` to recheck the same policy before GPU
allocation. The CLI uses defaults. Hosts can tighten these limits; increasing them
does not bypass fixed package, image-decoder, or device limits.

Texture storage is estimated as `sum(width * height * 4)` over resource declarations;
images shared across passes are decoded and uploaded once per resource. Separate
image IDs referencing the same file still count separately. Estimates exclude
uniforms, pipelines, driver overhead, previews, and host-owned presentation/readback
resources. This is a texture budget, not a complete GPU-memory budget. The aggregate
budget is checked after each image is decoded, so temporary peak memory can exceed it.
Package loading holds archive bytes and decoded files in
memory; these byte limits are not a total process-memory budget.

Profile 1 currently selects the existing feature-free device setup and Naga
capability checks. Full portable resource budgets, execution-time limits, shader
watchdogs, and crash isolation remain future work. Naga validation does not bound
shader execution time. Windows/DirectX rendering parity remains unverified.

## Use

```sh
# Export the embedded example. The destination must not already exist.
cargo run -- --export wallpaper.wisp --id example.shader-basic --name "Shader Basic" --author "Example Author"

# Custom shader and image use the same export path.
cargo run -- --shader examples/shader-basic/wallpaper.wgsl --image examples/shader-basic/background.png --export custom.wisp

# Validate without a GPU or display server.
cargo run -- --package wallpaper.wisp --check

# Window preview or deterministic snapshot.
cargo run -- --package wallpaper.wisp
cargo run -- --package wallpaper.wisp --output snapshot.png --time 3
```

Third-party tools can write this layout with a normal ZIP writer. For example,
Python's `zipfile.ZipFile("wallpaper.wisp", "w", compression=zipfile.ZIP_DEFLATED)`
writing to a regular file produces supported entries. JSON and WGSL are UTF-8.
No Steam client, installation step, or filesystem extraction is required.

Build and run the reproducible two-image example (Python standard library only):

```sh
python3 examples/texture-blend/build-package.py blend.wisp
cargo run -- --package blend.wisp --check
cargo run -- --package blend.wisp --param blend=0.6 --param 'tint=[0.5,1,0.8,1]'
cargo run -- --package blend.wisp --param animate=false --output blend.png --time 3
```

The example contains source images, shader, and program metadata in one portable
archive. Its builder requires a new output path, as does the CLI exporter.

## Program version 4 and shader ABI 3: compute

The accepted program/ABI pairs are `(1, 1)`, `(2, 2)`, `(3, 2)`, and `(4, 3)`.
Archive format and GPU profile remain 1. Fragment shaders keep the ABI 2 interface.
Version 4 adds these resource/pass descriptions:

```json
{"type": "storage_buffer", "id": "colors", "elements": 65536}
```

```json
{
  "type": "compute",
  "shader": "shaders/generate.wgsl",
  "images": [],
  "buffers": [{"resource": "colors", "writable": true}],
  "output": "generated",
  "dispatch": [32, 32, 1]
}
```

`output` names a declared `render_target`. The final pass must still be a fragment
pass targeting presentation. A compute pass writes one linear RGBA8 target and
may bind up to eight input images/targets and four storage buffers. Inputs and
outputs cannot alias. Textures have one producer, must be written before sampling,
and must be sampled by a later pass. Buffers have one writable binding in the
program and may be read by subsequent compute passes. Repeated bindings, missing
resources, multiple producers, and reads before a producer are rejected.

Each buffer element is a 16-byte `vec4<f32>`. Buffers are zero-initialized on GPU
allocation and retained between frames; a writable binding uses WGSL `read_write`
access. A shader must synchronize invocations correctly and initialize any state
it depends on. Separate passes provide ordering for cross-pass dependencies;
invocations within a dispatch do not have an implicit global barrier.

The runtime supplies:

| Binding | WGSL declaration / purpose |
| --- | --- |
| Group 0, binding 0 | `globals`, unchanged 48-byte uniform ABI |
| Group 1 | Input texture/sampler pairs, in `images` order; first is `background` / `background_sampler`, then `image_1`, etc. |
| Group 2, binding 0 | `params`, when parameters exist; unchanged 16-byte slots |
| Group 3, binding 0 | `output_image: texture_storage_2d<rgba8unorm, write>` |
| Group 3, bindings 1 onward | `buffer_0`, `buffer_1`, etc., as `array<vec4<f32>>`, in `buffers` order |

Compute files define `fn compute(id: vec3<u32>)`. The generated entry point uses
`@workgroup_size(8, 8, 1)` and passes `global_invocation_id`. `dispatch` counts
workgroups, so `[32, 32, 1]` launches a 256 × 256 grid. The shader must guard edge
invocations when dimensions are not multiples of eight. `globals.resolution` is
the output target's fixed physical size. Sampling in compute uses `textureLoad`
or an explicit LOD, such as `textureSampleLevel`.

```wgsl
fn compute(id: vec3<u32>) {
    let size = vec2<u32>(globals.resolution);
    if id.x >= size.x || id.y >= size.y || id.z != 0u { return; }
    let color = vec4<f32>(vec2<f32>(id.xy) / vec2<f32>(size), 0.5, 1.0);
    buffer_0[id.y * size.x + id.x] = color;
    textureStore(output_image, vec2<i32>(id.xy), color);
}
```

The [compute gradient builder](../examples/compute-gradient/build-package.py)
creates a complete package with two compute passes and fragment presentation.

Compute limits are enforced during package/CPU validation and again against the
selected GPU before resource creation:

- Each dispatch dimension: 1–65535; product: at most 1,048,576 workgroups.
- Workgroup size: fixed 8 × 8 × 1; declared workgroup storage: at most 16 KiB.
- Each storage buffer: 1–4,194,304 elements (64 MiB); at most four buffers.
- Default runtime storage-buffer total: 128 MiB.
- Default combined decoded images, intermediate targets, and buffers: 256 MiB.
- Existing shader, archive, texture dimension, pass, and parameter limits still apply.

Hosts can lower `RuntimeLimits.max_storage_bytes`, `max_resource_bytes`, and
`max_dispatch_groups` in addition to texture/pass limits. Estimates exclude GPU
driver overhead, pipelines, uniforms, transient loading memory, and host surfaces.
GPU creation checks compute support, dispatch limits, storage bindings, buffer
sizes, and device texture limits. No optional wgpu features are requested. Older
fragment-only packages remain usable on backends without compute support.
These checks bound declared resources; they do not bound shader execution time.

## Phase 9 compiled interactivity

Particle emitters and temporal feedback compile into existing version 4 / ABI 3
compute and fragment passes. There are no new resource or pass types. Storage is
zero-initialized for each renderer and persists across frames and parameter updates;
loading a new renderer resets it. Audio uses the existing four globals. Capture
source settings belong to the host, never to a wallpaper package. See
[Phase 9](phase-9.md) for generated effects, limits, and deterministic simulation
warmup. Versions 1–3 remain compatible and also receive audio globals.

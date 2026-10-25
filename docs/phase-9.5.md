# Phase 9.5: Layers, masks and effects

Easy Designer now builds image compositions and localized motion through the same
IR, compiler, and runtime as Visual Graph. No layer, preset, mask editor, or effect
interpreter was added to `wisp-runtime` or the daemon. Existing packages and older
Easy Designer projects remain supported.

## Try it

```sh
cargo run -p wisp -- --create examples/layers-effects
cargo run -p wispc -- examples/layers-effects --output layers.wisp
cargo run -- --package layers.wisp
cargo run -p wispd -- --package layers.wisp

# Generate editable examples and portable packages for all eight effects.
cargo run -p wisp-app --example layer-effects -- /tmp/layer-examples
cargo run -p wisp -- --create /tmp/layer-examples/layers-clothing
```

The checked-in example has a background with rain and a transparent character with
masked hair movement. The deliberately simple artwork makes the affected regions
and alpha edges easy to inspect. Creators can import their own prepared transparent
parts; automatic segmentation and skeletal/strand physics are outside this phase.

## Authoring workflow

1. Choose an image on Create, then open **Layers → Add layers & effects**.
2. **+ Image** imports a PNG, JPEG, or WebP above existing layers. The list
   displays front to back. Select a layer to edit it; Forward/Back changes its
   order, the checkbox changes visibility, and Remove deletes it from the stack.
3. Use **Adjust** for opacity and Alpha/Add/Multiply/Screen/Overlay blending. Each layer has its
   own X/Y position, X/Y scale, rotation, and X/Y pivot. Positions and pivots use
   normalized image coordinates; rotations are degrees. Images outside their
   transformed bounds are transparent. Composition uses straight alpha and linear
   RGB, including partially transparent backgrounds.
4. Use **Motion** to choose Hair or Clothing. A black mask is created automatically. Open
   **Mask → Paint mask…** and paint white on
   hair ends or cloth folds; leave the face and attachment regions black. The brush
   editor shows the untransformed layer with a teal selection overlay. Strokes
   appear immediately while dragging; Erase paints black. The painter
   fills the workspace, with **Done** to return to layers.
   The image keeps its aspect ratio and the brush stays circular. Continuous strokes interpolate between pointer samples. Brush radius,
   feathering, inversion, and mask routing are editable. A black effect mask causes
   no deformation at those pixels. Whole-scene parallax/light effects still operate
   independently; use Clean image for a completely stationary base scene.
5. The **Mask** tab supports importing, creating, clearing, and filling masks. **Affect layer opacity**
   routes coverage to composition; **Affect motion strength** routes it to the
   selected effect. Both may be enabled. Imported RGB masks convert to grayscale,
   including source alpha. Interactive paint edits use a retained 1024×1024 grayscale raster; the first
   stroke resamples an imported mask to this size. The source texture stays resident;
   brush events update only the affected raster region and upload ready RGBA pixels. Imported unpainted masks retain
   their original dimensions. Feathering is a bounded nine-tap filter, followed by
   inversion. Gray byte 128 represents approximately 50% coverage despite sRGB
   image texture decoding.
6. Wave and Warp expose direction, amplitude, frequency, and speed. Hair uses masked
   Wave; Clothing uses masked Warp with different defaults. Displacement samples a
   separate map, using perceptual RG around 0.5 as the neutral point. Set distortion
   strength and use a mask to restrict its region.
7. Droplets, Rain, and Ripples expose density, size, speed, and distortion/effect
   strength. Curved water surfaces bend the sampled scene using a water refraction
   ratio, with directional specular highlights, window/sky reflections, and Fresnel
   edge reflection. Rain has randomized sliding drops, small beads and wet trails.
   Weather processes the composite through the selected layer, so it also refracts
   lower layers, including through a transparent foreground image. Higher layers
   remain in front of it. A mask restricts the region and layer opacity attenuates
   the effect. Refraction clamps at viewport edges to avoid transparent holes.
8. Painting stays live independently of the wallpaper preview. Changed masks are
   snapshotted at most once every two seconds; PNG encoding and compilation run on
   a background worker, and painting continues while that snapshot compiles. Further
   strokes queue the next snapshot. Unchanged masks do not trigger compilation.
   Sliders update on release and discrete changes compile immediately. Invalid
   compilation retains the previous valid preview.
   Save editable project, export `.wisp`, or install in the library as before.

## Graph operations

All new operations are searchable on Graph. Input names below are graph edges;
`mode` and `kind` are properties. Graph angles are radians.

| Operation | Inputs | Output / properties |
| --- | --- | --- |
| `transform` | `uv`, `position`, `scale`, `pivot`: Vector2; `rotation`: Float | Vector2, inverse image transform |
| `mask` | `color`: Color; `feather`: Float; `invert`: Boolean | Float coverage |
| `layer` | `color`: Color; `uv`: Vector2; `opacity`, `mask`: Float | Color, coverage and bounds clipping |
| `blend` | `a`, `b`: Color | Color; `a` is background, `mode`: `alpha`, `add`, `multiply`, `screen`, `overlay` |
| `wave`, `warp` | `uv`, `direction`: Vector2; `mask`, `amplitude`, `frequency`, `speed`: Float | Vector2 |
| `displacement` | `uv`: Vector2; `map`: Color; `mask`, `strength`: Float | Vector2 |
| `weather` | `color`: Color; `uv`: Vector2; `mask`, `density`, `size`, `speed`, `strength`: Float | Color; `kind`: `droplets`, `rain`, `ripples` |

The three weather catalog entries create the corresponding `weather` kind. Feed
Mask into both Layer and Wave/Warp/Displacement/Weather for equivalent Easy Designer
routing. Masks are evaluated in the undeformed layer coordinates so anchors remain
fixed. Mask feather offsets are in graph evaluation coordinates. Feed transformed
UV into Image, then use Layer to clip the sampled coordinates.

Weather uses analytic water caps, independently sliding columns, wet tracks and
expanding rings. Each pixel evaluates nine neighboring cells; density changes the
grid size, not the iteration count. Surface coordinates compensate for output
aspect ratio. Surface normals and optical thickness drive refraction and specular
reflection rather than drawing uniform outlines.
These periodic effects need no persistent history, so a typical layered scene
compiles to one fragment pass and no intermediate targets or storage buffers.
For temporal accumulation, their Color output can feed the existing Feedback node;
Particles and Feedback retain the Phase 9 compiler's compute passes and storage
limits. This composition is covered by a regression test. There is no second
weather simulation implementation in the runtime.

## Persistence and limits

Layer projects contain the standard `project.json`, `graph.json`, copied `assets/`,
`designer.json`, and versioned `layers.json`. Mask rasters, transforms, visibility,
ordering, blend modes, effect routing, and settings all round-trip. Standard `wispc`
compilation produces byte-identical package output. Easy Designer rejects external
changes to its generated graph rather than silently replacing them; open such
projects in Graph. Layer format 2 applies weather after composition. Format 1 drafts
are checked against their original generated graph before being upgraded on open;
resaving writes the new graph. Save/export and Graph conversion flush the latest
paint raster, even if a preview snapshot is pending. Project saves and package
exports require new destinations, preserving existing files.

Only reachable image assets and nodes are exported. Hidden layers remain editable
in the project but their resources are pruned from the package. Authoring metadata
never enters the runtime program. Layer effects use existing program 2 / ABI 2;
blur/materialization or persistent feedback use the existing higher versions.

Defaults and limits:

- At most six editable layers and twenty retained source images/maps. Local image
  imports have no fixed file-size, dimension, or aggregate source-byte cap. Image
  and displacement originals remain intact in saved projects; imported masks retain
  their full source dimensions after grayscale conversion. Preview/export prepare
  copies within 4096-pixel dimensions and the available package/texture budgets;
  resolution reductions appear in the compiler report. Compilation still enforces
  eight live images, a 256 MiB combined GPU-resource budget, and 256 graph nodes.
  Each mask/map counts as an image. A scene over structural/resource limits remains
  editable and reports a compilation error. Source decoding requires enough RAM.
- Easy transforms have positive scale 0.05–3, normalized position −1–1, and pivot
  0–1. Shader transforms guard division with a minimum scale magnitude of 0.001.
- Easy motion amplitude is 0–0.15; shader amplitudes/displacement clamp to ±0.25,
  frequency to 0–64, and speed to ±10. Mask feather clamps to 0–0.1 in the shader
  (Easy control 0–0.05). Weather density clamps to 1–32, size 0.03–0.45, speed 0–5,
  and strength 0–1. None of these controls creates unbounded shader loops.
- Creator reports show passes, source image memory, intermediate texture memory,
  and persistent storage. Designs over four passes or 64 MiB of these resources
  receive a cost warning. Estimates exclude driver overhead and are not GPU timings.
- Wave/weather/mask evaluation is fused, so fewer passes does not mean zero shader
  cost. A feathered mask uses nine samples; weather may reevaluate its source at
  displaced coordinates. Materialize expensive shared branches in Graph if needed.

## Verification

Local Linux verification includes CPU project/package round trips for every effect,
all five blend modes, image/mask retention, paint/erase/import, invalid controls,
visibility pruning, externally edited graph rejection, node socket typing, and
layer-to-persistent-feedback compilation. GPU tests compare blends with a CPU
straight-alpha reference, check transformed bounds, black motion anchors, coverage,
inversion, feathering, and embedded-preview/export parity for every effect. Follow-up
regressions cover retained source textures during strokes, painting during preview
compilation, throttling, saved pending strokes, aspect-correct expanded coordinates,
legacy project migration, and refraction of a lower layer through a transparent
foreground. The refraction check compares against identical lighting with the
optical offset disabled.

```sh
cargo test --workspace
WGPU_BACKEND=vulkan cargo test -p wisp-compiler --test gpu -- --ignored
WGPU_BACKEND=gl cargo test -p wisp-compiler --test gpu -- --ignored
WGPU_BACKEND=vulkan cargo test -p wisp-app --lib -- --ignored
WGPU_BACKEND=gl cargo test -p wisp-app --lib -- --ignored
cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings
```

Eight generated effect packages rendered at identical fixed inputs on local Vulkan
and OpenGL differed by at most 1/255 per channel. The layered wallpaper presented
120 frames in nested Niri on both backends. Iced window smoke tests rendered 24
frames on each backend, and the Vulkan screenshot was visually inspected.

The same effect packages can be rendered manually on Linux Vulkan and Windows
DirectX 12 and compared with the existing tolerance. GPU tests and layer designer
captures can also be run locally. Native Windows execution, physical monitor
soak tests, and the earlier platform-verification gaps remain
unverified locally. No existing desktop assignments were changed.

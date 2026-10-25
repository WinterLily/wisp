# Phase 7: Easy Designer

Open **Create** in Wisp, choose a PNG, JPEG, or WebP image, select a preset,
and adjust the sliders. No shader or graph editing is required.

```sh
cargo build --workspace
cargo run -p wisp -- --create examples/shader-basic/background.png
# --import with an image also opens the designer.
cargo run -p wisp -- --import examples/shader-basic/background.png
```

## Creating a wallpaper

1. Choose an image, or use **Open project…** to continue a saved draft.
2. The canvas fills the workspace. Open **Project** to enter a name and author.
3. Open **Effects** to choose a preset, enable effects, and adjust their controls.
4. Move the pointer over the live preview to try parallax. Use **Pause** to freeze time.
5. **Project → Save project…** creates a new folder containing the image and authoring data.
6. **Project → Save to library** installs the current result. Select a monitor under **Display** and **Apply wallpaper**
   on the Library page to run it on the desktop.
7. **Project → Export wallpaper…** writes a portable `.wisp` package that works without Steam.

Only one panel opens at a time; select its toolbar button again or press Escape to
close it. **Focus** (F11) hides the toolbar and panels. Escape restores the controls
without losing your place. The canvas preserves the source image's proportions
as the workspace resizes. Image/project paths and replacement actions are under
**Project** after opening an image.

| Preset | Effects | Render passes |
| --- | --- | --- |
| Clean image | Brightness and contrast | 1 |
| Gentle motion | Mouse parallax, breathing light, vignette | 1 |
| Dreamy focus | Mouse parallax, soft blur, vignette | 3 |
| Breathing light | Animated brightness, vignette | 1 |

Parallax reserves a small crop proportional to its strength so moving the pointer
to an image corner never stretches the source edges. Zero strength preserves the
full image. Effects can be combined independently. Sliders control mouse movement
strength, light variation, animation speed, brightness, contrast, edge darkening,
and softness.
Only controls used by enabled effects are exposed in the exported wallpaper.
Their current values become package defaults; library users can change them per monitor.
Choosing another preset resets effect controls while retaining the name and author.
Choosing another image or opening another project replaces the current draft; save
unsaved work first. Switching pages retains the in-memory draft.

## Editable projects and export

A saved project contains:

```text
my-wallpaper-project/
├── designer.json
├── project.json
├── graph.json
└── assets/background.png   # or .jpg / .webp, detected from the image bytes
```

Use **Open project…** on the empty canvas or **Project → Open…** to select a saved folder. The image is copied into the project,
so moving or removing the original image does not break it. Project saving requires
a new folder name; use a new revision folder when saving further edits. Failed saves
leave the in-memory draft available. Package export similarly requires a new filename.
Saving to the library uses the existing immutable, content-addressed installation path.
The package's wallpaper ID derives from its source image hash; edits to that image's
wallpaper remain revisions of that ID.

The graph and project files are standard `wispc` inputs:

```sh
cargo run -p wispc -- /path/to/my-wallpaper-project --output wallpaper.wisp
cargo run -p wisp -- --create /path/to/my-wallpaper-project
cargo run -p wisp -- --create /path/to/my-wallpaper-project --check
```

`--create … --check` validates without a window or GPU and does not install an
unfinished draft. Easy Designer refuses to reopen a graph that has been edited
externally, so it cannot silently discard those edits. Such projects still import
through **Import → Choose project folder** and compile with `wispc`.

## Implementation and limits

Easy Mode lives in `wisp-app::designer` and lowers settings into the existing
`wisp-ir` graph. `wisp-compiler` emits the same program versions 2/3 and shader ABI 2
used by other fragment wallpapers. No designer data, compiler dependency, or
new effect interpretation is added to `wisp-runtime` or `wispd`.

Local PNG/JPEG/WebP imports have no fixed encoded-byte, dimension, or decoder
allocation cap. Editable projects retain full-resolution source bytes. Available
memory and codec support still determine what can be decoded. Preview/export
prepare separate image copies within the portable renderer's 4096-pixel dimensions
and texture/package budgets, preserving alpha and aspect ratio. Compatible images
retain their original encoding; oversized copies use lossless PNG with resampling
as needed. The compiler report identifies images reduced in resolution.
Settings, project metadata, graph files, and contained asset paths are checked.
Every export and installation recompiles and validates the current design.

The preview uses the existing Iced widget backed by `wisp-runtime`. Scalar edits
update uniforms immediately; effect toggles and presets compile on a background
worker. Failed compilation retains the last valid preview and disables saving the
failed result. The widget prepares GPU replacements before swapping its renderer.
Preview animation pauses outside Create/Library or when unfocused; page changes
retain the draft. Desktop execution remains independent of the GUI.

Blur uses two linear RGBA8 intermediate targets, preserving the source aspect ratio
with a maximum long edge of 1024 pixels (at most 8 MiB total). The display shows pass
count and intermediate storage, not measured GPU time. Images currently stretch to
the preview/output size, and package thumbnails show the prepared source image. Automatic
aspect fitting, rendered thumbnails, undo/history, arbitrary parameter declarations,
bloom, particles, and audio service are outside this initial designer. Wayland desktop
pointer globals remain centered; the preview and Windows host provide mouse input.
Breathing light animates using time on every host.

## Verification commands

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings
WGPU_BACKEND=vulkan cargo test -p wisp-app --lib -- --ignored
WGPU_BACKEND=gl cargo test -p wisp-app --lib -- --ignored

# Generate all four editable presets and their portable packages in a new directory.
cargo run -p wisp-app --example easy-presets -- /tmp/easy-presets
cargo run -p wisp -- --create /tmp/easy-presets/dreamy --frames 12 --screenshot /tmp/designer.png
cargo run -- --package /tmp/easy-presets/dreamy.wisp --output /tmp/dreamy.png --time 3
```

CPU tests cover all presets, live defaults, project round trips, exact `wispc`
package equivalence, independent library installation/export, malformed inputs,
path containment, overwrite protection, failed preview replacement, and CPU-only CLI
creation. GPU tests compare each preset's embedded preview with its exported package
within 1/255, and independently check time and mouse response. Native Windows UI,
file dialogs, desktop execution, and DirectX 12 parity still require Windows execution.

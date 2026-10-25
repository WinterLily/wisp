# Phase 8: Visual graph editor

Open **Graph** in Wisp and choose **New**, **Open…**, or **From Easy Designer**.
The latter copies the current Easy Designer draft into an independent graph document.
Existing standard compiler projects and saved Easy Designer projects open directly:

```sh
cargo run -p wisp -- --graph examples/graph-blur
cargo run -p wisp -- --graph examples/graph-fused --check
```

`--graph PROJECT --check` compiles and validates without a display or GPU and does
not install the draft. It is mutually exclusive with `--create` and `--import`.
The GUI library and desktop assignments continue to use their existing controls.

## Editing

- Search the node catalog and click a result to add it. All 25 currently implemented
  compiler operations are available, including explicit blur/materialize boundaries.
- Drag a node to move it. Drag the empty canvas or middle-drag to pan; scroll to zoom.
  **Fit** shows the whole graph; **Arrange** restores dependency columns. Node positions are saved with the project.
- Click an output dot, then an input dot to connect or replace that input. Socket
  labels show types; colors distinguish scalars, vectors, and colors. Right-click
  cancels a pending connection. Output nodes have no outgoing socket.
- Select a node to edit its properties. Use **Set** or Enter to commit each text
  property; partially typed values remain in the field until committed. Color and
  vector values use bracketed components, for example `[0.2, 0.5, 0.8, 1]`.
  Float, integer, boolean, color, and vector constants/parameters are supported.
  Changing a value type resets its value; parameter type changes also clear bounds.
- Each Parameter node added to the graph creates a float declaration. Its `parameter`
  property can reference an existing declaration; default and optional scalar bounds
  are editable below it. Blank bounds mean unbounded. The compiler checks limits.
- **Add image asset…** copies a PNG, JPEG, or WebP into the draft's asset collection.
  Select an Image node and choose an asset in its properties, then connect its UV input.
- Use **×** next to an input to disconnect it, **Delete node** to remove a node and
  its incoming/outgoing connections, or **Use as output** to select an Output node.

New nodes can have empty sockets. These are saved as empty references in an
unfinished graph and produce compiler diagnostics until connected. Incompatible
known socket types, invalid endpoints, and cycles are rejected without committing
the connection. The canonical `wisp-ir` checker validates the whole graph, including
unused nodes, before every compilation; property changes can also reveal errors
in downstream nodes. Diagnostics identify nodes and inputs where the checker has
that context. Shader, asset, and resource-limit errors retain the compiler's context.
Audio nodes receive normalized bands when [Phase 9 audio capture](phase-9.md) is enabled.

## Preview, save, and export

Committed structural/property edits compile on a background worker. The UI prevents
concurrent graph mutations while compilation runs; compilation results carry a
revision number so stale results cannot replace the current preview. Metadata and
layout changes do not require shader compilation. This is full recompilation, not
incremental shader/pass caching.

The live preview uses the existing `wisp-runtime` Iced widget. Failed compilation
leaves the last valid preview visible and animated. GPU renderer replacement follows
the widget's existing prepare-before-swap behavior. **Pause** freezes preview time.
The report shows pass count, live/pruned nodes, and intermediate texture storage;
it does not claim measured GPU time.

**Save project…** writes a new folder, including incomplete drafts:

```text
graph-project/
├── project.json
├── graph.json
├── layout.json
└── assets/
```

`layout.json` is version 1 and maps node IDs to finite `[x, y]` positions. It is
optional and ignored by the compiler. Valid source images, including disconnected
ones, are copied and renamed to safe local filenames. Missing asset declarations
remain missing placeholders and can be repaired by importing an image and selecting
it in the node. A missing unused asset does not block compilation. Existing paths
are bounded and checked for containment before reading; invalid images and unsafe
paths are rejected. Project opening permits semantic graph errors for repair, but
rejects unsupported versions, oversized metadata, and duplicate/empty node IDs.

**Export .wisp…** and **Save to library** recompile the document and validate the
result. Both are disabled after a failed compile; project saving remains available.
The package contains only runtime data. No canvas, layout, graph, or compiler logic
was added to the runtime or daemon. Export requires a new filename; project saving
requires a new folder. Opening another document or choosing New replaces the current
in-memory graph, so save work first. Switching pages retains drafts. Unsaved text
properties require **Set** before saving.

Saved graph projects compile unchanged with `wispc`. A graph copied from Easy Mode
is now an independent standard project; it has no `designer.json` and cannot be
converted back to Easy Mode automatically.

## Verification

```sh
cargo build --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings
WGPU_BACKEND=vulkan cargo test -p wisp-app --lib -- --ignored
WGPU_BACKEND=gl cargo test -p wisp-app --lib -- --ignored
cargo run -p wisp -- --graph examples/graph-blur --frames 12 --screenshot /tmp/graph.png
```

CPU coverage includes canvas events, typed connections, cycles, incomplete-draft
recovery, every catalog operation's sockets, source/layout round trips, deterministic
`wispc` equivalence, multiple assets, Easy Mode conversion, overwrite protection,
malformed files, compiler-result revisions, failed-preview preservation, and CLI
validation. GPU coverage compares graph edits and a one-pass to three-pass change
against separately exported packages through the shared runtime.

Undo/redo, multi-selection, automatic edge routing, custom node definitions,
incremental compilation, and shader editing are not implemented. The catalog is
limited to existing compiler operations; this phase does not add later-phase audio,
simulation, or shader-authoring functionality. Native Windows UI and DirectX 12
execution, physical monitor soak tests, and Rust 1.88 execution remain unverified.

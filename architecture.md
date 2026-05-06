# c4tui — Architecture

This document describes *how* c4tui is built. The *what* lives in [specification.md](./specification.md).

## 1. Overview

c4tui is a single-process Rust application. At runtime it owns three concurrent concerns:

1. **Workspace pipeline** — load DSL/JSON, invoke Structurizr CLI, produce per-view SVG, parse to (raster + hit-test map).
2. **Terminal session** — own the alternate screen, image registry, input parser, redraw loop.
3. **Navigation state** — current view, breadcrumb stack, pan/zoom transform per view, modal UI state.

These three are connected by an internal event channel.

## 2. Components

### 2.1 Workspace loader

- Accepts a path to `workspace.dsl`, `workspace.json`, or a directory containing one.
- Invokes `structurizr-cli` as a subprocess to export each view as **SVG**.
- Parses `workspace.json` (either user-provided or generated as a side-effect of export) to learn:
  - The list of views, names, and types (Landscape, SystemContext, Container, Component, Deployment, etc.).
  - The element-to-child-view mapping — for each element in each view, the view (if any) that drills into it.

### 2.2 SVG processor

For each exported SVG:

1. Parse with `usvg` (resvg's permissive front-end).
2. Walk the tree, extracting `(element_id, bounding_box_svg_coords)` for every group whose `id` corresponds to a Structurizr element.
3. Rasterize at the configured DPI multiplier (default 4×) into an RGBA buffer using `resvg`.
4. Compute and persist the affine transform between SVG coordinates and raster pixel coordinates.

The bbox map stays in SVG coordinates. Pan/zoom changes the SVG-to-screen transform, not the source data.

### 2.3 View store

In-memory, populated lazily as views are first visited:

```
ViewId → {
    name: String,
    view_type: ViewType,
    raster: Arc<RgbaImage>,
    bboxes: Vec<(ElementId, BBoxSvg)>,
    child_view_for_element: HashMap<ElementId, ViewId>,
    svg_to_raster: Affine,
}
```

Rasters are reused across navigation; only the SVG-to-screen transform changes during pan/zoom.

### 2.4 Terminal renderer

Owns the Kitty graphics image registry. Each view's raster is transmitted once with a stable image ID. Subsequent display commands specify the source rectangle and scale, driving pan/zoom without retransmission.

Layout responsibilities:

- Reserve a top status bar (one cell).
- Reserve a bottom command/help bar (one to two cells).
- The remaining area is the image canvas.
- View picker and other modals draw as cell-glyph overlays above the image (Kitty Z-index).

### 2.5 Input handler

Parses CSI sequences from stdin:

- Keyboard events (Kitty keyboard protocol if negotiated, otherwise xterm-style).
- Mouse events in SGR pixel mode (CSI ?1016).

Translates raw events into semantic events: `NavigateChild { element_id }`, `PanBy { dx, dy }`, `ZoomBy { factor }`, `OpenViewPicker`, `Back`, `Reload`, `Quit`.

### 2.6 Navigation state

```
NavState = {
    current: ViewId,
    breadcrumbs: Vec<ViewId>,
    transforms: HashMap<ViewId, ViewTransform>,
    modal: Option<Modal>,   // ViewPicker, ErrorDialog, Help
}
```

Transitions:

- `Click(px, py)` resolves to a hit element. If the element has a child view: push current onto breadcrumbs, set current to child.
- `Back`: pop breadcrumbs into current.
- View picker selection: set current to chosen view, clear breadcrumbs.

## 3. Data flow

```
workspace.dsl
     │
     ▼
[ structurizr-cli export -format <svg> ]   (subprocess: at load and on reload)
     │
     ▼
*.svg per view  +  workspace.json
     │
     ▼
[ SVG processor ]
     │
     ▼
View store (raster + bbox map per view)
     │
     ▼
[ Terminal renderer ] ◀──── [ Navigation state ] ◀──── [ Input handler ]
     │
     ▼
Kitty graphics escape sequences → terminal
```

## 4. Technology choices

| Layer | Choice | Why |
|---|---|---|
| Language | Rust | Single static binary, easy distribution, mature TUI/graphics ecosystem |
| TUI framework | ratatui | Widely used, immediate-mode, plays well with Kitty image overlay |
| SVG parse + rasterize | `usvg` + `resvg` + `tiny-skia` | Pure Rust, no system deps, print-quality output |
| Kitty protocol | direct encoder OR `ratatui-image` | Choice deferred to Phase 1 spike — see implementation plan |
| Async runtime | `tokio` | Subprocess and stdin-parsing both benefit |
| Workspace JSON model | `serde` + a hand-modeled subset of Structurizr's schema | Avoid generated full-schema bloat; model what we use |
| CLI parsing | `clap` | Standard |

### 4.1 Why SVG (not PlantUML or Mermaid)

`structurizr-cli` can export to PlantUML, Mermaid, DOT, and SVG. We use SVG because:

- It carries Structurizr element IDs natively in `<g id="...">` groups, making hit-testing a parse step rather than a coordinate-reverse-engineering problem.
- It is rasterizable at any DPI without re-exporting from the CLI.
- `resvg` is mature, fast, and produces print-quality output.

The cost is a transitive PlantUML/Graphviz dependency inside `structurizr-cli`'s export path, which we accept as out-of-process.

## 5. Key trade-offs

### 5.1 Subprocess on every (re)load

Cold start cost is dominated by `structurizr-cli` warmup (~2–5 s typical). We accept this for v1; the alternative is reimplementing Structurizr's DSL parser and layout pipeline in Rust, which is enormous scope creep with little user value.

### 5.2 Rasters are immutable

We never partially update a raster. Pan/zoom is a re-place of the same image with a different source rectangle. This keeps the renderer simple and matches what the Kitty protocol is good at.

### 5.3 Trust the SVG element IDs

We assume Structurizr's SVG export emits stable, parseable element IDs. If upstream changes the format we adapt; we deliberately do not hedge by also computing positions ourselves.

### 5.4 Kitty-only for v1

Sixel and iTerm2 inline-image fallbacks are real engineering effort for a worse experience. We refuse to run on unsupported terminals rather than ship a degraded path.

## 6. Security considerations

- c4tui invokes a subprocess with paths derived from CLI arguments. Argument-quoting must be airtight — never construct shell strings.
- Workspace files may reference URLs (themes). Default behavior in v1: no network fetches. URL-referenced themes are reported as unavailable; the diagram renders un-themed.
- No telemetry. No background update checks.

## 7. Distribution

- `cargo install c4tui` (publish to crates.io once stable).
- Pre-built binaries via GitHub Releases for macOS (arm64 + amd64) and Linux (amd64 + arm64).
- Homebrew tap.

Distribution is Phase 6 in the implementation plan.

# c4tui — Architecture

This document describes *how* c4tui is built. The *what* lives in [specification.md](./specification.md).

## 1. Overview

c4tui is a single-process Rust CLI with a small explicit update loop:

```text
raw terminal bytes
      │
      ▼
input parser → InputEvent → Command → AppState::apply
                                      │
                                      ├── Effect  ──► terminal / workspace side effect
                                      │
                                      ▼
                                 RenderFrame ──► TerminalBackend
```

The split keeps most application behavior testable without a real terminal:

- `event.rs` maps semantic input into app commands.
- `state.rs` owns navigation, pan/zoom transforms, update results, effects, and render-frame production.
- `app.rs` is the orchestration shell: read input, map command, apply state, perform side effects, render.
- `backend.rs` defines the terminal boundary used by both the real terminal session and tests.

## 2. Components

### 2.1 Workspace loader

The workspace pipeline accepts a path to `workspace.dsl`, `workspace.json`, or a directory containing one. When interactive viewing starts it invokes `structurizr-cli` as a subprocess to export SVG views and then parses `workspace.json` to learn:

- view keys, names, and types;
- element IDs present in each view;
- child-view relationships used for click-to-drill navigation.

Reload is modeled as an app effect. `Command::Reload` requests `Effect::ReloadWorkspace`; the app shell performs the subprocess/file I/O, then feeds success or failure back into `AppState`.

### 2.2 SVG processor

For each exported SVG, `render.rs` parses once with `usvg::Tree` and uses that same tree for both geometry and rasterization:

1. Walk `usvg` groups with non-empty IDs.
2. Use each group's computed absolute bounding box for hit testing.
3. Rasterize with `resvg`/`tiny-skia` at the configured DPI multiplier.
4. Encode the pixmap as PNG for Kitty image transmission.

Using `usvg` geometry avoids hand-parsing SVG XML attributes and correctly handles nested transforms, scale, rotation, paths, images, text, and other shapes that `usvg` resolves.

### 2.3 View store

`ViewStore` owns view metadata, lazy rendered-view caches, and per-view transforms. Public boundaries use typed IDs:

```text
ViewId → ViewInfo {
    key: String,
    name: String,
    view_type: String,
    svg_path: PathBuf,
    element_ids: HashSet<ElementId>,
    child_view_by_element_id: HashMap<ElementId, ViewId>,
}

ViewId → RenderedView {
    width: u32,
    height: u32,
    png: Vec<u8>,
    bboxes: Vec<ElementBBox>,
}

ViewId → ViewTransform
```

Rasters are reused across navigation; pan/zoom changes the source rectangle displayed from the same cached image.

### 2.4 App state and commands

`AppState` contains:

- current `ViewId`;
- breadcrumb stack;
- last drag point;
- per-view pan/zoom state stored in `ViewStore`.

`Command` values represent user intent: quit, open picker, select view, back, reload, help, drill by canvas point, zoom, pan, drag, and reset/fit. `AppState::apply(command, store)` mutates state and returns an `UpdateResult` containing:

- whether a render is needed;
- an optional `Effect` for imperative work such as quit, help, picker, reload, or image-cache clearing.

`RenderFrame` is the declarative render input: current view, breadcrumbs, and the current view transform.

### 2.5 Terminal backend

`TerminalBackend` is the imperative boundary:

- report terminal/canvas size;
- read `InputEvent`;
- render a `RenderFrame`;
- open picker/help/error/message dialogs;
- clear terminal image cache.

`TerminalSession` implements the trait for the real TTY. Tests use `FakeTerminalBackend` to script input and assert rendered frames/dialog effects without raw mode or a terminal emulator.

The concrete terminal session owns:

- alternate screen and raw mode cleanup;
- Kitty graphics image registry;
- SGR pixel mouse setup;
- image placement and source-rectangle updates;
- status/help/picker overlays.

### 2.6 Input handling

`input.rs` parses raw keyboard and mouse bytes from stdin into low-level `Key` values. The terminal backend converts mouse coordinates into canvas coordinates and emits `InputEvent`. `event.rs` maps those events plus configured keybindings into `Command` values.

Supported interactions include:

- view picker (`o`, arrows, Enter, Esc);
- pan/zoom (`+`, `-`, arrows, drag, wheel);
- reset/fit (`0`, `f`);
- click-to-drill and Backspace navigation;
- reload (`r`), help (`?`), quit (`q`).

## 3. Data flow

```text
workspace.dsl / workspace.json
        │
        ▼
structurizr-cli export -format svg       (load and reload effect)
        │
        ▼
exported SVGs + workspace.json
        │
        ├──► workspace metadata parser ──► ViewStore metadata
        │
        └──► usvg/resvg renderer ────────► RenderedView cache + hit boxes
                                                ▲
                                                │
raw input ─► InputEvent ─► Command ─► AppState ─┴─► RenderFrame ─► TerminalBackend ─► terminal
```

## 4. Technology choices

| Layer | Choice | Why |
|---|---|---|
| Language | Rust | Single static binary, strong CLI/tooling ecosystem |
| CLI parsing | `clap` | Standard typed argument parser |
| Workspace JSON | `serde` + hand-modeled Structurizr subset | Model only the schema needed for navigation |
| SVG geometry | `usvg` | Resolves SVG transforms and element bounds robustly |
| SVG rasterization | `resvg` + `tiny-skia` | Pure Rust, no system graphics dependencies |
| Terminal graphics | Direct Kitty protocol encoder | Precise image caching/source-rect control |
| Terminal capability/input | Direct terminal escape handling | Small surface area; no async runtime required today |
| Tests | Unit tests + fake terminal backend | Exercise state/backend behavior without a TTY |

### 4.1 Why SVG

`structurizr-cli` can export PlantUML, Mermaid, DOT, and SVG. c4tui uses SVG because:

- Structurizr element IDs are preserved as SVG group IDs, enabling hit testing.
- SVG can be rasterized at any DPI without re-exporting.
- `usvg`/`resvg` provide robust parsing, transform handling, and raster output in-process.

## 5. Key trade-offs

### 5.1 Subprocess on load/reload

Cold start is dominated by `structurizr-cli`. Reimplementing Structurizr DSL parsing and layout in Rust would be large scope creep, so c4tui treats the CLI as the authoritative exporter.

### 5.2 Kitty-only rendering

Sixel and iTerm2 fallback support would add substantial complexity for a weaker experience. c4tui currently detects unsupported terminals and fails clearly rather than shipping degraded rendering.

### 5.3 Cached rasters, mutable source rectangles

A rendered view's raster is immutable once cached. Pan/zoom is expressed by changing Kitty source-rectangle placement, avoiding retransmission and keeping interaction responsive.

### 5.4 Small explicit architecture over a UI framework

The app uses simple internal types (`InputEvent`, `Command`, `AppState`, `RenderFrame`, `TerminalBackend`) instead of a larger TUI framework. This keeps the core behavior easy to test and avoids pulling terminal protocol details into state updates.

## 6. Security considerations

- Subprocess execution uses argument vectors, not shell strings.
- Workspace paths come from CLI arguments and should be treated as untrusted file inputs.
- Workspace themes or remote references are not fetched by c4tui itself.
- No telemetry and no background update checks.

## 7. Distribution

- `cargo install c4tui`
- Homebrew tap: `brew install scshafe/tap/c4tui`
- GitHub Releases with macOS arm64/amd64 and Linux arm64/amd64 archives

Release automation is documented in [docs/releasing.md](./docs/releasing.md).

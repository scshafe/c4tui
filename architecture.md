# c4tui - Architecture

This document describes how c4tui is built. Product behavior is specified in
[specification.md](./specification.md).

## 1. Overview

c4tui is a single-process Rust CLI. It owns the Structurizr domain model and
application state, while delegating reusable terminal primitives to `tui-kit`.

```text
Structurizr workspace
        |
        v
structurizr export -> SVG views + workspace.json metadata
        |
        v
ViewCatalog + RenderCache + ViewportState
        |
        v
InputEvent -> Command -> AppState::apply
                              |
                              +--> Effect -> App side effect
                              |
                              v
                       RenderFrame -> TerminalBackend
```

The core split is:

- `state.rs` decides how commands change navigation state.
- `app.rs` performs effects, owns the active modal, and coordinates rendering.
- `view_catalog.rs` owns exported view metadata, workspace relationships, and
  the export-directory lifetime.
- `render_cache.rs` owns rendered SVG rasters and render-job discovery.
- `viewport_state.rs` owns per-view image widgets, transforms, placement
  policy, hit testing, pan, and zoom.
- `terminal.rs` is the real terminal backend.
- `backend.rs` defines the backend trait and fake backend used by tests.
- `tui-kit` supplies input types, focus scopes, component caching, grid widgets,
  image viewport math, terminal lifecycle, scheduler primitives, and status-line
  layout.

## 2. Workspace Pipeline

`workspace.rs` accepts a `workspace.dsl`, `workspace.json`, or directory. The
loader prefers `workspace.dsl` when both files exist.

For interactive viewing, c4tui calls:

```text
structurizr export --workspace <path> --format svg --output <temp-dir>
```

The resulting temporary directory is retained for the lifetime of the current
`ViewCatalog`. c4tui discovers SVG files in that directory and reads
`workspace.json` from one of:

1. the export output directory;
2. the source path with a `.json` extension;
3. the source path itself when the source is `workspace.json`.

### 2.1 View Metadata

Workspace metadata is parsed into `ViewInfo` records:

- key, name, kind, description;
- SVG path;
- element IDs present in the view;
- legend/key view link;
- related child/detail view keys grouped by parent element ID.

Structurizr parent fields used for detail links:

- `softwareSystemId`;
- `containerId`;
- `componentId`;
- `elementId`.

Multiple child/detail targets are preserved for one element. This is required
when a system has a container view, deployment/infrastructure-oriented views,
and dynamic views all scoped to the same element.

### 2.2 Relationship Metadata

The model parser indexes:

- elements by `ElementId`;
- relationships by `RelationshipId`;
- outgoing relationship IDs by source element;
- incoming relationship IDs by destination element.

`ViewCatalog::connection_candidates_for_element` joins those indexes with view
membership to produce navigable relationship candidates.

## 3. Rendering Pipeline

`render.rs` parses each SVG with `usvg` and rasterizes with `resvg`.

For each view:

1. Parse the SVG tree.
2. Extract bounding boxes for groups with element IDs.
3. Rasterize into RGBA pixels at the configured budget/quality.
4. Encode PNG bytes for Kitty image transmission.
5. Store the result as a `RenderedView`.

Hit testing uses the extracted bounding boxes and the active image viewport
transform. This supports optional mouse drill and element inspection, but the
keyboard link directory does not depend on mouse hit testing.

### 3.1 Image Pipeline

```text
Stage 1: SVG -> PNG
  owner: RenderPool / RenderCache
  work: resvg rasterizes exported SVGs into RenderedView { rgba, png, bboxes }

Stage 2: PNG -> Kitty image registry
  owner: TerminalSession / tui_kit::terminal::Terminal
  work: upload PNG bytes once per image id through ImageSurfaceRegistry

Stage 3: Kitty image -> placement
  owner: ViewportState / ImageViewportWidget
  work: compute source crop, cell rectangle, pan, zoom, and placement id on
        each interaction before TerminalSession applies the placement
```

The split is deliberate. Rasterization can happen asynchronously, image upload
is a synchronous terminal-side operation on first display, and placement is the
fast path that changes during pan, zoom, modal thumbnail rendering, and window
resize.

## 4. View Responsibilities

The former monolithic view store is split into three owners:

- `ViewCatalog` owns `Vec<ViewInfo>`, the parsed `WorkspaceModel`, relationship
  navigation indexes, detail/legend lookup, and the temporary export guard.
- `RenderCache` owns `HashMap<ViewId, RenderedView>`, the raster budget, lazy
  render insertion, and missing-render job discovery.
- `ViewportState` owns `HashMap<ViewId, ImageViewportWidget>`, per-view
  transforms, placement policy, hit testing, zoom, pan, reset, and viewport
  invalidation.

Important queries now live on the narrow owner that has the needed data:

- `ViewCatalog::child_view_ids_for_element(current, element_id)` resolves all
  detail views linked from an element in the current view.
- `ViewCatalog::connection_candidates_for_element(current, element_id)`
  resolves relationship traversal targets that have a destination view.
- `ViewCatalog::connection_candidate_counts_for_element` powers footer hints
  and counts.
- `ViewportState::element_at_canvas_point` maps optional mouse coordinates into
  an element ID using `RenderCache` and `ViewCatalog`.
- `RenderCache::render_jobs` identifies asynchronous rasterization work for
  views that are not cached yet.

## 5. State and Effects

`state.rs` contains app navigation state:

- current view;
- breadcrumbs;
- last drag point;
- pinned element.

`Command` values represent user intent: quit, open picker, open link directory,
select view, select child view, select link, inspect, reload, help, legend, pan,
zoom, reset, drag, and no-op.

`AppState::apply` returns an `UpdateResult`:

- the canvas metrics used for the command;
- whether a render is needed;
- an optional `Effect`.

Effects isolate imperative work from pure state transitions. The current effect
set is:

- open a modal via `Effect::OpenModal(ModalSpec)`;
- reload the workspace;
- clear terminal image cache;
- show help;
- toggle log view.

## 6. Navigation Model

### 6.1 Current Implementation

c4tui has three navigation surfaces:

- primary `LinkDirectory` for immediately reachable diagrams;
- global view picker for direct view selection;
- child/detail view picker when optional mouse drill hits an element with
  multiple related child views.

Direct global selection clears breadcrumbs. Linked traversal pushes the current
view before navigating.

### 6.2 Link Directory

The keyboard-first link directory unifies the "linked from here" behavior. It is
an app-owned `NavPicker<LinkCandidate>` configuration, sharing the same modal
lifecycle as the view and child/detail pickers.

The link directory consumes a `Vec<LinkCandidate>` assembled from:

- detail links for elements visible in the current view;
- connection links for the pinned/selected element, when present;
- optional legend/key link for the current view.

Implemented shape:

```rust,ignore
struct LinkCandidate {
    kind: LinkKind,
    target_view: ViewId,
    source_element: Option<ElementId>,
    label: String,
    detail: Option<String>,
    pinned_element_after_navigation: Option<ElementId>,
    secondary_tokens: Vec<String>,
}

enum LinkKind {
    Detail,
    ConnectionOutgoing,
    ConnectionIncoming,
    Legend,
}
```

Selecting a candidate dispatches a command that preserves the linked navigation
semantics:

- push current view onto breadcrumbs;
- set current view to the target;
- optionally set pinned element;
- clear drag state.

The directory does not require SVG hit testing. It is derived from metadata and
current app context.

## 7. UI Components

### 7.1 View Picker

`nav_picker.rs` implements the shared `NavPicker<T>` substrate used by the view
picker, child/detail picker, and link directory. It supports:

- all-view browsing;
- filtering;
- key-view hiding/showing;
- per-view thumbnails;
- limited subsets for child/detail drill choices.

### 7.2 Link Directory and Connection Links

`link_directory.rs` computes `LinkCandidate` values for detail links,
relationship links, and legend links. Relationship rows show direction,
connected element, relationship metadata, and target view.

### 7.3 Status Bar

`statusbar.rs` builds app-specific fragments and delegates width-aware layout
to `tui_kit::bar::layout_status_line`.

### 7.4 Log View and Dialogs

The log view is app-specific because it understands c4tui's in-memory log
buffer. Help and error dialogs use tui-kit dialog primitives through the
terminal backend.

## 8. Terminal Backend

`TerminalBackend` defines the imperative boundary:

- canvas metrics;
- render active frame;
- render/close active modal through `ActiveModal` and the `Modal` trait;
- show help and error dialogs;
- clear image cache;
- teardown image viewports.

`TerminalSession` implements the trait using `tui_kit::terminal::Terminal`,
`tui_kit::image`, and `tui_kit::widgets::image_viewport`.

Tests use `FakeTerminalBackend` to assert render calls, picker lifecycle, and
modal behavior without entering raw mode.

## 9. Event Loop

`app.rs` owns the foreground loop:

1. Receive `tui_kit::events::AppEvent`.
2. Route input by active focus scope.
3. Convert low-level input to a `Command` through `keymap.rs`.
4. Apply command to `AppState`.
5. Perform any returned effect.
6. Render or redraw the active modal.

Focus scopes distinguish root diagram mode, picker modal, link-directory modal,
log modal, and dialogs.

## 10. Background Rendering and Watching

`render_pool.rs` specializes `tui_kit::scheduler::Scheduler` for view rendering
priority. Hover/selection previews can request render work without blocking the
foreground input path.

The workspace watcher uses `tui_kit::watcher` when enabled in config. Reload
itself still flows through `Effect::ReloadWorkspace` so the state transition is
testable.

## 11. Testing Strategy

Tests avoid a real terminal. Coverage is organized around:

- workspace metadata parsing and relationship indexes;
- SVG bbox extraction, transforms, raster budget, and crop behavior;
- `ViewCatalog` navigation candidate queries;
- `RenderCache` render-job and lazy-cache behavior;
- `ViewportState` hit testing, zoom, pan, placement, and invalidation;
- `AppState` command/effect transitions;
- picker and link-directory rendering/keyboard behavior;
- terminal backend call sequencing through `FakeTerminalBackend`;
- status bar truncation and contextual hints.

Cross-repo testing guidance for tui-kit's `testkit` and c4tui's
`FakeTerminalBackend` is documented in the adjacent tui-kit repository at
`docs/testing-patterns.md`.

Manual smoke testing is still required for Kitty graphics behavior in a real
terminal, especially after changes to image placement, source cropping, or
terminal capability setup.

## 12. Security and Reliability

- Structurizr is invoked with argument vectors, not shell strings.
- Workspace files are treated as untrusted inputs.
- c4tui itself does not fetch remote themes; unresolved remote references are
  surfaced through Structurizr errors.
- Logs are written to the configured log file and in-memory buffer, not stderr
  while the alternate-screen UI is active.
- Terminal cleanup is owned by tui-kit terminal drop behavior.

## 13. Distribution

c4tui can be installed through Cargo, Homebrew, or GitHub release artifacts.
Release automation is documented in [docs/releasing.md](./docs/releasing.md).

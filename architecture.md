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
ViewStore + WorkspaceModel
        |
        v
InputEvent -> PendingCommand -> Command -> AppState::apply
                                      |
                                      +--> Effect -> App side effect
                                      |
                                      v
                               RenderFrame -> TerminalBackend
```

The core split is:

- `state.rs` decides how commands change navigation state.
- `app.rs` performs effects, owns modal slots, and coordinates rendering.
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
view store. c4tui discovers SVG files in that directory and reads
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

`ViewStore::connection_candidates_for_element` joins those indexes with view
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
keyboard link directory should not depend on mouse hit testing.

## 4. View Store

`view.rs` owns:

- `views: Vec<ViewInfo>`;
- the parsed `WorkspaceModel`;
- lazy rendered-view cache;
- image viewport widgets and per-view transforms;
- placement policy and raster budget;
- the temporary export guard.

Important view-store queries:

- `child_view_ids_for_element(current, element_id)` resolves all detail views
  linked from an element in the current view.
- `connection_candidates_for_element(current, element_id)` resolves
  relationship traversal targets that have a destination view.
- `navigable_connection_counts_for_element` powers footer hints and counts.
- `element_at_canvas_point` maps optional mouse coordinates into an element ID.

## 5. State and Effects

`state.rs` contains app navigation state:

- current view;
- breadcrumbs;
- last drag point;
- pinned element.

`Command` values represent user intent: quit, open picker, select view, select
child view, inspect, open connection picker, select connection, reload, help,
legend, pan, zoom, reset, drag, and no-op.

`AppState::apply` returns an `UpdateResult`:

- the canvas metrics used for the command;
- whether a render is needed;
- an optional `Effect`.

Effects isolate imperative work from pure state transitions. Examples:

- open a view picker;
- open a child-view picker;
- open a connection picker;
- reload the workspace;
- clear terminal image cache;
- show help;
- toggle log view.

## 6. Navigation Model

### 6.1 Current Implementation

c4tui currently has three navigation surfaces:

- global `ViewPicker` for direct view selection;
- child/detail view picker when an element has multiple related child views;
- `ConnectionPicker` for incoming/outgoing relationship traversal.

Direct global selection clears breadcrumbs. Linked traversal pushes the current
view before navigating.

### 6.2 Link Directory Target

The keyboard-first link directory should unify the "linked from here" behavior.
Architecturally, it should be an app-owned component built over
`tui_kit::widgets::grid`, similar to the existing view and connection pickers.

The link directory should consume a `Vec<LinkCandidate>` assembled from:

- detail links for elements visible in the current view;
- connection links for the pinned/selected element, when present;
- optional legend/key link for the current view.

Recommended shape:

```rust,ignore
struct LinkCandidate {
    kind: LinkKind,
    label: String,
    detail: Option<String>,
    target_view_id: ViewId,
    pinned_element_after_navigation: Option<ElementId>,
}

enum LinkKind {
    Detail,
    ConnectionOutgoing,
    ConnectionIncoming,
    Legend,
}
```

Selecting a candidate should dispatch a command that preserves the linked
navigation semantics:

- push current view onto breadcrumbs;
- set current view to the target;
- optionally set pinned element;
- clear drag state.

The directory should not require SVG hit testing. It is derived from metadata
and current app context.

## 7. UI Components

### 7.1 View Picker

`picker.rs` implements `ViewPicker`, a c4tui-specific component over
`tui_kit::widgets::grid`. It supports:

- all-view browsing;
- filtering;
- key-view hiding/showing;
- per-view thumbnails;
- limited subsets for child/detail drill choices.

### 7.2 Connection Picker

`connection_picker.rs` implements relationship traversal over
`tui_kit::widgets::grid`. Rows show direction, connected element, relationship
metadata, and target view.

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
- key translation;
- render active frame;
- draw/close picker modals;
- draw/close log view;
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
3. Convert low-level input to a `PendingCommand` through `keymap.rs`.
4. Resolve canvas-dependent commands.
5. Apply command to `AppState`.
6. Perform any returned effect.
7. Render or redraw the active modal.

Focus scopes distinguish root diagram mode, picker modal, connection picker
modal, log modal, and dialogs.

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
- `ViewStore` navigation candidate queries;
- `AppState` command/effect transitions;
- picker and connection picker rendering/keyboard behavior;
- terminal backend call sequencing through `FakeTerminalBackend`;
- status bar truncation and contextual hints.

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

# c4tui - Specification

This document describes what c4tui does. The implementation structure lives in
[architecture.md](./architecture.md); delivery planning lives in
[implementation-plan.md](./implementation-plan.md).

## 1. Purpose

c4tui is a terminal application for read-only browsing of Structurizr C4
workspaces. It renders exported diagram views as inline terminal images and
provides keyboard-first traversal between related diagrams.

c4tui is not a diagram editor and does not calculate C4 layouts itself. The
Structurizr workspace remains the source of truth.

## 2. Product Goals

- Render Structurizr views faithfully in a Kitty-graphics-capable terminal.
- Make the current diagram navigable through a compact "link directory" of
  immediately reachable diagrams.
- Let users traverse with the keyboard: select a linked diagram, press Enter,
  and maintain breadcrumbs for Back navigation.
- Preserve optional mouse-based element drill for users and terminals where it
  works, without making mouse precision the primary navigation model.
- Support relationship traversal: from a selected or pinned element, show
  incoming/outgoing relationships that lead to another view.
- Keep pan, zoom, reload, logging, and view search available during browsing.
- Fail loudly when required terminal or Structurizr capabilities are missing.

## 3. Non-goals

- Editing or saving Structurizr workspaces.
- Authoring new diagrams, views, docs, or ADRs.
- Reimplementing the Structurizr DSL parser, layout engine, or exporter.
- Browser-equivalent Structurizr UI fidelity.
- Supporting non-Kitty inline image protocols in v1.
- Running multiple workspaces in one session.
- Fetching cloud themes or other remote workspace dependencies directly from
  c4tui. The workspace/export step is responsible for resolving them.

## 4. Inputs

### 4.1 Workspace Source

c4tui accepts:

- a `workspace.dsl` file;
- a `workspace.json` file;
- a directory containing either file.

When a directory contains both `workspace.dsl` and `workspace.json`, c4tui
prefers `workspace.dsl` as the source of truth.

### 4.2 Structurizr Export

c4tui invokes the upstream `structurizr` binary on `PATH`:

```text
structurizr export --workspace <path> --format svg --output <temporary-dir>
```

The export must produce one SVG per view. When `workspace.json` is available,
c4tui also reads it for view metadata, element membership, parent view scopes,
and relationships.

### 4.3 Terminal Contract

Required:

- Kitty graphics protocol support.
- True color.

Recommended:

- SGR pixel mouse mode for optional mouse drill, drag, and wheel interactions.
- Accurate cell and pixel terminal size reporting.

If Kitty graphics is unavailable, c4tui exits non-zero with a diagnostic.

## 5. Core Concepts

### 5.1 View

A Structurizr view exported as SVG and represented by metadata: key, name,
kind, description, element IDs, optional legend view, and links to related
views.

### 5.2 Linked Diagram

A view reachable from the current context. c4tui recognizes two link families:

- **Detail links**: views scoped to an element visible in the current diagram,
  such as system context, container, component, and dynamic views. These are
  derived from Structurizr view parent fields: `softwareSystemId`,
  `containerId`, `componentId`, and `elementId`.
- **Connection links**: views containing an element connected by an incoming or
  outgoing relationship from the selected/pinned element.

### 5.3 Link Directory

The target primary navigation surface for web-like traversal. It shows the
numbered linked diagrams immediately reachable from the current view, highlights
the current selection, and navigates with Enter.

### 5.4 Breadcrumb

A stack of prior views. Navigating to a linked view pushes the current view.
Back pops the stack and returns to the previous view.

### 5.5 Selected/Pinned Element

An element context used for relationship traversal and footer details. It may be
set by inspection, optional mouse selection, or future keyboard selection.

## 6. Behavior

### 6.1 Startup

1. Parse CLI arguments and config.
2. Detect terminal capabilities.
3. Export the workspace with Structurizr.
4. Discover exported SVG views and parse workspace metadata.
5. Render the first view and enter the event loop.

### 6.2 Diagram Rendering

- The active view is rasterized from SVG and displayed with Kitty graphics.
- The raster is cached per view.
- Pan and zoom update image placement/source crop where possible instead of
  re-exporting the workspace.
- A footer shows view status, dimensions, transform settings, render progress,
  and contextual hints.

### 6.3 Global View Picker

The global picker lists all views, supports filtering, and navigates directly to
the chosen view. Direct global selection clears breadcrumbs because it is not a
linked traversal.

### 6.4 Link Directory

The link directory should:

- list only diagrams immediately reachable from the current context;
- show stable numbers for visible entries;
- highlight the currently selected entry;
- support Up/Down or `j`/`k` selection movement;
- support number-key selection where practical;
- navigate on Enter;
- cancel on Esc without changing navigation state.

Detail links should be available even without a selected element. Connection
links require a selected/pinned element context.

### 6.5 Optional Mouse Drill

Mouse clicks inside the rendered diagram may hit-test SVG element bounding boxes
and open a related-view picker or navigate directly when exactly one target
exists. This is an optional convenience path, not the primary navigation model.

### 6.6 Relationship Traversal

When an element context is selected, c4tui can show navigable incoming and
outgoing relationships. Each relationship candidate includes:

- direction;
- connected element name;
- relationship description and technology when present;
- target view.

Enter navigates to the selected target view and pins the connected element in
that destination.

### 6.7 Legend, Help, Log, Reload

- Legend views can be opened from the active view when exported.
- Help opens a modal command summary.
- Logs are written to an in-memory buffer and optional file; the log viewer is
  available in-app.
- Reload re-runs Structurizr export, rebuilds metadata, clears image caches, and
  resets navigation on success.

## 7. Error Handling

| Condition | Behavior |
|---|---|
| Terminal lacks Kitty graphics | Exit non-zero with supported-terminal guidance |
| Structurizr binary missing | Exit non-zero with the failed command context |
| Workspace path missing | Exit non-zero with the missing path |
| Structurizr export fails | Show/export upstream stdout and stderr in the error |
| Workspace metadata missing | Continue with exported SVGs, but linked navigation is reduced |
| Theme or remote dependency cannot resolve | Surface the Structurizr export/validation error |
| A view fails to rasterize | Report the failure and keep the app state recoverable where possible |

## 8. Configuration

c4tui reads `~/.config/c4tui/config.toml` when present, or a path supplied with
`--config`. Config includes:

- raster quality/budget;
- keybindings;
- zoom step presets;
- placement scale basis and overflow policy;
- workspace watch behavior.

CLI flags override config where both exist.

## 9. Out of Scope

- Network workspace loading.
- Exporting diagrams for external use beyond the Structurizr subprocess output.
- Persistent app session state.
- Terminal multiplexing workarounds that require emulator-specific state
  outside the Kitty protocol.

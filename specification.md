# c4tui — Specification

This document describes *what* c4tui does. The *how* lives in [architecture.md](./architecture.md); the *when* lives in [implementation-plan.md](./implementation-plan.md).

## 1. Purpose

c4tui is a terminal application that loads a Structurizr workspace and provides interactive read-only browsing of its C4 views. It is a viewer, not an editor. Its differentiating capability is **click-to-drill navigation**: clicking on a graphical element in a view that has a defined child view causes navigation into that child view.

## 2. Goals

- Faithful rendering of Structurizr-generated diagrams in a Kitty-graphics-capable terminal.
- Click-to-drill navigation matching the conceptual behavior of Structurizr's web UI.
- Pan and zoom on individual views.
- Single static binary, no background daemon, no JVM at runtime.
- Loud, clear failure when the environment cannot satisfy a feature — never silently render incorrectly.

## 3. Non-goals (v1)

- Editing the workspace.
- Authoring new diagrams or views.
- Supporting non-Kitty image protocols (Sixel, iTerm2 inline) — explicitly deferred.
- Diagram layout calculation. c4tui delegates layout to Structurizr's existing tooling and consumes the SVG output.
- Browser-equivalent fidelity (animations, hover tooltips, complex theme features).
- Themes that fetch from the Structurizr Cloud at runtime. Themes referenced by URL must already be resolvable at workspace-load time, or are reported as missing.

## 4. Inputs

### 4.1 Workspace source

c4tui accepts one of:

- A `workspace.dsl` file (the Structurizr DSL).
- A `workspace.json` file (the serialized workspace model).

When given a directory containing both, c4tui prefers `workspace.dsl` for source-of-truth correctness.

### 4.2 External tooling

c4tui invokes the Structurizr CLI (`structurizr-cli`) as a subprocess to convert the workspace into per-view SVG. The CLI must be on `PATH` or specified via `--structurizr-cli <path>`. The export contract is:

```
structurizr-cli export -workspace <path> -format <svg-format> -output <dir>
```

The exact format flag is determined in architecture.md; the requirement is that the resulting SVG carries Structurizr element IDs in `<g id="...">` groups.

### 4.3 Terminal contract

The terminal must support:

- Kitty graphics protocol: transmit, display, delete, persistent image IDs, Z-index.
- SGR pixel mouse mode (CSI ?1016h).
- Kitty keyboard protocol — recommended, not required for v1.
- Truecolor.

c4tui detects support at startup via terminal capability queries. It refuses to start if the minimums are not met.

## 5. Behavior

### 5.1 Startup

1. Parse CLI arguments.
2. Detect terminal capabilities. Abort with a diagnostic if Kitty graphics is absent.
3. Load the workspace and enumerate views.
4. Render the first view (default to `Landscape` if present, else the first defined view).

### 5.2 View navigation

- The current view is rendered as a high-DPI raster image, fit to the available terminal area.
- A status line displays: current view name, view type, breadcrumb of prior views.
- A view picker is accessible via a keybinding.
- Forward navigation: clicking an element with a defined child view drills into it.
- Backward navigation: a keybinding pops the breadcrumb stack.

### 5.3 Pan and zoom

- Zoom: keybindings or scroll wheel.
- Pan: arrow keys or click-and-drag (in regions not occupied by a click-target element).
- Re-rendering uses the cached high-DPI raster — no re-export required.

### 5.4 Click-to-drill semantics

When the user clicks within the rendered image:

1. Click pixel coordinates are mapped from terminal-space to image-space, accounting for current pan/zoom.
2. The click is hit-tested against per-element bounding boxes extracted from the SVG.
3. If the hit element has a child view (e.g., System → Container, Container → Component), c4tui pushes the current view onto the breadcrumb stack and navigates to the child view.
4. If the hit element has no child view, the click is ignored (an optional brief status hint may be shown).

### 5.5 Reload

c4tui supports an explicit reload keybinding that re-runs the SVG export and refreshes the current view. File-watch-based auto-reload is deferred.

## 6. Output

c4tui writes no files. All output is written to the terminal:

- Image data via Kitty graphics escape sequences.
- Text (status line, view picker, error dialogs) via standard cell rendering.

## 7. Error and failure modes

| Condition | Behavior |
|---|---|
| Terminal lacks Kitty graphics | Exit non-zero with diagnostic referencing supported terminals |
| Terminal lacks SGR pixel mouse | Start in keyboard-only mode; click-to-drill disabled with notice |
| `structurizr-cli` not found | Exit non-zero with installation pointer |
| Workspace file not found | Exit non-zero with diagnostic |
| Workspace DSL parse error | Exit non-zero, surface upstream error verbatim |
| Cloud theme unreachable | Render without theme; warn in status line; do not block |
| A single view fails to export | Skip in the picker; mark as broken; other views remain usable |

## 8. Configuration

A single optional config file `~/.config/c4tui/config.toml`:

- Default zoom step
- DPI multiplier for raster export (default 4×)
- Path to `structurizr-cli`
- Keybindings

Precedence: CLI flags override config; config overrides defaults.

## 9. Out of scope (explicit)

- Multiple workspaces simultaneously.
- Diff view between two workspace versions.
- Export to PNG/PDF (delegate to `structurizr-cli`).
- Embedding into other TUIs as a library.
- Network-fetched workspaces (HTTP/git URLs).

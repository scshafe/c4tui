# c4tui — Implementation Plan

This plan organizes work into phases. Each phase ends with a concrete, demonstrable artifact and explicit acceptance criteria. No phase is "MVP-ish" — each is shippable as far as it claims to go.

## Phase 0 — Project skeleton and capability detection

**Deliverable:** A `c4tui` binary that prints terminal capability detection results and exits.

- `cargo init` with the project name `c4tui`.
- CI: GitHub Actions running `cargo fmt --check`, `cargo clippy -- -D warnings`, and `cargo test` on macOS arm64 and `ubuntu-latest`.
- `clap`-based CLI argument scaffold. `--workspace` accepted but unused.
- Implement Kitty graphics support detection: send the protocol "query" command, parse the response within a 200 ms timeout, classify as supported / unsupported / unknown.
- Implement SGR pixel mouse mode detection: send `CSI ?1016h` then `CSI ?1016$p`, parse the DECRPM response.

**Acceptance:**

- Binary builds clean on macOS arm64 and Linux.
- Run in WezTerm: prints `Kitty graphics: yes; pixel mouse: yes; truecolor: yes`.
- Run in Apple Terminal: prints `Kitty graphics: no` and exits non-zero.

## Phase 1 — Static single-view rendering

**Deliverable:** Given a workspace, render the first view as a static raster in the terminal. No interactivity.

- Subprocess invocation of `structurizr-cli export -format <svg>`.
- Workspace.json parsing for: view list, view names, element list with IDs.
- SVG ingestion for the first view: parse with `usvg`, rasterize at 4× DPI with `resvg` to an RGBA buffer.
- Kitty graphics transmission: encode raster (PNG or compressed RGBA), transmit, place at origin of the canvas region.
- Hardcoded canvas region: full terminal minus a one-cell status bar at the top.
- Phase-1 spike decision: direct Kitty protocol encoder vs. `ratatui-image`. Document the call.

**Acceptance:**

- `c4tui --workspace ./architecture/diagrams/workspace.dsl` shows the first view as an inline image inside WezTerm or Kitty.
- Status bar shows the view name.
- Quit (`q`) cleanly removes the image and restores the cursor.

## Phase 2 — View picker and keyboard navigation

**Deliverable:** Switch between views with the keyboard.

- View picker overlay: a modal listing views by name and type.
- Keybindings: `o` opens the picker, arrow keys navigate, Enter selects, Esc cancels.
- Persistent image IDs per view; rasters transmitted only on first visit.

**Acceptance:**

- Open picker, pick a different view, image swaps within ~50 ms after first cold render.
- Repeated picks of the same view do not retransmit image data.
- Memory usage scales linearly with view count and stays under 50 MB for a ten-view workspace.

## Phase 3 — Pan and zoom on a single view

**Deliverable:** Navigate within a single view via pan/zoom.

- Per-view transform state: `(scale, offset_x, offset_y)`.
- Keybindings: `+`/`-` zoom, arrow keys pan, `0` reset, `f` fit-to-canvas.
- Mouse wheel zoom centered on the cursor.
- Click-and-drag pan in regions not occupied by an interactive element (Phase 4 reserves clicks on elements for drill).
- Implementation uses Kitty's source-rect parameters; raster is not retransmitted.

**Acceptance:**

- 4× zoom retains crisp lines (4× DPI raster has the headroom).
- Pan feels smooth (≥ 30 fps perceived) on a typical macOS arm64 + WezTerm.
- Reset returns to fit-to-canvas exactly.

## Phase 4 — Click-to-drill (the differentiator)

**Deliverable:** Clicking an element with a child view navigates into that view.

- Extract per-element bounding boxes during SVG ingestion: `Vec<(ElementId, BBoxSvg)>` per view.
- From workspace.json, build `HashMap<(ViewId, ElementId), ChildViewId>`.
- Enable SGR pixel mouse mode at startup (when supported).
- On click event:
  1. Map terminal pixel → image pixel via the current view transform.
  2. Map image pixel → SVG coord via the persisted SVG-to-raster affine.
  3. Hit-test against the bbox list (innermost wins for nested elements).
  4. If the hit element has a child view, push current onto the breadcrumb stack and navigate.
  5. Otherwise, ignore (optional status hint).
- Backward navigation: `Backspace` pops the breadcrumb stack.
- Breadcrumb display in the status bar.

**Acceptance:**

- On a workspace with at least Landscape → SystemContext → Container → Component, clicking traverses the full chain.
- Clicking on whitespace does nothing.
- Breadcrumb correctly reflects depth and supports backtracking to root.

## Phase 5 — Polish and robustness

**Deliverable:** v1.0 — releasable.

- Reload command (`r`): re-runs export and refreshes views.
- Error states (unparseable workspace, missing `structurizr-cli`, broken view export): each surfaces a clear in-terminal dialog.
- Configuration file (`~/.config/c4tui/config.toml`) for keybindings and DPI.
- Help overlay (`?`).
- Logging: `--log-file <path>` and `RUST_LOG=...`.
- Documentation: man page, README usage section finalized.
- Performance budget: cold start under 5 s on a ten-view workspace (subprocess-bound).

**Acceptance:**

- A new user can clone, install, and view their workspace without reading source.
- All keybindings discoverable from the help overlay.
- Fuzz inputs do not crash the binary.

## Phase 6 — Distribution

**Deliverable:** Multiple install paths, automated release pipeline.

- GitHub Actions release workflow: tag → build for {macOS arm64, macOS amd64, linux amd64, linux arm64} → attach to GitHub Release.
- Publish to crates.io.
- Homebrew tap.
- README install section updated.

**Acceptance:**

- `brew install scshafe/tap/c4tui` works on a clean Mac.
- `cargo install c4tui` works.
- A new release builds and publishes from a single tag push.

## Sequencing notes

- Phases 0 → 4 are strictly sequential; each builds load-bearing infrastructure for the next.
- Phase 5 may begin in parallel with the tail of Phase 4 once the navigation skeleton is in place.
- Phase 6 begins only after Phase 5 acceptance is met.

## Cross-cutting risks

| Risk | Mitigation |
|---|---|
| Structurizr CLI changes its SVG format | Pin a known-good CLI version; integration test on each upgrade |
| Kitty protocol differences across WezTerm/Kitty/Ghostty | Test matrix in CI where feasible; explicit manual smoke-test list otherwise |
| SGR pixel mouse not universally supported | Phase 0 detection gates the feature; missing → click-to-drill disabled with a clear notice |
| Workspaces with hundreds of views blow memory | Lazy raster generation lands in Phase 2; not v1 priority beyond that |

## Out of plan

These are intentionally not on the roadmap:

- Editing workspaces.
- Web UI / non-terminal output.
- Sixel or iTerm2 image protocol fallback.
- Diff between two workspace versions.
- Plugin system.

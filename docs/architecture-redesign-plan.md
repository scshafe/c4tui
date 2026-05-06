# c4tui architecture redesign plan

Status: complete. Phases A–E have been implemented; this document is retained as the design record for the hardening pass.

This plan describes the next architecture pass before using c4tui heavily. The goal is not to change user-visible behavior; it is to make the app easier to extend, test, and harden.

## Goals

1. Introduce an explicit event/update model: `InputEvent -> Command -> AppState -> RenderFrame`.
2. Add a small `TerminalBackend` trait so terminal I/O can be tested without a real TTY.
3. Replace manual SVG XML geometry extraction with `usvg`-based geometry/transform handling.
4. Use typed IDs (`ViewId`, `ElementId`) instead of raw `usize`/`String` across module boundaries.

## Non-goals

- No feature expansion in this pass.
- No terminal protocol fallback beyond Kitty.
- No async runtime migration unless the refactor exposes a concrete need.
- No large UI framework rewrite; keep the direct terminal renderer unless/until it becomes a bottleneck.

## Proposed target architecture

```text
Raw terminal bytes
      |
      v
input parser -> InputEvent -> command mapping -> Command
                                              |
                                              v
                                         AppState::update
                                              |
                                              v
                                          RenderFrame
                                              |
                                              v
                                      TerminalBackend::render
```

Core types:

- `AppState`: pure-ish model of current view, breadcrumbs, transforms, modal state, config, and reload status.
- `InputEvent`: semantic input from terminal parsing, e.g. `KeyPress`, `MouseClick`, `MouseDrag`, `Resize`.
- `Command`: app intent, e.g. `Quit`, `OpenPicker`, `SelectView(ViewId)`, `NavigateBack`, `Zoom`, `Pan`, `Reload`.
- `RenderFrame`: declarative description of what should be displayed: current image, source rect, title/status, modal/dialog, image cache mutations.
- `TerminalBackend`: minimal imperative boundary for raw mode, reads, writes, image transfer, dimensions, and cleanup.

## Phase A — Typed IDs first

This is the safest first cut because it exposes hidden coupling without changing control flow.

Tasks:

- Add `src/ids.rs` with:
  - `ViewId(usize)` newtype.
  - `ElementId(String)` newtype.
- Convert public structs and methods to typed IDs:
  - `ViewInfo.key` remains a Structurizr view key string, but runtime selection uses `ViewId`.
  - `ViewStore::{view, rendered_view, transform, set_transform, child_view_at_canvas_point}` use `ViewId`.
  - `ElementBBox.element_id` and `child_view_by_element_id` use `ElementId`.
- Keep internal indexing via `ViewId::index()` to avoid churn.
- Update tests around view lookup and hit testing.

Acceptance:

- Existing behavior unchanged.
- `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test` pass.

## Phase B — Extract pure app state/update loop

Tasks:

- Split `src/app.rs` into:
  - `state.rs`: `AppState`, breadcrumbs, current view, transforms, modal state.
  - `event.rs`: `InputEvent` and `Command`.
  - `app.rs`: orchestration shell only.
- Add `Command::from_input(event, config, modal_state)` or a small `InputMapper`.
- Add `AppState::apply(command, store) -> UpdateResult`.
- Add `RenderFrame::from_state(state, store)` or `state.render_frame(store)`.
- Keep reload as an effect request rather than pure state mutation:
  - `Command::Reload` returns `Effect::ReloadWorkspace`.
  - Orchestrator performs the I/O, then feeds `Command::ReloadSucceeded(new_store)` / `ReloadFailed(error_text)` back into state.

Acceptance:

- Unit tests can exercise navigation, picker selection, back, zoom, pan, help, and reload failure without a TTY.
- `App::run` becomes mostly: read event, map command, apply, perform effects, render frame.

## Phase C — Introduce `TerminalBackend`

Tasks:

- Define trait in `terminal.rs` or new `backend.rs`:

  ```rust
  pub trait TerminalBackend {
      fn size(&self) -> TerminalSize;
      fn read_input(&mut self) -> Result<InputEvent>;
      fn render(&mut self, frame: &RenderFrame, store: &mut ViewStore) -> Result<()>;
      fn clear_image_cache(&mut self) -> Result<()>;
      fn show_blocking_dialog(&mut self, frame: &RenderFrame) -> Result<()>;
  }
  ```

- Rename current `TerminalSession` to `KittyTerminalBackend` or implement the trait for it.
- Add `FakeTerminalBackend` under `#[cfg(test)]` that records rendered frames and supplies scripted input events.
- Move direct calls to `read_key()` out of modal loops; modal choices should be ordinary events handled by `AppState`.
- Keep RAII raw-mode cleanup in the concrete backend.

Acceptance:

- View picker/help/reload interactions test through fake backend.
- TTY-specific code is isolated to backend/input/tty modules.

## Phase D — Replace manual SVG geometry extraction with `usvg`

Tasks:

- Investigate `usvg` node APIs in `resvg/usvg 0.45.1` for group IDs, bounding boxes, and transforms.
- Parse SVG once with `usvg::Tree` and use that same tree for both:
  - rasterization;
  - element bbox extraction.
- Extract element bboxes by walking the `usvg` tree and accumulating transformed geometry bounds, rather than parsing XML attributes manually.
- Preserve Structurizr element IDs from groups.
- Remove `roxmltree` dependency if no longer needed.
- Add regression tests for:
  - nested transforms;
  - scale/rotate where supported;
  - paths/text/images if `usvg` exposes reliable bounds;
  - malformed SVG still returns error, not panic.

Acceptance:

- Hit testing handles nested/compound SVG transforms correctly.
- Existing malformed SVG regression still passes.
- `roxmltree` removed unless there is a documented reason to keep it.

## Phase E — Reconcile docs and release readiness

Tasks:

- Update `architecture.md` to reflect event/update/backend boundaries.
- Add a brief "architecture notes" section to README only if useful for contributors.
- Run full gates:
  - `cargo fmt --check`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo test`
  - `cargo run -- --help`
  - `cargo package --allow-dirty`
  - `git diff --check`
- Push and verify CI.
- Run manual release workflow smoke test again if release workflow files changed.

## Suggested commit breakdown

1. `refactor: introduce typed view and element ids`
2. `refactor: add app event and command model`
3. `refactor: render frames through terminal backend`
4. `fix: derive svg hit boxes from usvg geometry`
5. `docs: update architecture for event backend model`

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| Refactor temporarily breaks interactive behavior | Keep small commits, run existing tests after each phase, manually smoke test in WezTerm/Kitty when possible. |
| `usvg` does not expose exactly the geometry needed | Spike first; if needed, keep XML ID discovery but use `usvg`/computed bounds for geometry. Document the compromise. |
| Event/update model becomes over-engineered | Keep types minimal and driven by existing behavior; no Redux-style abstraction beyond what tests need. |
| Backend trait leaks too many terminal details | Start with `render(RenderFrame)` and `read_input()` only; add methods only when forced. |

## Recommended order

Do Phase A first, then B/C together in a narrow vertical slice for one interaction path (`q`, arrows, current-view render). Once that compiles and tests well, migrate the rest of the commands. Do Phase D after the app/backend refactor so SVG geometry changes are isolated from state-machine churn.

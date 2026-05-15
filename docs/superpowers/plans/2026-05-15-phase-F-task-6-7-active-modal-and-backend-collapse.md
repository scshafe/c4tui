# c4tui Phase F Tasks 6+7 — switch to ActiveModal + collapse TerminalBackend trait

**Scope:** complete the modal-lifecycle unification started in Task 5. Switch the three modal handlers and spawn-sites to drive `ActiveModal`; collapse `draw_picker`/`close_picker`/`draw_connection_picker`/`close_connection_picker`/`draw_log_view` on `TerminalBackend` into `render_modal`/`close_modal`. Single commit (parent plan §1898–1904).

**Parent plan section:** `docs/superpowers/plans/2026-05-12-phase-3-navpicker-modal-image-elements.md` lines 1644–2242.

**Pre-flight drift fixes** (parent plan drafts a `SecondaryClassified` trait that doesn't exist; `NavItem::is_secondary()` already lives on the trait, so any `impl<T: NavItem + SecondaryClassified>` becomes `impl<T: NavItem>`; `ComponentOutcome` is `#[non_exhaustive]` so matches need `_ => Continue` — `modal.rs:159` already does this).

---

## Step 1 — Extend `Modal` trait (`src/modal.rs`)

- Add to `Modal`:
  - `fn pre_render_placements_to_clear(&self) -> Vec<u32> { Vec::new() }`
  - `fn post_render_thumbnails(&self) -> Vec<ThumbnailPlacement> { Vec::new() }`
- Add `pub struct ThumbnailPlacement { pub view_id: ViewId, pub area: CellArea }` (re-export-equivalent of `ThumbnailCellArea`; pick one canonical home — keep `ThumbnailCellArea` in `nav_picker.rs` and have `ThumbnailPlacement` just be its alias, or vice versa; favor a single name).
- Decision: rename `ThumbnailCellArea` → `ThumbnailPlacement` and re-export from `modal.rs`. nav_picker keeps building it; modal & terminal consume it.
- `impl<T: NavItem> Modal for NavPickerModal<T>`: implement both:
  - `pre_render_placements_to_clear` → `vec![MAIN_PLACEMENT_ID]`
  - `post_render_thumbnails` → forward to `self.thumbnails(self.picker.inner())`, mapped to `ThumbnailPlacement`.
- `impl Modal for LogView`: `pre_render_placements_to_clear` → `vec![MAIN_PLACEMENT_ID]`. Also need to clear all placements (today's `draw_log_view` does `delete_all_placements`); the parent plan ignores this. **Add a third method** `clear_all_placements_pre_render(&self) -> bool { false }` returning true for LogView, false otherwise. Keep it small.
- Add `use tui_kit::image::MAIN_PLACEMENT_ID;` and `use tui_kit::layout::CellArea;`.

## Step 2 — `TerminalBackend` trait (`src/backend.rs`)

- Replace the five modal methods with:
  - `fn render_modal(&mut self, modal: &mut dyn Modal, store: &ViewStore) -> Result<()>`
  - `fn close_modal(&mut self, store: &ViewStore) -> Result<()>`
- Drop now-unused imports (`Cached`, `NavPicker`, `LogView`, `ViewNavItem`, `ConnectionNavItem`).
- Update `FakeTerminalCall`: replace `DrawPicker`/`ClosePicker`/`DrawConnectionPicker`/`CloseConnectionPicker`/`DrawLogView` with `RenderModal` and `CloseModal`. Replace counter fields with `modal_renders: usize`.
- `FakeTerminalBackend::{render_modal, close_modal}` push the new variants.

## Step 3 — `TerminalSession` (`src/terminal.rs`)

- Implement `render_modal`:
  1. Walk `modal.pre_render_placements_to_clear()` → `self.inner.images().delete_placement(id)`.
  2. If `modal.clear_all_placements_pre_render()` → `self.inner.images().delete_all_placements().ok();`.
  3. `self.inner.draw(|frame| { let area = frame.area(); let _ = modal.render(area, frame.buffer_mut()); })?;`
  4. Walk `modal.post_render_thumbnails()`:
     - For each thumb, look up cached render in `store`, paint via `render_viewport_image` (lifted from current `draw_thumbnail`).
     - Then teardown any placement-id slots **not** in this set (lifted from current `draw_picker`).
  5. `self.inner.images().flush()?;`
- Implement `close_modal`: teardown all picker placements + flush (lifted from current `close_picker`). The connection-picker had no extra teardown; folding into one path is fine because pickers without thumbnails won't have placements registered.
- Delete the five old methods (`draw_picker`, `close_picker`, `draw_connection_picker`, `close_connection_picker`, `draw_log_view`).
- Keep `draw_thumbnail` private; reuse it from inside `render_modal`.

## Step 4 — `App` (`src/app.rs`) — handler/spawn-site switch

- Remove `PickerSlot`, `ConnectionPickerSlot`, `LogSlot`, `PickerAction` types and their fields. Keep `DialogSlot` + `dialog_slot`.
- Drop `#[allow(dead_code)]` on `active_modal`.
- Remove old imports tied to deleted slot types: `LogView`, `LogViewOutcome`, `ConnectionNavItem`, `Cached`, `ComponentOutcome`, `NavOutcome` (some may still be used inside spawn-site builders — verify).
- Add `use crate::modal::{ActiveModal, LogModalOutcome, Modal, NavModal, NavModalOutcome, NavPickerModal};`.
- New method `handle_modal_key(&mut self, key, terminal)`:
  - Read `self.active_modal.as_mut()`.
  - `match` on `ActiveModal::Nav { modal, .. }` and `ActiveModal::Log { modal }`.
  - Nav: `modal.handle_key(key)` returns `NavModalOutcome`. On `Continue` → `terminal.render_modal(modal.as_mut(), &self.store)`. On `Select(target)` → take `on_select`, run `terminal.close_modal(&self.store)`, pop scope, dispatch `Command` (if any), request render, full re-render. On `Cancel` → close_modal, pop scope, re-render.
  - **Hover prefetch**: after a `Continue`, call `modal.currently_hovered_view()`; if `Some(hover)` and not rendered, request hover render.
  - Log: `modal.handle_key(key, self.clipboard.as_ref())?` returns `LogModalOutcome`. On `Continue` → `render_modal`. On `Close` → `close_modal`, `return_to_root`, full re-render.
- Update `handle_key_event` match:
  - `SCOPE_PICKER | SCOPE_CONNECTION_PICKER | SCOPE_LOG => self.handle_modal_key(key, terminal)`.
- Rewrite the three `Effect::Open*` arms in `handle_input`:
  - `Effect::OpenPicker` → push picker scope, build `NavPickerModal { picker, into_target: NavTarget::View, hovered_view: |p| p.selected().map(|i| i.view_id), thumbnails: artifact-collector }`, set `active_modal = Some(ActiveModal::Nav { modal, on_select: |NavTarget::View(v), _| Some(Command::SelectView(v)) })`, render_modal.
  - `Effect::OpenChildViewPicker` → same pattern, `into_target: NavTarget::ChildView`, `on_select` matches `NavTarget::ChildView` → `Command::SelectChildView`.
  - `Effect::OpenConnectionPicker` → push connection-picker scope, build `NavPickerModal<ConnectionNavItem>` with `into_target: NavTarget::Connection`, `hovered_view: |_| None`, `thumbnails: |_| Vec::new()`, `on_select` matches `NavTarget::Connection` → `Command::SelectConnection`, render_modal.
- Rewrite `toggle_log_view`:
  - If `matches!(self.active_modal, Some(ActiveModal::Log { .. }))` → `return_to_root()`.
  - Else: `return_to_root()`, push log scope, `active_modal = Some(ActiveModal::Log { modal: Box::new(LogView::new(...)) })`.
- Update `return_to_root`: pop scopes, `self.active_modal = None`, `self.dialog_slot = None`.
- Update `redraw_for_mode` modal branches → call `terminal.render_modal(active.as_modal_mut(), &self.store)`. Need a helper on `ActiveModal` to widen to `&mut dyn Modal` since it's an enum, e.g.:
  ```rust
  impl ActiveModal {
      pub fn as_modal_mut(&mut self) -> &mut dyn Modal {
          match self {
              ActiveModal::Nav { modal, .. } => modal.as_mut(),
              ActiveModal::Log { modal } => modal.as_mut(),
          }
      }
  }
  ```
- Mouse-routing in `handle_input_event`: SCOPE_PICKER/CONNECTION_PICKER/LOG/DIALOG → still absorbed (already true).

## Step 5 — Tests (`src/app.rs` `#[cfg(test)] mod tests`)

- Six test literals to update. Mapping:
  - `DrawPicker` → `RenderModal`
  - `ClosePicker` → `CloseModal`
  - `DrawConnectionPicker` → `RenderModal`
  - `CloseConnectionPicker` → `CloseModal`
- Tests affected (from current app.rs):
  - `picker_navigation_selects_view_by_keystrokes`
  - `picker_cancel_runs_full_image_lifecycle_back_to_main_view`
  - `click_on_element_with_multiple_related_views_opens_picker_and_drills`
  - `related_view_picker_cancel_preserves_current_view`
  - `connection_picker_selects_connection_by_keystrokes`
  - `connection_picker_cancel_returns_to_current_view`
  - `connection_picker_opens_empty_without_pinning_source`
- `terminal.picker_draws >= 1` assertion uses removed field → swap to `terminal.modal_renders >= 1`.

## Step 6 — Verification

```
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --package c4tui
```

If clippy is noisy on `dead_code` for now-orphaned imports, fix the imports.

## Step 7 — Commit

```
git add -A
git commit -m "collapse three modal lifecycles into ActiveModal + Modal trait + render_modal/close_modal"
```

---

## Risk notes

- `render_modal` taking `&mut dyn Modal` means the connection-picker (no thumbnails) goes through the same path as the view picker. Acceptable because `post_render_thumbnails()` returns empty for it, so the placement-teardown loop runs over an empty active-set — which is equivalent to "tear down every picker placement," matching today's behavior since nothing was painted.
- `close_modal(&ViewStore)` runs the picker-placement teardown unconditionally. For the connection picker today, `close_connection_picker` only flushed images. Running the broader teardown on a code path that registered no placements is a no-op — safe.
- Hover-prefetch lives inside `handle_modal_key` instead of inside the picker spawn-site. Today the picker spawn-site also runs the initial hover prefetch (so the initial view's thumbnail starts rendering before the user moves). Keep that initial prefetch in the spawn-site; the in-handler prefetch handles all subsequent moves.
- LogView's `delete_all_placements` call is the only modal that needs it. Adding `clear_all_placements_pre_render` as a `Modal` method earns its keep because it's the cleanest way to thread that distinction without exposing the LogView identity to terminal.rs.

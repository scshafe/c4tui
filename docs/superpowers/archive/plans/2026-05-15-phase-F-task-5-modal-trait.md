# Phase F Task 5 — Modal trait + ActiveModal (Bite-Sized Plan)

**Status:** ACTIVE · 2026-05-15
**Repo:** c4tui
**Parent (operational):** [tui-kit's `2026-05-14-revised-library-author-implementation-plan.md`](/Users/coleshaffer/Projects/tui-kit/docs/superpowers/plans/2026-05-14-revised-library-author-implementation-plan.md)
**Parent (detailed):** [c4tui's `2026-05-12-phase-3-navpicker-modal-image-elements.md`](./2026-05-12-phase-3-navpicker-modal-image-elements.md) §Task 5 (lines 1386–1640).
**Predecessor in graph:** Phase F Tasks 1–4 (shipped, `386a3d3`); tui-kit Phase 3 plan Task 8 (image_box deletion, shipped at `15c41ab` in tui-kit).
**Unblocks:** Phase F Tasks 6 (switch handlers + delete old slots) and 7 (TerminalBackend method collapse), and eventually tui-kit Phase C deletions of `Modal`/`Overlay`/etc. in `src/elements/mod.rs`.

## Goal (lifted from parent plan Task 5)

Land the `Modal` + `NavModal` + `LogModal` traits, the `NavPickerModal<T>`
concrete, and the `ActiveModal` enum **alongside** the existing
`picker_slot`/`connection_picker_slot`/`log_slot` fields. Do not switch any
handler yet. The next task (Task 6) is what moves call sites one by one;
this task ships the new abstraction unused so the switch is mechanical.

## Why this is the next on-path work

Phase F handoff
(`c4tui/docs/superpowers/handoffs/2026-05-14-phase-F-navpicker-consolidation-complete.md`)
identifies Task 5 (Modal trait + ActiveModal) as "the natural next sub-task."
Tasks 2–4 left two parallel optional slot fields (`picker_slot`,
`connection_picker_slot`) whose unification belongs to this task. Once it
lands, Tasks 6/7 can move handler-by-handler, and tui-kit Phase C can then
delete the retained widgets that this unification proves unused.

## Plan drift discovered (and resolved)

The parent plan's example code references `SecondaryClassified` as a trait
bound on `T` for `NavPickerModal<T>` and `NavModal` impls. **This trait does
not exist in the current c4tui code** (grep verified). The current `NavItem`
trait already includes `is_secondary()` as a default method (line 74 of
`src/nav_picker.rs`). Resolution: drop the `SecondaryClassified` bound; use
just `T: NavItem` where the parent plan used `T: NavItem + SecondaryClassified`.

Other shape facts the plan assumes that are confirmed in current code:

- `Cached::render_to_buffer(area, target)`, `Cached::handle_event(event)`,
  `Cached::inner()` all exist (`/Users/coleshaffer/Projects/tui-kit/src/component.rs:286-340`).
- `NavPicker<T: NavItem>` implements `BufferComponent<Event = KeyEvent, Message = NavOutcome<T::Output>>`
  (`src/nav_picker.rs:412-526`).
- `NavPicker::handle_event` is infallible in practice — it always returns
  `Ok(_)`. The plan's `.expect("infallible")` is honest.
- `LogView::handle_key(&mut self, KeyEvent, &dyn Clipboard) -> Result<LogViewOutcome>`
  matches the plan (`src/log_view.rs:101-105`).
- `render_log_view(area, &mut Buffer, &mut LogView) -> Result<()>` is a
  private free function in `src/terminal.rs:402`. Needs `pub(crate)`
  visibility.
- `NavTarget` has three variants today: `View(ViewId)`, `ChildView(ViewId)`,
  `Connection(ConnectionNavigationCandidate)` (`src/nav_items.rs:36-47`).
  The `ChildView` variant is the c4tui-side mechanism the Phase F handoff
  noted: `ChildViewNavItem` was collapsed into `ViewNavItem`, and the action
  context (`SelectView` vs `Drill`) lives on `PickerSlot.action`. In the new
  modal world, the per-spawn-site `into_target` closure encodes that
  context.
- `c4tui` is a binary crate with modules declared in `src/main.rs` (no
  `src/lib.rs`). New `mod modal;` line goes there.

## Non-goals

- No handler switching. `App::picker_slot`, `connection_picker_slot`, and
  `log_slot` continue to be the wired path. `active_modal` is a `None`-only
  field after this commit.
- No deletion of existing slot types or handlers.
- No tui-kit changes.
- No new tests in this commit. The abstraction is unused; tests come in
  Task 6 when call sites move.

## Commit plan (1 commit, additive)

Single commit per the parent plan. All steps land together.

### Step 1 — Create `c4tui/src/modal.rs`

File contents follow the parent plan's final draft (lines 1495–1620),
adapted for the drift correction:

```rust
//! Modal lifecycle types for c4tui.
//!
//! `ActiveModal` is the single piece of `App` state that captures
//! "we're showing something on top of the diagram canvas right now". It
//! will replace the three near-identical optional slot fields (picker,
//! connection-picker, log) once Task 6 switches the handlers; in this
//! task we ship the abstraction alongside the existing slots so the
//! switch can land call-site-by-call-site.
//!
//! `Modal` is the rendering substrate: a one-method trait covering
//! everything a modal needs to paint into a buffer. `NavModal` adds the
//! NavPicker-shaped key-handling step; `LogModal` adds the LogView-shaped
//! one (with a `&dyn Clipboard` parameter).
//!
//! Dialog stays out of this module: it's a one-shot render driven by
//! `show_dialog` / `show_help` etc., not part of the per-frame modal
//! redraw loop.

use anyhow::Result;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use tui_kit::component::{BufferComponent, Cached, ComponentOutcome};
use tui_kit::input::KeyEvent;

use crate::clipboard::Clipboard;
use crate::ids::ViewId;
use crate::log_view::{LogView, LogViewOutcome};
use crate::nav_items::NavTarget;
use crate::nav_picker::{NavItem, NavOutcome, NavPicker, ThumbnailCellArea};
use crate::state::AppState;

/// Render-side contract for any modal. The terminal layer calls this on
/// every frame the modal owns the screen.
pub trait Modal {
    fn render(&mut self, area: Rect, buffer: &mut Buffer) -> Result<()>;
}

/// Key-handling contract for a NavPicker-shaped modal. Implementations
/// hold a `Cached<NavPicker<T>>` and translate its typed
/// `NavOutcome<T::Output>` into the union `NavTarget` at the trait
/// boundary, so the holding code (`ActiveModal::Nav(...)`) sees one
/// concrete outcome type.
pub trait NavModal: Modal {
    fn handle_key(&mut self, key: KeyEvent) -> NavModalOutcome;

    /// Hover hook used by the view-picker thumbnail prefetch. Returns the
    /// view-id the picker is currently showing, if any. Other `NavModal`
    /// impls return `None`.
    fn currently_hovered_view(&self) -> Option<ViewId> {
        None
    }

    /// Provide thumbnail anchors (cell areas + view-ids) for the terminal
    /// layer to paint kitty placements after rendering. Default = empty.
    fn thumbnails(&self) -> Vec<ThumbnailCellArea> {
        Vec::new()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum NavModalOutcome {
    Continue,
    Select(NavTarget),
    Cancel,
}

/// `LogView`'s key handling needs a clipboard, so it gets its own trait.
pub trait LogModal: Modal {
    fn handle_key(
        &mut self,
        key: KeyEvent,
        clipboard: &dyn Clipboard,
    ) -> Result<LogModalOutcome>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogModalOutcome {
    Continue,
    Close,
}

/// Blanket `Modal` impl: any `Cached<C>` whose inner is a `BufferComponent`
/// with `Event = KeyEvent` (which `NavPicker<T: NavItem>` is) renders by
/// replaying its cached buffer.
impl<C> Modal for Cached<C>
where
    C: BufferComponent<Event = KeyEvent>,
{
    fn render(&mut self, area: Rect, buffer: &mut Buffer) -> Result<()> {
        self.render_to_buffer(area, buffer)
    }
}

impl Modal for LogView {
    fn render(&mut self, area: Rect, buffer: &mut Buffer) -> Result<()> {
        crate::terminal::render_log_view(area, buffer, self)
    }
}

impl LogModal for LogView {
    fn handle_key(
        &mut self,
        key: KeyEvent,
        clipboard: &dyn Clipboard,
    ) -> Result<LogModalOutcome> {
        match LogView::handle_key(self, key, clipboard)? {
            LogViewOutcome::Continue => Ok(LogModalOutcome::Continue),
            LogViewOutcome::Close => Ok(LogModalOutcome::Close),
        }
    }
}

/// Concrete `NavModal`: one per spawn-site flavor. The closure inside
/// converts the typed `T::Output` into the appropriate `NavTarget`
/// variant.
pub struct NavPickerModal<T: NavItem> {
    pub picker: Cached<NavPicker<T>>,
    pub into_target: Box<dyn Fn(T::Output) -> NavTarget + Send>,
    pub hovered_view: Box<dyn Fn(&NavPicker<T>) -> Option<ViewId> + Send>,
    pub thumbnails: Box<dyn Fn(&NavPicker<T>) -> Vec<ThumbnailCellArea> + Send>,
}

impl<T: NavItem> Modal for NavPickerModal<T> {
    fn render(&mut self, area: Rect, buffer: &mut Buffer) -> Result<()> {
        self.picker.render(area, buffer)
    }
}

impl<T: NavItem> NavModal for NavPickerModal<T> {
    fn handle_key(&mut self, key: KeyEvent) -> NavModalOutcome {
        match self.picker.handle_event(&key).expect("NavPicker::handle_event is infallible") {
            ComponentOutcome::Message(NavOutcome::Select(out)) => {
                NavModalOutcome::Select((self.into_target)(out))
            }
            ComponentOutcome::Message(NavOutcome::Cancel) => NavModalOutcome::Cancel,
            ComponentOutcome::Message(NavOutcome::Continue) => NavModalOutcome::Continue,
            ComponentOutcome::Handled => NavModalOutcome::Continue,
            ComponentOutcome::Ignored => NavModalOutcome::Continue,
        }
    }

    fn currently_hovered_view(&self) -> Option<ViewId> {
        (self.hovered_view)(self.picker.inner())
    }

    fn thumbnails(&self) -> Vec<ThumbnailCellArea> {
        (self.thumbnails)(self.picker.inner())
    }
}

/// Top-level slot owned by `App`. The dialog stays as a separate App
/// field; its lifecycle does not share the per-frame redraw loop these
/// two flavors do.
pub enum ActiveModal {
    Nav {
        modal: Box<dyn NavModal + Send>,
        on_select:
            Box<dyn FnOnce(NavTarget, &mut AppState) -> Option<crate::event::Command> + Send>,
    },
    Log {
        modal: Box<dyn LogModal + Send>,
    },
}
```

Notes on shape:

- `ComponentOutcome` (from `tui_kit::component`) has variants the parent
  plan didn't enumerate. `NavPicker::handle_event` returns `Message(...)`
  for non-`Continue` outcomes and `Handled` for `Continue` (per
  `src/nav_picker.rs:508-512`). The match above covers `Continue`/
  `Handled`/`Ignored` defensively because `ComponentOutcome` may grow
  variants (it's non-exhaustive in spirit if not in attribute). If a
  future variant slips through, the closure path is the only one with a
  surprise — fall through to `Continue` is the safe default for "the
  picker hasn't decided anything yet".
- `Send` bounds on the closures and trait objects match the parent plan.
  c4tui is single-threaded in production; if `Send` bites a future
  consumer it can be dropped (it doesn't bite today because closures
  capture `Send`-safe data).

### Step 2 — Make `render_log_view` `pub(crate)`

`c4tui/src/terminal.rs:402`: change `fn render_log_view(...)` to
`pub(crate) fn render_log_view(...)`. No other changes.

### Step 3 — Add `mod modal;` to `c4tui/src/main.rs`

Insert `mod modal;` alphabetically in the module list (between `mod
logger;` and `mod nav_items;` — check exact ordering at write time).

### Step 4 — Add `active_modal: Option<ActiveModal>` to `App`

In `c4tui/src/app.rs`:

- Add `use crate::modal::ActiveModal;` at the top.
- Add the field to the `App` struct (after the four existing slot
  fields).
- Initialize to `None` in `App::new`.
- **Do not** add it to `return_to_root` cleanup yet — that belongs to
  Task 6 (when handlers actually use it).

### Step 5 — Compile-only check

```bash
cd /Users/coleshaffer/Projects/c4tui
cargo build 2>&1 | tail -10        # expected: clean
cargo clippy --all-targets --quiet # expected: clean
cargo test --quiet                 # expected: 95 passed (unchanged from current)
cargo fmt --check                  # expected: clean
```

No new tests in this commit per the parent plan. The abstraction is
unused so far.

### Commit message

```
add Modal/NavModal/LogModal traits and ActiveModal enum

Lands the new modal abstraction alongside the existing slot fields
(picker, connection_picker, log) so Task 6 can move handlers
call-site-by-call-site without an all-or-nothing flip.

Modal is render-only. NavModal extends it with handle_key →
NavModalOutcome (translating typed NavOutcome<T::Output> into the
union NavTarget at the trait boundary) plus optional currently_hovered_view
and thumbnails hooks. LogModal extends Modal with handle_key that takes
a &dyn Clipboard.

NavPickerModal<T: NavItem> is the concrete: Cached<NavPicker<T>> +
three closures (into_target, hovered_view, thumbnails). One per
spawn-site. ActiveModal { Nav | Log } is the top-level slot, with the
Nav variant carrying its on_select closure for AppState-aware routing.

App grows an `active_modal: Option<ActiveModal>` field, initialized to
None. The four existing slot fields remain. render_log_view becomes
pub(crate) so modal::impl Modal for LogView can reach it.

Closes Phase 3 plan Task 5 (parent plan exit criterion is gated on
Tasks 5-8 landing together; Task 6 is the next on-path piece).
```

## Plan-level Exit criteria (Task 5)

- ✅ `c4tui/src/modal.rs` exists with `Modal`, `NavModal`, `LogModal`,
  `NavModalOutcome`, `LogModalOutcome`, `NavPickerModal<T>`, `ActiveModal`.
- ✅ `Modal` is implemented for `Cached<C: BufferComponent<Event=KeyEvent>>`
  (blanket) and for `LogView` (direct).
- ✅ `LogModal` is implemented for `LogView`.
- ✅ `NavModal` and `Modal` are implemented for `NavPickerModal<T: NavItem>`.
- ✅ `App` carries `active_modal: Option<ActiveModal>` (initialized `None`).
- ✅ `render_log_view` is `pub(crate)` in `src/terminal.rs`.
- ✅ `cargo build` + `cargo clippy --all-targets` + `cargo test` + `cargo
  fmt --check` all green; test count unchanged at 95.

## Risks and mitigations

- **Risk: `ComponentOutcome` variant set is wider than the parent plan
  anticipated.** *Mitigation:* the `NavModal::handle_key` match covers
  `Message(...)`, `Handled`, `Ignored` explicitly and falls through to
  `Continue` for all non-Select/non-Cancel outcomes. If `ComponentOutcome`
  grows a new variant, the match becomes inexhaustive and the compiler
  catches it.
- **Risk: `Send` bound on the closures or trait objects fails to compile
  because some captured type is `!Send`.** *Mitigation:* drop the `Send`
  bound. The `App` is single-threaded; `Send` is forward-looking but not
  load-bearing. Per the parent plan: "If it bites, drop `Send`."
- **Risk: `Cached<NavPicker<T>>` doesn't satisfy `BufferComponent<Event=KeyEvent>`
  because `Cached` isn't a `BufferComponent` itself.** *Mitigation:* the
  blanket impl is on `Cached<C>` directly (not via `BufferComponent`).
  Verified the impl bound is `C: BufferComponent<Event = KeyEvent>`, which
  `NavPicker<T: NavItem>` satisfies (`src/nav_picker.rs:412-413`).
- **Risk: ordering of `mod modal;` matters because c4tui's
  `src/main.rs` uses crate-relative `use crate::modal::...` later.**
  *Mitigation:* `mod` ordering does not affect resolution; alphabetic
  placement is cosmetic.
- **Risk: adding the `active_modal` field changes `App`'s `Debug` impl
  (which is hand-rolled).** *Mitigation:* the manual `Debug` impl at
  `src/app.rs:81-88` uses `finish_non_exhaustive`, so adding a field
  doesn't break anything.

## What this plan explicitly does **not** decide

- Whether `ActiveModal::Dialog` should subsume the dialog flow. The
  parent plan says no — dialog has different lifecycle. Same answer
  here.
- Whether `NavModalOutcome::Cancel` should carry data. No — Cancel is
  always action-less.
- Whether to add an `impl Debug for ActiveModal`. Trait objects don't
  derive Debug; if a future test needs it, add a manual impl then.

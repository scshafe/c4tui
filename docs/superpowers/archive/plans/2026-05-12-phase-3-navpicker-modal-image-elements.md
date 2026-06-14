# Phase 3 — NavPicker + ActiveModal + image-widget winner + elements decision

**Status:** PLANNED · 2026-05-13
**Phase of:** [`tui-kit + c4tui refactor roadmap`](./2026-05-12-tui-kit-c4tui-refactor-roadmap.md)
**Prior phases (assumed merged):**
- [Phase 1 — tui-kit primitive cleanup](./2026-05-12-phase-1-tui-kit-primitive-cleanup.md)
- [Phase 2 — Input event boundary cleanup](./2026-05-12-phase-2-input-event-boundary.md)

**Scope:** four items from the 2026-05-12 architecture review, executed together because they all touch the same code paths in `c4tui/src/app.rs`, `c4tui/src/backend.rs`, `c4tui/src/terminal.rs`, and the three picker structs.

- **#1** — Build `NavPicker<T: NavItem>` with `NavOutcome<T>`; collapse `ViewPicker`, `ConnectionPicker`, and the inline child-view picker into it.
- **#2** — Preserve the `elements` render-effect nucleus; do not validate it through `NavPicker`.
- **#4** — Pick the surviving image-viewport widget; delete the loser.
- **#5** — Unify three modal-slot lifecycles into one `ActiveModal` + one `render_modal` trait method.

---

## Plan preconditions (decisions baked in before any task)

### Decision A — `#2`: **PRESERVE `tui_kit::elements` as the render-effect nucleus.**

**Verdict.** The earlier deletion plan judged `elements` by whether
`NavPicker<T>` would read better as `Window + Stack + Grid` than as direct
ratatui. That is the wrong test under the updated direction. tui-kit should
remain a terminal-first substrate for local apps today, while keeping a path to
a future local renderer for remote apps over SSH. That future needs structured
render intent: buffer cells plus explicit image upload, placement, teardown,
and cleanup effects.

**What survives.** The useful nucleus is already present: `TerminalEffect`,
`EffectElement`, `ImageViewportElement`, area-transforming effect forwarding in
`Stack`, `Panel`, `Padded`, `Bordered`, and `Overlay`, plus grouped placement
teardown in `Window`. Those are exactly the mechanics a future renderer backend
will need. Phase 3 preserves them.

**What does not happen in Phase 3.** Do not port `NavPicker` through
`elements`; pickers remain direct `Grid`/ratatui components. Do not broaden
`elements` into a retained runtime. Do not design the SSH/client protocol here.
Later work should likely rename or split `TerminalEffect` into a
renderer-neutral `RenderEffect`, move local application behind an adapter, and
shrink broad `Window` responsibilities into a narrower effect scope where
appropriate.

**Consequence.** Phase 3 replaces the former deletion task with an elements
preservation checkpoint: keep the module compiling, keep the README/spec
language honest about its render-effect purpose, and defer shrink/rename work
to the future renderer-backend phase.

### Decision B — `#4`: **`image_viewport` wins. Delete `image_box`.**

**Verdict.** Production code paths that touch images all go through `image_viewport`:
- `c4tui/src/view.rs` holds `viewports: HashMap<ViewId, ImageViewportWidget>` and imports `ImageScale`, `ImageViewportPlacement`, `PixelDistance`, `ResizePolicy`, `ScaledPixelOffset`, `StepDirection`, `ViewportAxis`, `ViewportImage`, `ZoomDirection`, `ZoomFactor` — eleven types from `image_viewport`.
- `c4tui/src/terminal.rs` uses `ImageViewportInitialScale`, `ImageViewportOptions`, `ResizePolicy`, `ViewportImage`, plus calls `Terminal::render_image_viewport`, `Terminal::render_viewport_image`, `Terminal::teardown_image_viewport(s)` — all `image_viewport`-rooted APIs.

Grep proves it: `grep -rn "tui_kit::widgets::image_box\|use.*image_box::\|ImageBox" /Users/coleshaffer/Projects/c4tui/src/` returns **zero hits**. `ImageBox` is 907 lines of widget the README mistakenly recommends and that no real consumer touches.

**Why this is unambiguous.** The architecture review's framing assumed `image_box` might be "newer, smaller, cleaner". Reading both: `image_box` is a thinner convenience layer (centered crop, optional border, simpler state model), but it does not subsume `image_viewport`'s scale/offset/zoom-factor machinery. Migrating c4tui to `image_box` would mean rebuilding the pan/zoom/step semantics that `ImageViewport` already encodes — a Phase 6-sized rewrite, not a Phase 3 line-item. The README oversold it. We delete it.

**Consequence.** `tui-kit/src/widgets/image_box.rs` (907 lines) is deleted. The prelude entries `ImageBox`, `ImageBoxPlan`, `ImageBoxState` and the module `tui_kit::widgets::image_box` vanish. tui-kit/README.md's "ImageBox is the streamlined image viewport" section goes; the README is reconciled to point at `ImageViewport` exclusively (the deeper doc reconciliation lives in Phase 7, but the locally-misleading lines must go in this phase to keep the README honest).

### Decision C — `NavTarget` shape: **single union enum.**

```rust
pub enum NavTarget {
    View(ViewId),                                  // from current ViewPicker (top-level)
    ChildView(ViewId),                             // from OpenChildViewPicker effect
    Connection(ConnectionNavigationCandidate),     // from current ConnectionPicker
    // Phase 5 will add: Link(LinkCandidate)
}
```

**Why a union, not three picker variants.** The downstream routing seam is a single function: `ModalSlot::on_select(NavTarget, &mut AppState) -> ()` (more precisely, it produces a `Command` we feed into `state.apply`). One closure per spawn-site means each spawn-site already knows which variant it will receive; we lose no type discipline. The cost of three enum variants on `ActiveModal` is three render branches per redraw + three handler branches per key — which is exactly what we're collapsing. **One union** keeps the seam at one function pointer.

**Why this is forward-compatible with Phase 5.** `NavPicker<LinkCandidate>` in Phase 5 just maps its `Select(LinkCandidate)` outcome into `NavTarget::Link(...)` at the spawn-site (or, alternatively, `NavPicker<T>` parameterizes on `T: NavItem` and the spawn-site supplies a `T -> Command` closure — see the LinkCandidate sketch at the end of this plan).

### Decision D — `Modal` trait shape

LogView doesn't implement `BufferComponent` today and shouldn't have to: it owns a clipboard reference passed into `handle_key`, which doesn't fit `BufferComponent::handle_event(&Event)`. Dialog is stateless. NavPicker is fully `BufferComponent`-shaped.

We introduce a narrow `Modal` trait in `c4tui/src/backend.rs`:

```rust
pub trait Modal {
    fn render(&mut self, area: Rect, buffer: &mut Buffer) -> Result<()>;
}
```

That is the entire trait. `Cached<NavPicker<T>>` implements it via `render_to_buffer`. `LogView` implements it by calling the existing `render_log_view` helper. The `TerminalBackend::render_modal(&mut dyn Modal)` method takes that one trait. **The dialog stays out** of this trait — it's a one-shot render that doesn't share a per-frame redraw loop with anything, so it keeps its dedicated `show_dialog` / `show_help` / `show_message` / `show_error` methods. Three modal lifecycles collapse to one; the dialog is the fourth thing and stays the fourth thing because pulling it under `Modal` would force LogView and NavPicker to also handle the "render once, then absorb any key" semantics, which they don't want.

> **Speculation watchpoint:** `Modal` is exactly one method right now. If a future modal needs e.g. `on_resize` or `pre_render_image_teardown`, add it then. Do not pre-add it now.

---

## File-structure section

### tui-kit (substrate)

| Path | Action | Why |
|---|---|---|
| `tui-kit/src/elements.rs` | Preserve; optionally touch docs/comments only if they contradict the render-effect direction | Decision A |
| `tui-kit/src/widgets/image_box.rs` | **DELETE** (907 lines) | Decision B |
| `tui-kit/src/widgets/mod.rs` | Modify: drop `pub mod image_box;` | Decision B |
| `tui-kit/src/lib.rs` | No `elements` deletion | Decision A |
| `tui-kit/src/prelude.rs` | Modify: drop `ImageBox`/`ImageBoxPlan`/`ImageBoxState`; keep existing element exports unless Phase 3 code proves a specific export is wrong | Decisions A + B |
| `tui-kit/README.md` | Modify: remove `ImageBox` row from widget table; remove the `## ImageBox` section | Decision B |
| `tui-kit/specification.md`, `tui-kit/architecture.md` | Touch only the lines that contradict the post-deletion code (full reconciliation is Phase 7) | Honesty during the phase |
| (tests under `tui-kit/tests/`) | Delete any test that exercises `ImageBox`; keep elements tests that prove effect forwarding/teardown | Decisions A + B |

### c4tui (consumer)

| Path | Action | Why |
|---|---|---|
| `c4tui/src/picker.rs` | **DELETE** (696 lines) | Replaced by NavPicker |
| `c4tui/src/connection_picker.rs` | **DELETE** (542 lines) | Replaced by NavPicker |
| `c4tui/src/nav_picker.rs` | **CREATE** | The new `NavPicker<T: NavItem>` + `NavItem` + `NavOutcome<T>` |
| `c4tui/src/nav_items.rs` | **CREATE** | `NavTarget` enum + `ViewNavItem`/`ChildViewNavItem`/`ConnectionNavItem` (the concrete item types, each with their `impl NavItem`) |
| `c4tui/src/modal.rs` | **CREATE** | `Modal` trait, `ActiveModal` enum, `ModalSlot<T>` |
| `c4tui/src/backend.rs` | Modify: collapse three trait-method pairs into `render_modal` + `close_modal`; shrink FakeTerminalBackend | Item #5 |
| `c4tui/src/terminal.rs` | Modify: replace `draw_picker`/`close_picker`/`draw_connection_picker`/`close_connection_picker`/`draw_log_view` with `render_modal`/`close_modal` impls | Item #5 |
| `c4tui/src/app.rs` | Modify: replace three slot fields with `active_modal: Option<ActiveModal>`; replace three handlers with `handle_modal_key`; rewrite three `Effect::Open*Picker` arms to construct `ActiveModal::Nav(ModalSlot::new(...))` | Item #5 |
| `c4tui/src/state.rs` | Modify: `Effect::OpenChildViewPicker { target_view_ids }` and `Effect::OpenConnectionPicker { source_element_id }` and `Effect::OpenPicker` collapse to `Effect::OpenModal(ModalSpec)` (Phase 4 will further narrow this — we only do the consolidation needed to feed `ActiveModal` here) | Item #5 |
| `c4tui/src/lib.rs` | Modify: drop `mod picker; mod connection_picker;`, add `mod nav_picker; mod nav_items; mod modal;` | structure |

### What we deliberately do **not** touch in Phase 3

- `c4tui/src/view.rs` (the `ViewStore` split is Phase 6 — leave the file alone except for one cosmetic import path change if Phase 1's prelude slim moved any `image_viewport` types).
- `c4tui/src/event.rs` (the `Command`/`PendingCommand` collapse is Phase 4; we keep both enums working).
- `c4tui/src/keymap.rs` (no key-binding changes in Phase 3).
- `c4tui/src/render.rs`, `render_pool.rs`, `clipboard.rs`, `workspace.rs`, `logger.rs` (unaffected).

---

## Task ordering rationale

The roadmap suggested ordering is: (i) NavPicker shape → (ii) migrate first picker → (iii) migrate second picker → (iv) ActiveModal → (v) collapse trait methods → (vi) image-widget swap → (vii) elements checkpoint → (viii) verification.

I'm reordering slightly. **The image-widget deletion (vi) and the elements
preservation checkpoint (vii) belong at the end of the phase, after NavPicker
and ActiveModal compile clean.** Reason: the high-risk refactors should run
against a code state we know already compiles. The elements checkpoint should
confirm that no accidental dependency or doc contradiction was introduced; it
should not become a remote-render protocol project.

I'm also splitting the original (ii)/(iii) — "migrate first/second picker" — into three migrations because there are three current picker call sites (top-level ViewPicker, child-view ViewPicker via Effect, ConnectionPicker), and each migration is its own commit.

Final order:

1. **NavPicker shape (TDD, no consumers yet).** `nav_picker.rs` + `nav_items.rs` with `NavTarget`, `NavItem` trait, `NavPicker<T>` struct, `NavOutcome<T>`, `impl BufferComponent for NavPicker<T>`. All three `NavItem` impls (`ViewNavItem`, `ChildViewNavItem`, `ConnectionNavItem`) ship with this task. Tests in `nav_picker.rs` cover filter/arrow/Esc/Enter/Tab behavior using a fake `NavItem` and one real-typed test per concrete item.

2. **Migrate top-level ViewPicker → `NavPicker<NavTarget>`.** Edit `app.rs`'s `Effect::OpenPicker` arm to spawn a `NavPicker` configured from `ViewNavItem::collect(&store.views, &store.model, current)`. Edit `terminal.rs::draw_picker` and `close_picker` to take a `&mut Cached<NavPicker<NavTarget>>` (still using the old slot field names — those die in task 5). Existing tests for `picker_navigation_selects_view_by_keystrokes` and `picker_cancel_runs_full_image_lifecycle_back_to_main_view` should still pass.

3. **Migrate child-view picker → same NavPicker.** Edit `Effect::OpenChildViewPicker` arm to spawn the same `NavPicker<NavTarget>` configured with `ChildViewNavItem` items. Use `NavTarget::ChildView(_)` for outcomes. Test `click_on_element_with_multiple_related_views_opens_picker_and_drills` is the green light.

4. **Migrate ConnectionPicker → same NavPicker.** Edit `Effect::OpenConnectionPicker` arm to spawn the same `NavPicker<NavTarget>` configured with `ConnectionNavItem` items. Outcomes are `NavTarget::Connection(...)`. Test `connection_picker_selects_connection_by_keystrokes` is the green light. **At end of this task, `picker.rs` and `connection_picker.rs` are deletable.** Delete them. Drop the `mod picker; mod connection_picker;` lines from `lib.rs`.

5. **Introduce `Modal` trait + `ActiveModal` + `ModalSlot`.** Create `modal.rs`. Implement `Modal` for `Cached<NavPicker<NavTarget>>` and for `LogView`. `ActiveModal` enum has two variants for now: `Nav(ModalSlot<Cached<NavPicker<NavTarget>>>)` and `Log(ModalSlot<LogView>)`. (Dialog stays as today — see Decision D.) Add `active_modal: Option<ActiveModal>` field to `App`. **Do not delete the old slot fields yet** — both representations coexist for one commit so we can move handler-by-handler.

6. **Replace handlers with `handle_modal_key`.** Rewrite `handle_key_picker`/`handle_key_connection_picker`/`handle_key_log` into a single `handle_modal_key(key, terminal)` method that dispatches on `&mut self.active_modal`. Each `ActiveModal` variant carries its own `on_select` closure (`FnOnce(NavTarget, &mut AppState, CanvasMetrics) -> Effect` for Nav, `FnOnce(LogViewOutcome, &mut AppState) -> ()` for Log). Update the three `Effect::Open*` arms in `handle_input` to populate `active_modal` instead of the old slot fields. Delete the old slot fields and `PickerSlot`/`ConnectionPickerSlot`/`LogSlot` structs.

7. **Collapse `TerminalBackend` trait methods.** Replace `draw_picker`/`close_picker`/`draw_connection_picker`/`close_connection_picker`/`draw_log_view` with two trait methods: `render_modal(&mut dyn Modal)` and `close_modal()`. Update `TerminalSession` and `FakeTerminalBackend` accordingly. The `FakeTerminalCall` enum shrinks: `DrawPicker`/`ClosePicker`/`DrawConnectionPicker`/`CloseConnectionPicker`/`DrawLogView` collapse to `RenderModal` + `CloseModal`. Update the existing `App` tests' expected call lists.

8. **Delete `image_box`.** Remove `tui-kit/src/widgets/image_box.rs`, the `mod image_box;` line in `widgets/mod.rs`, and the three prelude re-exports (`ImageBox`, `ImageBoxPlan`, `ImageBoxState`). Edit `tui-kit/README.md` to remove the `ImageBox` row from the widget table and the `## ImageBox` section. Run `cargo test` in both repos.

9. **Elements preservation checkpoint.** Keep `tui-kit/src/elements.rs`. Confirm
   that `TerminalEffect`, `EffectElement`, `ImageViewportElement`, and
   area-transforming containers still compile and have tests for effect
   forwarding/teardown. Update only local docs/comments that still claim
   elements should be validated by NavPicker or deleted. Do not implement
   `RenderEffect`, remote transport, or an SSH client in this phase.

10. **Full verification.** `cargo test` in both repos green, `cargo clippy --all-targets --all-features -- -D warnings` clean, `cargo fmt --check` clean. Manual exercise: open c4tui, run through (a) open picker / select / drill / back, (b) connection picker via Enter on a pinned element, (c) child-view picker via clicking an element with multiple related views, (d) log view via L, (e) help dialog via ?. Each modal opens, accepts keys, closes cleanly.

---

## Tasks

Each task below is self-contained: exact paths, complete code where load-bearing, the test to write/run before the implementation, and a commit message. Steps within a task are ~2–10 minutes.

### Task 1 — NavPicker shape, in isolation (TDD)

**Goal:** A standalone `NavPicker<T: NavItem>` that compiles and tests green without any picker call site existing yet. This is the moment to get the abstraction right; nothing downstream constrains it.

**Files created:**
- `c4tui/src/nav_picker.rs`
- `c4tui/src/nav_items.rs`

**Files modified:**
- `c4tui/src/lib.rs` (add `mod nav_picker; mod nav_items;`)

#### Step 1.1 — Write `nav_items.rs` skeleton (no impls yet)

```rust
// c4tui/src/nav_items.rs
//! Concrete `NavItem` types and the `NavTarget` enum every NavPicker spawned
//! by c4tui produces.

use crate::ids::{ElementId, ViewId};
use crate::view::ConnectionNavigationCandidate;
use crate::workspace::{ViewInfo, ViewKind, WorkspaceModel};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::nav_picker::{NavItem, NavItemRow};

/// What the user just chose from a navigation picker.
///
/// One union for every NavPicker variant c4tui spawns. The downstream
/// callback (`ModalSlot::on_select`) matches on this to produce a `Command`.
#[derive(Debug, Clone, PartialEq)]
pub enum NavTarget {
    /// A view selected from the top-level view picker (clears breadcrumbs).
    View(ViewId),
    /// A view selected from the child-view picker spawned on multi-child drill
    /// (pushes a breadcrumb).
    ChildView(ViewId),
    /// A connection candidate selected from the connection picker (pushes a
    /// breadcrumb and pins the connected element).
    Connection(ConnectionNavigationCandidate),
    // Phase 5 will add:
    //   Link(LinkCandidate),
}

/// Item that yields `NavTarget::View(...)` when selected.
#[derive(Debug, Clone)]
pub struct ViewNavItem { /* fields per Step 1.6 */ }

/// Item that yields `NavTarget::ChildView(...)` when selected. Identical
/// fields to `ViewNavItem` — the only thing that differs is what the
/// spawn-site does with the resulting NavTarget variant. Keeping them as
/// separate types means the spawn-site's `on_select` closure receives a
/// typed NavTarget and the type-level discipline catches "I forgot to
/// switch which variant I'm building" errors at compile time.
#[derive(Debug, Clone)]
pub struct ChildViewNavItem { /* fields per Step 1.6 */ }

/// Item that yields `NavTarget::Connection(...)` when selected.
#[derive(Debug, Clone)]
pub struct ConnectionNavItem { /* fields per Step 1.6 */ }
```

Commit: `add NavTarget enum and empty NavItem types`.

#### Step 1.2 — Write `nav_picker.rs` `NavItem` trait + `NavOutcome` + tests stub

```rust
// c4tui/src/nav_picker.rs
//! Generic, keyboard-driven picker over any type implementing `NavItem`.
//!
//! NavPicker replaces three near-identical c4tui pickers (`ViewPicker`,
//! `ConnectionPicker`, and the inline child-view picker spawned by the
//! `OpenChildViewPicker` effect). Item-type specialisation happens entirely
//! through the `NavItem` trait; the layout, filter, scroll, and key handling
//! live here once.

use anyhow::Result;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};
use tui_kit::component::{
    BufferComponent, ComponentId, ComponentOutcome, DirtyReason, DirtyState,
};
use tui_kit::input::KeyEvent;
use tui_kit::layout::CellArea;
use tui_kit::widgets::grid::{Grid, GridStyle};

/// What a NavPicker reports up to its spawning code on each key.
#[derive(Debug, Clone, PartialEq)]
pub enum NavOutcome<T> {
    Continue,
    Select(T),
    Cancel,
}

/// One filterable, render-able row in a NavPicker.
///
/// The trait splits "what's filterable about this row" (`filter_text`,
/// `secondary_filter_tokens`) from "what does the row look like rendered"
/// (`render_into_canvas`) from "what does this row produce when selected"
/// (`Output` + `outcome`).
///
/// Item types are deliberately concrete c4tui structs (`ViewNavItem`,
/// `ConnectionNavItem`, `ChildViewNavItem`). Phase 5 adds a fourth
/// (`LinkCandidate`). Adding more is one `impl NavItem` block — no changes
/// to NavPicker itself.
pub trait NavItem: Clone {
    /// What `Enter` produces when this item is selected.
    type Output: Clone;

    /// Primary filterable string ("name", "label"). Lowercase-subsequence
    /// matched against the user's filter input.
    fn filter_text(&self) -> &str;

    /// Optional grouping section. Items with the same group sort together
    /// under a `── Group ──` header. None means "no header for this item"
    /// (used by ConnectionNavItem, which is flat).
    fn group(&self) -> Option<&str> {
        None
    }

    /// Secondary filterable tokens (element names contained inside the
    /// item, technology keywords on relationships, etc.). Used by ViewNavItem
    /// to surface "this view contains element X" matches.
    fn secondary_filter_tokens(&self) -> &[String] {
        &[]
    }

    /// Render this row into one Grid cell. The implementer owns the entire
    /// cell — title, detail rows, key hint, thumbnail anchor (see
    /// `record_thumbnail` below).
    fn render_into_canvas(
        &self,
        canvas: NavCellCanvas<'_>,
        selected: bool,
        filter: &str,
        sink: &mut dyn FnMut(NavRenderArtifact),
    );

    /// Produce the outcome the spawning code receives when this item is
    /// Enter-selected.
    fn outcome(&self) -> Self::Output;
}

/// Side-channel for things `render_into_canvas` wants to surface to the
/// spawning code without putting them in the buffer (thumbnails, hover hints).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum NavRenderArtifact {
    /// A cell area where the spawning code should later place an image
    /// thumbnail (kitty placement). Used by ViewNavItem to surface the
    /// thumbnail anchor for the existing thumbnail-rendering path in
    /// `TerminalSession::draw_picker`.
    Thumbnail { id: ThumbnailId, area: CellArea },
}

/// What the thumbnail belongs to. Today only views have thumbnails, but
/// we use an opaque newtype so Phase 5's LinkCandidate could grow one
/// without retouching this trait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThumbnailId(pub crate::ids::ViewId);

/// Thin wrapper exposing a `Grid` cell canvas with NavItem-friendly helpers.
pub struct NavCellCanvas<'a> {
    pub(crate) inner: tui_kit::widgets::grid::GridCellCanvas<'a>,
}

impl<'a> NavCellCanvas<'a> {
    pub fn width(&self) -> u16 {
        self.inner.width()
    }
    pub fn height(&self) -> u16 {
        self.inner.height()
    }
    pub fn style(&self) -> Style {
        self.inner.style()
    }
    pub fn set_string(&mut self, x: u16, y: u16, text: impl AsRef<str>, style: Style) {
        self.inner.set_string(x, y, text, style)
    }
    pub fn local_cell_area(&self, x: u16, y: u16, w: u16, h: u16) -> CellArea {
        self.inner.local_cell_area(x, y, w, h)
    }
}

/// Configuration for a NavPicker spawned by a c4tui call site.
#[derive(Debug, Clone)]
pub struct NavPickerConfig {
    pub id: ComponentId,
    /// Window title rendered in the block's top border.
    pub title: String,
    /// Footer hint rendered in the block's bottom border.
    pub footer_hint: String,
    /// Header text rendered on the first body row when filter is empty.
    pub default_header: String,
    /// Minimum width per cell in the Grid layout.
    pub min_cell_cols: u16,
    /// Cell height in the Grid layout.
    pub cell_rows: u16,
    /// Whether the picker accepts free-form filter typing. ViewPicker = yes;
    /// ConnectionPicker = no (it ignores Char keys).
    pub allows_filter: bool,
    /// Whether Tab toggles between "show only primary items" and "show all
    /// including legend/key items". ViewPicker = yes; ConnectionPicker = no.
    pub allows_secondary_toggle: bool,
    /// Label for the secondary-toggle state in the footer (e.g. "legends").
    /// Ignored if `allows_secondary_toggle` is false.
    pub secondary_toggle_label: &'static str,
}

/// Per-item flag deciding whether `allows_secondary_toggle == false` items
/// are hidden in the default view. ViewNavItem uses this to hide legend
/// views by default; other items pass-through.
pub trait SecondaryClassified {
    fn is_secondary(&self) -> bool {
        false
    }
}

#[derive(Debug)]
pub struct NavPicker<T: NavItem> {
    config: NavPickerConfig,
    items: Vec<T>,
    filter: String,
    show_secondary: bool,
    selected_index: usize,
    dirty: DirtyState,
    last_artifacts: Vec<NavRenderArtifact>,
}

impl<T: NavItem + SecondaryClassified> NavPicker<T> {
    pub fn new(config: NavPickerConfig, items: Vec<T>, initial_selection: usize) -> Self {
        let selected_index = initial_selection.min(items.len().saturating_sub(1));
        Self {
            config,
            items,
            filter: String::new(),
            show_secondary: false,
            selected_index,
            dirty: DirtyState::paint(DirtyReason::Explicit),
            last_artifacts: Vec::new(),
        }
    }

    pub fn selected(&self) -> Option<&T> {
        self.visible_items().get(self.selected_index).copied()
    }

    pub fn last_artifacts(&self) -> &[NavRenderArtifact] {
        &self.last_artifacts
    }

    fn visible_items(&self) -> Vec<&T> {
        self.items
            .iter()
            .filter(|item| self.show_secondary || !item.is_secondary())
            .filter(|item| matches_filter(&self.filter, *item))
            .collect()
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> NavOutcome<T::Output> {
        match key {
            KeyEvent::Esc => {
                if self.filter.is_empty() {
                    NavOutcome::Cancel
                } else {
                    self.filter.clear();
                    self.dirty.mark_paint(DirtyReason::Input);
                    self.dirty.mark_image_placement(DirtyReason::Input);
                    NavOutcome::Continue
                }
            }
            KeyEvent::CtrlC => NavOutcome::Cancel,
            KeyEvent::Enter => self
                .selected()
                .map(|item| NavOutcome::Select(item.outcome()))
                .unwrap_or(NavOutcome::Continue),
            KeyEvent::Up => {
                self.move_selection(-1);
                self.dirty.mark_paint(DirtyReason::Input);
                NavOutcome::Continue
            }
            KeyEvent::Down => {
                self.move_selection(1);
                self.dirty.mark_paint(DirtyReason::Input);
                NavOutcome::Continue
            }
            KeyEvent::Tab if self.config.allows_secondary_toggle => {
                self.show_secondary = !self.show_secondary;
                self.clamp_selection();
                self.dirty.mark_paint(DirtyReason::Input);
                self.dirty.mark_image_placement(DirtyReason::Input);
                NavOutcome::Continue
            }
            KeyEvent::Tab => {
                // ConnectionPicker used Tab as a synonym for Down. Preserve
                // that behavior when filter+secondary are both off.
                self.move_selection(1);
                self.dirty.mark_paint(DirtyReason::Input);
                NavOutcome::Continue
            }
            KeyEvent::Back if self.config.allows_filter => {
                self.filter.pop();
                self.clamp_selection();
                self.dirty.mark_paint(DirtyReason::Input);
                self.dirty.mark_image_placement(DirtyReason::Input);
                NavOutcome::Continue
            }
            KeyEvent::Char(c) if self.config.allows_filter => {
                self.filter.push(c);
                self.clamp_selection();
                self.dirty.mark_paint(DirtyReason::Input);
                self.dirty.mark_image_placement(DirtyReason::Input);
                NavOutcome::Continue
            }
            _ => NavOutcome::Continue,
        }
    }

    fn move_selection(&mut self, delta: i32) {
        let count = self.visible_items().len();
        if count == 0 {
            return;
        }
        let next = (self.selected_index as i32 + delta).rem_euclid(count as i32);
        self.selected_index = next as usize;
    }

    fn clamp_selection(&mut self) {
        let count = self.visible_items().len();
        if count == 0 {
            self.selected_index = 0;
            return;
        }
        if self.selected_index >= count {
            self.selected_index = count - 1;
        }
    }
}

impl<T: NavItem + SecondaryClassified> BufferComponent for NavPicker<T> {
    type Event = KeyEvent;
    type Message = NavOutcome<T::Output>;

    fn id(&self) -> &ComponentId {
        &self.config.id
    }

    fn render_buffer(&mut self, area: Rect, buffer: &mut Buffer) -> Result<()> {
        self.last_artifacts.clear();
        let block = Block::default()
            .borders(Borders::ALL)
            .title(self.config.title.clone())
            .title_bottom(self.config.footer_hint.clone());
        let inner = block.inner(area);
        Clear.render(area, buffer);
        block.render(area, buffer);
        if inner.height < 3 || inner.width < 8 {
            return Ok(());
        }

        let header_text = if self.filter.is_empty() {
            self.config.default_header.clone()
        } else {
            format!("Filter: {}", self.filter)
        };
        let header_avail = inner.width.saturating_sub(1) as usize;
        Paragraph::new(truncate(&header_text, header_avail)).render(
            Rect {
                x: inner.x,
                y: inner.y,
                width: inner.width,
                height: 1,
            },
            buffer,
        );

        let body = Rect {
            x: inner.x,
            y: inner.y + 2,
            width: inner.width,
            height: inner.height.saturating_sub(2),
        };

        let visible: Vec<T> = self.visible_items().into_iter().cloned().collect();
        if visible.is_empty() {
            buffer.set_string(body.x, body.y, "No matches", Style::default());
            return Ok(());
        }

        let style = GridStyle {
            selected_cell: Style::default().add_modifier(Modifier::REVERSED),
            scroll_up: "▲",
            scroll_down: "▼",
            ..GridStyle::default()
        };

        let filter = self.filter.clone();
        let selected_index = self.selected_index;
        let artifacts: &mut Vec<NavRenderArtifact> = &mut self.last_artifacts;

        Grid::new()
            .with_cell_rows(self.config.cell_rows)
            .with_min_cell_cols(self.config.min_cell_cols)
            .with_selected_index(Some(selected_index))
            .with_style(style)
            .render(body, buffer, &visible, |cell, canvas| {
                if canvas.width() < 4 {
                    return;
                }
                let nav_canvas = NavCellCanvas { inner: canvas };
                cell.item.render_into_canvas(
                    nav_canvas,
                    cell.selected,
                    &filter,
                    &mut |artifact| artifacts.push(artifact),
                );
            });

        Ok(())
    }

    fn handle_event(&mut self, event: &KeyEvent) -> Result<ComponentOutcome<NavOutcome<T::Output>>> {
        let outcome = self.handle_key(*event);
        Ok(match outcome {
            NavOutcome::Continue => ComponentOutcome::Handled,
            other => ComponentOutcome::Message(other),
        })
    }

    fn dirty(&self) -> &DirtyState {
        &self.dirty
    }

    fn mark_dirty(&mut self, reason: DirtyReason) {
        self.dirty.mark_paint(reason);
    }

    fn clear_dirty(&mut self) {
        self.dirty.clear();
    }
}

fn matches_filter<T: NavItem>(filter: &str, item: &T) -> bool {
    if filter.is_empty() {
        return true;
    }
    let needle = filter.to_ascii_lowercase();
    if subsequence_match(&needle, &item.filter_text().to_ascii_lowercase()) {
        return true;
    }
    item.secondary_filter_tokens()
        .iter()
        .any(|tok| subsequence_match(&needle, &tok.to_ascii_lowercase()))
}

fn subsequence_match(needle: &str, haystack: &str) -> bool {
    let mut h = haystack.chars();
    'outer: for nc in needle.chars() {
        for hc in h.by_ref() {
            if hc == nc {
                continue 'outer;
            }
        }
        return false;
    }
    true
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        text.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
    }
}
```

**Note on `Grid::new(...).render` and the borrow checker.** The closure we pass to `Grid::render` borrows `artifacts: &mut Vec<NavRenderArtifact>` while also borrowing `&visible`. That's fine: `visible` is owned outside the closure and is reborrowed shared. If you hit a borrow error, the fix is to extract `let mut artifacts = std::mem::take(&mut self.last_artifacts);` before the call and put it back after. Do not push this up to the trait.

#### Step 1.3 — Pure-logic tests of NavPicker (no concrete item yet)

Add to the bottom of `nav_picker.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone)]
    struct FakeItem {
        label: String,
        out: u32,
        secondary: bool,
    }
    impl NavItem for FakeItem {
        type Output = u32;
        fn filter_text(&self) -> &str {
            &self.label
        }
        fn render_into_canvas(
            &self,
            _canvas: NavCellCanvas<'_>,
            _selected: bool,
            _filter: &str,
            _sink: &mut dyn FnMut(NavRenderArtifact),
        ) {
        }
        fn outcome(&self) -> u32 {
            self.out
        }
    }
    impl SecondaryClassified for FakeItem {
        fn is_secondary(&self) -> bool {
            self.secondary
        }
    }

    fn config(allows_filter: bool, allows_secondary: bool) -> NavPickerConfig {
        NavPickerConfig {
            id: ComponentId::new("test"),
            title: " test ".into(),
            footer_hint: " hint ".into(),
            default_header: "hdr".into(),
            min_cell_cols: 10,
            cell_rows: 2,
            allows_filter,
            allows_secondary_toggle: allows_secondary,
            secondary_toggle_label: "secondary",
        }
    }

    fn items(strs: &[(&str, u32, bool)]) -> Vec<FakeItem> {
        strs.iter()
            .map(|(s, o, sec)| FakeItem {
                label: (*s).into(),
                out: *o,
                secondary: *sec,
            })
            .collect()
    }

    #[test]
    fn enter_selects_visible() {
        let mut p = NavPicker::new(
            config(true, false),
            items(&[("apple", 1, false), ("banana", 2, false)]),
            0,
        );
        assert_eq!(p.handle_key(KeyEvent::Enter), NavOutcome::Select(1));
    }

    #[test]
    fn esc_clears_filter_then_cancels() {
        let mut p = NavPicker::new(config(true, false), items(&[("apple", 1, false)]), 0);
        p.handle_key(KeyEvent::Char('a'));
        assert!(matches!(p.handle_key(KeyEvent::Esc), NavOutcome::Continue));
        assert_eq!(p.filter, "");
        assert_eq!(p.handle_key(KeyEvent::Esc), NavOutcome::Cancel);
    }

    #[test]
    fn filter_disabled_picker_ignores_char_keys() {
        let mut p = NavPicker::new(
            config(false, false),
            items(&[("apple", 1, false), ("banana", 2, false)]),
            0,
        );
        let outcome = p.handle_key(KeyEvent::Char('b'));
        assert_eq!(outcome, NavOutcome::Continue);
        assert_eq!(p.filter, "");
        // Enter should still produce the first item, not the second.
        assert_eq!(p.handle_key(KeyEvent::Enter), NavOutcome::Select(1));
    }

    #[test]
    fn tab_in_no_filter_no_toggle_picker_moves_selection() {
        // ConnectionPicker's historical Tab == Down behavior.
        let mut p = NavPicker::new(
            config(false, false),
            items(&[("a", 1, false), ("b", 2, false)]),
            0,
        );
        p.handle_key(KeyEvent::Tab);
        assert_eq!(p.handle_key(KeyEvent::Enter), NavOutcome::Select(2));
    }

    #[test]
    fn tab_in_secondary_toggle_picker_toggles_visibility() {
        let mut p = NavPicker::new(
            config(true, true),
            items(&[("primary", 1, false), ("legend", 2, true)]),
            0,
        );
        assert_eq!(p.visible_items().len(), 1);
        p.handle_key(KeyEvent::Tab);
        assert_eq!(p.visible_items().len(), 2);
    }

    #[test]
    fn arrows_wrap_through_visible() {
        let mut p = NavPicker::new(
            config(false, false),
            items(&[("a", 1, false), ("b", 2, false)]),
            0,
        );
        p.handle_key(KeyEvent::Down);
        p.handle_key(KeyEvent::Down);
        assert_eq!(p.handle_key(KeyEvent::Enter), NavOutcome::Select(1));
    }

    #[test]
    fn filter_keeps_selection_inside_visible_set() {
        let mut p = NavPicker::new(
            config(true, false),
            items(&[("apple", 1, false), ("banana", 2, false)]),
            1,
        );
        p.handle_key(KeyEvent::Char('a'));
        // 'a' matches both; selection stays clamped.
        assert_eq!(p.handle_key(KeyEvent::Enter), NavOutcome::Select(2));
        p.handle_key(KeyEvent::Char('p')); // only "apple" matches
        assert_eq!(p.handle_key(KeyEvent::Enter), NavOutcome::Select(1));
    }
}
```

Run: `cargo test --package c4tui -- nav_picker::tests`. All seven tests should pass before moving on. If borrow-checker issues bite on the closure-with-`artifacts`, address them here, not in the migration tasks.

Commit: `add NavPicker<T: NavItem> with TDD coverage of filter/arrow/Tab/Esc/Enter behavior`.

#### Step 1.4 — Write the three concrete `NavItem` impls

Fill in `nav_items.rs`. Lifting the existing rendering closures from `picker.rs::render_grid` and `connection_picker.rs::render_grid` into `NavItem::render_into_canvas` is the bulk of this step. The concrete fields:

```rust
// c4tui/src/nav_items.rs (continuing from Step 1.1)

use crate::nav_picker::{NavCellCanvas, NavItem, NavRenderArtifact, SecondaryClassified, ThumbnailId};

const THUMB_ROWS: u16 = 5;

#[derive(Debug, Clone)]
pub struct ViewNavItem {
    pub view_id: ViewId,
    pub kind: ViewKind,
    pub name: String,
    pub key: String,
    pub element_names: Vec<String>,
    pub description: Option<String>,
}

impl ViewNavItem {
    pub fn collect_all(views: &[ViewInfo], model: &WorkspaceModel) -> Vec<Self> {
        views
            .iter()
            .enumerate()
            .map(|(idx, info)| Self::from_info(ViewId::new(idx), info, model))
            .collect()
    }

    pub fn collect_for_view_ids(
        views: &[ViewInfo],
        model: &WorkspaceModel,
        view_ids: &[ViewId],
    ) -> Vec<Self> {
        view_ids
            .iter()
            .filter_map(|view_id| views.get(view_id.index()).map(|info| (*view_id, info)))
            .map(|(view_id, info)| Self::from_info(view_id, info, model))
            .collect()
    }

    fn from_info(view_id: ViewId, info: &ViewInfo, model: &WorkspaceModel) -> Self {
        let element_names = info
            .element_ids
            .iter()
            .filter_map(|id| model.elements.get(id).map(|e| e.name.clone()))
            .collect();
        Self {
            view_id,
            kind: info.kind,
            name: info.name.clone(),
            key: info.key.clone(),
            element_names,
            description: info.description.clone(),
        }
    }
}

impl NavItem for ViewNavItem {
    type Output = ViewId;

    fn filter_text(&self) -> &str {
        // Combining is hot-path-cheap here because it's only computed during
        // filter evaluation, which is per-key not per-render.
        // (If perf bites, cache in the struct.)
        // For now `filter_text` is just `name`; secondary tokens carry kind/key/etc.
        &self.name
    }

    fn group(&self) -> Option<&str> {
        Some(self.kind.label())
    }

    fn secondary_filter_tokens(&self) -> &[String] {
        &self.element_names
    }

    fn render_into_canvas(
        &self,
        mut canvas: NavCellCanvas<'_>,
        selected: bool,
        _filter: &str,
        sink: &mut dyn FnMut(NavRenderArtifact),
    ) {
        let marker = if selected { ">" } else { " " };
        let title = format!("{marker} {}", self.name);
        canvas.set_string(
            0,
            0,
            truncate(&title, canvas.width().saturating_sub(1) as usize),
            canvas.style(),
        );

        if canvas.height() > 1 {
            let image_rows = THUMB_ROWS.min(canvas.height().saturating_sub(1));
            let image_cols = canvas.width().saturating_sub(2);
            if image_rows > 0 && image_cols > 0 {
                let area = canvas.local_cell_area(1, 1, image_cols, image_rows);
                sink(NavRenderArtifact::Thumbnail {
                    id: ThumbnailId(self.view_id),
                    area,
                });
            }
        }

        let key_row = THUMB_ROWS.saturating_add(1);
        if key_row < canvas.height() {
            let style = if selected {
                canvas.style().add_modifier(Modifier::DIM)
            } else {
                Style::default().add_modifier(Modifier::DIM)
            };
            canvas.set_string(
                0,
                key_row,
                truncate(&self.key, canvas.width().saturating_sub(1) as usize),
                style,
            );
        }
    }

    fn outcome(&self) -> ViewId {
        self.view_id
    }
}

impl SecondaryClassified for ViewNavItem {
    fn is_secondary(&self) -> bool {
        self.kind.is_legend()
    }
}

/// Currently structurally identical to `ViewNavItem`. The distinct type
/// keeps the call site's intent explicit: a `ChildViewNavItem` will be
/// wrapped in `NavTarget::ChildView` at the spawn point, while a
/// `ViewNavItem` becomes `NavTarget::View`.
#[derive(Debug, Clone)]
pub struct ChildViewNavItem(pub ViewNavItem);

impl NavItem for ChildViewNavItem {
    type Output = ViewId;
    fn filter_text(&self) -> &str {
        self.0.filter_text()
    }
    fn group(&self) -> Option<&str> {
        self.0.group()
    }
    fn secondary_filter_tokens(&self) -> &[String] {
        self.0.secondary_filter_tokens()
    }
    fn render_into_canvas(
        &self,
        canvas: NavCellCanvas<'_>,
        selected: bool,
        filter: &str,
        sink: &mut dyn FnMut(NavRenderArtifact),
    ) {
        self.0.render_into_canvas(canvas, selected, filter, sink)
    }
    fn outcome(&self) -> ViewId {
        self.0.outcome()
    }
}

impl SecondaryClassified for ChildViewNavItem {
    fn is_secondary(&self) -> bool {
        self.0.is_secondary()
    }
}

#[derive(Debug, Clone)]
pub struct ConnectionNavItem {
    pub candidate: ConnectionNavigationCandidate,
    pub direction_label: &'static str,
    pub connected_element_name: String,
    pub relationship_detail: String,
    pub target_view: String,
}

impl ConnectionNavItem {
    pub fn collect_from_candidates(
        candidates: Vec<ConnectionNavigationCandidate>,
        store: &crate::view::ViewStore,
    ) -> Vec<Self> {
        use crate::view::ConnectionDirection;
        candidates
            .into_iter()
            .map(|candidate| {
                let relationship = store.model.relationships.get(&candidate.relationship_id);
                let connected_element_name = store
                    .model
                    .elements
                    .get(&candidate.connected_element_id)
                    .map(|element| element.name.clone())
                    .unwrap_or_else(|| candidate.connected_element_id.to_string());
                let relationship_detail = match (
                    relationship.and_then(|rel| rel.description.as_deref()),
                    relationship.and_then(|rel| rel.technology.as_deref()),
                ) {
                    (Some(d), Some(t)) => format!("{d} ({t})"),
                    (Some(d), None) => d.to_owned(),
                    (None, Some(t)) => t.to_owned(),
                    (None, None) => "relationship".to_owned(),
                };
                let target_view = store.view(candidate.view_id).name.clone();
                let direction_label = match candidate.direction {
                    ConnectionDirection::Outgoing => "to",
                    ConnectionDirection::Incoming => "from",
                };
                Self {
                    candidate,
                    direction_label,
                    connected_element_name,
                    relationship_detail,
                    target_view,
                }
            })
            .collect()
    }
}

impl NavItem for ConnectionNavItem {
    type Output = ConnectionNavigationCandidate;

    fn filter_text(&self) -> &str {
        &self.connected_element_name
    }

    fn render_into_canvas(
        &self,
        mut canvas: NavCellCanvas<'_>,
        selected: bool,
        _filter: &str,
        _sink: &mut dyn FnMut(NavRenderArtifact),
    ) {
        let marker = if selected { ">" } else { " " };
        let title = format!(
            "{marker} {} {}",
            self.direction_label, self.connected_element_name
        );
        canvas.set_string(
            0,
            0,
            truncate(&title, canvas.width().saturating_sub(1) as usize),
            canvas.style(),
        );

        if canvas.height() > 1 {
            let detail = format!("rel: {}", self.relationship_detail);
            canvas.set_string(
                0,
                1,
                truncate(&detail, canvas.width().saturating_sub(1) as usize),
                Style::default(),
            );
        }
        if canvas.height() > 2 {
            let target = format!("view: {}", self.target_view);
            canvas.set_string(
                0,
                2,
                truncate(&target, canvas.width().saturating_sub(1) as usize),
                Style::default().add_modifier(Modifier::DIM),
            );
        }
    }

    fn outcome(&self) -> ConnectionNavigationCandidate {
        self.candidate.clone()
    }
}

impl SecondaryClassified for ConnectionNavItem {}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        text.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
    }
}
```

Add `mod nav_picker; mod nav_items;` to `c4tui/src/lib.rs`.

Run `cargo build --package c4tui`. Expected: clean. No tests yet on the concrete items — the migration tasks will exercise them through the existing app tests.

Commit: `add ViewNavItem / ChildViewNavItem / ConnectionNavItem implementing NavItem`.

---

### Task 2 — Migrate top-level ViewPicker call-site to NavPicker

**Goal:** Replace one of three call sites. `picker.rs` is not deleted yet — we keep it as a typed reference for diffing while migrating.

#### Step 2.1 — Plumb `NavPicker<ViewNavItem>` into `Effect::OpenPicker`

In `c4tui/src/app.rs`, the `Effect::OpenPicker` arm currently builds `ViewPicker::new(...)`. Rewrite to:

```rust
Some(Effect::OpenPicker) => {
    use crate::nav_items::ViewNavItem;
    use crate::nav_picker::{NavPicker, NavPickerConfig};
    use tui_kit::component::ComponentId;

    let current = self.state.current();
    terminal.teardown_image_viewport(current)?;
    let items = ViewNavItem::collect_all(&self.store.views, &self.store.model);
    let initial = items
        .iter()
        .position(|item| item.view_id == current)
        .unwrap_or(0);
    let picker_inner = NavPicker::new(
        NavPickerConfig {
            id: ComponentId::new("c4tui-view-picker"),
            title: " View Picker ".into(),
            footer_hint:
                " type → filter | Tab → legends | Enter → select | Esc → cancel ".into(),
            default_header: "Pick a view  —  type to filter, Enter to select, Esc to cancel, Tab to toggle key views".into(),
            min_cell_cols: 22,
            cell_rows: 8,
            allows_filter: true,
            allows_secondary_toggle: true,
            secondary_toggle_label: "legends",
        },
        items,
        initial,
    );
    let last_hover = picker_inner.selected().map(|i| i.view_id).unwrap_or(current);
    if !self.store.has_rendered(last_hover) {
        let path = self.store.view(last_hover).svg_path.clone();
        self.scheduler.request(
            last_hover,
            RenderPriority::Hover,
            path,
            self.store.budget(),
        );
    }
    self.focus.push_scope(/* unchanged */)?;
    self.picker_slot = Some(PickerSlot {
        picker: Cached::new(picker_inner),
        last_hover,
        action: PickerAction::SelectView,
    });
    /* ... */
}
```

**Change `PickerSlot`** to hold `Cached<NavPicker<ViewNavItem>>` instead of `Cached<ViewPicker>`:

```rust
#[derive(Debug)]
struct PickerSlot {
    picker: Cached<NavPicker<ViewNavItem>>,
    last_hover: ViewId,
    action: PickerAction,
}
```

**Update `handle_key_picker`** to consume `NavOutcome<ViewId>` instead of `PickerOutcome`. The function body's match becomes `NavOutcome::Continue | NavOutcome::Select(view_id) | NavOutcome::Cancel`, and `selected_view_id()` becomes `slot.picker.inner().selected().map(|i| i.view_id)`. The `last_hover` update reads from `selected()` too.

**Update `terminal.rs::draw_picker`**: change the signature from `draw_picker(&mut self, picker: &mut Cached<ViewPicker>, store: &ViewStore)` to `draw_picker(&mut self, picker: &mut Cached<NavPicker<ViewNavItem>>, store: &ViewStore)`. The body changes one line: `let thumbs = picker.inner().thumbnails().to_vec();` becomes:

```rust
let thumbs: Vec<crate::picker::ThumbnailCellArea> = picker
    .inner()
    .last_artifacts()
    .iter()
    .filter_map(|a| match a {
        crate::nav_picker::NavRenderArtifact::Thumbnail { id, area } => {
            Some(crate::picker::ThumbnailCellArea {
                view_id: id.0,
                area: *area,
            })
        }
    })
    .collect();
```

(The `ThumbnailCellArea` type still lives in `picker.rs` — it gets moved to `nav_picker.rs` in Step 4.4 when picker.rs is deleted. For this step we leave it where it is.)

**Update `backend.rs::TerminalBackend::draw_picker` signature** identically. Update the `FakeTerminalBackend::draw_picker` impl signature.

#### Step 2.2 — Run the affected tests

```
cargo test --package c4tui -- picker_navigation_selects_view_by_keystrokes
cargo test --package c4tui -- picker_cancel_runs_full_image_lifecycle_back_to_main_view
```

Both should pass. If `picker_navigation_selects_view_by_keystrokes` fails because the `Down` key behaves differently when the picker is in `allows_secondary_toggle=true` mode: ViewPicker's original `move_selection` ran on `visible_view_ids`, which excluded legend views by default. NavPicker matches that — `move_selection` runs on `visible_items().len()`. The behavior should be identical.

If `picker_cancel_runs_full_image_lifecycle_back_to_main_view` fails because the picker is producing thumbnail artifacts where ViewPicker did not: confirm that the test asserts on `FakeTerminalCall::DrawPicker` not on thumbnail count. Spot-check the assertion list.

Commit: `migrate top-level view picker to NavPicker<ViewNavItem>`.

---

### Task 3 — Migrate child-view picker (OpenChildViewPicker effect) to NavPicker

**Goal:** Second call site. The child-view picker spawns a `ViewPicker::new_for_view_ids(...)` today; replace with `NavPicker<ChildViewNavItem>`.

#### Step 3.1 — Edit `Effect::OpenChildViewPicker` arm

```rust
Some(Effect::OpenChildViewPicker { target_view_ids }) => {
    use crate::nav_items::{ChildViewNavItem, ViewNavItem};
    use crate::nav_picker::{NavPicker, NavPickerConfig};
    use tui_kit::component::ComponentId;

    let current = self.state.current();
    terminal.teardown_image_viewport(current)?;
    let items: Vec<ChildViewNavItem> = ViewNavItem::collect_for_view_ids(
        &self.store.views,
        &self.store.model,
        &target_view_ids,
    )
    .into_iter()
    .map(ChildViewNavItem)
    .collect();
    let initial_view = target_view_ids.first().copied().unwrap_or(current);
    let initial = items
        .iter()
        .position(|i| i.0.view_id == initial_view)
        .unwrap_or(0);
    let picker_inner = NavPicker::new(
        NavPickerConfig {
            id: ComponentId::new("c4tui-child-view-picker"),
            title: " Related Views ".into(),
            footer_hint: " Enter → drill | Esc → cancel ".into(),
            default_header: "Pick a child view to drill into".into(),
            min_cell_cols: 22,
            cell_rows: 8,
            allows_filter: true,
            allows_secondary_toggle: false,
            secondary_toggle_label: "",
        },
        items,
        initial,
    );
    /* ... last_hover + scheduler.request + push_scope + slot assignment ... */
}
```

**Problem:** `PickerSlot::picker` is currently typed `Cached<NavPicker<ViewNavItem>>` after Task 2. Now we need it to also hold `Cached<NavPicker<ChildViewNavItem>>`. There are two ways forward:

**Option α — Unify ViewNavItem and ChildViewNavItem under a single concrete item.** Replace `ChildViewNavItem(pub ViewNavItem)` newtype with just using `ViewNavItem`, and let `PickerAction::SelectView` vs `PickerAction::Drill` (which already exists on `PickerSlot`) decide whether the resulting `NavTarget` variant is `View(_)` or `ChildView(_)`.

**Option β — Box the inner picker behind a dyn-compatible trait.** More machinery.

**Pick α.** The newtype was added for type-level safety, but `PickerAction` already encodes the intent. Remove `ChildViewNavItem` entirely; the spawn-site code in `handle_key_picker` already knows the action and produces the right `Command::Select{View,ChildView}`.

Edit `nav_items.rs`: delete `pub struct ChildViewNavItem(pub ViewNavItem)` and its impls.

Update Step 3.1 to use `ViewNavItem` directly:

```rust
let items: Vec<ViewNavItem> = ViewNavItem::collect_for_view_ids(/* ... */);
let picker_inner = NavPicker::new(/* config with title "Related Views" */, items, initial);
```

`PickerSlot` keeps its `Cached<NavPicker<ViewNavItem>>` typing. Both `Effect::OpenPicker` and `Effect::OpenChildViewPicker` produce `Cached<NavPicker<ViewNavItem>>` differing only in title/footer/action. Good — the type tells us less than we feared we'd lose.

(**Memory:** this is exactly the kind of consolidation the user has applauded in the past. The "Name don't rebuild" feedback memory says: when separate types reflect intent already encoded elsewhere, fold them. `PickerAction` is the existing name; we don't need a parallel structural type.)

#### Step 3.2 — Run the affected test

```
cargo test --package c4tui -- click_on_element_with_multiple_related_views_opens_picker_and_drills
cargo test --package c4tui -- related_view_picker_cancel_preserves_current_view
```

Both should pass.

Commit: `migrate child-view picker to NavPicker; drop ChildViewNavItem newtype`.

---

### Task 4 — Migrate ConnectionPicker call-site, then delete picker.rs and connection_picker.rs

#### Step 4.1 — Edit `Effect::OpenConnectionPicker` arm

```rust
Some(Effect::OpenConnectionPicker { source_element_id }) => {
    use crate::nav_items::ConnectionNavItem;
    use crate::nav_picker::{NavPicker, NavPickerConfig};
    use tui_kit::component::ComponentId;

    let current = self.state.current();
    let candidates = self
        .store
        .connection_candidates_for_element(current, &source_element_id);
    let source_element_name = self
        .store
        .model
        .elements
        .get(&source_element_id)
        .map(|e| e.name.clone())
        .unwrap_or_else(|| source_element_id.to_string());
    terminal.teardown_image_viewport(current)?;
    let items = ConnectionNavItem::collect_from_candidates(candidates, &self.store);
    let picker_inner = NavPicker::new(
        NavPickerConfig {
            id: ComponentId::new("c4tui-connection-picker"),
            title: " Connection Picker ".into(),
            footer_hint: " Enter → navigate | Esc → cancel ".into(),
            default_header: format!(
                "Connections for {}  -  Enter to navigate, Esc to cancel",
                source_element_name
            ),
            min_cell_cols: 34,
            cell_rows: 5,
            allows_filter: false,
            allows_secondary_toggle: false,
            secondary_toggle_label: "",
        },
        items,
        0,
    );
    self.focus.push_scope(/* SCOPE_CONNECTION_PICKER */)?;
    self.connection_picker_slot = Some(ConnectionPickerSlot {
        picker: Cached::new(picker_inner),
    });
    /* draw call */
}
```

`ConnectionPickerSlot::picker` becomes `Cached<NavPicker<ConnectionNavItem>>`. `handle_key_connection_picker` updates to match `NavOutcome<ConnectionNavigationCandidate>`.

**`terminal.rs::draw_connection_picker`** signature changes from `&mut Cached<ConnectionPicker>` to `&mut Cached<NavPicker<ConnectionNavItem>>`. Inside, the function is unchanged (just calls `picker.render_to_buffer(area, buffer)`).

#### Step 4.2 — Run the affected tests

```
cargo test --package c4tui -- connection_picker_selects_connection_by_keystrokes
cargo test --package c4tui -- connection_picker_cancel_returns_to_current_view
cargo test --package c4tui -- connection_picker_opens_empty_without_pinning_source
```

All three should pass. If `connection_picker_opens_empty_without_pinning_source` fails because `NavPicker` with `items.is_empty()` selects-on-Enter differently: confirm that `NavOutcome::Continue` is returned when `visible_items().is_empty()`. Read the `Enter` arm of `handle_key`: `self.selected().map(...).unwrap_or(NavOutcome::Continue)` — good, that matches the old behavior of `ConnectionPickerOutcome::Continue`.

#### Step 4.3 — Delete `picker.rs` and `connection_picker.rs`

```
git rm c4tui/src/picker.rs c4tui/src/connection_picker.rs
```

Remove `mod picker;` and `mod connection_picker;` from `c4tui/src/lib.rs`. Remove the `use crate::picker::{...}` and `use crate::connection_picker::{...}` lines from `app.rs`, `terminal.rs`, `state.rs` (if any), `backend.rs`.

**Move `ThumbnailCellArea`** from the deleted `picker.rs` into `nav_picker.rs` (just below `NavRenderArtifact`):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThumbnailCellArea {
    pub view_id: crate::ids::ViewId,
    pub area: CellArea,
}
```

Update `terminal.rs::draw_picker`'s thumbnail-extraction code (the closure from Step 2.1) to refer to `crate::nav_picker::ThumbnailCellArea` instead of `crate::picker::ThumbnailCellArea`.

#### Step 4.4 — Run the full c4tui test suite

```
cargo test --package c4tui
```

Expected: all green. If any test references `crate::picker::*` or `crate::connection_picker::*`, the imports got stale during Steps 2/3 — fix them now. If a test asserts on `PickerOutcome::*` or `ConnectionPickerOutcome::*` enum-variant names, rewrite the assertion to match `NavOutcome::*`.

Commit: `delete picker.rs and connection_picker.rs; one NavPicker now serves all three call sites`.

---

### Task 5 — Introduce `Modal` trait + `ActiveModal` (parallel to existing slot fields)

**Goal:** Land the new abstraction without yet removing the old. Both representations coexist briefly so Tasks 6–7 can move handler-by-handler.

#### Step 5.1 — Create `c4tui/src/modal.rs`

```rust
//! Modal lifecycle types for c4tui.
//!
//! `ActiveModal` is the single piece of `App` state that captures "we're
//! showing something on top of the diagram canvas right now". It replaces
//! three near-identical optional slot fields (picker, connection-picker, log)
//! that diverged only in their concrete type.
//!
//! `Modal` is the rendering substrate: a one-method trait covering
//! everything a modal needs to paint into a buffer. `NavPicker<T>` wrapped
//! in `Cached` implements `Modal`; `LogView` implements it by delegating to
//! the existing `render_log_view` helper.
//!
//! Dialog stays out of this module: it's a one-shot render driven by
//! `show_dialog` / `show_help` etc., not part of the per-frame modal
//! redraw loop.

use anyhow::Result;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use tui_kit::component::{BufferComponent, Cached};
use tui_kit::focus::FocusScopeKind;
use tui_kit::input::KeyEvent;

use crate::log_view::{LogView, LogViewOutcome};
use crate::nav_items::NavTarget;
use crate::nav_picker::{NavOutcome, NavPicker};
use crate::nav_items::{ConnectionNavItem, ViewNavItem};
use crate::state::Effect;

/// Minimal rendering contract every modal honours. `render` is called by
/// the terminal layer when the modal scope is on top of the focus stack
/// and the frame needs to refresh.
pub trait Modal {
    fn render(&mut self, area: Rect, buffer: &mut Buffer) -> Result<()>;
}

/// Blanket impl: any `Cached<C>` over a `BufferComponent` (which NavPicker is)
/// is a `Modal` simply by replaying its cached buffer.
impl<C> Modal for Cached<C>
where
    C: BufferComponent<Event = KeyEvent>,
{
    fn render(&mut self, area: Rect, buffer: &mut Buffer) -> Result<()> {
        self.render_to_buffer(area, buffer)
    }
}

/// `LogView` doesn't implement `BufferComponent` (its `handle_key` takes a
/// `&dyn Clipboard`, which doesn't fit `BufferComponent::Event = KeyEvent`).
/// We give it a direct `Modal` impl that calls into the existing
/// `render_log_view` helper in `terminal.rs`. The helper is moved here so
/// `Modal` is the source of truth for log rendering.
impl Modal for LogView {
    fn render(&mut self, area: Rect, buffer: &mut Buffer) -> Result<()> {
        crate::terminal::render_log_view(area, buffer, self)
    }
}

/// The two modal flavors c4tui has. The dialog is a separate piece of
/// state (see `App::dialog_slot`) because its lifecycle doesn't share the
/// redraw loop these two do.
pub enum ActiveModal {
    Nav(NavModalSlot),
    Log(LogModalSlot),
}

/// A NavPicker spawn instance. The `on_select` closure is the routing seam:
/// each spawn-site supplies one, capturing whatever context (view-ids, action
/// kind, source element) the selection callback needs to produce a `Command`.
pub struct NavModalSlot {
    pub picker: Cached<NavPicker<NavItemSlotKind>>,
    pub on_select: Box<dyn FnOnce(NavTarget) -> Effect + Send>,
}

/// We need one concrete type the `Cached<NavPicker<...>>` field can hold,
/// because `Cached<NavPicker<ViewNavItem>>` and
/// `Cached<NavPicker<ConnectionNavItem>>` are different types. There are
/// two viable shapes:
///
/// (1) `Box<dyn Modal>` for the picker — loses NavOutcome typing inside
///     `handle_modal_key` (we'd need a parallel `handle_key(&mut self, key)
///     -> Option<NavTarget>` shape on the trait object).
///
/// (2) An enum wrapping the three concrete item types. NavPicker stays
///     a generic struct; the enum disappears once spawn-time and resurfaces
///     only at the slot boundary.
///
/// We pick (2). `NavItemSlotKind` is below.
pub enum NavItemSlotKind {
    View(ViewNavItem),
    Connection(ConnectionNavItem),
}
```

**Wait.** Look at this carefully. `NavPicker<T: NavItem>` is generic. To erase that genericity at the slot level, we need either a trait object (`Box<dyn Modal>`) for the *picker itself* and a separate function pointer for "give me the current Enter-selection", or an enum-of-pickers. **Both are acceptable; let's reconsider.**

A trait object is cleaner: `Modal` already covers rendering. We add one method to cover "produce an outcome from a key" — but that gives back a `NavOutcome<T::Output>`, which is generic in `T::Output`. To erase that, the trait method must return `Option<NavTarget>` directly. That means the picker's NavOutcome→NavTarget translation lives **inside** the trait method, not at the spawn-site.

Better. Revise: introduce a richer `NavModal` trait that subsumes `Modal` and includes the key→`Option<ModalKeyOutcome>` step, where `ModalKeyOutcome` is the post-translation enum.

Replace the `modal.rs` content above with:

```rust
//! Modal lifecycle types for c4tui.

use anyhow::Result;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use tui_kit::component::{BufferComponent, Cached, ComponentOutcome};
use tui_kit::input::KeyEvent;

use crate::clipboard::Clipboard;
use crate::log_view::{LogView, LogViewOutcome};
use crate::nav_items::NavTarget;
use crate::nav_picker::{NavItem, NavOutcome, NavPicker, SecondaryClassified};
use crate::state::AppState;

/// Render-side contract for any modal. The terminal layer calls this on
/// every frame the modal owns the screen.
pub trait Modal {
    fn render(&mut self, area: Rect, buffer: &mut Buffer) -> Result<()>;
}

/// Key-handling contract for a NavPicker-shaped modal. Implementations
/// hold a `Cached<NavPicker<T>>` and translate its typed `NavOutcome<T::Output>`
/// into the union `NavTarget` at the trait boundary, so the holding code
/// (`ActiveModal::Nav(...)`) sees one concrete outcome type.
pub trait NavModal: Modal {
    fn handle_key(&mut self, key: KeyEvent) -> NavModalOutcome;
    /// Hover hook used by the view-picker thumbnail prefetch. Returns the
    /// view-id the picker is currently showing, if any. Other NavModal
    /// impls return None.
    fn currently_hovered_view(&self) -> Option<crate::ids::ViewId> {
        None
    }
    /// Provide thumbnail anchors (cell areas + view-ids) for the terminal
    /// layer to paint kitty placements after rendering. Default = empty.
    fn thumbnails(&self) -> Vec<crate::nav_picker::ThumbnailCellArea> {
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
    fn handle_key(&mut self, key: KeyEvent, clipboard: &dyn Clipboard) -> Result<LogModalOutcome>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogModalOutcome {
    Continue,
    Close,
}

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
    fn handle_key(&mut self, key: KeyEvent, clipboard: &dyn Clipboard) -> Result<LogModalOutcome> {
        match LogView::handle_key(self, key, clipboard)? {
            LogViewOutcome::Continue => Ok(LogModalOutcome::Continue),
            LogViewOutcome::Close => Ok(LogModalOutcome::Close),
        }
    }
}

/// Concrete NavModal: one per spawn-site flavour. The closure inside
/// converts the typed selection into a `NavTarget` variant.
pub struct NavPickerModal<T: NavItem + SecondaryClassified> {
    pub picker: Cached<NavPicker<T>>,
    pub into_target: Box<dyn Fn(T::Output) -> NavTarget + Send>,
    pub hovered_view: Box<dyn Fn(&NavPicker<T>) -> Option<crate::ids::ViewId> + Send>,
    pub thumbnails: Box<dyn Fn(&NavPicker<T>) -> Vec<crate::nav_picker::ThumbnailCellArea> + Send>,
}

impl<T: NavItem + SecondaryClassified> Modal for NavPickerModal<T> {
    fn render(&mut self, area: Rect, buffer: &mut Buffer) -> Result<()> {
        self.picker.render(area, buffer)
    }
}

impl<T: NavItem + SecondaryClassified> NavModal for NavPickerModal<T> {
    fn handle_key(&mut self, key: KeyEvent) -> NavModalOutcome {
        match self.picker.handle_event(&key).expect("infallible") {
            ComponentOutcome::Message(NavOutcome::Select(out)) => {
                NavModalOutcome::Select((self.into_target)(out))
            }
            ComponentOutcome::Message(NavOutcome::Cancel) => NavModalOutcome::Cancel,
            _ => NavModalOutcome::Continue,
        }
    }
    fn currently_hovered_view(&self) -> Option<crate::ids::ViewId> {
        (self.hovered_view)(self.picker.inner())
    }
    fn thumbnails(&self) -> Vec<crate::nav_picker::ThumbnailCellArea> {
        (self.thumbnails)(self.picker.inner())
    }
}

/// Top-level slot owned by `App`. The dialog stays as a separate App field.
pub enum ActiveModal {
    Nav {
        modal: Box<dyn NavModal + Send>,
        on_select: Box<dyn FnOnce(NavTarget, &mut AppState) -> Option<crate::event::Command> + Send>,
    },
    Log {
        modal: Box<dyn LogModal + Send>,
    },
}
```

The `Send` bound is there because `App` could be moved across threads in tests (it isn't today, but the compiler will demand it the moment we cross a closure). If it bites, drop `Send` — the `App` is single-threaded.

**Subtle:** `Cached::handle_event` returns a `Result<ComponentOutcome<...>>`; the `.expect("infallible")` is honest — `NavPicker::handle_event` never returns `Err`. Verify by reading the impl. Don't silently swallow real errors here.

Add `mod modal;` to `c4tui/src/lib.rs`.

#### Step 5.2 — Move `render_log_view` from `terminal.rs` to a `pub(crate)` function

In `c4tui/src/terminal.rs`, change the existing `fn render_log_view(...) -> Result<()>` to `pub(crate) fn render_log_view(...) -> Result<()>` so `modal.rs::impl Modal for LogView` can call it.

#### Step 5.3 — Add `active_modal: Option<ActiveModal>` to `App`

Add the field. Initialize to `None` in `App::new`. **Do not delete the old slot fields yet.** Both representations coexist.

#### Step 5.4 — Compile-only check

`cargo build --package c4tui`. Expected clean. No new tests at this step — the abstraction is unused so far. The next task wires it in.

Commit: `add Modal/NavModal/LogModal traits, ActiveModal enum, parallel to existing slots`.

---

### Task 6 — Switch the three handlers to `handle_modal_key`; delete old slots

#### Step 6.1 — Write `handle_modal_key`

In `app.rs`, add:

```rust
fn handle_modal_key(
    &mut self,
    key: KeyEvent,
    terminal: &mut impl TerminalBackend,
) -> Result<()> {
    let Some(active) = self.active_modal.as_mut() else {
        return Ok(());
    };
    match active {
        ActiveModal::Nav { modal, .. } => {
            let outcome = modal.handle_key(key);
            // Hover prefetch — only ViewNavItem-backed pickers expose a hover.
            if let Some(hover) = modal.currently_hovered_view() {
                if !self.store.has_rendered(hover) {
                    let path = self.store.view(hover).svg_path.clone();
                    self.scheduler
                        .request(hover, RenderPriority::Hover, path, self.store.budget());
                }
            }
            match outcome {
                NavModalOutcome::Continue => {
                    terminal.render_modal(modal.as_mut() as &mut dyn Modal)?;
                    Ok(())
                }
                NavModalOutcome::Select(target) => {
                    let canvas = terminal.canvas_metrics();
                    // Move on_select out, then run it.
                    let modal = self.active_modal.take().expect("checked above");
                    let ActiveModal::Nav { on_select, .. } = modal else {
                        unreachable!()
                    };
                    terminal.close_modal()?;
                    self.focus.pop_scope();
                    if let Some(command) = on_select(target, &mut self.state) {
                        self.state.apply(command, &mut self.store, canvas)?;
                    }
                    self.request_active_render();
                    terminal.render(&self.frame_with_progress(), &mut self.store)?;
                    Ok(())
                }
                NavModalOutcome::Cancel => {
                    terminal.close_modal()?;
                    self.active_modal = None;
                    self.focus.pop_scope();
                    terminal.render(&self.frame_with_progress(), &mut self.store)?;
                    Ok(())
                }
            }
        }
        ActiveModal::Log { modal } => {
            let outcome = modal.handle_key(key, self.clipboard.as_ref())?;
            match outcome {
                LogModalOutcome::Continue => {
                    terminal.render_modal(modal.as_mut() as &mut dyn Modal)?;
                    Ok(())
                }
                LogModalOutcome::Close => {
                    self.active_modal = None;
                    self.return_to_root();
                    terminal.render(&self.frame_with_progress(), &mut self.store)?;
                    Ok(())
                }
            }
        }
    }
}
```

**Subtle: `on_select` returning `Option<Command>`.** Some selections (NavTarget::View) translate to `Command::SelectView(_)`; others translate to `Command::SelectChildView(_)` or `Command::SelectConnection(_)`. The current code uses `PickerAction::SelectView | PickerAction::Drill` to discriminate. With NavTarget being a union, the discrimination collapses into the variant itself; the spawn-site supplies the `on_select` closure that matches on the variant and returns the right `Command`.

#### Step 6.2 — Rewrite the three `Effect::Open*` arms to populate `active_modal`

```rust
Some(Effect::OpenPicker) => {
    use crate::modal::{ActiveModal, NavPickerModal};
    // ... build NavPicker as before ...
    let modal = NavPickerModal {
        picker: Cached::new(picker_inner),
        into_target: Box::new(|view_id| NavTarget::View(view_id)),
        hovered_view: Box::new(|p| p.selected().map(|i| i.view_id)),
        thumbnails: Box::new(|p| {
            p.last_artifacts()
                .iter()
                .filter_map(|a| match a {
                    crate::nav_picker::NavRenderArtifact::Thumbnail { id, area } => {
                        Some(crate::nav_picker::ThumbnailCellArea {
                            view_id: id.0,
                            area: *area,
                        })
                    }
                })
                .collect()
        }),
    };
    self.focus.push_scope(/* SCOPE_PICKER */)?;
    self.active_modal = Some(ActiveModal::Nav {
        modal: Box::new(modal),
        on_select: Box::new(|target, _state| match target {
            NavTarget::View(view_id) => Some(crate::event::Command::SelectView(view_id)),
            _ => None,
        }),
    });
    terminal.render_modal(/* &mut self.active_modal.as_mut().unwrap() ... */)?;
}
Some(Effect::OpenChildViewPicker { target_view_ids }) => {
    // ... build NavPicker the same way ...
    let modal = NavPickerModal { /* ... */ };
    self.focus.push_scope(/* SCOPE_PICKER */)?;
    self.active_modal = Some(ActiveModal::Nav {
        modal: Box::new(modal),
        on_select: Box::new(|target, _state| match target {
            // Top-level View(_) is impossible here because items map through
            // ChildView in the spawn closure... wait, they don't yet.
            NavTarget::View(view_id) => Some(crate::event::Command::SelectChildView(view_id)),
            _ => None,
        }),
    });
}
```

Hmm — re-examining. Both `Effect::OpenPicker` and `Effect::OpenChildViewPicker` build `NavPicker<ViewNavItem>` whose `T::Output = ViewId`. The `into_target` closure differs:
- Top-level: `|v| NavTarget::View(v)`
- Child-view: `|v| NavTarget::ChildView(v)`

Then `on_select` on both is a one-variant match. That's correct and tells the story: the *picker* is the same, the *spawn intent* (captured at the spawn-site as the `into_target` closure + the `on_select` closure) is what makes them different. **`PickerAction` enum can be deleted** — its work has moved into the closures.

Connection spawn arm:

```rust
Some(Effect::OpenConnectionPicker { source_element_id }) => {
    // ... build NavPicker<ConnectionNavItem> ...
    let modal = NavPickerModal {
        picker: Cached::new(picker_inner),
        into_target: Box::new(|candidate| NavTarget::Connection(candidate)),
        hovered_view: Box::new(|_| None),
        thumbnails: Box::new(|_| Vec::new()),
    };
    self.focus.push_scope(/* SCOPE_CONNECTION_PICKER */)?;
    self.active_modal = Some(ActiveModal::Nav {
        modal: Box::new(modal),
        on_select: Box::new(|target, _state| match target {
            NavTarget::Connection(c) => Some(crate::event::Command::SelectConnection(c)),
            _ => None,
        }),
    });
}
```

And the log-view toggle:

```rust
fn toggle_log_view(&mut self) {
    if matches!(self.active_modal, Some(ActiveModal::Log { .. })) {
        self.active_modal = None;
        self.return_to_root();
        return;
    }
    self.return_to_root();
    self.focus.push_scope(/* SCOPE_LOG */)?;
    self.active_modal = Some(ActiveModal::Log {
        modal: Box::new(LogView::new(self.log_buffer.clone())),
    });
}
```

#### Step 6.3 — Update `redraw_for_mode` and `handle_key_event` dispatch

`handle_key_event`:

```rust
match self.active_scope() {
    SCOPE_PICKER | SCOPE_CONNECTION_PICKER | SCOPE_LOG => self.handle_modal_key(key, terminal),
    SCOPE_DIALOG => { /* unchanged */ }
    _ => self.handle_input(InputEvent::Key(key), terminal),
}
```

`redraw_for_mode`:

```rust
match self.active_scope() {
    SCOPE_PICKER | SCOPE_CONNECTION_PICKER | SCOPE_LOG => {
        if let Some(active) = self.active_modal.as_mut() {
            let m: &mut dyn Modal = match active {
                ActiveModal::Nav { modal, .. } => modal.as_mut(),
                ActiveModal::Log { modal } => modal.as_mut(),
            };
            terminal.render_modal(m)?;
        }
        Ok(())
    }
    SCOPE_DIALOG => Ok(()),
    _ => terminal.render(&frame, &mut self.store),
}
```

#### Step 6.4 — Delete the old slot fields and their types

In `App`:
- Delete `picker_slot: Option<PickerSlot>`, `connection_picker_slot: Option<ConnectionPickerSlot>`, `log_slot: Option<LogSlot>` fields.
- Delete the `PickerSlot`, `ConnectionPickerSlot`, `LogSlot`, `PickerAction` types.
- Delete `handle_key_picker`, `handle_key_connection_picker`, `handle_key_log` methods.
- `return_to_root` now becomes:

```rust
fn return_to_root(&mut self) {
    while self.focus.active_scope_id().map(FocusId::as_str) != Some(SCOPE_ROOT) {
        self.focus.pop_scope();
    }
    self.active_modal = None;
    self.dialog_slot = None;
}
```

#### Step 6.5 — Run app tests

```
cargo test --package c4tui -- picker_navigation_selects_view_by_keystrokes
cargo test --package c4tui -- picker_cancel_runs_full_image_lifecycle_back_to_main_view
cargo test --package c4tui -- click_on_element_with_multiple_related_views_opens_picker_and_drills
cargo test --package c4tui -- related_view_picker_cancel_preserves_current_view
cargo test --package c4tui -- connection_picker_selects_connection_by_keystrokes
cargo test --package c4tui -- connection_picker_cancel_returns_to_current_view
cargo test --package c4tui -- connection_picker_opens_empty_without_pinning_source
cargo test --package c4tui -- run_uses_backend_for_help_dialog
```

The first six rely on `FakeTerminalCall` variants that still match the old shape (`DrawPicker`, `ClosePicker`, `DrawConnectionPicker`, `CloseConnectionPicker`). Those are about to change in Task 7. Expect these tests to fail in Task 7 (or before, if `terminal.render_modal` calls and the `Fake` impl record under a different variant).

**To bridge:** in this commit, route `terminal.render_modal` and `terminal.close_modal` through the *old* fake-call variants (`DrawPicker` / `ClosePicker` etc.). The fake-backend impl in Step 7.2 will collapse them. This keeps the test suite green at every commit boundary.

In `FakeTerminalBackend::render_modal`, push `DrawPicker` (we don't yet know the modal type from the fake's vantage; that's fine because the next task introduces a generic `RenderModal` variant). For now route the new method through the existing test surface:

```rust
fn render_modal(&mut self, _modal: &mut dyn Modal) -> Result<()> {
    // Bridging during Task 6: keep emitting DrawPicker so existing tests
    // still pass. Task 7 collapses to RenderModal.
    self.calls.push(FakeTerminalCall::DrawPicker);
    self.picker_draws += 1;
    Ok(())
}

fn close_modal(&mut self) -> Result<()> {
    self.calls.push(FakeTerminalCall::ClosePicker);
    Ok(())
}
```

…**but this won't survive Task 7's call-list rewrites.** Better, accept that some test assertions will need updates in this commit, and update them now in lockstep. Specifically:
- `connection_picker_selects_connection_by_keystrokes`: change `DrawConnectionPicker` → `DrawPicker` and `CloseConnectionPicker` → `ClosePicker`. (The fake records both as `DrawPicker`/`ClosePicker` during this bridging window.)

This is ugly. **Better-still option:** do Tasks 6 and 7 in a single commit. The diff is bigger but every state is consistent.

**Pick: collapse Tasks 6 + 7 into one commit.** Continue with Task 7 below; commit at the end of Task 7.

Don't commit yet.

---

### Task 7 — Collapse `TerminalBackend` trait methods

#### Step 7.1 — Update the trait

In `c4tui/src/backend.rs`:

```rust
use crate::modal::Modal;

pub trait TerminalBackend {
    fn canvas_metrics(&self) -> CanvasMetrics;
    fn render(&mut self, frame: &RenderFrame, store: &mut ViewStore) -> Result<()>;
    fn teardown_image_viewport(&mut self, view_id: ViewId) -> Result<()>;
    fn render_modal(&mut self, modal: &mut dyn Modal) -> Result<()>;
    fn close_modal(&mut self) -> Result<()>;
    fn clear_image_cache(&mut self) -> Result<()>;
    fn show_message(&mut self, title: &str, message: &str) -> Result<()>;
    fn show_error(&mut self, title: &str, message: &str) -> Result<()>;
    fn show_help(&mut self, keys: &KeyBindings) -> Result<()>;
}
```

That's 8 methods, down from 11. The methods that disappear: `draw_picker`, `close_picker`, `draw_connection_picker`, `close_connection_picker`, `draw_log_view`. The methods that appear: `render_modal`, `close_modal`.

#### Step 7.2 — Update `FakeTerminalBackend`

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FakeTerminalCall {
    Render(ViewId),
    TeardownImageViewport(ViewId),
    RenderModal,
    CloseModal,
    ClearImageCache,
    ShowMessage,
    ShowError,
    ShowHelp,
}

#[derive(Debug)]
pub struct FakeTerminalBackend {
    canvas: CanvasMetrics,
    pub calls: Vec<FakeTerminalCall>,
    pub rendered_frames: Vec<RenderFrame>,
    pub cleared_image_cache: usize,
    pub messages: Vec<(String, String)>,
    pub errors: Vec<(String, String)>,
    pub help_count: usize,
    pub viewport_teardowns: Vec<ViewId>,
    pub modal_renders: usize,
}

impl TerminalBackend for FakeTerminalBackend {
    fn render_modal(&mut self, _modal: &mut dyn Modal) -> Result<()> {
        self.calls.push(FakeTerminalCall::RenderModal);
        self.modal_renders += 1;
        Ok(())
    }
    fn close_modal(&mut self) -> Result<()> {
        self.calls.push(FakeTerminalCall::CloseModal);
        Ok(())
    }
    /* ... rest as before, with picker_draws / connection_picker_draws / log_view_draws fields removed ... */
}
```

#### Step 7.3 — Update `TerminalSession` impls

In `c4tui/src/terminal.rs`:

```rust
impl TerminalBackend for TerminalSession {
    fn render_modal(&mut self, modal: &mut dyn Modal) -> Result<()> {
        // Three branches differ in image-pipeline teardown:
        //   - NavPicker (top-level view picker) needs thumbnail placements
        //     cleared post-render; this is a view-picker-specific concern.
        //   - NavPicker (connection / child-view picker) needs MAIN_PLACEMENT_ID
        //     cleared before render so the diagram doesn't bleed through.
        //   - LogView needs MAIN_PLACEMENT_ID + all placements cleared.
        // Today these branches live in three methods. The collapse to
        // `render_modal` requires the modal to communicate its image-pipeline
        // intent. Two options:
        //   (a) Add image-pipeline hooks to the Modal trait.
        //   (b) Move the image-pipeline calls into the modal's `render` impl
        //       (each impl knows what it needs).
        //
        // We choose (b). LogView's `render` impl already calls into
        // `render_log_view` here in terminal.rs; we extend it to call
        // `self.images().delete_placement(MAIN_PLACEMENT_ID)`. NavPickerModal's
        // `render` impl is more delicate (it needs to render thumbnails
        // afterward); we let `NavPickerModal::render` produce a list of
        // post-render thumbnail anchors via the existing `thumbnails()` method
        // on `NavModal`, and the terminal calls `paint_thumbnails(...)` after
        // `modal.render(...)`.
        //
        // Implementation: we cannot reach `NavModal::thumbnails` through
        // `&mut dyn Modal`. So we introduce a downcast: render_modal takes
        // `&mut dyn Modal`, but a separate `render_nav_modal` takes
        // `&mut dyn NavModal`. The App's handle_modal_key calls the right
        // one. The trait method count stays at two (render_modal,
        // close_modal); the differentiation lives in App. See Step 7.4.
        let mut canvas_rect = ratatui::layout::Rect::default();
        self.inner.draw(|frame| {
            let area = frame.area();
            let _ = modal.render(area, frame.buffer_mut());
            canvas_rect = area;
        })?;
        self.inner.images().flush()?;
        Ok(())
    }

    fn close_modal(&mut self) -> Result<()> {
        // Mirror the old behavior: clear placements that modals might have
        // left. The view-picker's per-view thumbnail teardown is moved into
        // the picker-spawn side (the spawn-site walks placements after the
        // modal is gone). Keep it simple here: flush.
        self.inner.images().flush()?;
        Ok(())
    }

    /* rest unchanged */
}
```

**Hmm. This bleeds the picker-thumbnail concern out into the spawn-site, which is messier than today's `close_picker(&ViewStore)`.** Reconsider.

**Better:** keep `render_modal` simple, and let the `Modal` trait have one optional method:

```rust
pub trait Modal {
    fn render(&mut self, area: Rect, buffer: &mut Buffer) -> Result<()>;
    /// Image-pipeline side-effects this modal needs *before* its
    /// buffer-render runs. Returns a list of placement-ids to clear (and
    /// nothing else, for now). Default: empty.
    fn pre_render_placements_to_clear(&self) -> Vec<u32> {
        Vec::new()
    }
    /// Image-pipeline side-effects this modal needs *after* its
    /// buffer-render runs. Returns a list of (image_id, placement_id, area)
    /// tuples to paint as kitty thumbnails. Default: empty.
    fn post_render_thumbnails(&self) -> Vec<ThumbnailPlacement> {
        Vec::new()
    }
}

pub struct ThumbnailPlacement {
    pub view_id: crate::ids::ViewId,
    pub area: tui_kit::layout::CellArea,
}
```

Then `TerminalSession::render_modal`:

```rust
fn render_modal(&mut self, modal: &mut dyn Modal) -> Result<()> {
    for placement_id in modal.pre_render_placements_to_clear() {
        self.inner.images().delete_placement(placement_id)?;
    }
    self.inner.draw(|frame| {
        let area = frame.area();
        let _ = modal.render(area, frame.buffer_mut());
    })?;
    for thumb in modal.post_render_thumbnails() {
        // We need the store to look up image data. Pass via render_modal arg?
        // — yes: render_modal signature gains `store: &ViewStore`. That's
        //   honest: thumbnails ARE store-coupled.
    }
    self.inner.images().flush()?;
    Ok(())
}
```

**Update signature** of `render_modal` to take `&ViewStore`:

```rust
fn render_modal(&mut self, modal: &mut dyn Modal, store: &ViewStore) -> Result<()>;
```

The thumbnail paint:

```rust
for thumb in modal.post_render_thumbnails() {
    if let Some(rendered) = store.cached_rendered_view(thumb.view_id) {
        let image = ViewportImage::new(rendered.raster_size, rendered.rgba.clone())?;
        self.inner.render_viewport_image(
            image,
            image_id_for_view(thumb.view_id),
            picker_placement_id(thumb.view_id.index()),
            &rendered.png,
            thumb.area,
            ImageViewportOptions {
                initial_scale: ImageViewportInitialScale::FitToBox,
                resize_policy: ResizePolicy::PreserveTopLeft,
            },
        )?;
    }
}
// Tear down any picker placements not in the current thumbnail set
// (the equivalent of today's close_picker(&store) call). This logic moves
// here, behind render_modal, since the modal knows its own surface.
let active_ids: std::collections::HashSet<u32> = modal
    .post_render_thumbnails()
    .iter()
    .map(|t| picker_placement_id(t.view_id.index()))
    .collect();
for index in 0..store.views.len() {
    let placement_id = picker_placement_id(index);
    if !active_ids.contains(&placement_id) {
        self.inner
            .teardown_image_viewport(image_id_for_view(ViewId::new(index)), placement_id)?;
    }
}
```

And `close_modal(&mut self, store: &ViewStore)` tears all picker placements down:

```rust
fn close_modal(&mut self, store: &ViewStore) -> Result<()>;

impl TerminalBackend for TerminalSession {
    fn close_modal(&mut self, store: &ViewStore) -> Result<()> {
        self.inner.teardown_image_viewports((0..store.views.len()).map(|index| {
            (
                image_id_for_view(ViewId::new(index)),
                picker_placement_id(index),
            )
        }))?;
        self.inner.images().delete_placement(MAIN_PLACEMENT_ID)?;
        self.inner.images().flush()?;
        Ok(())
    }
}
```

#### Step 7.4 — `Modal` impls supply `pre_render_placements_to_clear` and `post_render_thumbnails`

In `modal.rs`:

```rust
use tui_kit::image::MAIN_PLACEMENT_ID;

pub struct ThumbnailPlacement {
    pub view_id: crate::ids::ViewId,
    pub area: tui_kit::layout::CellArea,
}

pub trait Modal {
    fn render(&mut self, area: Rect, buffer: &mut Buffer) -> Result<()>;
    fn pre_render_placements_to_clear(&self) -> Vec<u32> {
        Vec::new()
    }
    fn post_render_thumbnails(&self) -> Vec<ThumbnailPlacement> {
        Vec::new()
    }
}

impl<C> Modal for Cached<C>
where
    C: BufferComponent<Event = KeyEvent>,
{
    fn render(&mut self, area: Rect, buffer: &mut Buffer) -> Result<()> {
        self.render_to_buffer(area, buffer)
    }
}

impl<T: NavItem + SecondaryClassified> Modal for NavPickerModal<T> {
    fn render(&mut self, area: Rect, buffer: &mut Buffer) -> Result<()> {
        self.picker.render(area, buffer)
    }
    fn pre_render_placements_to_clear(&self) -> Vec<u32> {
        // Connection-picker variant: clear the main diagram before rendering.
        // Top-level/child-view variants: also clear main (the spawn-site
        // already tore down the diagram via teardown_image_viewport, but
        // re-clearing is idempotent).
        vec![MAIN_PLACEMENT_ID]
    }
    fn post_render_thumbnails(&self) -> Vec<ThumbnailPlacement> {
        (self.thumbnails)(self.picker.inner())
            .into_iter()
            .map(|t| ThumbnailPlacement {
                view_id: t.view_id,
                area: t.area,
            })
            .collect()
    }
}

impl Modal for LogView {
    fn render(&mut self, area: Rect, buffer: &mut Buffer) -> Result<()> {
        crate::terminal::render_log_view(area, buffer, self)
    }
    fn pre_render_placements_to_clear(&self) -> Vec<u32> {
        // LogView needs the diagram cleared so clipboard yank is clean.
        vec![MAIN_PLACEMENT_ID]
    }
}
```

(Replace the old `crate::nav_picker::ThumbnailCellArea` type with `crate::modal::ThumbnailPlacement` — same fields. Pick whichever lives more naturally and delete the other.)

#### Step 7.5 — Update App to pass `&self.store` to render_modal / close_modal

Every `terminal.render_modal(...)` call in `app.rs` becomes `terminal.render_modal(modal, &self.store)`. Every `terminal.close_modal()` becomes `terminal.close_modal(&self.store)`. Update `FakeTerminalBackend::{render_modal, close_modal}` signatures to match (accept `&ViewStore` and ignore).

#### Step 7.6 — Update existing test assertions

Every `app.rs` test's `terminal.calls` assertion list updates as follows:

| Old call | New call |
|---|---|
| `FakeTerminalCall::DrawPicker` | `FakeTerminalCall::RenderModal` |
| `FakeTerminalCall::ClosePicker` | `FakeTerminalCall::CloseModal` |
| `FakeTerminalCall::DrawConnectionPicker` | `FakeTerminalCall::RenderModal` |
| `FakeTerminalCall::CloseConnectionPicker` | `FakeTerminalCall::CloseModal` |
| `FakeTerminalCall::DrawLogView` | `FakeTerminalCall::RenderModal` |

Cross-check each `assert_eq!(terminal.calls, vec![...])` literal in `c4tui/src/app.rs`'s `#[cfg(test)] mod tests` block. There are eight of them (search for `FakeTerminalCall::DrawPicker` and `FakeTerminalCall::DrawConnectionPicker`). Apply the swap.

#### Step 7.7 — Run the full c4tui test suite

```
cargo test --package c4tui
```

Expected: all green. If `picker_navigation_selects_view_by_keystrokes` shows `RenderModal` count off by one, recount: the new flow is `Render(first) → TeardownImageViewport → RenderModal (post-open) → RenderModal (post-Down) → CloseModal → Render(new)`.

#### Step 7.8 — Commit

```
git add -A
git commit -m "collapse three modal lifecycles into ActiveModal + Modal trait + render_modal/close_modal"
```

---

### Task 8 — Delete `tui-kit/src/widgets/image_box.rs` (and update prelude + README)

This is a subtractive task with no logical dependency on Tasks 1-7. Doing it after the new modal infrastructure is solid keeps the high-risk and low-risk work separate.

#### Step 8.1 — Delete the file

```
git rm tui-kit/src/widgets/image_box.rs
```

In `tui-kit/src/widgets/mod.rs`, remove the `pub mod image_box;` line.

In `tui-kit/src/prelude.rs`, remove:

```rust
pub use crate::widgets::image_box::{ImageBox, ImageBoxPlan, ImageBoxState};
```

#### Step 8.2 — Reconcile `tui-kit/README.md`

Search for `ImageBox` in `tui-kit/README.md`. Remove:
- The `widgets::image_box` row in the widget table at line ~33.
- The entire `## ImageBox` section (~96 to wherever it ends — the next `##` header).
- Any other reference.

(Full README/spec/architecture reconciliation is Phase 7. We only fix the locally-misleading lines here.)

#### Step 8.3 — Run tui-kit tests

```
cargo test --package tui-kit
```

Expected: green. Any test file that imports `tui_kit::widgets::image_box::*` or constructs `ImageBox::new(...)` should be deleted in this step (those tests had no production consumer; that's the whole reason we're cutting `image_box`).

Search: `grep -rln "ImageBox\b" tui-kit/`. Expected after fix: zero hits (or only in commit messages).

#### Step 8.4 — Confirm c4tui still compiles

```
cargo build --package c4tui
```

Expected: clean. c4tui never imported `image_box`, so this should be a no-op verification.

Commit: `delete tui-kit widgets::image_box (zero production consumers; image_viewport survives)`.

---

### Task 9 — Preserve `elements` as the render-effect substrate

#### Step 9.1 — Confirm the render-effect nucleus still compiles

```
grep -rn 'pub enum TerminalEffect\|pub trait EffectElement\|pub struct ImageViewportElement' tui-kit/src/elements.rs
```

Expected: hits for all three. If Phase 3 changes moved any of these symbols,
read the new location and update the docs in this task. Do not delete the
effect path.

#### Step 9.2 — Keep effect-forwarding tests

```
grep -rn 'forwards_effects\|teardown_effects\|ImageViewportElement' tui-kit/src/elements.rs
```

Expected: tests still cover area translation, grouped teardown, and image
viewport effects. If deleting `image_box` removed unrelated tests, do not
compensate by deleting element tests. These tests now defend the future
renderer-backend path.

#### Step 9.3 — Update local docs/comments if needed

```
rg -n 'DELETE.*elements|git rm .*elements|Task 9 .*Delete' tui-kit/README.md tui-kit/specification.md tui-kit/architecture.md tui-kit/docs/superpowers/plans/2026-05-12-tui-kit-c4tui-refactor-roadmap.md
```

Expected: no live docs saying Phase 3 should delete elements. Historical
mentions in older completed plans are allowed if they are clearly historical.

If comments in `src/elements.rs` still describe it as a broad UI framework,
tighten the language to "composable buffer rendering plus explicit render
effects." Keep code changes minimal.

#### Step 9.4 — Run tui-kit and c4tui tests

```
cargo test --package tui-kit
cargo test --package c4tui
```

Both green. c4tui does not need to import `elements` for Phase 3; the point is
to keep tui-kit's render-effect substrate intact for later renderer-backend
work.

Commit: `preserve elements render-effect substrate for future renderer backends`.

> Out of scope for this task: renaming `TerminalEffect` to `RenderEffect`,
> designing a wire protocol, adding `tui-kit-cli`, or replacing `Window` with
> `EffectScope`. Those belong in the future renderer-backend phase.

---

### Task 10 — Full verification

#### Step 10.1 — Clippy + fmt + test, both repos

```
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

Run in each repo. All four invocations clean.

#### Step 10.2 — Manual smoke test

Build c4tui and run it against a small Structurizr workspace. Walk through:

- [ ] **View picker** — `o` opens it; type to filter; `Tab` toggles legend visibility; `Enter` selects; thumbnail anchors paint.
- [ ] **View picker cancel** — `o` opens; `Esc` cancels with filter empty; main diagram repaints.
- [ ] **Child-view picker** — click an element with multiple related views; picker opens with the related views; arrow + `Enter` drills; `Backspace` returns.
- [ ] **Connection picker** — pin an element with `i`; press `Enter`; picker opens; select; navigation + pinning correct.
- [ ] **Log view** — `L` opens; `j/k` scroll; `y` yanks; `L` or `Esc` closes.
- [ ] **Dialog** — `?` shows help; any key closes.
- [ ] **Reload** — `r` reloads; if the workspace dir exists, reload succeeds and breadcrumbs clear.

Each modal-open should feel as fast as before; the unification doesn't add render work.

#### Step 10.3 — Line-count sanity check

```
wc -l tui-kit/src/elements.rs
wc -l tui-kit/src/widgets/image_box.rs 2>/dev/null || echo "image_box deleted (correct)"
wc -l c4tui/src/picker.rs c4tui/src/connection_picker.rs 2>/dev/null || echo "files deleted (correct)"
wc -l c4tui/src/nav_picker.rs c4tui/src/nav_items.rs c4tui/src/modal.rs c4tui/src/app.rs c4tui/src/backend.rs
```

Expected:
- `elements.rs` still exists.
- `image_box.rs`, `picker.rs`, and `connection_picker.rs` report "No such file or directory" (or the echoed message).
- `nav_picker.rs` ~400 lines.
- `nav_items.rs` ~200 lines.
- `modal.rs` ~120 lines.
- `app.rs` shorter than the original (~1,000-ish, down from 1,230 because three handlers + four slot types are gone).
- `backend.rs` ~110, down from 154.

Net delta: ~ -907 (image_box) - 1,238 (picker + connection_picker) + 720
(nav_picker + nav_items + modal), plus app/backend shrinkage. The repo is
smaller, but not at the cost of deleting the future render-effect path.

#### Step 10.4 — Doc check

```
grep -rln 'ImageBox\|image_box' tui-kit/ c4tui/
rg -n 'DELETE.*elements|git rm .*elements|Task 9 .*Delete' tui-kit/README.md tui-kit/specification.md tui-kit/architecture.md tui-kit/docs/superpowers/plans/2026-05-12-tui-kit-c4tui-refactor-roadmap.md
```

Expected: zero `ImageBox`/`image_box` hits in `src/` or `tests`. Maybe a few in archived plans (`docs/superpowers/plans/2026-05-12-phase-1-*.md` — those are historical references, leave them). The elements grep should not find live instructions to delete elements.

Commit (if any cleanup): `phase 3 verification cleanups`.

---

## Speculation watchpoints (re-check before declaring phase done)

Three things to verify the plan didn't over-engineer:

1. **Did `Modal::pre_render_placements_to_clear` and `post_render_thumbnails` earn their keep?** If both methods return constant values (`vec![MAIN_PLACEMENT_ID]` for one variant; empty otherwise) and the call site could simply inline the differentiation by enum-match on `ActiveModal`, then the trait methods are speculative. **Mitigation:** keep them now because they let `render_modal` take `&mut dyn Modal` rather than `&mut ActiveModal`, which lets `FakeTerminalBackend` not depend on c4tui's `ActiveModal` shape. If after Phase 4 the dispatch is simpler, revisit.

2. **Did `NavPickerModal`'s three `Box<dyn Fn>` fields earn their keep?** `into_target` and `hovered_view` and `thumbnails` could collapse into one method on a new `T`-bound trait — but that trait would itself be three-method, and the call sites would shrink no further. The closures are fine. Don't over-abstract.

3. **Should `NavCellCanvas` be its own type, or just `tui_kit::widgets::grid::GridCellCanvas`?** Currently it's a thin wrapper. If at end of phase the wrapper's methods are 1-for-1 forwarders, delete the wrapper and let `NavItem::render_into_canvas` take `GridCellCanvas<'_>` directly. Save 30 lines.

---

## Phase 5 forward-compat sketch — `LinkCandidate` as a fourth NavItem (NOT shipped here)

To validate that NavPicker is a real abstraction and not a three-cases-disguised-as-one merge, here is the Phase 5 `LinkCandidate` implementation in full. **Do not commit this; it lives only as a comment block to be deleted after the plan is reviewed.**

```rust
// In Phase 5, c4tui/src/link_directory.rs will contain:
//
// use crate::ids::{ElementId, ViewId};
// use crate::nav_picker::{NavItem, NavRenderArtifact, NavCellCanvas, SecondaryClassified};
// use ratatui::style::{Modifier, Style};
//
// #[derive(Debug, Clone)]
// pub struct LinkCandidate {
//     pub target_view: ViewId,
//     pub source_element: Option<ElementId>,
//     pub label: String,
//     pub pinned_element_after_navigation: Option<ElementId>,
// }
//
// impl NavItem for LinkCandidate {
//     type Output = LinkCandidate; // self carries enough to navigate
//
//     fn filter_text(&self) -> &str {
//         &self.label
//     }
//
//     fn render_into_canvas(
//         &self,
//         mut canvas: NavCellCanvas<'_>,
//         selected: bool,
//         _filter: &str,
//         _sink: &mut dyn FnMut(NavRenderArtifact),
//     ) {
//         let marker = if selected { ">" } else { " " };
//         canvas.set_string(0, 0, format!("{marker} {}", self.label), canvas.style());
//         if canvas.height() > 1 {
//             canvas.set_string(0, 1, format!("→ view {}", self.target_view.index()),
//                 Style::default().add_modifier(Modifier::DIM));
//         }
//     }
//
//     fn outcome(&self) -> LinkCandidate {
//         self.clone()
//     }
// }
//
// impl SecondaryClassified for LinkCandidate {}
//
// // And in nav_items.rs, NavTarget gains:
// //   Link(LinkCandidate),
// //
// // And in app.rs, a new Effect::OpenLinkDirectory triggers:
// //   let modal = NavPickerModal {
// //       picker: Cached::new(NavPicker::new(/* link-directory config */, candidates, 0)),
// //       into_target: Box::new(NavTarget::Link),
// //       hovered_view: Box::new(|p| p.selected().map(|c| c.target_view)),
// //       thumbnails: Box::new(|_| Vec::new()),
// //   };
// //   self.active_modal = Some(ActiveModal::Nav {
// //       modal: Box::new(modal),
// //       on_select: Box::new(|target, _state| match target {
// //           NavTarget::Link(c) => Some(Command::NavigateToLink {
// //               view: c.target_view,
// //               pin: c.pinned_element_after_navigation,
// //           }),
// //           _ => None,
// //       }),
// //   });
//
// Total: ~50 lines of new code in Phase 5. No changes to NavPicker. No
// changes to ActiveModal's variants. No changes to TerminalBackend. The
// abstraction holds.
```

This is the test of the abstraction. If, when Phase 5 begins, the LinkCandidate impl above doesn't slot in with ~50 lines of additive code, Phase 3 mis-designed `NavItem` or `NavTarget` and should be revised. Read the Phase 3 result with this sketch open.

---

## End-state self-check

By the end of this phase:

- [x] One `NavPicker<T: NavItem>` component with `NavOutcome<T>`. Three call sites use it.
- [x] One `ActiveModal` enum on `App`. One `render_modal`/`close_modal` pair on `TerminalBackend`. One `handle_modal_key` on `App`.
- [x] One image-viewport widget (`ImageViewport`) in tui-kit; the other (`ImageBox`) is gone.
- [x] `elements` module preserved as the render-effect substrate; no remote protocol work added.
- [x] `FakeTerminalBackend` has 8 trait-method impls (was 11) and 7 `FakeTerminalCall` variants (was 11).
- [x] All tests pass in both repos.
- [x] LinkCandidate-as-NavItem sketched and validates the abstraction.

Phase 3 done. Phase 4 (Command/Effect cleanup) is next; it will consume `NavTarget` and the now-trimmed `Effect` enum.

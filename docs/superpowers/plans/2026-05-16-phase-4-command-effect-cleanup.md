# c4tui Phase 4 — Command + Effect cleanup

**Scope:** c4tui-only cleanup of two parallel command enums (`PendingCommand` + `Command`) and the eleven-variant `Effect` enum. Three independent commits.

**Roadmap section:** `/Users/coleshaffer/Projects/tui-kit/docs/superpowers/plans/2026-05-12-tui-kit-c4tui-refactor-roadmap.md` lines 433–539.

**Architectural decisions** (locked by ground-truth, not deferred to task time):

1. **Option (A) — collapse `PendingCommand` into `Command`, keymap returns `Command` directly.** The roadmap floated A vs B. Ground-truth at `src/event.rs:70–115` shows `resolve()` is a 40-line one-to-one identity map: 24 variants are bit-for-bit identical (`PendingCommand::X → Command::X`), 3 variants are pure renames with no canvas use (`Inspect → InspectAt { 0.5, 0.5 }`; `Zoom → Zoom { anchor: Center }`; `ZoomAt → Zoom { anchor: Canvas { ... } }`), and only **`DragTo` actually uses the `canvas` argument** (it stuffs the whole `CanvasMetrics` into `Command::DragTo`). Option B's rationale ("leaving the canvas dependency in the type signature is genuinely more readable") doesn't apply — the keymap wrapper at `src/keymap.rs:122` already takes `canvas` for the mouse path, so there is no canvas dependency to surface. Option A simply deletes 70 lines.

2. **Lift cycling effects into `AppState` directly.** `CycleScaleBasis`/`CycleOverflow`/`CycleZoomStep` are pure state mutations on `AppState`. They went through `Effect` only because `Effect` was the universal "user did something" channel; that reason was wrong. Direct mutation in `AppState::apply`, no effect emit.

3. **Collapse the three `OpenPicker`-family variants into `Effect::OpenModal(ModalSpec)`.** Phase 3 reshaped the modal *runtime* (one `ActiveModal` enum, one `render_modal`) but left the *effect* enum with three modal-open variants. Unify them now while the code is fresh. `ModalSpec` carries the per-modal payload that drives picker construction:

   ```rust
   #[derive(Debug, Clone, PartialEq, Eq)]
   pub enum ModalSpec {
       ViewPicker,
       ChildViewPicker { target_view_ids: Vec<ViewId> },
       ConnectionPicker { source_element_id: ElementId },
   }
   ```

   `ShowHelp` and `ToggleLogView` are **not** folded into `ModalSpec`. Help is a `DialogSlot` (separate from `active_modal`); LogView lives under `ActiveModal::Log` but its open path doesn't carry a payload the way pickers do, and conflating "open log" with "open picker" forces a fake-payload variant on `ModalSpec`. Keep them as their own variants on `Effect`.

**End-state `Effect`** (six variants, down from eleven):

```rust
pub enum Effect {
    Quit,
    OpenModal(ModalSpec),
    ShowHelp,
    ToggleLogView,
    ReloadWorkspace,
    ClearImageCache,
}
```

**Pre-flight checks done:**

- `PendingCommand::Inspect`'s `(0.5, 0.5)` hardcoded center matches today's behavior — no behavior change from inlining it.
- `KitKeyMap<PendingCommand>` is purely a `KeyTrigger → PendingCommand` lookup; binding `Command` instead is a type-parameter swap, no API surface change in tui-kit.
- `Effect` is `Eq + PartialEq` (line 368); the `Vec<ViewId>` and `ElementId` inside `ModalSpec` are already `Eq + PartialEq`, so `Effect` stays `Eq + PartialEq` post-change. Three tests (`src/state.rs:572,681,712,716`) compare `result.effect` against `Some(Effect::...)` — they'll continue to work.

---

## Task 1 — Collapse `PendingCommand` into `Command`

**Files:** `src/event.rs`, `src/keymap.rs`, anywhere `PendingCommand` is referenced (likely `src/app.rs`).

### Step 1.1 — Update bindings in `src/keymap.rs`

The keymap's `KitKeyMap<PendingCommand>` becomes `KitKeyMap<Command>`. Rewrite the bindings (~25 `map.bind(...)` calls) using `Command` directly. Per-binding translation:

- 24 trivial: `PendingCommand::X` → `Command::X` (Quit, OpenPicker, Reload, Help, Back, ShowLegend, OpenConnectionPicker, ClearOrQuit, Pan, ResetView, EndDrag, ToggleLog, CycleScaleBasis, CycleOverflow, CycleZoomStep, Noop, etc.).
- `PendingCommand::Inspect` → `Command::InspectAt { canvas_x: 0.5, canvas_y: 0.5 }` (matches old `resolve()` hardcoded center).
- `PendingCommand::Zoom { factor }` → `Command::Zoom { factor, anchor: ZoomAnchor::Center }`.

Type alias: `pub type KeyMap = KitKeyMap<Command>;`.

### Step 1.2 — Inline canvas in the mouse path

`keymap.rs:122–~140` is the trait method `resolve(&self, event: InputEvent, canvas: CanvasMetrics) -> PendingCommand`. Change return type to `Command`. The three mouse arms:

- `MouseEvent::Click { x, y }` → drill-at: `Command::DrillAt { canvas_x, canvas_y }` (use the existing `mouse_to_canvas_fraction` conversion).
- `MouseEvent::WheelUp/WheelDown { x, y }` → was `PendingCommand::ZoomAt`, now `Command::Zoom { factor, anchor: ZoomAnchor::Canvas { canvas_x, canvas_y } }`.
- `MouseEvent::Drag { x, y }` → was `PendingCommand::DragTo { x, y }`, now `Command::DragTo { x, y, canvas }` (the field that previously made canvas load-bearing).
- `MouseEvent::Release` → was `PendingCommand::EndDrag`, now `Command::EndDrag`.
- `InputEvent::Key(key)` → `self.lookup(key).unwrap_or(Command::Noop)`.

### Step 1.3 — Delete `PendingCommand` and `resolve()`

In `src/event.rs`:

- Delete the `PendingCommand` enum (lines 30–68).
- Delete `impl PendingCommand { fn resolve(...) }` (lines 70–115).
- Keep `mouse_to_canvas_fraction`, `ZoomAnchor`, `Command`, and the `TuiKitInputEvent` re-export.

In `src/app.rs` (and anywhere else): every `PendingCommand` reference becomes `Command`. Every `.resolve(canvas)` call disappears — the keymap returns `Command` already.

### Step 1.4 — Verify and commit

```
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

Commit: `collapse PendingCommand into Command (drop 70-line identity resolve)`.

---

## Task 2 — Lift cycling effects into `AppState::apply`

**Files:** `src/state.rs`, `src/app.rs`.

### Step 2.1 — Find the cycling targets on `AppState`

Locate the state fields that `Effect::CycleScaleBasis`/`CycleOverflow`/`CycleZoomStep` mutate in `src/app.rs:646–660`. Each effect arm flips a field on `AppState` (or on a substate the app holds). Lift that mutation into `AppState::apply`.

### Step 2.2 — Replace effect-emit with direct mutation

In `src/state.rs`, replace these three lines with direct field-cycling:

- Line 88: `result.effect = Some(Effect::CycleScaleBasis);` → cycle the basis field on `self` (or the appropriate substate). Set `result.render = true` if a re-render is needed.
- Line 92: `result.effect = Some(Effect::CycleOverflow);` → cycle the overflow field.
- Line 96: `result.effect = Some(Effect::CycleZoomStep);` → cycle the zoom-step field.

The cycling logic currently lives in `src/app.rs:646–660`; move it (or refactor into small helpers on `AppState`).

### Step 2.3 — Delete the variants and the app-side arms

- Delete `Effect::CycleScaleBasis`, `Effect::CycleOverflow`, `Effect::CycleZoomStep` from `src/state.rs:378–380`.
- Delete the three matching arms in `src/app.rs:646–660`.

### Step 2.4 — Verify and commit

```
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

Commit: `lift cycling effects into AppState; Effect drops 3 variants`.

---

## Task 3 — Collapse modal opens into `Effect::OpenModal(ModalSpec)`

**Files:** `src/state.rs`, `src/app.rs`, possibly `src/modal.rs` (if `ModalSpec` lives there).

### Step 3.1 — Add `ModalSpec`

Place `ModalSpec` in `src/state.rs` next to `Effect` (it's an `Effect` payload, not a modal-runtime type). Derive `Debug, Clone, PartialEq, Eq`.

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModalSpec {
    ViewPicker,
    ChildViewPicker { target_view_ids: Vec<ViewId> },
    ConnectionPicker { source_element_id: ElementId },
}
```

### Step 3.2 — Reshape `Effect`

Replace the three modal variants on `Effect` with `OpenModal(ModalSpec)`:

```rust
pub enum Effect {
    Quit,
    OpenModal(ModalSpec),
    ShowHelp,
    ToggleLogView,
    ReloadWorkspace,
    ClearImageCache,
}
```

### Step 3.3 — Update emit sites in `AppState::apply`

In `src/state.rs`:

- Line 57 (`Effect::OpenPicker`) → `Effect::OpenModal(ModalSpec::ViewPicker)`.
- Line 139 (`Effect::OpenChildViewPicker { target_view_ids }`) → `Effect::OpenModal(ModalSpec::ChildViewPicker { target_view_ids })`.
- Line 165 (`Effect::OpenConnectionPicker { source_element_id }`) → `Effect::OpenModal(ModalSpec::ConnectionPicker { source_element_id })`.

Update the three test assertions in `src/state.rs` (line ~572 and any others under `mod tests`) to match.

### Step 3.4 — Update consumer in `src/app.rs`

Replace the three separate `Some(Effect::OpenPicker)`, `Some(Effect::OpenChildViewPicker { ... })`, `Some(Effect::OpenConnectionPicker { ... })` arms with one `Some(Effect::OpenModal(spec))` arm that matches on `spec` and dispatches to the same three spawn-site bodies as before. The bodies don't change — only the matching shape consolidates.

Decision point: keep the three picker-spawn bodies inline (one big match on `spec`), or factor each into a `fn open_view_picker`, `fn open_child_view_picker`, `fn open_connection_picker` helper. **Prefer inline** unless the inline match exceeds ~80 lines; the existing spawn-site bodies are already self-contained.

### Step 3.5 — Verify and commit

```
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

Commit: `collapse three modal-open Effects into OpenModal(ModalSpec)`.

---

## Task 4 — End-of-phase verification

### Step 4.1 — Full clean run

```
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

All green. tui-kit untouched — no need to re-run it.

### Step 4.2 — Line-count sanity check

```
wc -l src/event.rs src/state.rs src/keymap.rs src/app.rs
```

Expected:

- `event.rs` ~95 (was 171; ~70 lines removed by Task 1).
- `state.rs` slightly larger if the cycling mutations grew helper code, slightly smaller if they inlined cleanly. Expect within ±20 of 719.
- `keymap.rs` ~245 (was 256; small reduction).
- `app.rs` ~1240 (was 1274; cycling arms removed, modal arms consolidated).

If any file grew substantially without a clear reason, investigate before declaring done.

### Step 4.3 — `Effect` final-form check

```
grep -n 'pub enum Effect' src/state.rs
grep -c 'Effect::' src/
```

`Effect` should have exactly 6 variants. The grep count tells you how many call sites reference Effect variants — useful as a delta check against pre-Phase-4 numbers.

---

## End-state self-check

By the end of Phase 4:

- [ ] `PendingCommand` and `resolve()` are deleted. Keymap returns `Command`.
- [ ] `Effect` has 6 variants: `Quit`, `OpenModal(ModalSpec)`, `ShowHelp`, `ToggleLogView`, `ReloadWorkspace`, `ClearImageCache`.
- [ ] `ModalSpec` carries the three picker payloads.
- [ ] Cycling logic lives in `AppState`, not `Effect`.
- [ ] `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test` all clean in c4tui.
- [ ] Three independent commits, each behaviorally orthogonal.

---

## Forward links

- **Phase 5 (LinkDirectory)** can now treat `ModalSpec` as the extensibility point: add `ModalSpec::LinkDirectory { ... }` plus a matching `NavTarget::Link(...)` and a single new arm in the `Effect::OpenModal` dispatch in `app.rs`. No new effect, no new picker, no new modal type. This is the test of whether Phases 3+4 actually earned their abstractions.
- **`InspectAt`'s hardcoded `(0.5, 0.5)`** carried over from Phase 4 unchanged. If a future phase wants `i` to inspect at the cursor (rather than canvas center), that's a behavior change, not refactor work; flag separately.

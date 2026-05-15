# Handoff — c4tui Phase F Task 5 (Modal trait + ActiveModal) complete

**Date:** 2026-05-15
**Repo:** c4tui
**Scope:** This handoff covers c4tui only. tui-kit was unaffected (no
tui-kit changes; tui-kit `Cached` API is consumed unchanged).

## State

| Repo | Branch | HEAD | Tests | clippy | fmt | Origin |
|------|--------|------|-------|--------|-----|--------|
| `c4tui` | `main` | `030accf` | 95 passing | clean | clean | pushed |

Verify locally:

```bash
cd /Users/coleshaffer/Projects/c4tui
cargo test --quiet 2>&1 | grep "test result:"
cargo clippy --all-targets --quiet
cargo fmt --check
```

## What shipped

Two commits.

| Commit | Subject |
|--------|---------|
| `86cb3f4` | docs(plans): add Phase F Task 5 bite-sized modal-trait plan |
| `030accf` | add Modal/NavModal/LogModal traits and ActiveModal enum |

The implementation commit lands the new modal abstraction **alongside**
the existing slot fields. No handler switching, no slot deletion — that
is Task 6's job. Task 5's value is that Task 6 becomes mechanical: it
moves call sites one by one against an already-landed target.

### What `030accf` adds

- `c4tui/src/modal.rs` (~180 lines):
  - `Modal` trait — render-only contract.
  - `NavModal: Modal` — adds `handle_key → NavModalOutcome` (which
    translates `NavOutcome<T::Output>` into the union `NavTarget` at the
    trait boundary), plus optional `currently_hovered_view` and
    `thumbnails` hooks.
  - `LogModal: Modal` — adds `handle_key` with a `&dyn Clipboard`
    parameter.
  - `NavModalOutcome` (`Continue | Select(NavTarget) | Cancel`) and
    `LogModalOutcome` (`Continue | Close`).
  - Blanket `Modal` impl for `Cached<C: BufferComponent<Event = KeyEvent>>`
    — covers `NavPicker<T: NavItem>`.
  - Direct `Modal` and `LogModal` impls for `LogView`.
  - `NavPickerModal<T: NavItem>` — the concrete `NavModal`. Holds a
    `Cached<NavPicker<T>>` plus three closures (`into_target`,
    `hovered_view`, `thumbnails`) — one per spawn-site.
  - `ActiveModal { Nav { modal, on_select } | Log { modal } }` — the
    top-level slot. Dialog stays separate (different lifecycle).
- `src/app.rs`: `App.active_modal: Option<ActiveModal>` field, initialized
  to `None`, silenced with `#[allow(dead_code)]` until Task 6.
- `src/main.rs`: `mod modal;` line.
- `src/terminal.rs`: `render_log_view` becomes `pub(crate)` so
  `impl Modal for LogView` can reach it.

### Design decisions worth remembering

- **`SecondaryClassified` trait bound dropped.** The parent plan
  referenced a trait that does not exist in current c4tui code. The
  current `NavItem` trait already carries `is_secondary()` as a default
  method. Just `T: NavItem` is sufficient.
- **`ComponentOutcome` is `#[non_exhaustive]`.** The `NavModal::handle_key`
  match enumerates all current variants and uses a `_ => Continue`
  catch-all. If the variant set grows and the new variant happens to
  carry a NavPicker `Select`/`Cancel` analogue, that's a Task 6+
  consideration.
- **Complex closure types factored into aliases.** `IntoNavTarget<O>`,
  `HoveredViewFn<T>`, `ThumbnailFn<T>`, `NavOnSelect`. Keeps the struct
  and enum definitions readable, satisfies `clippy::type_complexity`,
  and gives Task 6 named hooks to reach for at spawn-time.
- **`Send` bounds kept.** The parent plan said drop if it bites. It did
  not bite — every captured type at the spawn-sites is `Send`-safe.

### What was NOT changed

- The four existing slot fields (`picker_slot`, `connection_picker_slot`,
  `dialog_slot`, `log_slot`) and every handler that reads/writes them.
- `App::return_to_root` cleanup — does not touch `active_modal` yet.
- Any test. The abstraction is unused; tests come in Task 6.

## Plan-level Exit criteria (Task 5)

All met:

- ✅ `c4tui/src/modal.rs` exists with `Modal`, `NavModal`, `LogModal`,
  `NavModalOutcome`, `LogModalOutcome`, `NavPickerModal<T>`, `ActiveModal`.
- ✅ `Modal` blanket impl for `Cached<C: BufferComponent<Event=KeyEvent>>`
  and direct impl for `LogView`.
- ✅ `LogModal for LogView`.
- ✅ `Modal for NavPickerModal<T>` and `NavModal for NavPickerModal<T>`.
- ✅ `App.active_modal: Option<ActiveModal>` (initialized `None`).
- ✅ `render_log_view` is `pub(crate)`.
- ✅ Build, clippy, test, fmt all clean; test count unchanged at 95.

## What's next for c4tui

- **Task 6 — Switch handlers and delete old slots.** The natural next
  sub-task. Mechanical: each spawn-site that currently writes
  `self.picker_slot = Some(...)` or `self.connection_picker_slot =
  Some(...)` or `self.log_slot = Some(...)` switches to building an
  `ActiveModal` and writing `self.active_modal = Some(...)`. Each
  handler that currently dispatches via the three slot fields switches
  to one match on `&mut self.active_modal`. Once all three are migrated,
  the old slot fields are deleted in the same commit (or in a single
  follow-up).

- **Task 7 — `TerminalBackend` method collapse.** After Task 6, the
  three method pairs on `TerminalBackend` (`render_picker` /
  `close_picker`, `render_connection_picker` / `close_connection_picker`,
  `render_log` / `close_log`) collapse into one pair (`render_modal` /
  `close_modal`).

- **Task 8 — already shipped on the tui-kit side** (`image_box`
  deletion, `15c41ab` in tui-kit). Nothing more to do here.

After Tasks 6 and 7 land, the parent-plan exit criterion #5
("c4tui placeholders cleared") is materially complete, and tui-kit Phase
C can begin deletion of the retained widgets in `src/elements/mod.rs`
that this consolidation proves unused.

## Cross-repo context

tui-kit (`main` at `b29b655`) is unaffected. The `Cached` API surface,
`BufferComponent` trait shape, and `KeyEvent` type are all consumed
unchanged. `cargo test` against the current tui-kit `main` passes; no
tui-kit changes are needed for Task 6 either.

## How to pick up c4tui work from here

```bash
cd /Users/coleshaffer/Projects/c4tui
git pull
# Read the Phase 3 detailed plan, Task 6 onward:
sed -n '1644,1909p' docs/superpowers/plans/2026-05-12-phase-3-navpicker-modal-image-elements.md
```

Task 6's parent-plan section is ~265 lines; its bite-sized plan is the
next artifact a future agent should write before execution.

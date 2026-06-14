# Handoff — c4tui Phase F Tasks 1–4 (NavPicker consolidation) complete

**Date:** 2026-05-14
**Repo:** c4tui
**Scope:** This handoff covers c4tui only. For the parallel tui-kit work (Phase B — RenderEffect contract), see
`/Users/coleshaffer/Projects/tui-kit/docs/superpowers/handoffs/2026-05-14-phase-B-render-effect-complete.md`.

## State

| Repo | Branch | HEAD | Tests | clippy | fmt | Origin |
|------|--------|------|-------|--------|-----|--------|
| `c4tui` | `main` | `386a3d3` | 95 passing | clean | clean | pushed |

Verify locally:

```bash
cd /Users/coleshaffer/Projects/c4tui
cargo test --quiet 2>&1 | grep "test result:"
cargo clippy --all-targets --quiet
cargo fmt --check
```

## What shipped in this session

The four Phase F NavPicker-consolidation sub-tasks from the (tui-kit-resident) Phase 3 plan landed across 9 commits.

| Commit | Subject |
|--------|---------|
| `0740e14` | add NavTarget enum and empty NavItem types |
| `3f65427` | add NavPicker<T: NavItem> with TDD coverage of filter/arrow/Tab/Esc/Enter behavior |
| `43a9707` | add ViewNavItem / ChildViewNavItem / ConnectionNavItem implementing NavItem |
| `2eddb6e` | refactor(nav_picker): replace mode-flag booleans with enum and cache visible indices |
| `d104b29` | test(nav_items): lock in name-only filter surface for the three NavItem types |
| `1b1dd7c` | migrate top-level view picker to NavPicker<ViewNavItem> |
| `618eb7a` | migrate child-view picker to NavPicker; drop ChildViewNavItem newtype |
| `5875dfb` | fix(picker): tighten action discipline and cover filter-no-toggle tab path |
| `386a3d3` | delete picker.rs and connection_picker.rs; one NavPicker now serves all three call sites |

### What was consolidated

| Before | After |
|--------|-------|
| `src/picker.rs` (696 lines, `ViewPicker`) | deleted |
| `src/connection_picker.rs` (542 lines, `ConnectionPicker`) | deleted |
| Inline child-view picker spawned via `Effect::OpenChildViewPicker` | folded into `NavPicker<ViewNavItem>` (Task 2 forced this via shared `picker_slot`) |
| Three separate picker types + three rendering paths + three keymaps | One generic `NavPicker<T: NavItem>` in `src/nav_picker.rs` |
| Two concrete item types planned (`ViewNavItem`, `ChildViewNavItem`, `ConnectionNavItem`) | Two surviving (`ViewNavItem`, `ConnectionNavItem` — `ChildViewNavItem` collapsed in Task 3, with intent carried by `PickerSlot.action`) |

Net code change: `+1,412` / `−1,275` across 8 files. Two source files deleted (1,238 lines).

### Design decisions worth remembering

- **`NavPickerMode` enum** replaced the original plan's three boolean flags on `NavPickerConfig` — chosen during Task 1's review pass when the abstraction was still consumer-free.
- **`visible_indices: Vec<usize>` cache** in `NavPicker` — recomputed only on filter/secondary-toggle change, not per-keypress. Makes per-key paths allocation-free.
- **Shared `picker_slot` field** forced both `Effect::OpenPicker` and `Effect::OpenChildViewPicker` to migrate together in Task 2. The plan's literal task ordering was adjusted by necessity. Task 3's actual scope became "swap to `ChildViewNavItem` and differentiate config" rather than "migrate the call site"; we ultimately chose to collapse `ChildViewNavItem` entirely and carry the variant intent on `PickerSlot.action` (`SelectView | Drill`).
- **`ConnectionPickerSlot` is a separate field on `App`** (Approach A in Task 4) rather than a unified slot. The two `NavItem` impls have different `Output` types (`ViewId` vs `ConnectionNavigationCandidate`); unification belongs to the Modal-trait work in the next sub-task.

### Behavioral divergence to flag for the operator

The legacy `ConnectionPicker` treated `KeyEvent::Tab` as a synonym for `KeyEvent::Down`. `NavPicker<ConnectionNavItem>` uses `NavPickerMode::Flat`, which silently drops `Tab`. No production test depended on the synonym, but operator muscle memory may notice. If you want the synonym back, the cheapest fix is binding `Tab → Down` at c4tui's connection-picker focus scope (`src/keymap.rs`), not patching `NavPicker`.

The connection-picker footer hint also changed character set from ASCII `->` to Unicode `→` (now consistent with the other two picker call sites).

## Plan references

Plans for this work currently live in **tui-kit's repo**:

- **Strategic operational plan:** `/Users/coleshaffer/Projects/tui-kit/docs/superpowers/plans/2026-05-14-revised-library-author-implementation-plan.md`. Phase F is the c4tui validation track.
- **Per-phase detailed plan (older joint sequence):** `docs/superpowers/plans/2026-05-12-phase-3-navpicker-modal-image-elements.md` (this repo). This is the plan whose Tasks 1–4 just landed. Tasks 5–8 (Modal unification, terminal-backend collapse, image-widget winner, elements decision) describe what's next.

## What's next for c4tui

Within Phase F, the remaining sub-tasks (per the operational plan and Phase 3 plan's Tasks 5–8):

- **Modal handling unification** — introduce `Modal` trait, `ActiveModal` enum, and a single `render_modal` / `close_modal` pair on `TerminalBackend`. This unifies `picker_slot` and `connection_picker_slot` and folds `LogView` under the same abstraction. (Phase 3 plan's Decision D specifies the trait shape.)
- **Single image-viewport path wired** — confirm `ImageViewport` (the surviving widget) is the path through `view.rs` / `terminal.rs`. tui-kit's deletion of `image_box` lives in tui-kit's Phase C.
- **`LinkDirectory`** — implement the keyboard-first link-navigation surface (`NavTarget::Link` is already reserved in `NavTarget`).
- **Command/Effect cleanup** — collapse the duplication left around picker open/close commands and effects.
- **View catalog / render cache / viewport state split** — once picker and LinkDirectory dependencies are narrow.

Each of these can pull tui-kit `elements` types in — when one lands, *update tui-kit's Phase C tracking* to record which retained widgets the sub-task consumed (or confirmed unused). c4tui Phase F's reports drive tui-kit's Phase C deletion decisions.

Outside Phase F, the c4tui repo also carries the existing `implementation-plan.md` and `docs/architecture-redesign-plan.md` which describe earlier project sequencing — those predate the library-author direction.

## Cross-repo context

In parallel with this work, tui-kit shipped Phase B (`TerminalEffect` → `RenderEffect` rename and data-only contract lock). c4tui consumes zero effect types from tui-kit's `elements` module, so Phase B was invisible to this repo — `cargo test` against tui-kit's new `main` is unchanged. See tui-kit's Phase B handoff for the library-side narrative.

## How to pick up c4tui work from here

```bash
cd /Users/coleshaffer/Projects/c4tui
git pull
# Read the operational plan in tui-kit's repo:
cat /Users/coleshaffer/Projects/tui-kit/docs/superpowers/plans/2026-05-14-revised-library-author-implementation-plan.md
# And the Phase 3 detailed plan (lives here in c4tui; Tasks 5–8 are the next chunk):
cat docs/superpowers/plans/2026-05-12-phase-3-navpicker-modal-image-elements.md
```

The Phase 3 plan's Task 5 (Modal trait + ActiveModal) is the natural next sub-task: it unifies the slot infrastructure that Tasks 2–4 left as two parallel fields (`picker_slot`, `connection_picker_slot`), and it folds `LogView` under the same abstraction.

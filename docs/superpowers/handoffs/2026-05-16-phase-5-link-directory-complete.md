# Handoff — c4tui Phase 5 LinkDirectory complete

**Date:** 2026-05-16
**Repo:** c4tui
**Scope:** Implements Phase 5 from the tui-kit + c4tui refactor roadmap.

## State

Phase 5 is implemented in the working tree after `bb7d4a0`.

Verification:

```bash
cd /Users/coleshaffer/Projects/c4tui
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
```

All three pass. Test count is 97.

Cross-repo plan artifact:

- `/Users/coleshaffer/Projects/tui-kit/docs/superpowers/plans/2026-05-12-phase-5-link-directory.md`

## What shipped

- Added `src/link_directory.rs` with `LinkKind`, `LinkCandidate`, and
  `collect_link_candidates`.
- Link candidates cover:
  - detail links from child/detail metadata on the current view;
  - relationship links for the pinned element;
  - the current view's legend/key link.
- `LinkCandidate` implements `NavItem`, so LinkDirectory is a
  `NavPicker<LinkCandidate>` and reuses `ActiveModal::Nav`.
- Root Enter now opens LinkDirectory via `Command::OpenLinkDirectory` and
  `ModalSpec::Link`.
- Selecting a link dispatches `Command::SelectLink`, pushes breadcrumbs, switches
  views, clears drag state, and optionally pins the linked element.
- The old root connection-picker command/modal path was deleted. Relationship
  traversal now flows through LinkDirectory.
- README, specification, architecture, help text, and status hints now describe
  Enter as the link-directory path.

## Tests added or updated

- Pure link-candidate tests for detail, connection, legend, no-pinned, and
  filter-token behavior.
- App integration tests for:
  - selecting a detail link with no pinned element;
  - selecting a relationship link after pinning via inspect;
  - canceling LinkDirectory;
  - opening an empty LinkDirectory.
- State/keymap tests for `OpenLinkDirectory` and `SelectLink`.

## What's next

1. **Phase 6 — ViewStore split.** Next roadmap phase. It needs a concrete
   plan before implementation:
   `/Users/coleshaffer/Projects/tui-kit/docs/superpowers/plans/2026-05-12-tui-kit-c4tui-refactor-roadmap.md`
   §652-743.
2. **Operator smoke test remains pending.** The checklist is still scaffolded at
   `/Users/coleshaffer/Projects/tui-kit/docs/superpowers/handoffs/2026-05-15-image-smoke-checklist.md`.
   It requires live terminal/SSH/container execution by the operator.

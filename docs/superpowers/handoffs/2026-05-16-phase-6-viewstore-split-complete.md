# Phase 6 - ViewStore Split Complete

## Summary

Phase 6 split the former `ViewStore` into three explicit owners:

- `ViewCatalog`: exported view metadata, workspace model, relationship navigation data, and export lifetime guard.
- `RenderCache`: rendered SVG rasters, raster budget, render-job discovery, and scheduler merge points.
- `ViewportState`: viewport widgets, transforms, placement policy, hit testing, zoom, pan, and reset behavior.

`src/view.rs` and the `ViewStore` type were removed. `App` now owns the three split types directly, and call sites pass the narrowest owner set they need.

## Key Changes

- `TerminalBackend::render` takes `&ViewCatalog`, `&mut RenderCache`, and `&mut ViewportState`.
- Modal rendering takes only `&ViewCatalog` and `&RenderCache`; modal close takes only `&ViewCatalog`.
- `AppState::apply` takes explicit catalog/cache/viewport references for commands that need rendered hit testing, zoom, pan, or reset behavior.
- `RenderScheduler::drain_into` inserts into `RenderCache` and invalidates matching `ViewportState` entries.
- `LinkDirectory` candidate collection now depends only on `&ViewCatalog`.
- Existing store tests were redistributed to `view_catalog` and `viewport_state`.

## Verification

- `cargo fmt --check` in `c4tui`
- `cargo clippy --all-targets --all-features -- -D warnings` in `c4tui`
- `cargo test` in `c4tui`: 97 passed
- `cargo fmt --check` in `tui-kit`
- `cargo clippy --all-targets --all-features -- -D warnings` in `tui-kit`
- `cargo test` in `tui-kit`: 153 lib tests, 11 parity tests, 2 doc-tests ignored

## Remaining

- Operator image smoke test remains pending at
  `/Users/coleshaffer/Projects/tui-kit/docs/superpowers/handoffs/2026-05-15-image-smoke-checklist.md`.

# c4tui

Interactive terminal viewer for Structurizr C4 diagrams. Click-to-drill navigation across views, rendered via the Kitty graphics protocol.

## Status

Pre-alpha. Phases 0–4 are implemented: the binary detects terminal capabilities, exports/rasterizes Structurizr SVG views, displays them with Kitty graphics, provides a keyboard view picker with per-view image caching, supports pan/zoom via Kitty source rectangles, and can click-to-drill into child views when Structurizr metadata links an element to another view. See [implementation-plan.md](./implementation-plan.md) for the phased roadmap.

## Usage

```sh
cargo run -- --workspace ./workspace.dsl
```

With `--workspace`, c4tui exports the workspace to SVG with `structurizr-cli`, renders the first exported view inline, opens a view picker with `o`, switches views with Up/Down + Enter, pans with arrow keys or mouse drag, zooms with `+`/`-` or the mouse wheel, resets fit with `0`/`f`, drills into child views by clicking diagram elements, goes back with Backspace, and exits with `q`. Without `--workspace`, it only probes terminal capabilities and exits. It returns non-zero when Kitty graphics support is unavailable, because inline image rendering requires it.

## What it does (intended)

Open a Structurizr workspace (`workspace.dsl` or `workspace.json`) and browse its views from a terminal. Each view is rendered as a high-DPI raster image, displayed inline via the Kitty graphics protocol. Clicking on an element that has a child view (e.g., a Container in a System Context view) navigates into the corresponding view, mirroring the C4 navigation experience from the official Structurizr web UI — but inside your terminal, in a single static binary, with no daemon.

## Why

Structurizr Lite/Local is an excellent local-first browser for C4 diagrams, but using it requires Docker (or a JVM) plus a graphical browser. For terminal-centric workflows, that is heavy. c4tui aims to deliver the essential read-and-navigate experience entirely inside a modern terminal.

## Requirements

- A terminal that supports the **Kitty graphics protocol** (Kitty, WezTerm, Ghostty)
- A terminal that supports **SGR pixel mouse mode** (CSI 1016) for click-to-drill
- True color (24-bit)
- A reachable `structurizr-cli` binary on `PATH` (used to export views to SVG)
- Read access to a Structurizr workspace file

Sixel and iTerm2 inline-image fallbacks are explicitly out of scope for v1.

## Documents

- [specification.md](./specification.md) — what c4tui does, contracts, failure modes
- [architecture.md](./architecture.md) — components, data flow, technology choices
- [implementation-plan.md](./implementation-plan.md) — phased delivery and acceptance criteria

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](./LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](./LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.

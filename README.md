# c4tui

Interactive terminal viewer for Structurizr C4 diagrams. Click-to-drill navigation across views, rendered via the Kitty graphics protocol.

## Status

Pre-alpha. The repository currently contains specification, architecture, and implementation plan only — no code yet. See [implementation-plan.md](./implementation-plan.md) for the phased roadmap.

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

TBD.

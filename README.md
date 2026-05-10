# c4tui

Interactive terminal viewer for Structurizr C4 diagrams. Click-to-drill navigation across views, rendered via the Kitty graphics protocol.

## Status

Pre-alpha moving toward v1. Phases 0–6 are implemented, and the architecture hardening pass is complete: the binary detects terminal capabilities, exports/rasterizes Structurizr SVG views, displays them with Kitty graphics, provides a keyboard view picker with per-view image caching, supports pan/zoom via Kitty source rectangles, click-to-drill navigation, reload, help, config, and file logging. See [implementation-plan.md](./implementation-plan.md) for the phased roadmap.

## Install

From crates.io:

```sh
cargo install c4tui
```

From Homebrew:

```sh
brew install scshafe/tap/c4tui
```

From source:

```sh
git clone https://github.com/scshafe/c4tui.git
cd c4tui
cargo install --path .
```

GitHub Releases include prebuilt archives for macOS arm64, macOS amd64, Linux amd64, and Linux arm64. The release pipeline is triggered by pushing a `vMAJOR.MINOR.PATCH` tag; it builds those archives, publishes the crate with `CARGO_REGISTRY_TOKEN`, and updates `scshafe/homebrew-tap` with `HOMEBREW_TAP_TOKEN`.

## Usage

```sh
cargo run -- --workspace ./workspace.dsl
```

With `--workspace`, c4tui exports the workspace to SVG with the `structurizr` CLI (resolved from `PATH`), renders the first exported view inline, opens a view picker with `o`, switches views with Up/Down + Enter, pans with arrow keys or mouse drag, zooms with `+`/`-` or the mouse wheel, resets fit with `0`/`f`, drills into child views by clicking diagram elements, goes back with Backspace, reloads the workspace with `r`, shows help with `?`, opens the in-app log viewer with `L`, and exits with `q`. Without `--workspace`, it only probes terminal capabilities and exits. It returns non-zero when Kitty graphics support is unavailable, because inline image rendering requires it.

Useful flags:

```sh
c4tui --workspace ./workspace.dsl \
  --config ~/.config/c4tui/config.toml \
  --log-file ./c4tui.log
```

### Logs

c4tui never writes log lines to stderr while the alt-screen UI is active, so warnings can't bleed into the diagram. Every log record is buffered in memory (last 1000 entries) and, if `--log-file` is supplied, also tee'd to the file. Filtering is `RUST_LOG` as usual:

```sh
RUST_LOG=c4tui=debug c4tui --workspace ./workspace.dsl --log-file ./c4tui.log
```

Press **`L`** while running to open the in-app log viewer. Inside the viewer:

| key | action |
|---|---|
| `j` / `↓` | scroll down one line |
| `k` / `↑` | scroll up one line |
| `g` / `G` | jump to oldest / newest |
| `y` | yank visible lines to the system clipboard |
| `Y` | yank the entire buffer |
| `c` | clear the in-memory buffer |
| `Esc` / `q` | close the viewer |

### Clipboard

Yank uses a pluggable clipboard backend. The built-in `DefaultClipboard` shells out to `pbcopy` on macOS for text (lands directly in the OS clipboard) and falls back to `/tmp/c4tui-yank.txt` on other platforms. Image yank is currently always a fallback file (`/tmp/c4tui-yank-{epoch}.png`); native image-to-clipboard requires a platform-specific extension and is not in the built-in. The `Clipboard` trait in `src/clipboard.rs` is the seam — a future crate (e.g., `c4tui-clipboard-macos` using `NSPasteboard`) implements it and is swapped in at `App::new` time.

## Configuration

c4tui reads `~/.config/c4tui/config.toml` when present, or a path supplied with `--config`.

```toml
# Raster scale used before sending images to the terminal. Clamped to 1.0..8.0.
dpi_scale = 4.0

[keybindings]
quit = "q"
open_picker = "o"
reload = "r"
help = "?"
zoom_in = "+"
zoom_out = "-"
reset = "0"
fit = "f"
```

All runtime keybindings are discoverable from the `?` help overlay.

### Zoom and placement

The `+` / `-` step factors and the diagram placement policy are also configurable. Defaults render the diagram fit to the terminal at zoom 1.0 and let the magnified image's logical cell rect exceed the canvas at high zoom (with c4tui clamping the actual Kitty placement so the status / footer bars are preserved).

```toml
[zoom]
in_factor = 1.5    # each `+` magnifies by this ratio (>1.0)
out_factor = 0.667 # each `-` shrinks by this ratio (<1.0)

[placement]
# Where zoom=1.0 starts. Choices:
#   "fit"    — image fit to terminal (default; whole diagram visible)
#   "native" — image at native pixel size (magnifier mode; usually overflows)
#   "fill"   — image fills both terminal dimensions (crops the longer aspect)
scale_basis = "fit"

# What happens when zoom > what the canvas can show. Choices:
#   "fit_within"          — zoom can't grow past fit
#   "crop"                — sample window scrolls; image always centered+letterboxed
#   "overflow_cells"      — (default) cell rect may exceed canvas; pan still works
#   "overflow_source"     — send full source; terminal scales to canvas-sized cells
#   "prevent_zoom_beyond" — like fit_within but preserves the requested scale value
overflow = "overflow_cells"
```

## What it does (intended)

Open a Structurizr workspace (`workspace.dsl` or `workspace.json`) and browse its views from a terminal. Each view is rendered as a high-DPI raster image, displayed inline via the Kitty graphics protocol. Clicking on an element that has a child view (e.g., a Container in a System Context view) navigates into the corresponding view, mirroring the C4 navigation experience from the official Structurizr web UI — but inside your terminal, in a single static binary, with no daemon.

## Why

Structurizr Lite/Local is an excellent local-first browser for C4 diagrams, but using it requires Docker (or a JVM) plus a graphical browser. For terminal-centric workflows, that is heavy. c4tui aims to deliver the essential read-and-navigate experience entirely inside a modern terminal.

## Requirements

- A terminal that supports the **Kitty graphics protocol** (Kitty, WezTerm, Ghostty)
- A terminal that supports **SGR pixel mouse mode** (CSI 1016) for click-to-drill
- True color (24-bit)
- A reachable `structurizr` binary on `PATH` (the upstream Structurizr CLI from <https://github.com/structurizr/structurizr>; see notes below)
- Read access to a Structurizr workspace file

Sixel and iTerm2 inline-image fallbacks are explicitly out of scope for v1.

### Note on the Structurizr CLI

c4tui shells out to `structurizr export --workspace … --format svg --output …`. The legacy `structurizr-cli` (archived 2026-02-01) was superseded by the consolidated `structurizr` tool. **SVG/PNG export in the new tool is rendered through Playwright**, so the build of `structurizr` you install must include the Playwright exporter. Distributions tagged "application" on the upstream releases page bundle it; library-only releases do not.

## Documents

- [specification.md](./specification.md) — what c4tui does, contracts, failure modes
- [architecture.md](./architecture.md) — components, data flow, technology choices
- [implementation-plan.md](./implementation-plan.md) — phased delivery and acceptance criteria
- [docs/c4tui.1](./docs/c4tui.1) — man page source
- [docs/releasing.md](./docs/releasing.md) — release automation and required secrets
- [docs/architecture-redesign-plan.md](./docs/architecture-redesign-plan.md) — completed architecture hardening pass

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](./LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](./LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.

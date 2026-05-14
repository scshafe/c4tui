//! Generic, keyboard-driven picker over any type implementing [`NavItem`].
//!
//! NavPicker is the substrate for the three near-identical c4tui pickers
//! (`ViewPicker`, `ConnectionPicker`, and the inline child-view picker spawned
//! by the `OpenChildViewPicker` effect). Item-type specialisation happens
//! entirely through the [`NavItem`] trait; layout, filter, scroll, and key
//! handling live here once.
//!
//! This file is introduced by Phase 3 Task 1 Step 1.2 (trait + struct) and
//! Step 1.3 (TDD coverage of filter / arrow / Tab / Esc / Enter behavior).
//! The three concrete `NavItem` impls live in `nav_items.rs` and arrive in
//! Step 1.4. Migration of the existing pickers to NavPicker is deferred to
//! Tasks 2-4 -- nothing downstream constrains this file yet.

#![allow(dead_code)]

use anyhow::Result;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};
use tui_kit::component::{BufferComponent, ComponentId, ComponentOutcome, DirtyReason, DirtyState};
use tui_kit::input::KeyEvent;
use tui_kit::layout::CellArea;
use tui_kit::widgets::grid::{Grid, GridCellCanvas, GridStyle};

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
/// (`LinkCandidate`). Adding more is one `impl NavItem` block -- no changes
/// to NavPicker itself.
pub trait NavItem: Clone {
    /// What `Enter` produces when this item is selected.
    type Output: Clone;

    /// Primary filterable string ("name", "label"). Lowercase-subsequence
    /// matched against the user's filter input.
    fn filter_text(&self) -> &str;

    /// Optional grouping section. Items with the same group sort together
    /// under a `── Group ──` header. `None` means "no header for this item"
    /// (used by ConnectionNavItem, which is flat).
    fn group(&self) -> Option<&str> {
        None
    }

    /// Secondary filterable tokens (element names contained inside the
    /// item, technology keywords on relationships, etc.). Used by
    /// `ViewNavItem` to surface "this view contains element X" matches.
    fn secondary_filter_tokens(&self) -> &[String] {
        &[]
    }

    /// Render this row into one Grid cell. The implementer owns the entire
    /// cell -- title, detail rows, key hint, thumbnail anchor (see
    /// `sink`/`NavRenderArtifact` below).
    fn render_into_canvas(
        &self,
        canvas: NavCellCanvas<'_, '_>,
        selected: bool,
        filter: &str,
        sink: &mut dyn FnMut(NavRenderArtifact),
    );

    /// Produce the outcome the spawning code receives when this item is
    /// Enter-selected.
    fn outcome(&self) -> Self::Output;
}

/// Side-channel for things `render_into_canvas` wants to surface to the
/// spawning code without putting them in the buffer (thumbnails, hover
/// hints).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum NavRenderArtifact {
    /// A cell area where the spawning code should later place an image
    /// thumbnail (kitty placement). Used by `ViewNavItem` to surface the
    /// thumbnail anchor for the existing thumbnail-rendering path in
    /// `TerminalSession::draw_picker`.
    Thumbnail { id: ThumbnailId, area: CellArea },
}

/// What the thumbnail belongs to. Today only views have thumbnails, but
/// we use an opaque newtype so Phase 5's LinkCandidate could grow one
/// without retouching this trait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThumbnailId(pub crate::ids::ViewId);

/// Thin wrapper exposing a [`GridCellCanvas`] with NavItem-friendly helpers.
///
/// The Grid render callback hands the cell renderer `&mut GridCellCanvas<'_>`.
/// `NavCellCanvas` borrows that exclusive reference so item implementations
/// see a NavPicker-shaped surface without coupling to Grid internals.
pub struct NavCellCanvas<'canvas, 'buf> {
    pub(crate) inner: &'canvas mut GridCellCanvas<'buf>,
}

impl<'canvas, 'buf> NavCellCanvas<'canvas, 'buf> {
    pub fn new(inner: &'canvas mut GridCellCanvas<'buf>) -> Self {
        Self { inner }
    }

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
    /// ConnectionPicker = no (it ignores `Char` keys).
    pub allows_filter: bool,
    /// Whether Tab toggles between "show only primary items" and "show all
    /// including legend/key items". ViewPicker = yes; ConnectionPicker = no.
    pub allows_secondary_toggle: bool,
    /// Label for the secondary-toggle state in the footer (e.g. "legends").
    /// Ignored if `allows_secondary_toggle` is false.
    pub secondary_toggle_label: &'static str,
}

/// Per-item flag deciding whether items are hidden in the default view.
///
/// `ViewNavItem` uses this to hide legend views by default; other items
/// pass-through.
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
        let selected_index = if items.is_empty() {
            0
        } else {
            initial_selection.min(items.len() - 1)
        };
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

    pub fn config(&self) -> &NavPickerConfig {
        &self.config
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    pub fn show_secondary(&self) -> bool {
        self.show_secondary
    }

    pub fn selected_index(&self) -> usize {
        self.selected_index
    }

    pub fn selected(&self) -> Option<T> {
        self.visible_items()
            .into_iter()
            .nth(self.selected_index)
            .cloned()
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
                    self.clamp_selection();
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
                // `ConnectionPicker` used Tab as a synonym for Down. Preserve
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
            self.selected_index = 0;
            return;
        }
        let count_i = count as i32;
        let next = (self.selected_index as i32 + delta).rem_euclid(count_i);
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
        let selected_index = self.selected_index.min(visible.len().saturating_sub(1));

        // The Grid callback holds an exclusive borrow of `canvas`. We move
        // `last_artifacts` out for the duration of the call so the closure
        // can borrow `&mut artifacts` without aliasing `self`, then restore.
        let mut artifacts = std::mem::take(&mut self.last_artifacts);

        Grid::new()
            .with_cell_rows(self.config.cell_rows)
            .with_min_cell_cols(self.config.min_cell_cols)
            .with_selected_index(Some(selected_index))
            .with_style(style)
            .render(body, buffer, &visible, |cell, canvas| {
                if canvas.width() < 4 {
                    return;
                }
                let nav_canvas = NavCellCanvas::new(canvas);
                cell.item
                    .render_into_canvas(nav_canvas, cell.selected, &filter, &mut |artifact| {
                        artifacts.push(artifact)
                    });
            });

        self.last_artifacts = artifacts;

        Ok(())
    }

    fn handle_event(
        &mut self,
        event: &KeyEvent,
    ) -> Result<ComponentOutcome<NavOutcome<T::Output>>> {
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
            _canvas: NavCellCanvas<'_, '_>,
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
        assert_eq!(p.filter(), "");
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
        assert_eq!(p.filter(), "");
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

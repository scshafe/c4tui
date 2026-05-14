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
/// `ConnectionNavItem`). Future Phase 5 work may add a third
/// (`LinkCandidate`). Adding more is one `impl NavItem` block -- no
/// changes to NavPicker itself.
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

    /// Whether this item is hidden in the default view of a NavPicker that
    /// supports a secondary toggle.
    ///
    /// `ViewNavItem` returns `true` for legend/key views; other items
    /// pass-through. NavPicker respects this only when its mode enables
    /// secondary toggling -- a `Flat` picker ignores it entirely.
    fn is_secondary(&self) -> bool {
        false
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
/// we use an opaque newtype so a future Phase 5 `LinkCandidate` could grow
/// one without retouching this trait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThumbnailId(crate::ids::ViewId);

impl ThumbnailId {
    /// Construct a thumbnail id for a view. Today every thumbnail belongs
    /// to a view; new sources should add their own constructor rather than
    /// exposing the inner field.
    pub fn view(view_id: crate::ids::ViewId) -> Self {
        Self(view_id)
    }

    pub fn view_id(self) -> crate::ids::ViewId {
        self.0
    }
}

/// Thin wrapper exposing a [`GridCellCanvas`] with NavItem-friendly helpers.
///
/// The Grid render callback hands the cell renderer `&mut GridCellCanvas<'_>`.
/// `NavCellCanvas` borrows that exclusive reference so item implementations
/// see a NavPicker-shaped surface without coupling to Grid internals.
///
/// The two lifetimes are: `'canvas` is the lifetime of the exclusive borrow
/// of the underlying `GridCellCanvas`; `'buf` is the lifetime of the buffer
/// that canvas writes into. Both come from Grid's render callback and
/// outlive a single `render_into_canvas` invocation only.
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

/// Behavioral mode for a NavPicker.
///
/// Replaces three implicit-invariant booleans with one enum match. A
/// `Filterable` picker accepts free-form filter typing and (optionally)
/// toggles legend/secondary items with Tab. A `Flat` picker ignores `Char`,
/// `Back`, and `Tab` keys -- it is pure arrow-and-Enter navigation over a
/// fixed list, used today by ConnectionPicker.
#[derive(Debug, Clone)]
pub enum NavPickerMode {
    Filterable {
        /// When true, `Tab` toggles `show_secondary`. When false, `Tab` is a
        /// no-op (Filterable pickers never use it as a Down synonym; the
        /// filter input owns Tab semantics).
        allows_secondary_toggle: bool,
        /// Footer label for the secondary-toggle state. Ignored when
        /// `allows_secondary_toggle` is false.
        secondary_label: &'static str,
    },
    Flat,
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
    /// Behavioral mode. See [`NavPickerMode`].
    pub mode: NavPickerMode,
}

#[derive(Debug)]
pub struct NavPicker<T: NavItem> {
    config: NavPickerConfig,
    items: Vec<T>,
    /// Indices into `items` for the currently visible rows. Recomputed by
    /// `recompute_visible_indices` whenever the filter, secondary-toggle
    /// state, or items list changes. `selected_index` is an index into this
    /// cache (not into `items`).
    visible_indices: Vec<usize>,
    filter: String,
    show_secondary: bool,
    selected_index: usize,
    dirty: DirtyState,
    last_artifacts: Vec<NavRenderArtifact>,
}

impl<T: NavItem> NavPicker<T> {
    pub fn new(config: NavPickerConfig, items: Vec<T>, initial_selection: usize) -> Self {
        let mut picker = Self {
            config,
            items,
            visible_indices: Vec::new(),
            filter: String::new(),
            show_secondary: false,
            selected_index: 0,
            dirty: DirtyState::paint(DirtyReason::Explicit),
            last_artifacts: Vec::new(),
        };
        picker.recompute_visible_indices();
        if !picker.visible_indices.is_empty() {
            picker.selected_index = initial_selection.min(picker.visible_indices.len() - 1);
        }
        picker
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

    /// Index into `self.items` (not into the visible-indices cache) of the
    /// currently selected item, or `None` if no items are visible.
    pub fn selected_index_in_items(&self) -> Option<usize> {
        self.visible_indices.get(self.selected_index).copied()
    }

    /// Borrow the currently selected item without cloning. `None` when the
    /// visible set is empty (e.g. no items, or filter excludes all).
    pub fn selected(&self) -> Option<&T> {
        self.items.get(self.selected_index_in_items()?)
    }

    pub fn last_artifacts(&self) -> &[NavRenderArtifact] {
        &self.last_artifacts
    }

    /// Borrow the visible items in display order. Cheap: walks the cached
    /// index list rather than re-filtering.
    pub fn visible_items(&self) -> impl Iterator<Item = &T> {
        self.visible_indices
            .iter()
            .filter_map(|&idx| self.items.get(idx))
    }

    pub fn visible_len(&self) -> usize {
        self.visible_indices.len()
    }

    /// Recompute `visible_indices` from the current filter and secondary
    /// state. Callers must invoke this whenever `filter`, `show_secondary`,
    /// or `items` change.
    fn recompute_visible_indices(&mut self) {
        let respect_secondary = matches!(
            self.config.mode,
            NavPickerMode::Filterable {
                allows_secondary_toggle: true,
                ..
            }
        );
        self.visible_indices.clear();
        for (idx, item) in self.items.iter().enumerate() {
            if respect_secondary && !self.show_secondary && item.is_secondary() {
                continue;
            }
            if !matches_filter(&self.filter, item) {
                continue;
            }
            self.visible_indices.push(idx);
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> NavOutcome<T::Output> {
        let allows_filter = matches!(self.config.mode, NavPickerMode::Filterable { .. });
        let allows_secondary_toggle = matches!(
            self.config.mode,
            NavPickerMode::Filterable {
                allows_secondary_toggle: true,
                ..
            }
        );

        match key {
            KeyEvent::Esc => {
                if self.filter.is_empty() {
                    NavOutcome::Cancel
                } else {
                    self.filter.clear();
                    self.recompute_visible_indices();
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
            KeyEvent::Tab if allows_secondary_toggle => {
                self.show_secondary = !self.show_secondary;
                self.recompute_visible_indices();
                self.clamp_selection();
                self.dirty.mark_paint(DirtyReason::Input);
                self.dirty.mark_image_placement(DirtyReason::Input);
                NavOutcome::Continue
            }
            KeyEvent::Back if allows_filter => {
                if self.filter.pop().is_some() {
                    self.recompute_visible_indices();
                    self.clamp_selection();
                    self.dirty.mark_paint(DirtyReason::Input);
                    self.dirty.mark_image_placement(DirtyReason::Input);
                }
                NavOutcome::Continue
            }
            KeyEvent::Char(c) if allows_filter => {
                self.filter.push(c);
                self.recompute_visible_indices();
                self.clamp_selection();
                self.dirty.mark_paint(DirtyReason::Input);
                self.dirty.mark_image_placement(DirtyReason::Input);
                NavOutcome::Continue
            }
            _ => NavOutcome::Continue,
        }
    }

    fn move_selection(&mut self, delta: i32) {
        let count = self.visible_indices.len();
        if count == 0 {
            self.selected_index = 0;
            return;
        }
        let count_i = count as i32;
        let next = (self.selected_index as i32 + delta).rem_euclid(count_i);
        self.selected_index = next as usize;
    }

    fn clamp_selection(&mut self) {
        let count = self.visible_indices.len();
        if count == 0 {
            self.selected_index = 0;
            return;
        }
        if self.selected_index >= count {
            self.selected_index = count - 1;
        }
    }
}

impl<T: NavItem> BufferComponent for NavPicker<T> {
    type Event = KeyEvent;
    type Message = NavOutcome<T::Output>;

    fn id(&self) -> &ComponentId {
        &self.config.id
    }

    fn render_buffer(&mut self, area: Rect, buffer: &mut Buffer) -> Result<()> {
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

        if self.visible_indices.is_empty() {
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
        let selected_index = self
            .selected_index
            .min(self.visible_indices.len().saturating_sub(1));

        let visible: Vec<T> = self
            .visible_indices
            .iter()
            .filter_map(|&idx| self.items.get(idx).cloned())
            .collect();

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

/// Truncate `text` to at most `max` chars, appending `…` when truncation
/// occurs. Shared between NavPicker chrome and NavItem render impls.
pub(crate) fn truncate(text: &str, max: usize) -> String {
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

        fn is_secondary(&self) -> bool {
            self.secondary
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

    fn filterable_config(allows_secondary: bool) -> NavPickerConfig {
        NavPickerConfig {
            id: ComponentId::new("test"),
            title: " test ".into(),
            footer_hint: " hint ".into(),
            default_header: "hdr".into(),
            min_cell_cols: 10,
            cell_rows: 2,
            mode: NavPickerMode::Filterable {
                allows_secondary_toggle: allows_secondary,
                secondary_label: "secondary",
            },
        }
    }

    fn flat_config() -> NavPickerConfig {
        NavPickerConfig {
            id: ComponentId::new("test"),
            title: " test ".into(),
            footer_hint: " hint ".into(),
            default_header: "hdr".into(),
            min_cell_cols: 10,
            cell_rows: 2,
            mode: NavPickerMode::Flat,
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
            filterable_config(false),
            items(&[("apple", 1, false), ("banana", 2, false)]),
            0,
        );
        assert_eq!(p.handle_key(KeyEvent::Enter), NavOutcome::Select(1));
    }

    #[test]
    fn esc_clears_filter_then_cancels() {
        let mut p = NavPicker::new(filterable_config(false), items(&[("apple", 1, false)]), 0);
        p.handle_key(KeyEvent::Char('a'));
        assert!(matches!(p.handle_key(KeyEvent::Esc), NavOutcome::Continue));
        assert_eq!(p.filter(), "");
        assert_eq!(p.handle_key(KeyEvent::Esc), NavOutcome::Cancel);
    }

    #[test]
    fn flat_picker_ignores_char_keys() {
        let mut p = NavPicker::new(
            flat_config(),
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
    fn flat_picker_tab_is_noop() {
        // Per project's rip-and-replace policy: NavPicker no longer treats
        // Tab as a Down synonym for Flat pickers. Down stays bound to Down;
        // Tab in a Flat picker is a no-op. Migrating consumers update their
        // tests / keymaps to bind Down explicitly.
        let mut p = NavPicker::new(flat_config(), items(&[("a", 1, false), ("b", 2, false)]), 0);
        p.handle_key(KeyEvent::Tab);
        assert_eq!(p.handle_key(KeyEvent::Enter), NavOutcome::Select(1));
    }

    #[test]
    fn tab_in_secondary_toggle_picker_toggles_visibility() {
        let mut p = NavPicker::new(
            filterable_config(true),
            items(&[("primary", 1, false), ("legend", 2, true)]),
            0,
        );
        assert_eq!(p.visible_len(), 1);
        p.handle_key(KeyEvent::Tab);
        assert_eq!(p.visible_len(), 2);
    }

    #[test]
    fn arrows_wrap_through_visible() {
        let mut p = NavPicker::new(flat_config(), items(&[("a", 1, false), ("b", 2, false)]), 0);
        p.handle_key(KeyEvent::Down);
        p.handle_key(KeyEvent::Down);
        assert_eq!(p.handle_key(KeyEvent::Enter), NavOutcome::Select(1));
    }

    #[test]
    fn filter_keeps_selection_inside_visible_set() {
        let mut p = NavPicker::new(
            filterable_config(false),
            items(&[("apple", 1, false), ("banana", 2, false)]),
            1,
        );
        p.handle_key(KeyEvent::Char('a'));
        // 'a' matches both; selection stays clamped.
        assert_eq!(p.handle_key(KeyEvent::Enter), NavOutcome::Select(2));
        p.handle_key(KeyEvent::Char('p')); // only "apple" matches
        assert_eq!(p.handle_key(KeyEvent::Enter), NavOutcome::Select(1));
    }

    #[test]
    fn empty_items_construct_cleanly() {
        let p: NavPicker<FakeItem> = NavPicker::new(filterable_config(false), Vec::new(), 0);
        assert_eq!(p.selected_index(), 0);
        assert_eq!(p.visible_len(), 0);
        assert!(p.selected().is_none());
    }

    #[test]
    fn enter_on_empty_items_is_continue_not_panic() {
        let mut p: NavPicker<FakeItem> = NavPicker::new(filterable_config(false), Vec::new(), 0);
        assert_eq!(p.handle_key(KeyEvent::Enter), NavOutcome::Continue);
    }

    #[test]
    fn enter_on_filtered_empty_visible_is_continue_not_panic() {
        let mut p = NavPicker::new(filterable_config(false), items(&[("apple", 1, false)]), 0);
        p.handle_key(KeyEvent::Char('z')); // matches nothing
        assert_eq!(p.visible_len(), 0);
        assert!(p.selected().is_none());
        assert_eq!(p.handle_key(KeyEvent::Enter), NavOutcome::Continue);
    }

    #[test]
    fn tab_on_flat_empty_picker_is_noop() {
        let mut p: NavPicker<FakeItem> = NavPicker::new(flat_config(), Vec::new(), 0);
        assert_eq!(p.handle_key(KeyEvent::Tab), NavOutcome::Continue);
        assert_eq!(p.selected_index(), 0);
    }

    #[test]
    fn backspace_on_empty_filter_is_noop() {
        let mut p = NavPicker::new(
            filterable_config(false),
            items(&[("apple", 1, false), ("banana", 2, false)]),
            0,
        );
        assert_eq!(p.filter(), "");
        assert_eq!(p.handle_key(KeyEvent::Back), NavOutcome::Continue);
        assert_eq!(p.filter(), "");
        assert_eq!(p.visible_len(), 2);
        // Selection still works.
        assert_eq!(p.handle_key(KeyEvent::Enter), NavOutcome::Select(1));
    }
}

#![allow(dead_code)]

use crate::ids::ViewId;
use crate::workspace::{ViewInfo, ViewKind, WorkspaceModel};
use anyhow::Result;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};
use tui_kit::component::{BufferComponent, ComponentId, ComponentOutcome, DirtyReason, DirtyState};
use tui_kit::input::Key;
use tui_kit::layout::CellArea;

#[derive(Debug, Clone)]
pub struct PickerItem {
    pub view_id: ViewId,
    pub kind: ViewKind,
    pub name: String,
    pub key: String,
    pub description: Option<String>,
    pub element_names: Vec<String>,
    pub matched_via_element: Option<String>,
}

#[derive(Debug)]
pub struct ViewPicker {
    id: ComponentId,
    items: Vec<PickerItem>,
    filter: String,
    show_keys: bool,
    selected_view: ViewId,
    dirty: DirtyState,
    last_thumbnails: Vec<ThumbnailCellArea>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThumbnailCellArea {
    pub view_id: ViewId,
    pub area: CellArea,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PickerOutcome {
    Continue,
    Select(ViewId),
    Cancel,
}

#[derive(Debug, Clone)]
pub struct RenderedPicker {
    pub header: String,
    pub footer: String,
    pub lines: Vec<PickerLine>,
    pub selected_visible_row: Option<usize>,
}

#[derive(Debug, Clone)]
pub enum PickerLine {
    Header(String),
    Item {
        view_id: ViewId,
        marker: &'static str,
        primary: String,
        detail: Option<String>,
        selected: bool,
    },
    Empty(String),
}

const TILE_MIN_COLS: u16 = 22;
const TILE_ROWS: u16 = 8;
const THUMB_ROWS: u16 = 5;

impl ViewPicker {
    pub fn new(views: &[ViewInfo], model: &WorkspaceModel, current: ViewId) -> Self {
        let items = views
            .iter()
            .enumerate()
            .map(|(idx, info)| {
                let element_names = info
                    .element_ids
                    .iter()
                    .filter_map(|id| model.elements.get(id).map(|e| e.name.clone()))
                    .collect();
                PickerItem {
                    view_id: ViewId::new(idx),
                    kind: info.kind,
                    name: info.name.clone(),
                    key: info.key.clone(),
                    description: info.description.clone(),
                    element_names,
                    matched_via_element: None,
                }
            })
            .collect();
        Self {
            id: ComponentId::new("c4tui-picker"),
            items,
            filter: String::new(),
            show_keys: false,
            selected_view: current,
            dirty: DirtyState::paint(DirtyReason::Explicit),
            last_thumbnails: Vec::new(),
        }
    }

    pub fn handle_key(&mut self, key: Key) -> PickerOutcome {
        match key {
            Key::Esc => {
                if self.filter.is_empty() {
                    PickerOutcome::Cancel
                } else {
                    self.filter.clear();
                    self.dirty.mark_paint(DirtyReason::Input);
                    self.dirty.mark_image_placement(DirtyReason::Input);
                    PickerOutcome::Continue
                }
            }
            Key::CtrlC => PickerOutcome::Cancel,
            Key::Enter => {
                let visible = self.visible_view_ids();
                if let Some(target) = visible
                    .iter()
                    .find(|id| **id == self.selected_view)
                    .copied()
                {
                    PickerOutcome::Select(target)
                } else if let Some(first) = visible.first().copied() {
                    PickerOutcome::Select(first)
                } else {
                    PickerOutcome::Continue
                }
            }
            Key::Up => {
                self.move_selection(-1);
                self.dirty.mark_paint(DirtyReason::Input);
                PickerOutcome::Continue
            }
            Key::Down => {
                self.move_selection(1);
                self.dirty.mark_paint(DirtyReason::Input);
                PickerOutcome::Continue
            }
            Key::Tab => {
                self.show_keys = !self.show_keys;
                self.clamp_selection();
                self.dirty.mark_paint(DirtyReason::Input);
                self.dirty.mark_image_placement(DirtyReason::Input);
                PickerOutcome::Continue
            }
            Key::Back => {
                self.filter.pop();
                self.clamp_selection();
                self.dirty.mark_paint(DirtyReason::Input);
                self.dirty.mark_image_placement(DirtyReason::Input);
                PickerOutcome::Continue
            }
            Key::Char(c) => {
                self.filter.push(c);
                self.clamp_selection();
                self.dirty.mark_paint(DirtyReason::Input);
                self.dirty.mark_image_placement(DirtyReason::Input);
                PickerOutcome::Continue
            }
            _ => PickerOutcome::Continue,
        }
    }

    /// Build the layout-level rendering of the picker. Kept public for tests
    /// and introspection; production rendering goes through [`render_buffer`].
    pub fn render(&self) -> RenderedPicker {
        let visible = self.visible_items();
        let groups = group_items_by_kind(&visible);
        let selected = self.effective_selection(&visible);
        let mut lines = Vec::new();
        let mut selected_visible_row: Option<usize> = None;

        if visible.is_empty() {
            lines.push(PickerLine::Empty(if self.items.is_empty() {
                "No views available".to_owned()
            } else {
                format!("No views match '{}'", self.filter)
            }));
        } else {
            for (kind, group_items) in groups {
                lines.push(PickerLine::Header(kind_heading(kind)));
                for item in group_items {
                    let is_selected = Some(item.view_id) == selected;
                    let marker = if is_selected { ">" } else { " " };
                    let primary = format!("{}  ({})", item.name, item.key);
                    let element_match = matched_element_for(&self.filter, item);
                    let detail = if let Some(elem) = element_match {
                        Some(format!("contains {}", elem))
                    } else {
                        item.description.clone()
                    };
                    if is_selected {
                        selected_visible_row = Some(lines.len());
                    }
                    lines.push(PickerLine::Item {
                        view_id: item.view_id,
                        marker,
                        primary,
                        detail,
                        selected: is_selected,
                    });
                }
            }
        }

        let header = if self.filter.is_empty() {
            "Pick a view  —  type to filter, Enter to select, Esc to cancel, Tab to toggle key views".to_owned()
        } else {
            format!("Filter: {}", self.filter)
        };
        let footer = format!(
            "showing {}/{} views{}{}",
            visible.len(),
            self.items.len(),
            if self.show_keys {
                " (incl. legends)"
            } else {
                ""
            },
            if !self.filter.is_empty() {
                format!("  matching '{}'", self.filter)
            } else {
                String::new()
            }
        );

        RenderedPicker {
            header,
            footer,
            lines,
            selected_visible_row,
        }
    }

    pub fn selected_view_id(&self) -> ViewId {
        self.selected_view
    }

    /// Last-render thumbnail placements.
    /// Re-populated on every [`render_buffer`] call; the slice is a snapshot
    /// of the most recent render and remains valid until the next one.
    pub fn thumbnails(&self) -> &[ThumbnailCellArea] {
        &self.last_thumbnails
    }

    fn move_selection(&mut self, delta: i32) {
        let visible = self.visible_view_ids();
        if visible.is_empty() {
            return;
        }
        let current_idx = visible
            .iter()
            .position(|id| *id == self.selected_view)
            .map(|i| i as i32)
            .unwrap_or(0);
        let next = (current_idx + delta).rem_euclid(visible.len() as i32) as usize;
        self.selected_view = visible[next];
    }

    fn clamp_selection(&mut self) {
        let visible = self.visible_view_ids();
        if visible.contains(&self.selected_view) {
            return;
        }
        if let Some(first) = visible.first().copied() {
            self.selected_view = first;
        }
    }

    fn effective_selection(&self, visible: &[&PickerItem]) -> Option<ViewId> {
        if visible
            .iter()
            .any(|item| item.view_id == self.selected_view)
        {
            Some(self.selected_view)
        } else {
            visible.first().map(|item| item.view_id)
        }
    }

    fn visible_items(&self) -> Vec<&PickerItem> {
        self.items
            .iter()
            .filter(|item| self.show_keys || !item.kind.is_legend())
            .filter(|item| matches_filter(&self.filter, item))
            .collect()
    }

    fn visible_view_ids(&self) -> Vec<ViewId> {
        self.visible_items()
            .into_iter()
            .map(|item| item.view_id)
            .collect()
    }
}

impl BufferComponent for ViewPicker {
    type Event = Key;
    type Message = PickerOutcome;

    fn id(&self) -> &ComponentId {
        &self.id
    }

    fn render_buffer(&mut self, area: Rect, buffer: &mut Buffer) -> Result<()> {
        let rendered = self.render();
        self.last_thumbnails.clear();
        let visible_refs = self.visible_items();
        let selected = self.effective_selection(&visible_refs);
        let visible: Vec<PickerItem> = visible_refs.into_iter().cloned().collect();

        let block = Block::default()
            .borders(Borders::ALL)
            .title(" View Picker ")
            .title_bottom(" type → filter | Tab → legends | Enter → select | Esc → cancel ");
        let inner = block.inner(area);
        block.render(area, buffer);
        if inner.height < 3 || inner.width < 8 {
            return Ok(());
        }

        let header_avail = inner.width.saturating_sub(1) as usize;
        Paragraph::new(truncate(&rendered.header, header_avail)).render(
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

        self.render_grid(body, buffer, &visible, selected);
        Ok(())
    }

    fn handle_event(&mut self, event: &Key) -> Result<ComponentOutcome<PickerOutcome>> {
        let outcome = self.handle_key(*event);
        Ok(match outcome {
            PickerOutcome::Continue => ComponentOutcome::Handled,
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

impl ViewPicker {
    fn render_grid(
        &mut self,
        body: Rect,
        buffer: &mut Buffer,
        visible: &[PickerItem],
        selected: Option<ViewId>,
    ) {
        if visible.is_empty() {
            let text = if self.items.is_empty() {
                "No views available".to_owned()
            } else {
                format!("No views match '{}'", self.filter)
            };
            buffer.set_string(body.x, body.y, text, Style::default());
            return;
        }

        let columns = grid_columns(body.width, visible.len());
        let tile_cols = body.width / columns;
        let selected_idx = visible
            .iter()
            .position(|item| Some(item.view_id) == selected)
            .unwrap_or(0);
        let selected_row = selected_idx as u16 / columns;
        let total_rows = ((visible.len() as u16).saturating_add(columns - 1)) / columns;
        let total_virtual = total_rows.saturating_mul(TILE_ROWS);
        let selected_top = selected_row.saturating_mul(TILE_ROWS);
        let selected_bottom = selected_top.saturating_add(TILE_ROWS);
        let scroll = if total_virtual <= body.height {
            0
        } else if selected_bottom > body.height {
            selected_bottom.saturating_sub(body.height)
        } else {
            0
        };

        for (idx, item) in visible.iter().enumerate() {
            let grid_col = idx as u16 % columns;
            let grid_row = idx as u16 / columns;
            let virtual_y = grid_row.saturating_mul(TILE_ROWS);
            if virtual_y + TILE_ROWS <= scroll || virtual_y >= scroll + body.height {
                continue;
            }

            let tile_x = body.x.saturating_add(grid_col.saturating_mul(tile_cols));
            let tile_y = body.y.saturating_add(virtual_y.saturating_sub(scroll));
            if tile_y >= body.y + body.height {
                continue;
            }

            let tile_width = if grid_col + 1 == columns {
                body.width
                    .saturating_sub(tile_cols.saturating_mul(columns.saturating_sub(1)))
            } else {
                tile_cols
            };
            if tile_width < 4 {
                continue;
            }

            let is_selected = Some(item.view_id) == selected;
            let style = if is_selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            let marker = if is_selected { ">" } else { " " };
            let title = format!("{marker} {}", item.name);
            buffer.set_string(
                tile_x,
                tile_y,
                truncate(&title, tile_width.saturating_sub(1) as usize),
                style,
            );

            let image_y = tile_y.saturating_add(1);
            if image_y < body.y + body.height {
                let image_rows = THUMB_ROWS.min((body.y + body.height).saturating_sub(image_y));
                let image_cols = tile_width.saturating_sub(2);
                if image_rows > 0 && image_cols > 0 {
                    let image_x = tile_x.saturating_add(1);
                    self.last_thumbnails.push(ThumbnailCellArea {
                        view_id: item.view_id,
                        area: CellArea::new(image_x, image_y, image_cols, image_rows),
                    });
                }
            }

            let key_row = tile_y.saturating_add(THUMB_ROWS).saturating_add(1);
            if key_row < body.y + body.height {
                buffer.set_string(
                    tile_x,
                    key_row,
                    truncate(&item.key, tile_width.saturating_sub(1) as usize),
                    Style::default().add_modifier(Modifier::DIM),
                );
            }
        }

        if scroll > 0 {
            buffer.set_string(
                body.x + body.width.saturating_sub(1),
                body.y,
                "▲",
                Style::default(),
            );
        }
        if scroll + body.height < total_virtual {
            buffer.set_string(
                body.x + body.width.saturating_sub(1),
                body.y + body.height.saturating_sub(1),
                "▼",
                Style::default(),
            );
        }
    }
}

fn grid_columns(width: u16, items: usize) -> u16 {
    if items == 0 {
        return 1;
    }
    (width / TILE_MIN_COLS).max(1).min(items as u16)
}

fn matches_filter(filter: &str, item: &PickerItem) -> bool {
    if filter.is_empty() {
        return true;
    }
    let needle = filter.to_ascii_lowercase();
    let primary = format!(
        "{} {} {} {}",
        item.name,
        item.key,
        item.kind.label(),
        item.description.as_deref().unwrap_or("")
    )
    .to_ascii_lowercase();
    if subsequence_match(&needle, &primary) {
        return true;
    }
    item.element_names
        .iter()
        .any(|name| subsequence_match(&needle, &name.to_ascii_lowercase()))
}

pub fn matched_element_for<'a>(filter: &str, item: &'a PickerItem) -> Option<&'a str> {
    if filter.is_empty() {
        return None;
    }
    let needle = filter.to_ascii_lowercase();
    item.element_names
        .iter()
        .find(|name| subsequence_match(&needle, &name.to_ascii_lowercase()))
        .map(String::as_str)
}

fn subsequence_match(needle: &str, haystack: &str) -> bool {
    let mut h_iter = haystack.chars();
    'outer: for nc in needle.chars() {
        for hc in h_iter.by_ref() {
            if hc == nc {
                continue 'outer;
            }
        }
        return false;
    }
    true
}

fn group_items_by_kind<'a>(items: &[&'a PickerItem]) -> Vec<(ViewKind, Vec<&'a PickerItem>)> {
    let order = [
        ViewKind::SystemLandscape,
        ViewKind::SystemContext,
        ViewKind::Container,
        ViewKind::Component,
        ViewKind::Dynamic,
        ViewKind::Deployment,
        ViewKind::Filtered,
        ViewKind::Custom,
        ViewKind::Image,
        ViewKind::Unknown,
        ViewKind::Key,
    ];
    let mut groups: Vec<(ViewKind, Vec<&PickerItem>)> = Vec::new();
    for kind in order {
        let group: Vec<&PickerItem> = items.iter().copied().filter(|i| i.kind == kind).collect();
        if !group.is_empty() {
            groups.push((kind, group));
        }
    }
    groups
}

fn kind_heading(kind: ViewKind) -> String {
    let label = match kind {
        ViewKind::SystemLandscape => "System Landscape",
        ViewKind::SystemContext => "System Context",
        ViewKind::Container => "Containers",
        ViewKind::Component => "Components",
        ViewKind::Dynamic => "Dynamic",
        ViewKind::Deployment => "Deployment",
        ViewKind::Filtered => "Filtered",
        ViewKind::Custom => "Custom",
        ViewKind::Image => "Image",
        ViewKind::Key => "Legends",
        ViewKind::Unknown => "Other",
    };
    format!("── {label} ──")
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
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;

    fn view(name: &str, kind: ViewKind, key: &str) -> ViewInfo {
        ViewInfo {
            key: key.to_owned(),
            name: name.to_owned(),
            kind,
            description: None,
            svg_path: PathBuf::from(format!("{key}.svg")),
            element_ids: HashSet::new(),
            child_view_by_element_id: HashMap::new(),
            primary_view_key: None,
            key_view_key: None,
        }
    }

    fn views() -> Vec<ViewInfo> {
        vec![
            view("System Context", ViewKind::SystemContext, "system"),
            view("Containers", ViewKind::Container, "containers"),
            view("Containers (key)", ViewKind::Key, "Containers-key"),
            view(
                "Agent Coordination",
                ViewKind::Container,
                "AgentCoordination",
            ),
            view(
                "Agent Coordination (key)",
                ViewKind::Key,
                "AgentCoordination-key",
            ),
        ]
    }

    #[test]
    fn hides_key_views_by_default() {
        let picker = ViewPicker::new(&views(), &WorkspaceModel::default(), ViewId::first());
        let rendered = picker.render();
        let item_count = rendered
            .lines
            .iter()
            .filter(|l| matches!(l, PickerLine::Item { .. }))
            .count();
        assert_eq!(item_count, 3);
    }

    #[test]
    fn tab_toggles_key_visibility() {
        let mut picker = ViewPicker::new(&views(), &WorkspaceModel::default(), ViewId::first());
        picker.handle_key(Key::Tab);
        let rendered = picker.render();
        let item_count = rendered
            .lines
            .iter()
            .filter(|l| matches!(l, PickerLine::Item { .. }))
            .count();
        assert_eq!(item_count, 5);
    }

    #[test]
    fn typing_filters_by_subsequence() {
        let mut picker = ViewPicker::new(&views(), &WorkspaceModel::default(), ViewId::first());
        for c in "agnt".chars() {
            picker.handle_key(Key::Char(c));
        }
        let visible = picker.visible_items();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].name, "Agent Coordination");
    }

    #[test]
    fn esc_clears_filter_then_cancels() {
        let mut picker = ViewPicker::new(&views(), &WorkspaceModel::default(), ViewId::first());
        picker.handle_key(Key::Char('a'));
        let outcome = picker.handle_key(Key::Esc);
        assert_eq!(outcome, PickerOutcome::Continue);
        assert!(picker.filter.is_empty());
        let outcome = picker.handle_key(Key::Esc);
        assert_eq!(outcome, PickerOutcome::Cancel);
    }

    #[test]
    fn enter_selects_first_visible_when_current_filtered_out() {
        let mut picker = ViewPicker::new(&views(), &WorkspaceModel::default(), ViewId::first());
        for c in "agen".chars() {
            picker.handle_key(Key::Char(c));
        }
        let outcome = picker.handle_key(Key::Enter);
        match outcome {
            PickerOutcome::Select(id) => assert_eq!(id, ViewId::new(3)),
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn arrows_cycle_through_visible_items() {
        let mut picker = ViewPicker::new(&views(), &WorkspaceModel::default(), ViewId::first());
        picker.handle_key(Key::Down);
        assert_eq!(picker.selected_view_id(), ViewId::new(1));
        picker.handle_key(Key::Down);
        assert_eq!(picker.selected_view_id(), ViewId::new(3));
        picker.handle_key(Key::Up);
        assert_eq!(picker.selected_view_id(), ViewId::new(1));
    }

    #[test]
    fn render_buffer_marks_thumbnail_positions() {
        let mut picker = ViewPicker::new(&views(), &WorkspaceModel::default(), ViewId::first());
        let mut buffer = Buffer::empty(Rect::new(0, 0, 80, 20));
        picker
            .render_buffer(Rect::new(0, 0, 80, 20), &mut buffer)
            .unwrap();
        let thumbs = picker.thumbnails();
        assert!(!thumbs.is_empty(), "thumbnails recorded for visible items");
        for thumb in thumbs {
            assert!(thumb.area.origin.col > 0);
            assert!(thumb.area.size.cols > 0);
            assert!(thumb.area.size.rows > 0);
        }
    }
}

use crate::ids::ElementId;
#[cfg(test)]
use crate::ids::ViewId;
use crate::view::{ConnectionDirection, ConnectionNavigationCandidate, ViewStore};
use anyhow::Result;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};
use tui_kit::component::{BufferComponent, ComponentId, ComponentOutcome, DirtyReason, DirtyState};
use tui_kit::input::Key;
use tui_kit::widgets::grid::{Grid, GridStyle};

#[derive(Debug)]
pub struct ConnectionPicker {
    id: ComponentId,
    source_element_name: String,
    items: Vec<ConnectionPickerItem>,
    selected_index: usize,
    dirty: DirtyState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionPickerItem {
    pub candidate: ConnectionNavigationCandidate,
    pub direction: ConnectionDirection,
    pub connected_element_name: String,
    pub relationship_detail: String,
    pub target_view: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionPickerOutcome {
    Continue,
    Select(ConnectionNavigationCandidate),
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedConnectionPicker {
    pub header: String,
    pub footer: String,
    pub items: Vec<RenderedConnectionPickerItem>,
    pub selected_index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedConnectionPickerItem {
    pub marker: &'static str,
    pub direction: &'static str,
    pub connected_element_name: String,
    pub relationship_detail: String,
    pub target_view: String,
    pub selected: bool,
}

const TILE_MIN_COLS: u16 = 34;
const TILE_ROWS: u16 = 5;

impl ConnectionPicker {
    pub fn new(
        source_element_id: &ElementId,
        candidates: Vec<ConnectionNavigationCandidate>,
        store: &ViewStore,
    ) -> Self {
        let source_element_name = store
            .model
            .elements
            .get(source_element_id)
            .map(|element| element.name.clone())
            .unwrap_or_else(|| source_element_id.to_string());
        let items = candidates
            .into_iter()
            .map(|candidate| {
                let relationship = store.model.relationships.get(&candidate.relationship_id);
                let connected_element_name = store
                    .model
                    .elements
                    .get(&candidate.connected_element_id)
                    .map(|element| element.name.clone())
                    .unwrap_or_else(|| candidate.connected_element_id.to_string());
                let relationship_detail = match (
                    relationship.and_then(|rel| rel.description.as_deref()),
                    relationship.and_then(|rel| rel.technology.as_deref()),
                ) {
                    (Some(description), Some(technology)) => {
                        format!("{description} ({technology})")
                    }
                    (Some(description), None) => description.to_owned(),
                    (None, Some(technology)) => technology.to_owned(),
                    (None, None) => "relationship".to_owned(),
                };
                let target_view = store.view(candidate.view_id).name.clone();
                ConnectionPickerItem {
                    direction: candidate.direction,
                    candidate,
                    connected_element_name,
                    relationship_detail,
                    target_view,
                }
            })
            .collect();
        Self {
            id: ComponentId::new("c4tui-connection-picker"),
            source_element_name,
            items,
            selected_index: 0,
            dirty: DirtyState::paint(DirtyReason::Explicit),
        }
    }

    pub fn handle_key(&mut self, key: Key) -> ConnectionPickerOutcome {
        match key {
            Key::Esc | Key::CtrlC => ConnectionPickerOutcome::Cancel,
            Key::Enter => self
                .selected_candidate()
                .cloned()
                .map(ConnectionPickerOutcome::Select)
                .unwrap_or(ConnectionPickerOutcome::Continue),
            Key::Up => {
                self.move_selection(-1);
                self.dirty.mark_paint(DirtyReason::Input);
                ConnectionPickerOutcome::Continue
            }
            Key::Down | Key::Tab => {
                self.move_selection(1);
                self.dirty.mark_paint(DirtyReason::Input);
                ConnectionPickerOutcome::Continue
            }
            _ => ConnectionPickerOutcome::Continue,
        }
    }

    pub fn render(&self) -> RenderedConnectionPicker {
        let items = self
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let selected = index == self.selected_index;
                RenderedConnectionPickerItem {
                    marker: if selected { ">" } else { " " },
                    direction: direction_label(item.direction),
                    connected_element_name: item.connected_element_name.clone(),
                    relationship_detail: item.relationship_detail.clone(),
                    target_view: item.target_view.clone(),
                    selected,
                }
            })
            .collect::<Vec<_>>();
        RenderedConnectionPicker {
            header: format!(
                "Connections for {}  -  Enter to navigate, Esc to cancel",
                self.source_element_name
            ),
            footer: format!(
                "showing {} navigable {}",
                self.items.len(),
                plural(self.items.len(), "connection", "connections")
            ),
            selected_index: (!self.items.is_empty()).then_some(self.selected_index),
            items,
        }
    }

    pub fn selected_candidate(&self) -> Option<&ConnectionNavigationCandidate> {
        self.items
            .get(self.selected_index)
            .map(|item| &item.candidate)
    }

    #[cfg(test)]
    pub fn selected_view_id(&self) -> Option<ViewId> {
        self.selected_candidate().map(|candidate| candidate.view_id)
    }

    fn move_selection(&mut self, delta: i32) {
        if self.items.is_empty() {
            return;
        }
        let next = (self.selected_index as i32 + delta).rem_euclid(self.items.len() as i32);
        self.selected_index = next as usize;
    }
}

impl BufferComponent for ConnectionPicker {
    type Event = Key;
    type Message = ConnectionPickerOutcome;

    fn id(&self) -> &ComponentId {
        &self.id
    }

    fn render_buffer(&mut self, area: Rect, buffer: &mut Buffer) -> Result<()> {
        let rendered = self.render();
        Clear.render(area, buffer);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Connection Picker ")
            .title_bottom(" Enter -> navigate | Esc -> cancel ");
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

        let footer_y = inner.y + inner.height.saturating_sub(1);
        Paragraph::new(truncate(&rendered.footer, header_avail)).render(
            Rect {
                x: inner.x,
                y: footer_y,
                width: inner.width,
                height: 1,
            },
            buffer,
        );

        let body = Rect {
            x: inner.x,
            y: inner.y + 2,
            width: inner.width,
            height: inner.height.saturating_sub(3),
        };
        self.render_grid(body, buffer);
        Ok(())
    }

    fn handle_event(&mut self, event: &Key) -> Result<ComponentOutcome<ConnectionPickerOutcome>> {
        let outcome = self.handle_key(*event);
        Ok(match outcome {
            ConnectionPickerOutcome::Continue => ComponentOutcome::Handled,
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

impl ConnectionPicker {
    fn render_grid(&mut self, body: Rect, buffer: &mut Buffer) {
        if self.items.is_empty() {
            buffer.set_string(
                body.x,
                body.y,
                format!("No navigable connections for {}", self.source_element_name),
                Style::default(),
            );
            return;
        }

        let style = GridStyle {
            selected_cell: Style::default().add_modifier(Modifier::REVERSED),
            scroll_up: "^",
            scroll_down: "v",
            ..GridStyle::default()
        };
        Grid::new()
            .with_cell_rows(TILE_ROWS)
            .with_min_cell_cols(TILE_MIN_COLS)
            .with_selected_index(Some(self.selected_index))
            .with_style(style)
            .render(body, buffer, &self.items, |cell, canvas| {
                if canvas.width() < 4 {
                    return;
                }
                let item = cell.item;
                let marker = if cell.selected { ">" } else { " " };
                let title = format!(
                    "{marker} {} {}",
                    direction_label(item.direction),
                    item.connected_element_name
                );
                canvas.set_string(
                    0,
                    0,
                    truncate(&title, canvas.width().saturating_sub(1) as usize),
                    canvas.style(),
                );

                if canvas.height() > 1 {
                    let detail = format!("rel: {}", item.relationship_detail);
                    canvas.set_string(
                        0,
                        1,
                        truncate(&detail, canvas.width().saturating_sub(1) as usize),
                        Style::default(),
                    );
                }
                if canvas.height() > 2 {
                    let target = format!("view: {}", item.target_view);
                    canvas.set_string(
                        0,
                        2,
                        truncate(&target, canvas.width().saturating_sub(1) as usize),
                        Style::default().add_modifier(Modifier::DIM),
                    );
                }
            });
    }
}

fn direction_label(direction: ConnectionDirection) -> &'static str {
    match direction {
        ConnectionDirection::Outgoing => "to",
        ConnectionDirection::Incoming => "from",
    }
}

fn plural(count: usize, singular: &'static str, plural: &'static str) -> &'static str {
    if count == 1 {
        singular
    } else {
        plural
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        text.chars().take(max.saturating_sub(1)).collect::<String>() + "..."
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{ElementId, RelationshipId};
    use crate::render::RasterBudget;
    use crate::workspace::{ElementKind, ElementMetadata, RelationshipMetadata, ViewInfo};
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;

    fn store() -> ViewStore {
        let views = vec![
            ViewInfo {
                key: "api".to_owned(),
                name: "API".to_owned(),
                kind: crate::workspace::ViewKind::Container,
                description: None,
                svg_path: PathBuf::from("api.svg"),
                element_ids: HashSet::from([ElementId::new("api")]),
                child_view_keys_by_element_id: HashMap::new(),
                primary_view_key: None,
                key_view_key: None,
            },
            ViewInfo {
                key: "database".to_owned(),
                name: "Database View".to_owned(),
                kind: crate::workspace::ViewKind::Component,
                description: None,
                svg_path: PathBuf::from("database.svg"),
                element_ids: HashSet::from([ElementId::new("database")]),
                child_view_keys_by_element_id: HashMap::new(),
                primary_view_key: None,
                key_view_key: None,
            },
            ViewInfo {
                key: "queue".to_owned(),
                name: "Queue View".to_owned(),
                kind: crate::workspace::ViewKind::Component,
                description: None,
                svg_path: PathBuf::from("queue.svg"),
                element_ids: HashSet::from([ElementId::new("queue")]),
                child_view_keys_by_element_id: HashMap::new(),
                primary_view_key: None,
                key_view_key: None,
            },
        ];
        let mut model = crate::workspace::WorkspaceModel::default();
        model.elements.insert(
            ElementId::new("api"),
            ElementMetadata {
                id: ElementId::new("api"),
                name: "API".to_owned(),
                description: None,
                technology: None,
                tags: Vec::new(),
                kind: ElementKind::Container,
            },
        );
        model.elements.insert(
            ElementId::new("database"),
            ElementMetadata {
                id: ElementId::new("database"),
                name: "Database".to_owned(),
                description: None,
                technology: Some("PostgreSQL".to_owned()),
                tags: Vec::new(),
                kind: ElementKind::Container,
            },
        );
        model.elements.insert(
            ElementId::new("queue"),
            ElementMetadata {
                id: ElementId::new("queue"),
                name: "Queue".to_owned(),
                description: None,
                technology: None,
                tags: Vec::new(),
                kind: ElementKind::Container,
            },
        );
        model.relationships.insert(
            RelationshipId::new("r1"),
            RelationshipMetadata {
                id: RelationshipId::new("r1"),
                source_id: ElementId::new("api"),
                destination_id: ElementId::new("database"),
                description: Some("Reads from".to_owned()),
                technology: Some("JDBC".to_owned()),
                tags: Vec::new(),
            },
        );
        model.relationships.insert(
            RelationshipId::new("r2"),
            RelationshipMetadata {
                id: RelationshipId::new("r2"),
                source_id: ElementId::new("queue"),
                destination_id: ElementId::new("api"),
                description: Some("Delivers events".to_owned()),
                technology: None,
                tags: Vec::new(),
            },
        );
        ViewStore::new(
            views,
            RasterBudget {
                quality: 1.0,
                ..RasterBudget::default()
            },
        )
        .unwrap()
        .with_model(model)
    }

    fn candidates() -> Vec<ConnectionNavigationCandidate> {
        vec![
            ConnectionNavigationCandidate {
                relationship_id: RelationshipId::new("r1"),
                direction: ConnectionDirection::Outgoing,
                connected_element_id: ElementId::new("database"),
                view_id: ViewId::new(1),
            },
            ConnectionNavigationCandidate {
                relationship_id: RelationshipId::new("r2"),
                direction: ConnectionDirection::Incoming,
                connected_element_id: ElementId::new("queue"),
                view_id: ViewId::new(2),
            },
        ]
    }

    #[test]
    fn render_includes_connection_fields() {
        let store = store();
        let picker = ConnectionPicker::new(&ElementId::new("api"), candidates(), &store);
        let rendered = picker.render();

        assert!(rendered.header.contains("Connections for API"));
        assert_eq!(rendered.footer, "showing 2 navigable connections");
        assert_eq!(rendered.items.len(), 2);
        assert_eq!(rendered.items[0].direction, "to");
        assert_eq!(rendered.items[0].connected_element_name, "Database");
        assert_eq!(rendered.items[0].relationship_detail, "Reads from (JDBC)");
        assert_eq!(rendered.items[0].target_view, "Database View");
        assert!(rendered.items[0].selected);
        assert_eq!(rendered.items[1].direction, "from");
    }

    #[test]
    fn keyboard_navigation_selects_and_wraps() {
        let store = store();
        let mut picker = ConnectionPicker::new(&ElementId::new("api"), candidates(), &store);

        picker.handle_key(Key::Down);
        assert_eq!(picker.selected_view_id(), Some(ViewId::new(2)));
        picker.handle_key(Key::Down);
        assert_eq!(picker.selected_view_id(), Some(ViewId::new(1)));

        match picker.handle_key(Key::Enter) {
            ConnectionPickerOutcome::Select(candidate) => {
                assert_eq!(candidate.relationship_id, RelationshipId::new("r1"));
            }
            other => panic!("expected select, got {other:?}"),
        }
    }

    #[test]
    fn esc_cancels() {
        let store = store();
        let mut picker = ConnectionPicker::new(&ElementId::new("api"), candidates(), &store);

        assert_eq!(picker.handle_key(Key::Esc), ConnectionPickerOutcome::Cancel);
    }

    #[test]
    fn render_buffer_uses_grid_surface() {
        let store = store();
        let mut picker = ConnectionPicker::new(&ElementId::new("api"), candidates(), &store);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 80, 20));

        picker
            .render_buffer(Rect::new(0, 0, 80, 20), &mut buffer)
            .unwrap();
        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(text.contains("Connection Picker"));
        assert!(text.contains("Database"));
    }
}

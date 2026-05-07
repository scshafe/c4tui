#![allow(dead_code)]

use crate::ids::ViewId;
use tui_kit::input::Key;
use crate::workspace::{ViewInfo, ViewKind, WorkspaceModel};

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
    items: Vec<PickerItem>,
    filter: String,
    show_keys: bool,
    selected_view: ViewId,
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
            items,
            filter: String::new(),
            show_keys: false,
            selected_view: current,
        }
    }

    pub fn handle_key(&mut self, key: Key) -> PickerOutcome {
        match key {
            Key::Esc => {
                if self.filter.is_empty() {
                    PickerOutcome::Cancel
                } else {
                    self.filter.clear();
                    PickerOutcome::Continue
                }
            }
            Key::CtrlC => PickerOutcome::Cancel,
            Key::Enter => {
                let visible = self.visible_view_ids();
                if let Some(target) = visible.iter().find(|id| **id == self.selected_view).copied()
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
                PickerOutcome::Continue
            }
            Key::Down => {
                self.move_selection(1);
                PickerOutcome::Continue
            }
            Key::Tab => {
                self.show_keys = !self.show_keys;
                self.clamp_selection();
                PickerOutcome::Continue
            }
            Key::Back => {
                self.filter.pop();
                self.clamp_selection();
                PickerOutcome::Continue
            }
            Key::Char(c) => {
                self.filter.push(c);
                self.clamp_selection();
                PickerOutcome::Continue
            }
            _ => PickerOutcome::Continue,
        }
    }

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
            if self.show_keys { " (incl. legends)" } else { "" },
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
        let next = (current_idx + delta)
            .rem_euclid(visible.len() as i32) as usize;
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
        if visible.iter().any(|item| item.view_id == self.selected_view) {
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
            view("Agent Coordination", ViewKind::Container, "AgentCoordination"),
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
}

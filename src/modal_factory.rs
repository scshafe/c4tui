use crate::event::Command;
use crate::ids::ViewId;
use crate::link_directory::{collect_link_candidates, LinkCandidate};
use crate::modal::{ActiveModal, NavPickerModal};
use crate::nav_items::{NavTarget, ViewNavItem};
use crate::nav_picker::{
    NavPicker, NavPickerConfig, NavPickerMode, NavRenderArtifact, ThumbnailCellArea,
};
use crate::state::{AppState, ModalSpec};
use crate::view_catalog::ViewCatalog;
use tui_kit::component::{Cached, ComponentId};

pub(crate) const SCOPE_PICKER: &str = "picker";
pub(crate) const SCOPE_LINK_DIRECTORY: &str = "link-directory";

pub(crate) const FOCUS_NODE_PICKER: &str = "picker-list";
pub(crate) const FOCUS_NODE_LINK_DIRECTORY: &str = "link-directory-list";

pub(crate) struct ModalSpawn {
    pub scope_id: &'static str,
    pub focus_node_id: &'static str,
    pub active_modal: ActiveModal,
    pub initial_hover: Option<ViewId>,
}

pub(crate) fn spawn_modal(spec: ModalSpec, catalog: &ViewCatalog, state: &AppState) -> ModalSpawn {
    match spec {
        ModalSpec::View => spawn_view_picker(catalog, state.current()),
        ModalSpec::ChildView { target_view_ids } => {
            spawn_child_view_picker(catalog, state.current(), target_view_ids)
        }
        ModalSpec::Link => spawn_link_directory(catalog, state),
    }
}

fn spawn_view_picker(catalog: &ViewCatalog, current: ViewId) -> ModalSpawn {
    let items = ViewNavItem::collect_all(&catalog.views, &catalog.model);
    let initial = items
        .iter()
        .position(|item| item.view_id == current)
        .unwrap_or(0);
    let picker = NavPicker::new(view_picker_config(), items, initial);
    let initial_hover = picker
        .selected()
        .map(|item| item.view_id)
        .unwrap_or(current);
    let modal = NavPickerModal {
        picker: Cached::new(picker),
        into_target: Box::new(NavTarget::View),
        hovered_view: Box::new(|picker| picker.selected().map(|item| item.view_id)),
        thumbnails: Box::new(thumbnails_from_view_picker),
    };

    ModalSpawn {
        scope_id: SCOPE_PICKER,
        focus_node_id: FOCUS_NODE_PICKER,
        active_modal: ActiveModal::Nav {
            modal: Box::new(modal),
            on_select: Box::new(|target, _state| match target {
                NavTarget::View(view_id) => Some(Command::SelectView(view_id)),
                _ => None,
            }),
        },
        initial_hover: Some(initial_hover),
    }
}

fn spawn_child_view_picker(
    catalog: &ViewCatalog,
    current: ViewId,
    target_view_ids: Vec<ViewId>,
) -> ModalSpawn {
    let items = ViewNavItem::collect_for_view_ids(&catalog.views, &catalog.model, &target_view_ids);
    let initial_view = target_view_ids.first().copied().unwrap_or(current);
    let initial = items
        .iter()
        .position(|item| item.view_id == initial_view)
        .unwrap_or(0);
    let picker = NavPicker::new(child_view_picker_config(), items, initial);
    let initial_hover = picker
        .selected()
        .map(|item| item.view_id)
        .unwrap_or(initial_view);
    let modal = NavPickerModal {
        picker: Cached::new(picker),
        into_target: Box::new(NavTarget::ChildView),
        hovered_view: Box::new(|picker| picker.selected().map(|item| item.view_id)),
        thumbnails: Box::new(thumbnails_from_view_picker),
    };

    ModalSpawn {
        scope_id: SCOPE_PICKER,
        focus_node_id: FOCUS_NODE_PICKER,
        active_modal: ActiveModal::Nav {
            modal: Box::new(modal),
            on_select: Box::new(|target, _state| match target {
                NavTarget::ChildView(view_id) => Some(Command::SelectChildView(view_id)),
                _ => None,
            }),
        },
        initial_hover: Some(initial_hover),
    }
}

fn spawn_link_directory(catalog: &ViewCatalog, state: &AppState) -> ModalSpawn {
    let current = state.current();
    let pinned_element = state.pinned_element().cloned();
    let items = collect_link_candidates(catalog, current, pinned_element.as_ref());
    let picker = NavPicker::new(link_directory_config(), items, 0);
    let initial_hover = picker
        .selected()
        .map(|item| item.target_view)
        .filter(|target| *target != current);
    let modal: NavPickerModal<LinkCandidate> = NavPickerModal {
        picker: Cached::new(picker),
        into_target: Box::new(NavTarget::Link),
        hovered_view: Box::new(|picker| picker.selected().map(|item| item.target_view)),
        thumbnails: Box::new(|_| Vec::new()),
    };

    ModalSpawn {
        scope_id: SCOPE_LINK_DIRECTORY,
        focus_node_id: FOCUS_NODE_LINK_DIRECTORY,
        active_modal: ActiveModal::Nav {
            modal: Box::new(modal),
            on_select: Box::new(|target, _state| match target {
                NavTarget::Link(candidate) => Some(Command::SelectLink(candidate)),
                _ => None,
            }),
        },
        initial_hover,
    }
}

fn view_picker_config() -> NavPickerConfig {
    NavPickerConfig {
        id: ComponentId::new("c4tui-view-picker"),
        title: " View Picker ".into(),
        footer_hint: " type → filter | Tab → legends | Enter → select | Esc → cancel ".into(),
        default_header:
            "Pick a view  —  type to filter, Enter to select, Esc to cancel, Tab to toggle key views"
                .into(),
        min_cell_cols: 22,
        cell_rows: 8,
        mode: NavPickerMode::Filterable {
            allows_secondary_toggle: true,
        },
    }
}

fn child_view_picker_config() -> NavPickerConfig {
    NavPickerConfig {
        id: ComponentId::new("c4tui-child-view-picker"),
        title: " Related Views ".into(),
        footer_hint: " type → filter | Enter → drill | Esc → cancel ".into(),
        default_header: "Pick a child view to drill into".into(),
        min_cell_cols: 22,
        cell_rows: 8,
        mode: NavPickerMode::Filterable {
            allows_secondary_toggle: false,
        },
    }
}

fn link_directory_config() -> NavPickerConfig {
    NavPickerConfig {
        id: ComponentId::new("c4tui-link-directory"),
        title: " Link Directory ".into(),
        footer_hint: " type → filter | Enter → navigate | Esc → cancel ".into(),
        default_header: "Linked diagrams from here  -  Enter to navigate, Esc to cancel".into(),
        min_cell_cols: 34,
        cell_rows: 5,
        mode: NavPickerMode::Filterable {
            allows_secondary_toggle: false,
        },
    }
}

/// Convert a `ViewNavItem` picker's per-frame artifacts into the thumbnail
/// anchors the terminal layer paints after rendering.
fn thumbnails_from_view_picker(picker: &NavPicker<ViewNavItem>) -> Vec<ThumbnailCellArea> {
    picker
        .last_artifacts()
        .iter()
        .map(|artifact| match artifact {
            NavRenderArtifact::Thumbnail { id, area } => ThumbnailCellArea {
                view_id: id.view_id(),
                area: *area,
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ElementId;
    use crate::workspace::{ElementKind, ElementMetadata, ViewInfo, ViewKind, WorkspaceModel};
    use std::path::PathBuf;

    fn view(
        key: &str,
        name: &str,
        kind: ViewKind,
        elements: &[&str],
        child_links: &[(&str, &[&str])],
    ) -> ViewInfo {
        ViewInfo {
            key: key.to_owned(),
            name: name.to_owned(),
            kind,
            description: None,
            svg_path: PathBuf::from(format!("{key}.svg")),
            element_ids: elements.iter().copied().map(ElementId::new).collect(),
            child_view_keys_by_element_id: child_links
                .iter()
                .map(|(element, keys)| {
                    (
                        ElementId::new(*element),
                        keys.iter().map(|key| (*key).to_owned()).collect(),
                    )
                })
                .collect(),
            primary_view_key: None,
            key_view_key: None,
        }
    }

    fn catalog() -> ViewCatalog {
        let mut model = WorkspaceModel::default();
        model.elements.insert(
            ElementId::new("system"),
            ElementMetadata {
                id: ElementId::new("system"),
                name: "System".to_owned(),
                description: None,
                technology: None,
                tags: Vec::new(),
                kind: ElementKind::SoftwareSystem,
            },
        );

        ViewCatalog::new(vec![
            view(
                "system",
                "System Context",
                ViewKind::SystemContext,
                &["system"],
                &[("system", &["containers"][..])],
            ),
            view("containers", "Containers", ViewKind::Container, &[], &[]),
            view("components", "Components", ViewKind::Component, &[], &[]),
        ])
        .unwrap()
        .with_model(model)
    }

    #[test]
    fn view_modal_targets_select_view_and_reports_current_hover() {
        let catalog = catalog();
        let spawn = spawn_modal(ModalSpec::View, &catalog, &AppState::default());

        assert_eq!(spawn.scope_id, SCOPE_PICKER);
        assert_eq!(spawn.focus_node_id, FOCUS_NODE_PICKER);
        assert_eq!(spawn.initial_hover, Some(ViewId::first()));

        let ActiveModal::Nav { modal, on_select } = spawn.active_modal else {
            panic!("view picker should spawn a nav modal");
        };
        assert_eq!(modal.currently_hovered_view(), Some(ViewId::first()));

        let mut state = AppState::default();
        assert_eq!(
            on_select(NavTarget::View(ViewId::new(1)), &mut state),
            Some(Command::SelectView(ViewId::new(1)))
        );
    }

    #[test]
    fn child_view_modal_targets_select_child_view_and_first_candidate_hover() {
        let catalog = catalog();
        let spawn = spawn_modal(
            ModalSpec::ChildView {
                target_view_ids: vec![ViewId::new(2), ViewId::new(1)],
            },
            &catalog,
            &AppState::default(),
        );

        assert_eq!(spawn.scope_id, SCOPE_PICKER);
        assert_eq!(spawn.focus_node_id, FOCUS_NODE_PICKER);
        assert_eq!(spawn.initial_hover, Some(ViewId::new(2)));

        let ActiveModal::Nav { modal, on_select } = spawn.active_modal else {
            panic!("child view picker should spawn a nav modal");
        };
        assert_eq!(modal.currently_hovered_view(), Some(ViewId::new(2)));

        let mut state = AppState::default();
        assert_eq!(
            on_select(NavTarget::ChildView(ViewId::new(2)), &mut state),
            Some(Command::SelectChildView(ViewId::new(2)))
        );
    }

    #[test]
    fn link_directory_modal_targets_select_link_and_reports_target_hover() {
        let catalog = catalog();
        let spawn = spawn_modal(ModalSpec::Link, &catalog, &AppState::default());

        assert_eq!(spawn.scope_id, SCOPE_LINK_DIRECTORY);
        assert_eq!(spawn.focus_node_id, FOCUS_NODE_LINK_DIRECTORY);
        assert_eq!(spawn.initial_hover, Some(ViewId::new(1)));

        let ActiveModal::Nav { modal, on_select } = spawn.active_modal else {
            panic!("link directory should spawn a nav modal");
        };
        assert_eq!(modal.currently_hovered_view(), Some(ViewId::new(1)));

        let candidate = collect_link_candidates(&catalog, ViewId::first(), None)
            .into_iter()
            .next()
            .expect("fixture has one detail link");
        let mut state = AppState::default();
        assert_eq!(
            on_select(NavTarget::Link(candidate.clone()), &mut state),
            Some(Command::SelectLink(candidate))
        );
    }

    #[test]
    fn link_directory_without_candidates_does_not_prime_current_view() {
        let catalog = ViewCatalog::new(vec![view(
            "single",
            "Single",
            ViewKind::SystemContext,
            &[],
            &[],
        )])
        .unwrap();

        let spawn = spawn_modal(ModalSpec::Link, &catalog, &AppState::default());

        assert_eq!(spawn.initial_hover, None);
    }
}

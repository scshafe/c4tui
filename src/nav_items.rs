//! Concrete `NavItem` types and the `NavTarget` enum every NavPicker spawned
//! by c4tui produces.
//!
//! Step 1.1 introduced the [`NavTarget`] enum and empty struct shells.
//! Step 1.4 (this commit) lands the concrete fields and the three
//! `impl NavItem` blocks (`ViewNavItem`, `ChildViewNavItem`,
//! `ConnectionNavItem`) plus one real-typed test per item that exercises
//! the trait surface against the production types.

#![allow(dead_code)]

use ratatui::style::{Modifier, Style};

use crate::ids::ViewId;
use crate::nav_picker::{truncate, NavCellCanvas, NavItem, NavRenderArtifact, ThumbnailId};
use crate::view::ConnectionNavigationCandidate;
use crate::workspace::{ViewInfo, ViewKind, WorkspaceModel};

const THUMB_ROWS: u16 = 5;

/// What the user just chose from a navigation picker.
///
/// One union for every NavPicker variant c4tui spawns. The downstream
/// callback (`ModalSlot::on_select`, introduced in Task 5) matches on this
/// variant to produce a `Command`.
///
/// Phase 5 will add a `Link(LinkCandidate)` variant for connection-link
/// navigation; the spawn-site closure is the only seam that needs to learn
/// about the new variant.
#[derive(Debug, Clone, PartialEq)]
pub enum NavTarget {
    /// A view selected from the top-level view picker (clears breadcrumbs).
    View(ViewId),
    /// A view selected from the child-view picker spawned on multi-child drill
    /// (pushes a breadcrumb).
    ChildView(ViewId),
    /// A connection candidate selected from the connection picker (pushes a
    /// breadcrumb and pins the connected element).
    Connection(ConnectionNavigationCandidate),
    // Phase 5 will add:
    //   Link(LinkCandidate),
}

/// Item that yields `NavTarget::View(...)` when selected from the top-level
/// view picker.
#[derive(Debug, Clone)]
pub struct ViewNavItem {
    pub view_id: ViewId,
    pub kind: ViewKind,
    pub name: String,
    pub key: String,
    pub element_names: Vec<String>,
    pub description: Option<String>,
}

impl ViewNavItem {
    /// Build a `ViewNavItem` collection covering every view in `views`, in
    /// the order they appear in the store.
    pub fn collect_all(views: &[ViewInfo], model: &WorkspaceModel) -> Vec<Self> {
        views
            .iter()
            .enumerate()
            .map(|(idx, info)| Self::from_info(ViewId::new(idx), info, model))
            .collect()
    }

    /// Build a `ViewNavItem` collection restricted to the given view ids,
    /// preserving the order supplied. Items whose id falls outside the
    /// `views` slice are silently skipped.
    pub fn collect_for_view_ids(
        views: &[ViewInfo],
        model: &WorkspaceModel,
        view_ids: &[ViewId],
    ) -> Vec<Self> {
        view_ids
            .iter()
            .filter_map(|view_id| views.get(view_id.index()).map(|info| (*view_id, info)))
            .map(|(view_id, info)| Self::from_info(view_id, info, model))
            .collect()
    }

    fn from_info(view_id: ViewId, info: &ViewInfo, model: &WorkspaceModel) -> Self {
        let element_names = info
            .element_ids
            .iter()
            .filter_map(|id| model.elements.get(id).map(|e| e.name.clone()))
            .collect();
        Self {
            view_id,
            kind: info.kind,
            name: info.name.clone(),
            key: info.key.clone(),
            element_names,
            description: info.description.clone(),
        }
    }
}

impl NavItem for ViewNavItem {
    type Output = ViewId;

    fn filter_text(&self) -> &str {
        // `filter_text` is just `name`; secondary tokens carry the element
        // names that should also match this view in the filter. The other
        // text fields (`key`, `kind.label()`, `description`) are NOT
        // searchable -- this is a deliberate narrowing of the legacy
        // picker's filter surface, locked in by `nav_picker.rs` tests.
        &self.name
    }

    fn group(&self) -> Option<&str> {
        Some(self.kind.label())
    }

    fn secondary_filter_tokens(&self) -> &[String] {
        &self.element_names
    }

    fn is_secondary(&self) -> bool {
        self.kind.is_legend()
    }

    fn render_into_canvas(
        &self,
        mut canvas: NavCellCanvas<'_, '_>,
        selected: bool,
        _filter: &str,
        sink: &mut dyn FnMut(NavRenderArtifact),
    ) {
        let marker = if selected { ">" } else { " " };
        let title = format!("{marker} {}", self.name);
        canvas.set_string(
            0,
            0,
            truncate(&title, canvas.width().saturating_sub(1) as usize),
            canvas.style(),
        );

        if canvas.height() > 1 {
            let image_rows = THUMB_ROWS.min(canvas.height().saturating_sub(1));
            let image_cols = canvas.width().saturating_sub(2);
            if image_rows > 0 && image_cols > 0 {
                let area = canvas.local_cell_area(1, 1, image_cols, image_rows);
                sink(NavRenderArtifact::Thumbnail {
                    id: ThumbnailId::view(self.view_id),
                    area,
                });
            }
        }

        let key_row = THUMB_ROWS.saturating_add(1);
        if key_row < canvas.height() {
            let style = if selected {
                canvas.style().add_modifier(Modifier::DIM)
            } else {
                Style::default().add_modifier(Modifier::DIM)
            };
            canvas.set_string(
                0,
                key_row,
                truncate(&self.key, canvas.width().saturating_sub(1) as usize),
                style,
            );
        }
    }

    fn outcome(&self) -> ViewId {
        self.view_id
    }
}

/// Item that yields `NavTarget::ChildView(...)` when selected from the
/// drill-down picker spawned by `Effect::OpenChildViewPicker`.
///
/// Currently structurally identical to `ViewNavItem`. The distinct type
/// keeps the call site's intent explicit: a `ChildViewNavItem` will be
/// wrapped in `NavTarget::ChildView` at the spawn point, while a
/// `ViewNavItem` becomes `NavTarget::View`.
///
/// No `collect_all` constructor by design -- child-view pickers are always
/// scoped to a candidate list (the children of the current drilled
/// element), not over every view in the workspace.
#[derive(Debug, Clone)]
pub struct ChildViewNavItem(pub ViewNavItem);

impl ChildViewNavItem {
    pub fn collect_for_view_ids(
        views: &[ViewInfo],
        model: &WorkspaceModel,
        view_ids: &[ViewId],
    ) -> Vec<Self> {
        ViewNavItem::collect_for_view_ids(views, model, view_ids)
            .into_iter()
            .map(Self)
            .collect()
    }
}

impl NavItem for ChildViewNavItem {
    type Output = ViewId;

    fn filter_text(&self) -> &str {
        self.0.filter_text()
    }

    fn group(&self) -> Option<&str> {
        self.0.group()
    }

    fn secondary_filter_tokens(&self) -> &[String] {
        self.0.secondary_filter_tokens()
    }

    fn is_secondary(&self) -> bool {
        self.0.is_secondary()
    }

    fn render_into_canvas(
        &self,
        canvas: NavCellCanvas<'_, '_>,
        selected: bool,
        filter: &str,
        sink: &mut dyn FnMut(NavRenderArtifact),
    ) {
        self.0.render_into_canvas(canvas, selected, filter, sink)
    }

    fn outcome(&self) -> ViewId {
        self.0.outcome()
    }
}

/// Item that yields `NavTarget::Connection(...)` when selected from the
/// connection picker.
#[derive(Debug, Clone)]
pub struct ConnectionNavItem {
    pub candidate: ConnectionNavigationCandidate,
    pub direction_label: &'static str,
    pub connected_element_name: String,
    pub relationship_detail: String,
    pub target_view: String,
}

impl ConnectionNavItem {
    /// Construct `ConnectionNavItem` rows from a candidate list, resolving
    /// element and relationship names from the `ViewStore`'s workspace
    /// model.
    pub fn collect_from_candidates(
        candidates: Vec<ConnectionNavigationCandidate>,
        store: &crate::view::ViewStore,
    ) -> Vec<Self> {
        use crate::view::ConnectionDirection;
        candidates
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
                    (Some(d), Some(t)) => format!("{d} ({t})"),
                    (Some(d), None) => d.to_owned(),
                    (None, Some(t)) => t.to_owned(),
                    (None, None) => "relationship".to_owned(),
                };
                let target_view = store.view(candidate.view_id).name.clone();
                let direction_label = match candidate.direction {
                    ConnectionDirection::Outgoing => "to",
                    ConnectionDirection::Incoming => "from",
                };
                Self {
                    candidate,
                    direction_label,
                    connected_element_name,
                    relationship_detail,
                    target_view,
                }
            })
            .collect()
    }
}

impl NavItem for ConnectionNavItem {
    type Output = ConnectionNavigationCandidate;

    fn filter_text(&self) -> &str {
        &self.connected_element_name
    }

    fn render_into_canvas(
        &self,
        mut canvas: NavCellCanvas<'_, '_>,
        selected: bool,
        _filter: &str,
        _sink: &mut dyn FnMut(NavRenderArtifact),
    ) {
        let marker = if selected { ">" } else { " " };
        let title = format!(
            "{marker} {} {}",
            self.direction_label, self.connected_element_name
        );
        canvas.set_string(
            0,
            0,
            truncate(&title, canvas.width().saturating_sub(1) as usize),
            canvas.style(),
        );

        if canvas.height() > 1 {
            let detail = format!("rel: {}", self.relationship_detail);
            canvas.set_string(
                0,
                1,
                truncate(&detail, canvas.width().saturating_sub(1) as usize),
                Style::default(),
            );
        }
        if canvas.height() > 2 {
            let target = format!("view: {}", self.target_view);
            canvas.set_string(
                0,
                2,
                truncate(&target, canvas.width().saturating_sub(1) as usize),
                Style::default().add_modifier(Modifier::DIM),
            );
        }
    }

    fn outcome(&self) -> ConnectionNavigationCandidate {
        self.candidate.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;

    use crate::ids::{ElementId, RelationshipId};
    use crate::view::{ConnectionDirection, ConnectionNavigationCandidate};
    use crate::workspace::{ElementKind, ElementMetadata, RelationshipMetadata};

    fn element(id: &str, name: &str, kind: ElementKind) -> (ElementId, ElementMetadata) {
        let id = ElementId::new(id);
        (
            id.clone(),
            ElementMetadata {
                id,
                name: name.to_owned(),
                description: None,
                technology: None,
                tags: Vec::new(),
                kind,
            },
        )
    }

    fn view_info(key: &str, name: &str, kind: ViewKind, elements: &[&str]) -> ViewInfo {
        let mut element_ids = HashSet::new();
        for e in elements {
            element_ids.insert(ElementId::new(*e));
        }
        ViewInfo {
            key: key.to_owned(),
            name: name.to_owned(),
            kind,
            description: None,
            svg_path: PathBuf::from(format!("{key}.svg")),
            element_ids,
            child_view_keys_by_element_id: HashMap::new(),
            primary_view_key: None,
            key_view_key: None,
        }
    }

    #[test]
    fn view_nav_item_exposes_name_kind_and_element_names() {
        let mut model = WorkspaceModel::default();
        let (id, meta) = element("1", "User", ElementKind::Person);
        model.elements.insert(id, meta);
        let views = vec![view_info(
            "landscape",
            "Landscape",
            ViewKind::SystemContext,
            &["1"],
        )];

        let items = ViewNavItem::collect_all(&views, &model);
        assert_eq!(items.len(), 1);
        let item = &items[0];

        assert_eq!(item.filter_text(), "Landscape");
        assert_eq!(item.group(), Some("SystemContext"));
        assert_eq!(item.secondary_filter_tokens(), &["User".to_owned()]);
        assert_eq!(item.outcome(), ViewId::new(0));
        assert!(!item.is_secondary());
    }

    #[test]
    fn child_view_nav_item_delegates_through_inner_view_item() {
        let model = WorkspaceModel::default();
        let views = vec![
            view_info("a", "Alpha", ViewKind::Container, &[]),
            view_info("b-key", "Beta (key)", ViewKind::Key, &[]),
        ];

        let items = ChildViewNavItem::collect_for_view_ids(
            &views,
            &model,
            &[ViewId::new(1), ViewId::new(0)],
        );

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].filter_text(), "Beta (key)");
        assert!(items[0].is_secondary(), "key views are secondary");
        assert_eq!(items[0].outcome(), ViewId::new(1));
        assert_eq!(items[1].outcome(), ViewId::new(0));
    }

    #[test]
    fn connection_nav_item_resolves_names_and_relationship_detail() {
        // Build a minimal ViewStore so collect_from_candidates can look up
        // element/relationship metadata and view names.
        let mut model = WorkspaceModel::default();
        let (user_id, user_meta) = element("1", "User", ElementKind::Person);
        let (system_id, system_meta) = element("2", "Billing", ElementKind::SoftwareSystem);
        model.elements.insert(user_id.clone(), user_meta);
        model.elements.insert(system_id.clone(), system_meta);

        let rel_id = RelationshipId::new("10");
        model.relationships.insert(
            rel_id.clone(),
            RelationshipMetadata {
                id: rel_id.clone(),
                source_id: user_id.clone(),
                destination_id: system_id.clone(),
                description: Some("Uses".to_owned()),
                technology: Some("HTTPS".to_owned()),
                tags: Vec::new(),
            },
        );

        let views = vec![view_info(
            "landscape",
            "Landscape",
            ViewKind::SystemContext,
            &["1", "2"],
        )];

        let store = crate::view::ViewStore::new(views, crate::render::RasterBudget::default())
            .unwrap()
            .with_model(model);

        let candidate = ConnectionNavigationCandidate {
            relationship_id: rel_id.clone(),
            direction: ConnectionDirection::Outgoing,
            connected_element_id: system_id.clone(),
            view_id: ViewId::new(0),
        };

        let items = ConnectionNavItem::collect_from_candidates(vec![candidate.clone()], &store);
        assert_eq!(items.len(), 1);
        let item = &items[0];

        assert_eq!(item.filter_text(), "Billing");
        assert_eq!(item.direction_label, "to");
        assert_eq!(item.relationship_detail, "Uses (HTTPS)");
        assert_eq!(item.target_view, "Landscape");
        assert_eq!(item.outcome(), candidate);
        assert!(!item.is_secondary());
    }

    /// Locks in the name-only filter surface of `ViewNavItem`. The legacy
    /// `picker.rs` also matched against the view key, kind label, and
    /// description; that wider surface is intentionally NOT inherited. If
    /// we ever want to widen it again the right move is to extend
    /// `secondary_filter_tokens`, not `filter_text`.
    #[test]
    fn view_nav_item_filter_text_surface_is_name_only() {
        let model = WorkspaceModel::default();
        let mut info = view_info("zzz-key", "Alpha", ViewKind::Container, &[]);
        info.description = Some("Some description text".to_owned());
        let views = vec![info];

        let items = ViewNavItem::collect_all(&views, &model);
        let item = &items[0];

        // The name is in `filter_text()`.
        assert_eq!(item.filter_text(), "Alpha");
        // The key, kind label, and description are NOT in either filter
        // surface (`filter_text` is name only; `secondary_filter_tokens`
        // is element names only).
        let secondary_tokens: Vec<&String> = item.secondary_filter_tokens().iter().collect();
        assert!(
            !secondary_tokens.iter().any(|t| t.contains("zzz")),
            "view key is not part of the filter surface"
        );
        assert!(
            !secondary_tokens.iter().any(|t| t.contains("Container")),
            "kind label is not part of the filter surface"
        );
        assert!(
            !secondary_tokens.iter().any(|t| t.contains("description")),
            "description is not part of the filter surface"
        );
    }

    /// `ChildViewNavItem` delegates to its inner `ViewNavItem`, so it
    /// inherits the same name-only filter surface. Lock that in.
    #[test]
    fn child_view_nav_item_filter_text_surface_is_name_only() {
        let model = WorkspaceModel::default();
        let mut info = view_info("zzz-key", "Beta", ViewKind::Container, &[]);
        info.description = Some("Some description text".to_owned());
        let views = vec![info];

        let items = ChildViewNavItem::collect_for_view_ids(&views, &model, &[ViewId::new(0)]);
        let item = &items[0];

        assert_eq!(item.filter_text(), "Beta");
        let secondary_tokens: Vec<&String> = item.secondary_filter_tokens().iter().collect();
        assert!(!secondary_tokens.iter().any(|t| t.contains("zzz")));
        assert!(!secondary_tokens.iter().any(|t| t.contains("Container")));
        assert!(!secondary_tokens.iter().any(|t| t.contains("description")));
    }

    /// `ConnectionNavItem` filters on the connected element name only.
    /// The relationship detail, technology, direction label, and target
    /// view name are NOT in the filter surface.
    #[test]
    fn connection_nav_item_filter_text_surface_is_connected_name_only() {
        let mut model = WorkspaceModel::default();
        let (user_id, user_meta) = element("1", "User", ElementKind::Person);
        let (system_id, system_meta) = element("2", "Billing", ElementKind::SoftwareSystem);
        model.elements.insert(user_id.clone(), user_meta);
        model.elements.insert(system_id.clone(), system_meta);

        let rel_id = RelationshipId::new("10");
        model.relationships.insert(
            rel_id.clone(),
            RelationshipMetadata {
                id: rel_id.clone(),
                source_id: user_id.clone(),
                destination_id: system_id.clone(),
                description: Some("ZetaDescription".to_owned()),
                technology: Some("ZetaTech".to_owned()),
                tags: Vec::new(),
            },
        );

        let views = vec![view_info(
            "zzz-key",
            "ZetaView",
            ViewKind::SystemContext,
            &["1", "2"],
        )];

        let store = crate::view::ViewStore::new(views, crate::render::RasterBudget::default())
            .unwrap()
            .with_model(model);

        let candidate = ConnectionNavigationCandidate {
            relationship_id: rel_id.clone(),
            direction: ConnectionDirection::Outgoing,
            connected_element_id: system_id.clone(),
            view_id: ViewId::new(0),
        };

        let items = ConnectionNavItem::collect_from_candidates(vec![candidate], &store);
        let item = &items[0];

        assert_eq!(item.filter_text(), "Billing");
        // Nothing else feeds into the filter surface.
        assert!(item.secondary_filter_tokens().is_empty());
        // Sanity-check that relationship/technology/view text exist on the
        // item but are not in either filter surface.
        assert!(item.relationship_detail.contains("ZetaDescription"));
        assert!(item.relationship_detail.contains("ZetaTech"));
        assert_eq!(item.target_view, "ZetaView");
    }
}

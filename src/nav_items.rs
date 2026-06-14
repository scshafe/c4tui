//! Concrete `NavItem` types and the `NavTarget` enum every NavPicker spawned
//! by c4tui produces.
//!
//! Step 1.1 introduced the [`NavTarget`] enum and empty struct shells.
//! Step 1.4 landed the concrete fields and `impl NavItem` blocks
//! plus real-typed tests that exercise the trait surface against the
//! production types.
//!
//! Task 3 (Phase F) collapsed the original `ChildViewNavItem(pub ViewNavItem)`
//! newtype into `ViewNavItem`: both the top-level and child-view pickers
//! reuse `Cached<NavPicker<ViewNavItem>>`, and `PickerSlot.action`
//! (`SelectView` vs `Drill`) carries the variant intent that the newtype
//! previously encoded.

use ratatui::style::{Modifier, Style};

use crate::ids::ViewId;
use crate::link_directory::LinkCandidate;
use crate::nav_picker::{truncate, NavCellCanvas, NavItem, NavRenderArtifact, ThumbnailId};
use crate::workspace::{ViewInfo, ViewKind, WorkspaceModel};

const THUMB_ROWS: u16 = 5;

/// What the user just chose from a navigation picker.
///
/// One union for every NavPicker variant c4tui spawns. The downstream
/// callback (`ModalSlot::on_select`, introduced in Task 5) matches on this
/// variant to produce a `Command`.
///
/// The spawn-site closure is the only seam that needs to learn about each
/// variant.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum NavTarget {
    /// A view selected from the top-level view picker (clears breadcrumbs).
    View(ViewId),
    /// A view selected from the child-view picker spawned on multi-child drill
    /// (pushes a breadcrumb).
    ChildView(ViewId),
    /// A link-directory candidate selected from the keyboard-first directory.
    Link(LinkCandidate),
}

/// Item that yields `NavTarget::View(...)` when selected from the top-level
/// view picker.
#[derive(Debug, Clone)]
pub(crate) struct ViewNavItem {
    pub view_id: ViewId,
    pub kind: ViewKind,
    pub name: String,
    pub key: String,
    pub element_names: Vec<String>,
}

impl ViewNavItem {
    /// Build a `ViewNavItem` collection covering every view in `views`, in
    /// the order they appear in the store.
    pub(crate) fn collect_all(views: &[ViewInfo], model: &WorkspaceModel) -> Vec<Self> {
        views
            .iter()
            .enumerate()
            .map(|(idx, info)| Self::from_info(ViewId::new(idx), info, model))
            .collect()
    }

    /// Build a `ViewNavItem` collection restricted to the given view ids,
    /// preserving the order supplied. Items whose id falls outside the
    /// `views` slice are silently skipped.
    pub(crate) fn collect_for_view_ids(
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

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;

    use crate::ids::ElementId;
    use crate::workspace::{ElementKind, ElementMetadata};

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
        assert_eq!(item.secondary_filter_tokens(), &["User".to_owned()]);
        assert_eq!(item.outcome(), ViewId::new(0));
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
}

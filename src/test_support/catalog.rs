use std::path::PathBuf;

use crate::ids::ElementId;
use crate::view_catalog::ViewCatalog;
use crate::workspace::{ViewInfo, ViewKind, WorkspaceModel};

#[derive(Debug, Clone)]
pub(crate) struct TestViewBuilder {
    key: String,
    name: String,
    kind: ViewKind,
    description: Option<String>,
    svg_path: PathBuf,
    element_ids: std::collections::HashSet<ElementId>,
    child_view_keys_by_element_id: std::collections::HashMap<ElementId, Vec<String>>,
    primary_view_key: Option<String>,
    key_view_key: Option<String>,
}

impl TestViewBuilder {
    pub(crate) fn new(key: &str) -> Self {
        Self {
            key: key.to_owned(),
            name: key.to_owned(),
            kind: ViewKind::SystemContext,
            description: None,
            svg_path: PathBuf::from(format!("{key}.svg")),
            element_ids: std::collections::HashSet::new(),
            child_view_keys_by_element_id: std::collections::HashMap::new(),
            primary_view_key: None,
            key_view_key: None,
        }
    }

    pub(crate) fn named(mut self, name: &str) -> Self {
        self.name = name.to_owned();
        self
    }

    pub(crate) fn kind(mut self, kind: ViewKind) -> Self {
        self.kind = kind;
        self
    }

    pub(crate) fn svg_path(mut self, svg_path: impl Into<PathBuf>) -> Self {
        self.svg_path = svg_path.into();
        self
    }

    pub(crate) fn elements<'a>(mut self, ids: impl IntoIterator<Item = &'a str>) -> Self {
        self.element_ids = ids.into_iter().map(ElementId::new).collect();
        self
    }

    pub(crate) fn child_link<'a>(
        mut self,
        element_id: &str,
        child_keys: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        self.child_view_keys_by_element_id.insert(
            ElementId::new(element_id),
            child_keys.into_iter().map(str::to_owned).collect(),
        );
        self
    }

    pub(crate) fn key_view_key(mut self, key: &str) -> Self {
        self.key_view_key = Some(key.to_owned());
        self
    }

    pub(crate) fn build(self) -> ViewInfo {
        ViewInfo {
            key: self.key,
            name: self.name,
            kind: self.kind,
            description: self.description,
            svg_path: self.svg_path,
            element_ids: self.element_ids,
            child_view_keys_by_element_id: self.child_view_keys_by_element_id,
            primary_view_key: self.primary_view_key,
            key_view_key: self.key_view_key,
        }
    }
}

pub(crate) fn catalog(views: impl IntoIterator<Item = ViewInfo>) -> ViewCatalog {
    ViewCatalog::new(views.into_iter().collect()).unwrap()
}

pub(crate) fn catalog_with_model(
    views: impl IntoIterator<Item = ViewInfo>,
    model: WorkspaceModel,
) -> ViewCatalog {
    catalog(views).with_model(model)
}

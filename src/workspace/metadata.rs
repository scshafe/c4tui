use std::collections::HashSet;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::ids::ElementId;

use super::json::{StructurizrViewJson, StructurizrWorkspaceJson};

#[derive(Debug)]
pub(super) struct ViewMetadata {
    pub(super) views: Vec<ViewMetadataEntry>,
}

#[derive(Debug)]
pub(super) struct ViewMetadataEntry {
    pub(super) key: String,
    pub(super) name: String,
    pub(super) view_type: String,
    pub(super) description: Option<String>,
    pub(super) element_ids: HashSet<ElementId>,
    pub(super) parent_element_id: Option<ElementId>,
}

impl ViewMetadata {
    pub(super) fn load(path: &Path) -> Result<Self> {
        let json = fs::read_to_string(path).with_context(|| {
            format!("failed to read workspace metadata from {}", path.display())
        })?;
        let workspace: StructurizrWorkspaceJson =
            serde_json::from_str(&json).with_context(|| {
                format!("failed to parse workspace metadata from {}", path.display())
            })?;
        let Some(views) = workspace.views else {
            return Ok(Self { views: Vec::new() });
        };

        let mut entries = Vec::new();
        push_views(
            &mut entries,
            "SystemLandscape",
            views.system_landscape_views,
        );
        push_views(&mut entries, "SystemContext", views.system_context_views);
        push_views(&mut entries, "Container", views.container_views);
        push_views(&mut entries, "Component", views.component_views);
        push_views(&mut entries, "Dynamic", views.dynamic_views);
        push_views(&mut entries, "Deployment", views.deployment_views);
        push_views(&mut entries, "Filtered", views.filtered_views);
        push_views(&mut entries, "Custom", views.custom_views);
        push_views(&mut entries, "Image", views.image_views);

        Ok(Self { views: entries })
    }

    pub(super) fn find_for_svg_stem(&self, stem: &str) -> Option<&ViewMetadataEntry> {
        let normalized_stem = normalize_view_key(stem);
        self.views.iter().find(|view| {
            normalize_view_key(&view.key) == normalized_stem
                || normalize_view_key(&view.name) == normalized_stem
        })
    }
}

fn push_views(
    entries: &mut Vec<ViewMetadataEntry>,
    view_type: &str,
    views: Option<Vec<StructurizrViewJson>>,
) {
    let Some(views) = views else {
        return;
    };

    for view in views {
        let key = view
            .key
            .or_else(|| view.name.clone())
            .or_else(|| view.title.clone())
            .unwrap_or_else(|| "view".to_owned());
        let name = view
            .title
            .clone()
            .or(view.name.clone())
            .unwrap_or_else(|| key.clone());
        let parent_element_id = view
            .container_id
            .clone()
            .or_else(|| view.component_id.clone())
            .or_else(|| view.element_id.clone())
            .or_else(|| view.software_system_id.clone())
            .map(ElementId::new);
        let element_ids = view
            .elements
            .into_iter()
            .map(|element| ElementId::new(element.id))
            .collect();
        entries.push(ViewMetadataEntry {
            key,
            name,
            view_type: view_type.to_owned(),
            description: view.description.filter(|s| !s.is_empty()),
            element_ids,
            parent_element_id,
        });
    }
}

pub(super) fn normalize_view_key(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect()
}

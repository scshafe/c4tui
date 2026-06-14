use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::ids::ElementId;

use super::export::ExportedWorkspace;
use super::metadata::{normalize_view_key, ViewMetadata};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ViewKind {
    SystemLandscape,
    SystemContext,
    Container,
    Component,
    Dynamic,
    Deployment,
    Filtered,
    Custom,
    Image,
    Key,
    Unknown,
}

impl ViewKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::SystemLandscape => "SystemLandscape",
            Self::SystemContext => "SystemContext",
            Self::Container => "Container",
            Self::Component => "Component",
            Self::Dynamic => "Dynamic",
            Self::Deployment => "Deployment",
            Self::Filtered => "Filtered",
            Self::Custom => "Custom",
            Self::Image => "Image",
            Self::Key => "Key",
            Self::Unknown => "Unknown",
        }
    }

    pub(crate) fn parse(label: &str) -> Self {
        match label {
            "SystemLandscape" => Self::SystemLandscape,
            "SystemContext" => Self::SystemContext,
            "Container" => Self::Container,
            "Component" => Self::Component,
            "Dynamic" => Self::Dynamic,
            "Deployment" => Self::Deployment,
            "Filtered" => Self::Filtered,
            "Custom" => Self::Custom,
            "Image" => Self::Image,
            "Key" => Self::Key,
            _ => Self::Unknown,
        }
    }

    pub(crate) const fn is_legend(self) -> bool {
        matches!(self, Self::Key)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ViewInfo {
    pub key: String,
    pub name: String,
    pub kind: ViewKind,
    pub description: Option<String>,
    pub svg_path: PathBuf,
    pub element_ids: HashSet<ElementId>,
    pub child_view_keys_by_element_id: HashMap<ElementId, Vec<String>>,
    pub primary_view_key: Option<String>,
    pub key_view_key: Option<String>,
}

pub(crate) fn discover_views(exported: &ExportedWorkspace) -> Result<Vec<ViewInfo>> {
    let mut svg_files = list_svg_files(exported.output_dir())?;
    svg_files.sort();

    let metadata = exported
        .workspace_json()
        .and_then(|path| ViewMetadata::load(path).ok());

    let mut views = Vec::with_capacity(svg_files.len());
    for svg_path in svg_files {
        let stem = svg_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("view")
            .to_owned();
        let is_legend = stem.ends_with("-key") || stem.ends_with("_key");
        let primary_stem = if is_legend {
            Some(
                stem.trim_end_matches("-key")
                    .trim_end_matches("_key")
                    .to_owned(),
            )
        } else {
            None
        };
        let meta_match_stem = primary_stem.clone().unwrap_or_else(|| stem.clone());
        let meta = metadata
            .as_ref()
            .and_then(|m| m.find_for_svg_stem(&meta_match_stem));
        let kind = if is_legend {
            ViewKind::Key
        } else {
            meta.map_or(ViewKind::Unknown, |m| ViewKind::parse(&m.view_type))
        };
        let display_name = if is_legend {
            meta.map(|m| format!("{} (key)", m.name))
                .unwrap_or_else(|| stem.replace('_', " "))
        } else {
            meta.map_or_else(|| stem.replace('_', " "), |m| m.name.clone())
        };
        views.push(ViewInfo {
            key: stem.clone(),
            name: display_name,
            kind,
            description: meta.and_then(|m| m.description.clone()),
            svg_path,
            element_ids: meta.map(|m| m.element_ids.clone()).unwrap_or_default(),
            child_view_keys_by_element_id: HashMap::new(),
            primary_view_key: primary_stem,
            key_view_key: None,
        });
    }

    wire_child_views(&mut views, metadata.as_ref());
    wire_key_views(&mut views);

    Ok(views)
}

fn wire_key_views(views: &mut [ViewInfo]) {
    let primary_keys: Vec<(usize, String)> = views
        .iter()
        .enumerate()
        .filter(|(_, v)| v.kind == ViewKind::Key)
        .filter_map(|(i, v)| v.primary_view_key.clone().map(|k| (i, k)))
        .collect();
    for (key_idx, primary_stem) in primary_keys {
        if let Some(primary_idx) = views.iter().position(|v| v.key == primary_stem) {
            let key_key = views[key_idx].key.clone();
            views[primary_idx].key_view_key = Some(key_key);
        }
    }
}

fn wire_child_views(views: &mut [ViewInfo], metadata: Option<&ViewMetadata>) {
    let Some(metadata) = metadata else {
        return;
    };

    let view_key_to_index = views
        .iter()
        .enumerate()
        .map(|(index, view)| (normalize_view_key(&view.key), index))
        .collect::<HashMap<_, _>>();

    let mut child_specs = Vec::new();
    for entry in &metadata.views {
        if let Some(parent_element_id) = &entry.parent_element_id {
            if let Some(child_index) = view_key_to_index.get(&normalize_view_key(&entry.key)) {
                child_specs.push((parent_element_id.clone(), *child_index));
            }
        }
    }

    for (parent_element_id, child_index) in child_specs {
        let child_key = views[child_index].key.clone();
        let parent_indices = views
            .iter()
            .enumerate()
            .filter_map(|(parent_index, parent_view)| {
                (parent_index != child_index
                    && parent_view.element_ids.contains(&parent_element_id))
                .then_some(parent_index)
            })
            .collect::<Vec<_>>();
        for parent_index in parent_indices {
            views[parent_index]
                .child_view_keys_by_element_id
                .entry(parent_element_id.clone())
                .or_default()
                .push(child_key.clone());
        }
    }
}

fn list_svg_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut svg_files = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "svg") {
            svg_files.push(path);
        }
    }
    Ok(svg_files)
}

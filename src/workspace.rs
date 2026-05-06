use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

#[derive(Debug)]
pub struct WorkspaceSource {
    pub path: PathBuf,
}

#[derive(Debug)]
pub struct ExportedWorkspace {
    _temp_dir: TempDir,
    output_dir: PathBuf,
    workspace_json: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct ViewInfo {
    pub key: String,
    pub name: String,
    pub view_type: String,
    pub svg_path: PathBuf,
    pub element_ids: HashSet<String>,
    pub child_view_by_element_id: HashMap<String, String>,
}

pub fn resolve_workspace(input: &Path) -> Result<WorkspaceSource> {
    let path = if input.is_dir() {
        let dsl = input.join("workspace.dsl");
        let json = input.join("workspace.json");
        if dsl.exists() {
            dsl
        } else if json.exists() {
            json
        } else {
            bail!(
                "workspace directory {} contains neither workspace.dsl nor workspace.json",
                input.display()
            );
        }
    } else {
        input.to_path_buf()
    };

    if !path.exists() {
        bail!("workspace file not found: {}", path.display());
    }

    Ok(WorkspaceSource { path })
}

pub fn export_workspace(
    workspace: &WorkspaceSource,
    structurizr_cli: &Path,
    svg_format: &str,
) -> Result<ExportedWorkspace> {
    let temp_dir = TempDir::new().context("failed to create temporary export directory")?;
    let output_dir = temp_dir.path().to_path_buf();

    let output = Command::new(structurizr_cli)
        .arg("export")
        .arg("-workspace")
        .arg(&workspace.path)
        .arg("-format")
        .arg(svg_format)
        .arg("-output")
        .arg(&output_dir)
        .output()
        .with_context(|| format!("failed to run {}", structurizr_cli.display()))?;

    if !output.status.success() {
        bail!(
            "structurizr-cli export failed with status {}\n{}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let workspace_json = find_workspace_json(&workspace.path, &output_dir);

    Ok(ExportedWorkspace {
        _temp_dir: temp_dir,
        output_dir,
        workspace_json,
    })
}

fn find_workspace_json(workspace_path: &Path, output_dir: &Path) -> Option<PathBuf> {
    let candidates = [
        output_dir.join("workspace.json"),
        workspace_path.with_extension("json"),
        workspace_path.to_path_buf(),
    ];

    candidates.into_iter().find(|path| {
        path.file_name()
            .is_some_and(|name| name == "workspace.json")
            && path.exists()
    })
}

pub fn discover_views(exported: &ExportedWorkspace) -> Result<Vec<ViewInfo>> {
    let mut svg_files = list_svg_files(&exported.output_dir)?;
    svg_files.sort();

    let metadata = exported
        .workspace_json
        .as_deref()
        .and_then(|path| ViewMetadata::load(path).ok());

    let mut views = Vec::with_capacity(svg_files.len());
    for svg_path in svg_files {
        let stem = svg_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("view")
            .to_owned();
        let meta = metadata.as_ref().and_then(|m| m.find_for_svg_stem(&stem));
        views.push(ViewInfo {
            key: meta.map(|m| m.key.clone()).unwrap_or_else(|| stem.clone()),
            name: meta
                .map(|m| m.name.clone())
                .unwrap_or_else(|| stem.replace('_', " ")),
            view_type: meta
                .map(|m| m.view_type.clone())
                .unwrap_or_else(|| "Unknown".to_owned()),
            svg_path,
            element_ids: meta.map(|m| m.element_ids.clone()).unwrap_or_default(),
            child_view_by_element_id: HashMap::new(),
        });
    }

    wire_child_views(&mut views, metadata.as_ref());

    Ok(views)
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
                .child_view_by_element_id
                .entry(parent_element_id.clone())
                .or_insert_with(|| child_key.clone());
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

#[derive(Debug)]
struct ViewMetadata {
    views: Vec<ViewMetadataEntry>,
}

#[derive(Debug)]
struct ViewMetadataEntry {
    key: String,
    name: String,
    view_type: String,
    element_ids: HashSet<String>,
    parent_element_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StructurizrWorkspaceJson {
    views: Option<StructurizrViewsJson>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StructurizrViewsJson {
    system_landscape_views: Option<Vec<StructurizrViewJson>>,
    system_context_views: Option<Vec<StructurizrViewJson>>,
    container_views: Option<Vec<StructurizrViewJson>>,
    component_views: Option<Vec<StructurizrViewJson>>,
    dynamic_views: Option<Vec<StructurizrViewJson>>,
    deployment_views: Option<Vec<StructurizrViewJson>>,
    filtered_views: Option<Vec<StructurizrViewJson>>,
    custom_views: Option<Vec<StructurizrViewJson>>,
    image_views: Option<Vec<StructurizrViewJson>>,
}

#[derive(Debug, Deserialize)]
struct StructurizrViewJson {
    key: Option<String>,
    name: Option<String>,
    title: Option<String>,
    #[serde(default)]
    elements: Vec<StructurizrViewElementJson>,
    #[serde(default, rename = "softwareSystemId")]
    software_system_id: Option<String>,
    #[serde(default, rename = "containerId")]
    container_id: Option<String>,
    #[serde(default, rename = "componentId")]
    component_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StructurizrViewElementJson {
    id: String,
}

impl ViewMetadata {
    fn load(path: &Path) -> Result<Self> {
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

    fn find_for_svg_stem(&self, stem: &str) -> Option<&ViewMetadataEntry> {
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
        let name = view.title.or(view.name).unwrap_or_else(|| key.clone());
        let parent_element_id = view
            .container_id
            .clone()
            .or_else(|| view.component_id.clone())
            .or_else(|| view.software_system_id.clone());
        let element_ids = view
            .elements
            .into_iter()
            .map(|element| element.id)
            .collect();
        entries.push(ViewMetadataEntry {
            key,
            name,
            view_type: view_type.to_owned(),
            element_ids,
            parent_element_id,
        });
    }
}

fn normalize_view_key(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_workspace_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspace.dsl");
        fs::write(&path, "workspace {}\n").unwrap();
        assert_eq!(resolve_workspace(&path).unwrap().path, path);
    }

    #[test]
    fn prefers_dsl_over_json_in_workspace_directory() {
        let dir = tempfile::tempdir().unwrap();
        let dsl = dir.path().join("workspace.dsl");
        let json = dir.path().join("workspace.json");
        fs::write(&dsl, "workspace {}\n").unwrap();
        fs::write(json, "{}\n").unwrap();
        assert_eq!(resolve_workspace(dir.path()).unwrap().path, dsl);
    }

    #[test]
    fn parses_workspace_view_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let json = dir.path().join("workspace.json");
        fs::write(
            &json,
            r#"{
              "views": {
                "systemLandscapeViews": [{"key":"landscape", "title":"Landscape"}],
                "containerViews": [{"key":"containers", "name":"Containers"}]
              }
            }"#,
        )
        .unwrap();

        let metadata = ViewMetadata::load(&json).unwrap();
        assert_eq!(metadata.views.len(), 2);
        assert_eq!(
            metadata.find_for_svg_stem("landscape").unwrap().name,
            "Landscape"
        );
        assert_eq!(
            metadata.find_for_svg_stem("containers").unwrap().view_type,
            "Container"
        );
    }

    #[test]
    fn wires_child_views_from_workspace_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let json = dir.path().join("workspace.json");
        fs::write(
            &json,
            r#"{
              "views": {
                "systemLandscapeViews": [{"key":"landscape", "elements":[{"id":"1"}]}],
                "systemContextViews": [{"key":"system", "softwareSystemId":"1", "elements":[{"id":"2"}]}],
                "containerViews": [{"key":"containers", "softwareSystemId":"1", "elements":[{"id":"2"}]}]
              }
            }"#,
        )
        .unwrap();
        fs::write(dir.path().join("landscape.svg"), "<svg />").unwrap();
        fs::write(dir.path().join("system.svg"), "<svg />").unwrap();
        fs::write(dir.path().join("containers.svg"), "<svg />").unwrap();

        let exported = ExportedWorkspace {
            _temp_dir: tempfile::tempdir().unwrap(),
            output_dir: dir.path().to_path_buf(),
            workspace_json: Some(json),
        };
        let views = discover_views(&exported).unwrap();
        let landscape = views.iter().find(|view| view.key == "landscape").unwrap();
        assert_eq!(
            landscape.child_view_by_element_id.get("1"),
            Some(&"system".to_owned())
        );
    }
}

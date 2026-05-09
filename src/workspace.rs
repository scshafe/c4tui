use crate::ids::ElementId;
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

#[derive(Debug, Clone, Default)]
pub struct WorkspaceModel {
    pub elements: HashMap<ElementId, ElementMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementMetadata {
    pub id: ElementId,
    pub name: String,
    pub description: Option<String>,
    pub technology: Option<String>,
    pub tags: Vec<String>,
    pub kind: ElementKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementKind {
    Person,
    SoftwareSystem,
    Container,
    Component,
    DeploymentNode,
    InfrastructureNode,
    SoftwareSystemInstance,
    ContainerInstance,
}

impl ElementKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Person => "Person",
            Self::SoftwareSystem => "Software System",
            Self::Container => "Container",
            Self::Component => "Component",
            Self::DeploymentNode => "Deployment Node",
            Self::InfrastructureNode => "Infrastructure Node",
            Self::SoftwareSystemInstance => "Software System Instance",
            Self::ContainerInstance => "Container Instance",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewKind {
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
    pub fn label(self) -> &'static str {
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

    pub fn parse(label: &str) -> Self {
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

    pub const fn is_legend(self) -> bool {
        matches!(self, Self::Key)
    }
}

#[derive(Debug, Clone)]
pub struct ViewInfo {
    pub key: String,
    pub name: String,
    pub kind: ViewKind,
    pub description: Option<String>,
    pub svg_path: PathBuf,
    pub element_ids: HashSet<ElementId>,
    pub child_view_by_element_id: HashMap<ElementId, String>,
    pub primary_view_key: Option<String>,
    pub key_view_key: Option<String>,
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

pub fn load_workspace_model(exported: &ExportedWorkspace) -> WorkspaceModel {
    let Some(path) = exported.workspace_json.as_deref() else {
        return WorkspaceModel::default();
    };
    let Ok(text) = fs::read_to_string(path) else {
        return WorkspaceModel::default();
    };
    let Ok(raw): std::result::Result<StructurizrWorkspaceJson, _> = serde_json::from_str(&text)
    else {
        return WorkspaceModel::default();
    };
    let mut elements = HashMap::new();
    if let Some(model) = raw.model {
        for person in model.people.unwrap_or_default() {
            insert_element(&mut elements, person, ElementKind::Person);
        }
        for system in model.software_systems.unwrap_or_default() {
            for container in system.containers.clone().unwrap_or_default() {
                for component in container.components.clone().unwrap_or_default() {
                    insert_element(&mut elements, component, ElementKind::Component);
                }
                insert_element(&mut elements, container, ElementKind::Container);
            }
            insert_element(&mut elements, system, ElementKind::SoftwareSystem);
        }
        for node in model.deployment_nodes.unwrap_or_default() {
            collect_deployment(&mut elements, node);
        }
    }
    WorkspaceModel { elements }
}

fn collect_deployment(
    elements: &mut HashMap<ElementId, ElementMetadata>,
    node: StructurizrNodeJson,
) {
    for instance in node.software_system_instances.clone().unwrap_or_default() {
        insert_element(elements, instance, ElementKind::SoftwareSystemInstance);
    }
    for instance in node.container_instances.clone().unwrap_or_default() {
        insert_element(elements, instance, ElementKind::ContainerInstance);
    }
    for child in node.children.clone().unwrap_or_default() {
        collect_deployment(elements, child);
    }
    for infra in node.infrastructure_nodes.clone().unwrap_or_default() {
        insert_element(elements, infra, ElementKind::InfrastructureNode);
    }
    insert_element(elements, node, ElementKind::DeploymentNode);
}

fn insert_element<T: ElementJson>(
    map: &mut HashMap<ElementId, ElementMetadata>,
    raw: T,
    kind: ElementKind,
) {
    let id = ElementId::new(raw.id());
    let entry = ElementMetadata {
        id: id.clone(),
        name: raw.name().to_owned(),
        description: raw
            .description()
            .map(str::to_owned)
            .filter(|s| !s.is_empty()),
        technology: raw
            .technology()
            .map(str::to_owned)
            .filter(|s| !s.is_empty()),
        tags: raw
            .tags_str()
            .map(|s| {
                s.split(',')
                    .map(|t| t.trim().to_owned())
                    .filter(|t| !t.is_empty())
                    .collect()
            })
            .unwrap_or_default(),
        kind,
    };
    map.insert(id, entry);
}

trait ElementJson {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn description(&self) -> Option<&str>;
    fn technology(&self) -> Option<&str>;
    fn tags_str(&self) -> Option<&str>;
}

impl ElementJson for StructurizrElementJson {
    fn id(&self) -> &str {
        self.id.as_deref().unwrap_or("")
    }
    fn name(&self) -> &str {
        self.name.as_deref().unwrap_or("")
    }
    fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }
    fn technology(&self) -> Option<&str> {
        self.technology.as_deref()
    }
    fn tags_str(&self) -> Option<&str> {
        self.tags.as_deref()
    }
}

impl ElementJson for StructurizrSystemJson {
    fn id(&self) -> &str {
        self.id.as_deref().unwrap_or("")
    }
    fn name(&self) -> &str {
        self.name.as_deref().unwrap_or("")
    }
    fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }
    fn technology(&self) -> Option<&str> {
        None
    }
    fn tags_str(&self) -> Option<&str> {
        self.tags.as_deref()
    }
}

impl ElementJson for StructurizrContainerJson {
    fn id(&self) -> &str {
        self.id.as_deref().unwrap_or("")
    }
    fn name(&self) -> &str {
        self.name.as_deref().unwrap_or("")
    }
    fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }
    fn technology(&self) -> Option<&str> {
        self.technology.as_deref()
    }
    fn tags_str(&self) -> Option<&str> {
        self.tags.as_deref()
    }
}

impl ElementJson for StructurizrNodeJson {
    fn id(&self) -> &str {
        self.id.as_deref().unwrap_or("")
    }
    fn name(&self) -> &str {
        self.name.as_deref().unwrap_or("")
    }
    fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }
    fn technology(&self) -> Option<&str> {
        self.technology.as_deref()
    }
    fn tags_str(&self) -> Option<&str> {
        self.tags.as_deref()
    }
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
            child_view_by_element_id: HashMap::new(),
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
    description: Option<String>,
    element_ids: HashSet<ElementId>,
    parent_element_id: Option<ElementId>,
}

#[derive(Debug, Deserialize)]
struct StructurizrWorkspaceJson {
    views: Option<StructurizrViewsJson>,
    model: Option<StructurizrModelJson>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StructurizrModelJson {
    people: Option<Vec<StructurizrElementJson>>,
    software_systems: Option<Vec<StructurizrSystemJson>>,
    deployment_nodes: Option<Vec<StructurizrNodeJson>>,
}

#[derive(Debug, Deserialize, Clone)]
struct StructurizrElementJson {
    id: Option<String>,
    name: Option<String>,
    description: Option<String>,
    technology: Option<String>,
    tags: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct StructurizrSystemJson {
    id: Option<String>,
    name: Option<String>,
    description: Option<String>,
    tags: Option<String>,
    containers: Option<Vec<StructurizrContainerJson>>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct StructurizrContainerJson {
    id: Option<String>,
    name: Option<String>,
    description: Option<String>,
    technology: Option<String>,
    tags: Option<String>,
    components: Option<Vec<StructurizrElementJson>>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct StructurizrNodeJson {
    id: Option<String>,
    name: Option<String>,
    description: Option<String>,
    technology: Option<String>,
    tags: Option<String>,
    children: Option<Vec<StructurizrNodeJson>>,
    infrastructure_nodes: Option<Vec<StructurizrElementJson>>,
    software_system_instances: Option<Vec<StructurizrElementJson>>,
    container_instances: Option<Vec<StructurizrElementJson>>,
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
    description: Option<String>,
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
        let name = view
            .title
            .clone()
            .or(view.name.clone())
            .unwrap_or_else(|| key.clone());
        let parent_element_id = view
            .container_id
            .clone()
            .or_else(|| view.component_id.clone())
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

fn normalize_view_key(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_alphanumeric)
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

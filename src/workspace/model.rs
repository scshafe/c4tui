use std::collections::HashMap;
use std::fs;

use crate::ids::{ElementId, RelationshipId};

use super::export::ExportedWorkspace;
use super::json::{
    StructurizrContainerJson, StructurizrElementJson, StructurizrNodeJson,
    StructurizrRelationshipJson, StructurizrSystemJson, StructurizrWorkspaceJson,
};

#[derive(Debug, Clone, Default)]
pub struct WorkspaceModel {
    pub elements: HashMap<ElementId, ElementMetadata>,
    pub relationships: HashMap<RelationshipId, RelationshipMetadata>,
    pub outgoing_relationships_by_element: HashMap<ElementId, Vec<RelationshipId>>,
    pub incoming_relationships_by_element: HashMap<ElementId, Vec<RelationshipId>>,
}

impl WorkspaceModel {
    #[cfg(test)]
    pub fn connection_counts(&self, element_id: &ElementId) -> ConnectionCounts {
        ConnectionCounts {
            outgoing: self.outgoing_relationships(element_id).count(),
            incoming: self.incoming_relationships(element_id).count(),
        }
    }

    pub fn outgoing_relationships(
        &self,
        element_id: &ElementId,
    ) -> impl Iterator<Item = &RelationshipMetadata> {
        self.outgoing_relationships_by_element
            .get(element_id)
            .into_iter()
            .flatten()
            .filter_map(|relationship_id| self.relationships.get(relationship_id))
    }

    pub fn incoming_relationships(
        &self,
        element_id: &ElementId,
    ) -> impl Iterator<Item = &RelationshipMetadata> {
        self.incoming_relationships_by_element
            .get(element_id)
            .into_iter()
            .flatten()
            .filter_map(|relationship_id| self.relationships.get(relationship_id))
    }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationshipMetadata {
    pub id: RelationshipId,
    pub source_id: ElementId,
    pub destination_id: ElementId,
    pub description: Option<String>,
    pub technology: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConnectionCounts {
    pub outgoing: usize,
    pub incoming: usize,
}

impl ConnectionCounts {
    pub const fn total(self) -> usize {
        self.outgoing + self.incoming
    }
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

pub fn load_workspace_model(exported: &ExportedWorkspace) -> WorkspaceModel {
    let Some(path) = exported.workspace_json() else {
        return WorkspaceModel::default();
    };
    let Ok(text) = fs::read_to_string(path) else {
        return WorkspaceModel::default();
    };
    let Ok(raw): std::result::Result<StructurizrWorkspaceJson, _> = serde_json::from_str(&text)
    else {
        return WorkspaceModel::default();
    };
    let mut workspace_model = WorkspaceModel::default();
    if let Some(raw_model) = raw.model {
        for person in raw_model.people.unwrap_or_default() {
            collect_element(&mut workspace_model, person, ElementKind::Person);
        }
        for system in raw_model.software_systems.unwrap_or_default() {
            for container in system.containers.clone().unwrap_or_default() {
                for component in container.components.clone().unwrap_or_default() {
                    collect_element(&mut workspace_model, component, ElementKind::Component);
                }
                collect_element(&mut workspace_model, container, ElementKind::Container);
            }
            collect_element(&mut workspace_model, system, ElementKind::SoftwareSystem);
        }
        for node in raw_model.deployment_nodes.unwrap_or_default() {
            collect_deployment(&mut workspace_model, node);
        }
    }
    workspace_model
}

fn collect_deployment(model: &mut WorkspaceModel, node: StructurizrNodeJson) {
    for instance in node.software_system_instances.clone().unwrap_or_default() {
        collect_element(model, instance, ElementKind::SoftwareSystemInstance);
    }
    for instance in node.container_instances.clone().unwrap_or_default() {
        collect_element(model, instance, ElementKind::ContainerInstance);
    }
    for child in node.children.clone().unwrap_or_default() {
        collect_deployment(model, child);
    }
    for infra in node.infrastructure_nodes.clone().unwrap_or_default() {
        collect_element(model, infra, ElementKind::InfrastructureNode);
    }
    collect_element(model, node, ElementKind::DeploymentNode);
}

fn collect_element<T: ElementJson>(model: &mut WorkspaceModel, raw: T, kind: ElementKind) {
    let source_id = insert_element(&mut model.elements, &raw, kind);
    for relationship in raw.relationships() {
        insert_relationship(model, &source_id, relationship);
    }
}

fn insert_element<T: ElementJson>(
    map: &mut HashMap<ElementId, ElementMetadata>,
    raw: &T,
    kind: ElementKind,
) -> ElementId {
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
        tags: parse_tags(raw.tags_str()),
        kind,
    };
    map.insert(id, entry);
    ElementId::new(raw.id())
}

fn insert_relationship(
    model: &mut WorkspaceModel,
    source_id: &ElementId,
    raw: StructurizrRelationshipJson,
) {
    let Some(destination_id) = raw
        .destination_id
        .filter(|destination_id| !destination_id.is_empty())
        .map(ElementId::new)
    else {
        return;
    };
    let source_id = raw
        .source_id
        .filter(|source_id| !source_id.is_empty())
        .map(ElementId::new)
        .unwrap_or_else(|| source_id.clone());
    let relationship_id = raw
        .id
        .filter(|id| !id.is_empty())
        .map(RelationshipId::new)
        .unwrap_or_else(|| {
            RelationshipId::new(format!(
                "{}->{}#{}",
                source_id,
                destination_id,
                model.relationships.len() + 1
            ))
        });
    if model.relationships.contains_key(&relationship_id) {
        return;
    }
    let relationship = RelationshipMetadata {
        id: relationship_id.clone(),
        source_id: source_id.clone(),
        destination_id: destination_id.clone(),
        description: raw
            .description
            .filter(|description| !description.is_empty()),
        technology: raw.technology.filter(|technology| !technology.is_empty()),
        tags: parse_tags(raw.tags.as_deref()),
    };
    model
        .outgoing_relationships_by_element
        .entry(source_id)
        .or_default()
        .push(relationship_id.clone());
    model
        .incoming_relationships_by_element
        .entry(destination_id)
        .or_default()
        .push(relationship_id.clone());
    model.relationships.insert(relationship_id, relationship);
}

fn parse_tags(tags: Option<&str>) -> Vec<String> {
    tags.map(|s| {
        s.split(',')
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
            .collect()
    })
    .unwrap_or_default()
}

trait ElementJson {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn description(&self) -> Option<&str>;
    fn technology(&self) -> Option<&str>;
    fn tags_str(&self) -> Option<&str>;
    fn relationships(&self) -> Vec<StructurizrRelationshipJson>;
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
    fn relationships(&self) -> Vec<StructurizrRelationshipJson> {
        self.relationships.clone().unwrap_or_default()
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
    fn relationships(&self) -> Vec<StructurizrRelationshipJson> {
        self.relationships.clone().unwrap_or_default()
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
    fn relationships(&self) -> Vec<StructurizrRelationshipJson> {
        self.relationships.clone().unwrap_or_default()
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
    fn relationships(&self) -> Vec<StructurizrRelationshipJson> {
        self.relationships.clone().unwrap_or_default()
    }
}

use crate::ids::{ElementId, RelationshipId};
use crate::workspace::{ElementKind, ElementMetadata, RelationshipMetadata, WorkspaceModel};

pub(crate) fn element(id: &str, name: &str, kind: ElementKind) -> (ElementId, ElementMetadata) {
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

pub(crate) fn container(id: &str, name: &str) -> (ElementId, ElementMetadata) {
    element(id, name, ElementKind::Container)
}

pub(crate) fn relationship(id: &str, source: &str, destination: &str) -> RelationshipMetadata {
    RelationshipMetadata {
        id: RelationshipId::new(id),
        source_id: ElementId::new(source),
        destination_id: ElementId::new(destination),
        description: None,
        technology: None,
        tags: Vec::new(),
    }
}

#[derive(Debug, Default)]
pub(crate) struct TestModelBuilder {
    model: WorkspaceModel,
}

impl TestModelBuilder {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn element(mut self, id: &str, name: &str, kind: ElementKind) -> Self {
        let (id, element) = element(id, name, kind);
        self.model.elements.insert(id, element);
        self
    }

    pub(crate) fn container(mut self, id: &str, name: &str) -> Self {
        let (id, element) = container(id, name);
        self.model.elements.insert(id, element);
        self
    }

    pub(crate) fn relationship(mut self, relationship: RelationshipMetadata) -> Self {
        self.model
            .relationships
            .insert(relationship.id.clone(), relationship);
        self
    }

    pub(crate) fn outgoing(mut self, element_id: &str, relationship_ids: &[&str]) -> Self {
        self.model.outgoing_relationships_by_element.insert(
            ElementId::new(element_id),
            relationship_ids
                .iter()
                .copied()
                .map(RelationshipId::new)
                .collect(),
        );
        self
    }

    pub(crate) fn incoming(mut self, element_id: &str, relationship_ids: &[&str]) -> Self {
        self.model.incoming_relationships_by_element.insert(
            ElementId::new(element_id),
            relationship_ids
                .iter()
                .copied()
                .map(RelationshipId::new)
                .collect(),
        );
        self
    }

    pub(crate) fn build(self) -> WorkspaceModel {
        self.model
    }
}

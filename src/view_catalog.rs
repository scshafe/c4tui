use crate::ids::{ElementId, RelationshipId, ViewId};
use crate::workspace::{
    ConnectionCounts, ElementMetadata, ExportedWorkspace, ViewInfo, WorkspaceModel,
};
use anyhow::{bail, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConnectionDirection {
    Outgoing,
    Incoming,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConnectionNavigationCandidate {
    pub relationship_id: RelationshipId,
    pub direction: ConnectionDirection,
    pub connected_element_id: ElementId,
    pub view_id: ViewId,
}

#[derive(Debug)]
pub(crate) struct ViewCatalog {
    pub views: Vec<ViewInfo>,
    pub model: WorkspaceModel,
    export_guard: Option<ExportedWorkspace>,
}

fn direction_rank(direction: ConnectionDirection) -> u8 {
    match direction {
        ConnectionDirection::Outgoing => 0,
        ConnectionDirection::Incoming => 1,
    }
}

impl ViewCatalog {
    pub(crate) fn new(views: Vec<ViewInfo>) -> Result<Self> {
        if views.is_empty() {
            bail!("no exported SVG views were found");
        }

        Ok(Self {
            views,
            model: WorkspaceModel::default(),
            export_guard: None,
        })
    }

    pub(crate) fn with_model(mut self, model: WorkspaceModel) -> Self {
        self.model = model;
        self
    }

    pub(crate) fn with_export(mut self, export: ExportedWorkspace) -> Self {
        self.export_guard = Some(export);
        self
    }

    pub(crate) fn view(&self, id: ViewId) -> &ViewInfo {
        &self.views[id.index()]
    }

    pub(crate) fn view_count(&self) -> usize {
        self.views.len()
    }

    pub(crate) fn view_id_by_key(&self, key: &str) -> Option<ViewId> {
        self.views
            .iter()
            .position(|view| view.key == key)
            .map(ViewId::new)
    }

    pub(crate) fn element_metadata(&self, id: &ElementId) -> Option<&ElementMetadata> {
        self.model.elements.get(id)
    }

    pub(crate) fn connection_candidates_for_element(
        &self,
        current: ViewId,
        element_id: &ElementId,
    ) -> Vec<ConnectionNavigationCandidate> {
        let outgoing = self
            .model
            .outgoing_relationships(element_id)
            .flat_map(|rel| {
                self.candidate_views_for_element(current, &rel.destination_id)
                    .into_iter()
                    .map(move |view_id| ConnectionNavigationCandidate {
                        relationship_id: rel.id.clone(),
                        direction: ConnectionDirection::Outgoing,
                        connected_element_id: rel.destination_id.clone(),
                        view_id,
                    })
            });
        let incoming = self
            .model
            .incoming_relationships(element_id)
            .flat_map(|rel| {
                self.candidate_views_for_element(current, &rel.source_id)
                    .into_iter()
                    .map(move |view_id| ConnectionNavigationCandidate {
                        relationship_id: rel.id.clone(),
                        direction: ConnectionDirection::Incoming,
                        connected_element_id: rel.source_id.clone(),
                        view_id,
                    })
            });

        let mut candidates = outgoing.chain(incoming).collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            direction_rank(left.direction)
                .cmp(&direction_rank(right.direction))
                .then_with(|| {
                    self.element_label(&left.connected_element_id)
                        .cmp(self.element_label(&right.connected_element_id))
                })
                .then_with(|| {
                    self.view(left.view_id)
                        .name
                        .cmp(&self.view(right.view_id).name)
                })
                .then_with(|| left.relationship_id.cmp(&right.relationship_id))
        });
        candidates.dedup_by(|left, right| {
            left.direction == right.direction
                && left.connected_element_id == right.connected_element_id
                && left.view_id == right.view_id
        });
        candidates
    }

    pub(crate) fn connection_candidate_counts_for_element(
        &self,
        current: ViewId,
        element_id: &ElementId,
    ) -> ConnectionCounts {
        self.connection_candidates_for_element(current, element_id)
            .into_iter()
            .fold(ConnectionCounts::default(), |mut counts, candidate| {
                match candidate.direction {
                    ConnectionDirection::Outgoing => counts.outgoing += 1,
                    ConnectionDirection::Incoming => counts.incoming += 1,
                }
                counts
            })
    }

    fn candidate_views_for_element(&self, current: ViewId, element_id: &ElementId) -> Vec<ViewId> {
        self.views
            .iter()
            .enumerate()
            .filter_map(|(index, view)| {
                let view_id = ViewId::new(index);
                (view_id != current
                    && !view.kind.is_legend()
                    && view.element_ids.contains(element_id))
                .then_some(view_id)
            })
            .collect()
    }

    fn element_label<'a>(&'a self, element_id: &'a ElementId) -> &'a str {
        self.model
            .elements
            .get(element_id)
            .map(|element| element.name.as_str())
            .unwrap_or_else(|| element_id.as_str())
    }

    pub(crate) fn child_view_ids_for_element(
        &self,
        current: ViewId,
        element_id: &ElementId,
    ) -> Vec<ViewId> {
        self.views[current.index()]
            .child_view_keys_by_element_id
            .get(element_id)
            .into_iter()
            .flatten()
            .filter_map(|child_key| self.view_id_by_key(child_key))
            .collect()
    }
}

pub(crate) fn image_id_for_view(id: ViewId) -> u32 {
    (id.index() as u32) + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::catalog::TestViewBuilder;
    use crate::test_support::workspace::{relationship, TestModelBuilder};
    use crate::workspace::ViewKind;
    use std::fs;

    fn view_with_elements(key: &str, kind: ViewKind, ids: &[&str]) -> ViewInfo {
        TestViewBuilder::new(key)
            .kind(kind)
            .elements(ids.iter().copied())
            .build()
    }

    #[test]
    fn creates_catalog_for_non_empty_views() {
        let dir = tempfile::tempdir().unwrap();
        let svg = dir.path().join("landscape.svg");
        fs::write(&svg, "<svg />").unwrap();
        let catalog = ViewCatalog::new(vec![TestViewBuilder::new("landscape")
            .named("Landscape")
            .kind(ViewKind::SystemLandscape)
            .svg_path(svg)
            .build()])
        .unwrap();

        assert_eq!(catalog.views.len(), 1);
        assert_eq!(image_id_for_view(ViewId::first()), 1);
    }

    #[test]
    fn rejects_empty_catalog() {
        assert!(ViewCatalog::new(Vec::new()).is_err());
    }

    #[test]
    fn connection_candidates_follow_outgoing_and_incoming_relationships_to_views() {
        let model = TestModelBuilder::new()
            .relationship(relationship("r1", "api", "database"))
            .relationship(relationship("r2", "browser", "api"))
            .relationship(relationship("r3", "api", "cache"))
            .relationship(relationship("r4", "api", "database"))
            .outgoing("api", &["r1", "r3", "r4"])
            .incoming("api", &["r2"])
            .build();

        let catalog = ViewCatalog::new(vec![
            view_with_elements(
                "current",
                ViewKind::Container,
                &["api", "database", "browser"],
            ),
            view_with_elements("database-view", ViewKind::Component, &["database"]),
            view_with_elements("browser-view", ViewKind::SystemContext, &["browser"]),
            view_with_elements("database-key", ViewKind::Key, &["database"]),
        ])
        .unwrap()
        .with_model(model);

        let candidates =
            catalog.connection_candidates_for_element(ViewId::first(), &ElementId::new("api"));

        assert_eq!(
            candidates,
            vec![
                ConnectionNavigationCandidate {
                    relationship_id: RelationshipId::new("r1"),
                    direction: ConnectionDirection::Outgoing,
                    connected_element_id: ElementId::new("database"),
                    view_id: ViewId::new(1),
                },
                ConnectionNavigationCandidate {
                    relationship_id: RelationshipId::new("r2"),
                    direction: ConnectionDirection::Incoming,
                    connected_element_id: ElementId::new("browser"),
                    view_id: ViewId::new(2),
                },
            ]
        );
        assert_eq!(
            catalog
                .connection_candidate_counts_for_element(ViewId::first(), &ElementId::new("api")),
            ConnectionCounts {
                outgoing: 1,
                incoming: 1
            }
        );
    }
}

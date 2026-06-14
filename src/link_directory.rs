use crate::ids::{ElementId, ViewId};
use crate::nav_picker::{truncate, NavCellCanvas, NavItem, NavRenderArtifact};
use crate::view_catalog::{ConnectionDirection, ViewCatalog};
use ratatui::style::{Modifier, Style};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    Detail,
    ConnectionOutgoing,
    ConnectionIncoming,
    Legend,
}

impl LinkKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Detail => "detail",
            Self::ConnectionOutgoing => "outgoing",
            Self::ConnectionIncoming => "incoming",
            Self::Legend => "legend",
        }
    }

    const fn rank(self) -> u8 {
        match self {
            Self::Detail => 0,
            Self::ConnectionOutgoing => 1,
            Self::ConnectionIncoming => 2,
            Self::Legend => 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkCandidate {
    pub kind: LinkKind,
    pub target_view: ViewId,
    pub source_element: Option<ElementId>,
    pub label: String,
    pub detail: Option<String>,
    pub pinned_element_after_navigation: Option<ElementId>,
    pub(crate) secondary_tokens: Vec<String>,
}

impl NavItem for LinkCandidate {
    type Output = LinkCandidate;

    fn filter_text(&self) -> &str {
        &self.label
    }

    fn secondary_filter_tokens(&self) -> &[String] {
        &self.secondary_tokens
    }

    fn render_into_canvas(
        &self,
        mut canvas: NavCellCanvas<'_, '_>,
        selected: bool,
        _filter: &str,
        _sink: &mut dyn FnMut(NavRenderArtifact),
    ) {
        let marker = if selected { ">" } else { " " };
        let title = format!("{marker} {}", self.label);
        canvas.set_string(
            0,
            0,
            truncate(&title, canvas.width().saturating_sub(1) as usize),
            canvas.style(),
        );

        if canvas.height() > 1 {
            let kind = format!("kind: {}", self.kind.label());
            canvas.set_string(
                0,
                1,
                truncate(&kind, canvas.width().saturating_sub(1) as usize),
                Style::default().add_modifier(Modifier::DIM),
            );
        }

        if canvas.height() > 2 {
            if let Some(detail) = &self.detail {
                canvas.set_string(
                    0,
                    2,
                    truncate(detail, canvas.width().saturating_sub(1) as usize),
                    Style::default(),
                );
            }
        }

        if canvas.height() > 3 {
            if let Some(element) = &self.pinned_element_after_navigation {
                let pin = format!("pin: {element}");
                canvas.set_string(
                    0,
                    3,
                    truncate(&pin, canvas.width().saturating_sub(1) as usize),
                    Style::default().add_modifier(Modifier::DIM),
                );
            }
        }
    }

    fn outcome(&self) -> LinkCandidate {
        self.clone()
    }
}

pub fn collect_link_candidates(
    catalog: &ViewCatalog,
    current: ViewId,
    pinned_element: Option<&ElementId>,
) -> Vec<LinkCandidate> {
    let mut candidates = Vec::new();
    let current_view = catalog.view(current);

    for (source_element, child_keys) in &current_view.child_view_keys_by_element_id {
        for child_key in child_keys {
            let Some(target_view) = catalog.view_id_by_key(child_key) else {
                continue;
            };
            if target_view == current {
                continue;
            }
            let source_name = element_label(catalog, source_element);
            let view = catalog.view(target_view);
            candidates.push(LinkCandidate {
                kind: LinkKind::Detail,
                target_view,
                source_element: Some(source_element.clone()),
                label: view.name.clone(),
                detail: Some(format!("detail from {source_name}")),
                pinned_element_after_navigation: None,
                secondary_tokens: vec![
                    source_name,
                    source_element.to_string(),
                    view.key.clone(),
                    view.kind.label().to_owned(),
                ],
            });
        }
    }

    if let Some(source_element) = pinned_element {
        for candidate in catalog.connection_candidates_for_element(current, source_element) {
            let kind = match candidate.direction {
                ConnectionDirection::Outgoing => LinkKind::ConnectionOutgoing,
                ConnectionDirection::Incoming => LinkKind::ConnectionIncoming,
            };
            let connected_name = element_label(catalog, &candidate.connected_element_id);
            let relationship = catalog.model.relationships.get(&candidate.relationship_id);
            let relationship_detail = match (
                relationship.and_then(|rel| rel.description.as_deref()),
                relationship.and_then(|rel| rel.technology.as_deref()),
            ) {
                (Some(description), Some(technology)) => format!("{description} ({technology})"),
                (Some(description), None) => description.to_owned(),
                (None, Some(technology)) => technology.to_owned(),
                (None, None) => "relationship".to_owned(),
            };
            let direction = match candidate.direction {
                ConnectionDirection::Outgoing => "to",
                ConnectionDirection::Incoming => "from",
            };
            let view = catalog.view(candidate.view_id);
            candidates.push(LinkCandidate {
                kind,
                target_view: candidate.view_id,
                source_element: Some(source_element.clone()),
                label: view.name.clone(),
                detail: Some(format!(
                    "{direction} {connected_name} - {relationship_detail}"
                )),
                pinned_element_after_navigation: Some(candidate.connected_element_id.clone()),
                secondary_tokens: vec![
                    connected_name,
                    candidate.connected_element_id.to_string(),
                    relationship_detail,
                    view.key.clone(),
                    view.kind.label().to_owned(),
                ],
            });
        }
    }

    if let Some(key_view_key) = &current_view.key_view_key {
        if let Some(target_view) = catalog.view_id_by_key(key_view_key) {
            if target_view != current {
                let view = catalog.view(target_view);
                candidates.push(LinkCandidate {
                    kind: LinkKind::Legend,
                    target_view,
                    source_element: None,
                    label: view.name.clone(),
                    detail: Some(format!("legend for {}", current_view.name)),
                    pinned_element_after_navigation: None,
                    secondary_tokens: vec![view.key.clone(), view.kind.label().to_owned()],
                });
            }
        }
    }

    candidates.sort_by(|left, right| {
        left.kind
            .rank()
            .cmp(&right.kind.rank())
            .then_with(|| left.label.cmp(&right.label))
            .then_with(|| left.detail.cmp(&right.detail))
            .then_with(|| left.target_view.cmp(&right.target_view))
            .then_with(|| left.source_element.cmp(&right.source_element))
            .then_with(|| {
                left.pinned_element_after_navigation
                    .cmp(&right.pinned_element_after_navigation)
            })
    });
    candidates.dedup_by(|left, right| {
        left.kind == right.kind
            && left.target_view == right.target_view
            && left.source_element == right.source_element
            && left.pinned_element_after_navigation == right.pinned_element_after_navigation
    });
    candidates
}

fn element_label(catalog: &ViewCatalog, element_id: &ElementId) -> String {
    catalog
        .model
        .elements
        .get(element_id)
        .map(|element| element.name.clone())
        .unwrap_or_else(|| element_id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{
        ElementKind, ElementMetadata, RelationshipMetadata, ViewInfo, ViewKind, WorkspaceModel,
    };

    fn element(id: &str, name: &str) -> (ElementId, ElementMetadata) {
        let id = ElementId::new(id);
        (
            id.clone(),
            ElementMetadata {
                id,
                name: name.to_owned(),
                description: None,
                technology: None,
                tags: Vec::new(),
                kind: ElementKind::Container,
            },
        )
    }

    fn relationship(id: &str, source: &str, destination: &str) -> RelationshipMetadata {
        RelationshipMetadata {
            id: crate::ids::RelationshipId::new(id),
            source_id: ElementId::new(source),
            destination_id: ElementId::new(destination),
            description: Some(format!("{source} to {destination}")),
            technology: Some("HTTPS".to_owned()),
            tags: Vec::new(),
        }
    }

    fn view(
        key: &str,
        name: &str,
        kind: ViewKind,
        elements: &[&str],
        child_links: &[(&str, &[&str])],
        key_view_key: Option<&str>,
    ) -> ViewInfo {
        ViewInfo {
            key: key.to_owned(),
            name: name.to_owned(),
            kind,
            description: None,
            svg_path: std::path::PathBuf::from(format!("{key}.svg")),
            element_ids: elements.iter().copied().map(ElementId::new).collect(),
            child_view_keys_by_element_id: child_links
                .iter()
                .map(|(element, keys)| {
                    (
                        ElementId::new(*element),
                        keys.iter().map(|key| (*key).to_owned()).collect(),
                    )
                })
                .collect(),
            primary_view_key: None,
            key_view_key: key_view_key.map(str::to_owned),
        }
    }

    fn catalog() -> ViewCatalog {
        let mut model = WorkspaceModel::default();
        for (id, element) in [
            element("api", "API"),
            element("database", "Database"),
            element("browser", "Browser"),
        ] {
            model.elements.insert(id, element);
        }
        let outgoing = relationship("r1", "api", "database");
        let incoming = relationship("r2", "browser", "api");
        model
            .outgoing_relationships_by_element
            .insert(ElementId::new("api"), vec![outgoing.id.clone()]);
        model
            .incoming_relationships_by_element
            .insert(ElementId::new("api"), vec![incoming.id.clone()]);
        model.relationships.insert(outgoing.id.clone(), outgoing);
        model.relationships.insert(incoming.id.clone(), incoming);

        ViewCatalog::new(vec![
            view(
                "current",
                "Current",
                ViewKind::Container,
                &["api", "database", "browser"],
                &[("api", &["api-detail", "api-detail"][..])],
                Some("current-key"),
            ),
            view(
                "api-detail",
                "API Detail",
                ViewKind::Component,
                &["api"],
                &[],
                None,
            ),
            view(
                "database-view",
                "Database View",
                ViewKind::Component,
                &["database"],
                &[],
                None,
            ),
            view(
                "browser-view",
                "Browser View",
                ViewKind::SystemContext,
                &["browser"],
                &[],
                None,
            ),
            view("current-key", "Current Key", ViewKind::Key, &[], &[], None),
        ])
        .unwrap()
        .with_model(model)
    }

    #[test]
    fn collects_detail_connection_and_legend_candidates() {
        let candidates =
            collect_link_candidates(&catalog(), ViewId::first(), Some(&ElementId::new("api")));

        assert_eq!(
            candidates
                .iter()
                .map(|candidate| (
                    candidate.kind,
                    candidate.target_view,
                    candidate.source_element.clone(),
                    candidate.pinned_element_after_navigation.clone(),
                ))
                .collect::<Vec<_>>(),
            vec![
                (
                    LinkKind::Detail,
                    ViewId::new(1),
                    Some(ElementId::new("api")),
                    None,
                ),
                (
                    LinkKind::ConnectionOutgoing,
                    ViewId::new(2),
                    Some(ElementId::new("api")),
                    Some(ElementId::new("database")),
                ),
                (
                    LinkKind::ConnectionIncoming,
                    ViewId::new(3),
                    Some(ElementId::new("api")),
                    Some(ElementId::new("browser")),
                ),
                (LinkKind::Legend, ViewId::new(4), None, None),
            ]
        );
    }

    #[test]
    fn omits_connection_candidates_without_pinned_element() {
        let candidates = collect_link_candidates(&catalog(), ViewId::first(), None);

        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.kind)
                .collect::<Vec<_>>(),
            vec![LinkKind::Detail, LinkKind::Legend]
        );
    }

    #[test]
    fn link_candidate_filter_tokens_include_source_and_target_context() {
        let candidates =
            collect_link_candidates(&catalog(), ViewId::first(), Some(&ElementId::new("api")));
        let detail = candidates
            .iter()
            .find(|candidate| candidate.kind == LinkKind::Detail)
            .unwrap();
        let connection = candidates
            .iter()
            .find(|candidate| candidate.kind == LinkKind::ConnectionOutgoing)
            .unwrap();

        assert!(detail.secondary_tokens.contains(&"API".to_owned()));
        assert!(detail.secondary_tokens.contains(&"api-detail".to_owned()));
        assert!(connection.secondary_tokens.contains(&"Database".to_owned()));
        assert!(connection
            .secondary_tokens
            .contains(&"api to database (HTTPS)".to_owned()));
    }
}

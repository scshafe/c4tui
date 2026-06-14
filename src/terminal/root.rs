use crate::ids::{ElementId, ViewId};
use crate::render_cache::RenderCache;
use crate::state::RenderFrame;
use crate::view_catalog::{image_id_for_view, ViewCatalog};
use crate::viewport_state::ViewportState;
use crate::workspace::ConnectionCounts;
use anyhow::Result;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::widgets::{Clear, Paragraph, Widget};
use tui_kit::image::{ImageSurface, MAIN_PLACEMENT_ID};
use tui_kit::layout::{CanvasMetrics, CellArea, CellSize};

use super::session::TerminalSession;
use super::status;

pub(super) fn render_view(
    session: &mut TerminalSession,
    frame: &RenderFrame,
    catalog: &ViewCatalog,
    cache: &mut RenderCache,
    viewport: &mut ViewportState,
) -> Result<()> {
    let canvas = session.canvas();
    let view_id = frame.current;
    let image_id = image_id_for_view(view_id);
    let placement = viewport.placement(cache, catalog, view_id, canvas)?;
    let transform = viewport.transform(view_id);
    let (status_text, footer_text) =
        status::render_status_and_footer(status::StatusRenderRequest {
            session,
            frame,
            catalog,
            cache,
            view_id,
            placement,
            transform,
            canvas,
        })?;

    let mut canvas_rect = Rect::default();
    session.inner.draw(|frame| {
        let chunks = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(frame.area());
        Paragraph::new(status_text).render(chunks[0], frame.buffer_mut());
        Clear.render(chunks[1], frame.buffer_mut());
        Paragraph::new(footer_text).render(chunks[2], frame.buffer_mut());
        canvas_rect = chunks[1];
    })?;

    let screen_canvas = CanvasMetrics::new(
        CellSize::new(canvas_rect.width, canvas_rect.height),
        canvas.cell_pixel.or_fallback(),
    );
    let target = CellArea::new(
        canvas_rect.x,
        canvas_rect.y,
        canvas_rect.width,
        canvas_rect.height,
    );
    let mut parts = viewport.viewport_render_parts(cache, catalog, view_id, screen_canvas)?;
    session.inner.render_image_viewport(
        &mut parts.widget,
        image_id,
        MAIN_PLACEMENT_ID,
        parts.png,
        target,
    )?;
    session.inner.images().flush()?;
    Ok(())
}

pub(super) fn pinned_connection_counts(
    view_id: ViewId,
    pinned_element: Option<&ElementId>,
    catalog: &ViewCatalog,
) -> Option<ConnectionCounts> {
    pinned_element
        .map(|id| catalog.connection_candidate_counts_for_element(view_id, id))
        .filter(|connections| connections.total() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::RelationshipId;
    use crate::workspace::{RelationshipMetadata, ViewInfo, WorkspaceModel};
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn view_with_elements(key: &str, elements: &[&str]) -> ViewInfo {
        ViewInfo {
            key: key.to_owned(),
            name: key.to_owned(),
            kind: crate::workspace::ViewKind::Container,
            description: None,
            svg_path: PathBuf::from(format!("{key}.svg")),
            element_ids: elements.iter().map(|id| ElementId::new(*id)).collect(),
            child_view_keys_by_element_id: HashMap::new(),
            primary_view_key: None,
            key_view_key: None,
        }
    }

    fn relationship(id: &str, source: &str, destination: &str) -> RelationshipMetadata {
        RelationshipMetadata {
            id: RelationshipId::new(id),
            source_id: ElementId::new(source),
            destination_id: ElementId::new(destination),
            description: None,
            technology: None,
            tags: Vec::new(),
        }
    }

    #[test]
    fn pinned_connection_counts_use_navigable_candidates() {
        let mut model = WorkspaceModel::default();
        model.relationships.insert(
            RelationshipId::new("r1"),
            relationship("r1", "api", "database"),
        );
        model.relationships.insert(
            RelationshipId::new("r2"),
            relationship("r2", "api", "cache"),
        );
        model.relationships.insert(
            RelationshipId::new("r3"),
            relationship("r3", "browser", "api"),
        );
        model.outgoing_relationships_by_element.insert(
            ElementId::new("api"),
            vec![RelationshipId::new("r1"), RelationshipId::new("r2")],
        );
        model
            .incoming_relationships_by_element
            .insert(ElementId::new("api"), vec![RelationshipId::new("r3")]);

        let catalog = ViewCatalog::new(vec![
            view_with_elements("current", &["api", "database", "cache", "browser"]),
            view_with_elements("database", &["database"]),
        ])
        .unwrap()
        .with_model(model);

        let counts =
            pinned_connection_counts(ViewId::first(), Some(&ElementId::new("api")), &catalog);

        assert_eq!(
            counts,
            Some(ConnectionCounts {
                outgoing: 1,
                incoming: 0
            })
        );
    }
}

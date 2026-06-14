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
    use crate::test_support::catalog::TestViewBuilder;
    use crate::test_support::workspace::{relationship, TestModelBuilder};

    fn view_with_elements(key: &str, elements: &[&str]) -> crate::workspace::ViewInfo {
        TestViewBuilder::new(key)
            .kind(crate::workspace::ViewKind::Container)
            .elements(elements.iter().copied())
            .build()
    }

    #[test]
    fn pinned_connection_counts_use_navigable_candidates() {
        let model = TestModelBuilder::new()
            .relationship(relationship("r1", "api", "database"))
            .relationship(relationship("r2", "api", "cache"))
            .relationship(relationship("r3", "browser", "api"))
            .outgoing("api", &["r1", "r2"])
            .incoming("api", &["r3"])
            .build();

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

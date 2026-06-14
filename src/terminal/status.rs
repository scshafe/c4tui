use crate::ids::ViewId;
use crate::render_cache::RenderCache;
use crate::state::RenderFrame;
use crate::statusbar::StatusContext;
use crate::view_catalog::ViewCatalog;
use anyhow::Result;
use tui_kit::layout::{CanvasMetrics, Placement, ViewTransform};

use super::root::pinned_connection_counts;
use super::session::TerminalSession;

pub(super) struct StatusRenderRequest<'a> {
    pub session: &'a TerminalSession,
    pub frame: &'a RenderFrame,
    pub catalog: &'a ViewCatalog,
    pub cache: &'a mut RenderCache,
    pub view_id: ViewId,
    pub placement: Placement,
    pub transform: ViewTransform,
    pub canvas: CanvasMetrics,
}

pub(super) fn render_status_and_footer(
    request: StatusRenderRequest<'_>,
) -> Result<(String, String)> {
    let StatusRenderRequest {
        session,
        frame,
        catalog,
        cache,
        view_id,
        placement,
        transform,
        canvas,
    } = request;
    let view = catalog.view(view_id).clone();
    let breadcrumb_names: Vec<String> = frame
        .breadcrumbs
        .iter()
        .map(|id| catalog.view(*id).name.clone())
        .collect();
    let pinned_meta = frame
        .pinned_element
        .as_ref()
        .and_then(|id| catalog.element_metadata(id))
        .cloned();
    let pinned_connection_counts =
        pinned_connection_counts(view_id, frame.pinned_element.as_ref(), catalog);
    let rendered = cache.rendered_view(catalog, view_id)?;
    let breadcrumb_refs: Vec<&str> = breadcrumb_names.iter().map(String::as_str).collect();
    let workspace_path = session.workspace_path.as_deref();
    let context = StatusContext {
        view: &view,
        breadcrumb_names: breadcrumb_refs,
        transform,
        placement,
        canvas,
        rendered,
        config: &session.config,
        pinned_element: pinned_meta.as_ref(),
        pinned_element_connections: pinned_connection_counts,
        render_progress: frame.render_progress,
        workspace_path,
    };
    Ok((
        session.status_bar.render(&context, canvas.cells.cols),
        session.footer_bar.render(&context, canvas.cells.cols),
    ))
}

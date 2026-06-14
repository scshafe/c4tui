use crate::ids::ViewId;
use crate::nav_picker::ThumbnailCellArea;
use crate::render_cache::RenderCache;
use crate::view_catalog::{image_id_for_view, ViewCatalog};
use anyhow::Result;
use tui_kit::image::{picker_placement_id, ImageSurface};
use tui_kit::layout::CellArea;
use tui_kit::widgets::image_viewport::{
    ImageViewportInitialScale, ImageViewportOptions, ResizePolicy, ViewportImage,
};

use super::session::TerminalSession;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ThumbnailPaintPlan {
    pub active: Vec<ThumbnailCellArea>,
    pub stale: Vec<(u32, u32)>,
}

pub(super) fn close_modal_placements(
    session: &mut TerminalSession,
    catalog: &ViewCatalog,
) -> Result<()> {
    session
        .inner
        .teardown_image_viewports((0..catalog.view_count()).map(|index| {
            (
                image_id_for_view(ViewId::new(index)),
                picker_placement_id(index),
            )
        }))?;
    session.inner.images().flush()?;
    Ok(())
}

pub(super) fn teardown_root_viewport(session: &mut TerminalSession, view_id: ViewId) -> Result<()> {
    session.inner.teardown_image_viewport(
        image_id_for_view(view_id),
        tui_kit::image::MAIN_PLACEMENT_ID,
    )?;
    session.inner.images().flush()?;
    Ok(())
}

pub(super) fn clear_image_cache(session: &mut TerminalSession) -> Result<()> {
    session.inner.images().forget_all()?;
    session.inner.images().flush()?;
    Ok(())
}

pub(super) fn thumbnail_paint_plan(
    catalog: &ViewCatalog,
    active: Vec<ThumbnailCellArea>,
) -> ThumbnailPaintPlan {
    let active_placements = active
        .iter()
        .map(|thumb| picker_placement_id(thumb.view_id.index()))
        .collect::<std::collections::HashSet<_>>();
    let stale = (0..catalog.view_count())
        .map(|index| {
            (
                image_id_for_view(ViewId::new(index)),
                picker_placement_id(index),
            )
        })
        .filter(|(_, placement_id)| !active_placements.contains(placement_id))
        .collect();

    ThumbnailPaintPlan { active, stale }
}

pub(super) fn paint_thumbnails(
    session: &mut TerminalSession,
    cache: &RenderCache,
    plan: ThumbnailPaintPlan,
) -> Result<()> {
    if plan.active.is_empty() {
        return Ok(());
    }

    session.inner.teardown_image_viewports(plan.stale)?;
    for thumb in &plan.active {
        draw_thumbnail(session, thumb.view_id, thumb.area, cache)?;
    }
    Ok(())
}

fn draw_thumbnail(
    session: &mut TerminalSession,
    view_id: ViewId,
    area: CellArea,
    cache: &RenderCache,
) -> Result<bool> {
    let image_id = image_id_for_view(view_id);
    let Some(rendered) = cache.cached_rendered_view(view_id) else {
        return Ok(false);
    };
    let image = ViewportImage::new(rendered.raster_size, rendered.rgba.clone())?;
    session.inner.render_viewport_image(
        image,
        image_id,
        picker_placement_id(view_id.index()),
        &rendered.png,
        area,
        ImageViewportOptions {
            initial_scale: ImageViewportInitialScale::FitToBox,
            resize_policy: ResizePolicy::PreserveTopLeft,
        },
    )?;
    Ok(true)
}

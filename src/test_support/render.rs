use crate::ids::ViewId;
use crate::render::{RasterBudget, RenderedView};
use crate::render_cache::RenderCache;
use crate::view_catalog::ViewCatalog;
use crate::viewport_state::ViewportState;
use tui_kit::layout::{CanvasMetrics, CellPixel, CellSize, PixelSize};

pub(crate) fn raster_budget() -> RasterBudget {
    RasterBudget {
        quality: 1.0,
        ..RasterBudget::default()
    }
}

pub(crate) fn render_cache() -> RenderCache {
    RenderCache::new(raster_budget())
}

pub(crate) fn render_parts(catalog: ViewCatalog) -> (ViewCatalog, RenderCache, ViewportState) {
    (catalog, render_cache(), ViewportState::default())
}

pub(crate) fn canvas(cols: u16, rows: u16, cell_width: u16, cell_height: u16) -> CanvasMetrics {
    CanvasMetrics::new(
        CellSize::new(cols, rows),
        CellPixel::new(cell_width, cell_height),
    )
}

pub(crate) fn rendered_view(raster: PixelSize) -> RenderedView {
    let rgba_len = raster.width.saturating_mul(raster.height).saturating_mul(4) as usize;
    RenderedView {
        natural_size: raster,
        raster_size: raster,
        png: Vec::new(),
        rgba: vec![255; rgba_len],
        bboxes: Vec::new(),
    }
}

pub(crate) fn cached_render_parts(
    catalog: ViewCatalog,
    view_id: ViewId,
    rendered: RenderedView,
    viewport: ViewportState,
) -> (ViewCatalog, RenderCache, ViewportState) {
    let mut cache = render_cache();
    cache.insert_rendered(view_id, rendered);
    (catalog, cache, viewport)
}

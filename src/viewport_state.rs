use crate::config::PlacementChoiceConfig;
use crate::ids::{ElementId, ViewId};
use crate::render_cache::RenderCache;
use crate::view_catalog::ViewCatalog;
use anyhow::Result;
use std::collections::HashMap;
use tui_kit::layout::{
    CanvasMetrics, CellRoundingPolicy, ImageAnchorPolicy, ImagePoint, ImageZoomLimitPolicy,
    PixelSize, Placement, PlacementEngine, PlacementPolicy, ViewTransform, MAX_SCALE, MIN_SCALE,
};
use tui_kit::widgets::image_viewport::{
    ImageScale, ImageViewportWidget, ResizePolicy, ScaledPixelOffset, ViewportImage,
};

#[derive(Debug)]
pub(crate) struct ViewportState {
    transforms: HashMap<ViewId, ViewTransform>,
    engine: PlacementEngine,
}

pub(crate) struct ViewportRenderParts<'a> {
    pub png: &'a [u8],
    pub widget: ImageViewportWidget,
}

impl ViewportState {
    pub(crate) fn new(placement_policy: PlacementPolicy) -> Self {
        Self {
            transforms: HashMap::new(),
            engine: PlacementEngine::new(placement_policy.clone())
                .expect("c4tui placement policy is valid"),
        }
    }

    pub(crate) fn set_placement_policy(&mut self, policy: PlacementPolicy) {
        self.engine =
            PlacementEngine::new(policy.clone()).expect("c4tui placement policy is valid");
        self.transforms.clear();
    }

    pub(crate) fn invalidate(&mut self, id: ViewId) {
        self.transforms.remove(&id);
    }

    pub(crate) fn transform(&self, id: ViewId) -> ViewTransform {
        self.transforms.get(&id).copied().unwrap_or_default()
    }

    pub(crate) fn placement(
        &mut self,
        cache: &mut RenderCache,
        catalog: &ViewCatalog,
        id: ViewId,
        canvas: CanvasMetrics,
    ) -> Result<Placement> {
        let image = self.rendered_size(cache, catalog, id)?;
        Ok(self.engine.place(image, canvas, self.transform(id)))
    }

    pub(crate) fn viewport_render_parts<'a>(
        &'a mut self,
        cache: &'a mut RenderCache,
        catalog: &ViewCatalog,
        id: ViewId,
        canvas: CanvasMetrics,
    ) -> Result<ViewportRenderParts<'a>> {
        let placement = self.placement(cache, catalog, id, canvas)?;
        let rendered = cache.rendered_view(catalog, id)?;
        let png = rendered.png.as_slice();
        let widget = widget_from_canonical_placement(
            rendered.raster_size,
            &rendered.rgba,
            canvas,
            placement,
        )?;
        Ok(ViewportRenderParts { png, widget })
    }

    pub(crate) fn reset_viewport(
        &mut self,
        cache: &mut RenderCache,
        catalog: &ViewCatalog,
        id: ViewId,
        _canvas: CanvasMetrics,
    ) -> Result<()> {
        let _ = self.rendered_size(cache, catalog, id)?;
        self.transforms.insert(id, ViewTransform::fit());
        Ok(())
    }

    pub(crate) fn zoom_view(
        &mut self,
        cache: &mut RenderCache,
        catalog: &ViewCatalog,
        id: ViewId,
        factor: f32,
        canvas: CanvasMetrics,
    ) -> Result<()> {
        if (factor - 1.0).abs() < f32::EPSILON {
            return Ok(());
        }
        let image = self.rendered_size(cache, catalog, id)?;
        let next = self
            .engine
            .zoomed_at(self.transform(id), factor, 0.5, 0.5, image, canvas);
        self.transforms.insert(id, next);
        Ok(())
    }

    pub(crate) fn pan_view(
        &mut self,
        cache: &mut RenderCache,
        catalog: &ViewCatalog,
        id: ViewId,
        dx_fraction: f32,
        dy_fraction: f32,
        canvas: CanvasMetrics,
    ) -> Result<()> {
        let image = self.rendered_size(cache, catalog, id)?;
        let next = self
            .engine
            .panned(self.transform(id), dx_fraction, dy_fraction, image, canvas);
        self.transforms.insert(id, next);
        Ok(())
    }

    pub(crate) fn element_at_canvas_point(
        &mut self,
        cache: &mut RenderCache,
        catalog: &ViewCatalog,
        id: ViewId,
        canvas_x: f32,
        canvas_y: f32,
        canvas: CanvasMetrics,
    ) -> Result<Option<ElementId>> {
        let image_point =
            self.image_point_at_canvas(cache, catalog, id, canvas_x, canvas_y, canvas)?;
        if !image_point.inside {
            return Ok(None);
        }
        let rendered = cache.rendered_view(catalog, id)?;
        Ok(rendered
            .bboxes
            .iter()
            .filter(|bbox| bbox.contains(image_point.x, image_point.y))
            .min_by(|left, right| left.area().total_cmp(&right.area()))
            .map(|bbox| bbox.element_id.clone()))
    }

    fn image_point_at_canvas(
        &mut self,
        cache: &mut RenderCache,
        catalog: &ViewCatalog,
        id: ViewId,
        canvas_x: f32,
        canvas_y: f32,
        canvas: CanvasMetrics,
    ) -> Result<ImagePoint> {
        let image = self.rendered_size(cache, catalog, id)?;
        Ok(self
            .engine
            .canvas_to_image(self.transform(id), canvas_x, canvas_y, image, canvas))
    }

    fn rendered_size(
        &mut self,
        cache: &mut RenderCache,
        catalog: &ViewCatalog,
        id: ViewId,
    ) -> Result<PixelSize> {
        Ok(cache.rendered_view(catalog, id)?.raster_size)
    }

    #[cfg(test)]
    pub(crate) fn child_view_at_canvas_point(
        &mut self,
        cache: &mut RenderCache,
        catalog: &ViewCatalog,
        id: ViewId,
        canvas_x: f32,
        canvas_y: f32,
        canvas: CanvasMetrics,
    ) -> Result<Option<ViewId>> {
        Ok(self
            .child_views_at_canvas_point(cache, catalog, id, canvas_x, canvas_y, canvas)?
            .into_iter()
            .next())
    }

    #[cfg(test)]
    pub(crate) fn child_views_at_canvas_point(
        &mut self,
        cache: &mut RenderCache,
        catalog: &ViewCatalog,
        id: ViewId,
        canvas_x: f32,
        canvas_y: f32,
        canvas: CanvasMetrics,
    ) -> Result<Vec<ViewId>> {
        let image_point =
            self.image_point_at_canvas(cache, catalog, id, canvas_x, canvas_y, canvas)?;
        if !image_point.inside {
            return Ok(Vec::new());
        }
        let rendered = cache.rendered_view(catalog, id)?;
        let hit_element = rendered
            .bboxes
            .iter()
            .filter(|bbox| bbox.contains(image_point.x, image_point.y))
            .min_by(|left, right| left.area().total_cmp(&right.area()))
            .map(|bbox| bbox.element_id.clone());

        let Some(hit_element) = hit_element else {
            return Ok(Vec::new());
        };
        Ok(catalog.child_view_ids_for_element(id, &hit_element))
    }
}

impl Default for ViewportState {
    fn default() -> Self {
        Self::new(diagram_placement_policy(&PlacementChoiceConfig::default()))
    }
}

fn widget_from_canonical_placement(
    raster_size: PixelSize,
    rgba: &[u8],
    canvas: CanvasMetrics,
    placement: Placement,
) -> Result<ImageViewportWidget> {
    let image = ViewportImage::new(raster_size, rgba.to_vec())?;
    let mut widget = ImageViewportWidget::from_image(image, canvas);
    widget.set_resize_policy(ResizePolicy::PreserveCenter);
    widget.set_scale(ImageScale::new(f64::from(placement.effective_scale))?);
    let cell_pixel = canvas.cell_pixel.or_fallback();
    let origin_x = i64::from(placement.origin.col) * i64::from(cell_pixel.width);
    let origin_y = i64::from(placement.origin.row) * i64::from(cell_pixel.height);
    let scale = f64::from(placement.effective_scale);
    widget.set_offset(ScaledPixelOffset::new(
        (f64::from(placement.source.x) * scale).round() as i64 - origin_x,
        (f64::from(placement.source.y) * scale).round() as i64 - origin_y,
    ));
    Ok(widget)
}

pub(crate) fn diagram_placement_policy(choice: &PlacementChoiceConfig) -> PlacementPolicy {
    PlacementPolicy {
        scale_basis: choice.scale_basis.as_policy(),
        zoom_limit: ImageZoomLimitPolicy::ClampScale {
            min: MIN_SCALE,
            max: MAX_SCALE,
        },
        // Default behavior is "B" — the image's logical cell rect is allowed
        // to exceed canvas bounds. Sample windowing still applies internally so
        // pan via center_x works as before; the consumer (c4tui's terminal
        // layer) clamps the cell rect before issuing Kitty placements so the
        // status / footer bars are not overwritten.
        overflow: choice.overflow.as_policy(),
        anchor: ImageAnchorPolicy::Center,
        min_visible_pixels: PixelSize::new(1, 1),
        cell_rounding: CellRoundingPolicy::Nearest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{ElementBBox, RasterBudget, RenderedView};
    use crate::workspace::{ViewInfo, ViewKind};
    use std::collections::HashMap;
    use std::fs;
    use tui_kit::layout::{CellPixel, CellSize};

    fn budget() -> RasterBudget {
        RasterBudget {
            quality: 1.0,
            ..RasterBudget::default()
        }
    }

    fn canvas() -> CanvasMetrics {
        CanvasMetrics::new(CellSize::new(200, 50), CellPixel::new(8, 16))
    }

    fn view_info() -> ViewInfo {
        ViewInfo {
            key: "view".to_owned(),
            name: "View".to_owned(),
            kind: ViewKind::SystemLandscape,
            description: None,
            svg_path: std::path::PathBuf::from("unused.svg"),
            element_ids: std::collections::HashSet::new(),
            child_view_keys_by_element_id: std::collections::HashMap::new(),
            primary_view_key: None,
            key_view_key: None,
        }
    }

    fn rendered_view(raster: PixelSize) -> RenderedView {
        let rgba_len = raster.width.saturating_mul(raster.height).saturating_mul(4) as usize;
        RenderedView {
            natural_size: raster,
            raster_size: raster,
            png: Vec::new(),
            rgba: vec![255; rgba_len],
            bboxes: Vec::new(),
        }
    }

    fn cached_parts(
        raster: PixelSize,
        policy: PlacementPolicy,
    ) -> (ViewCatalog, RenderCache, ViewportState) {
        let catalog = ViewCatalog::new(vec![view_info()]).unwrap();
        let mut cache = RenderCache::new(budget());
        let viewport = ViewportState::new(policy);
        cache.insert_rendered(ViewId::first(), rendered_view(raster));
        (catalog, cache, viewport)
    }

    #[test]
    fn hit_tests_child_view_at_canvas_point() {
        let dir = tempfile::tempdir().unwrap();
        let parent_svg = dir.path().join("parent.svg");
        let child_svg = dir.path().join("child.svg");
        fs::write(
            &parent_svg,
            r#"<svg width="100" height="100"><g id="1"><rect x="10" y="10" width="80" height="80"/></g></svg>"#,
        )
        .unwrap();
        fs::write(&child_svg, r#"<svg width="100" height="100" />"#).unwrap();
        let mut child_view_keys_by_element_id = HashMap::new();
        child_view_keys_by_element_id
            .insert(crate::ids::ElementId::new("1"), vec!["child".to_owned()]);
        let catalog = ViewCatalog::new(vec![
            ViewInfo {
                key: "parent".to_owned(),
                name: "Parent".to_owned(),
                kind: ViewKind::SystemContext,
                description: None,
                svg_path: parent_svg,
                element_ids: std::collections::HashSet::from([ElementId::new("1")]),
                child_view_keys_by_element_id,
                primary_view_key: None,
                key_view_key: None,
            },
            ViewInfo {
                key: "child".to_owned(),
                name: "Child".to_owned(),
                kind: ViewKind::Container,
                description: None,
                svg_path: child_svg,
                element_ids: std::collections::HashSet::new(),
                child_view_keys_by_element_id: std::collections::HashMap::new(),
                primary_view_key: None,
                key_view_key: None,
            },
        ])
        .unwrap();
        let mut cache = RenderCache::new(budget());
        let mut viewport = ViewportState::default();

        let canvas = canvas();
        let placement = viewport
            .placement(&mut cache, &catalog, ViewId::first(), canvas)
            .unwrap();
        let center_col = f32::from(placement.origin.col) + f32::from(placement.size.cols) / 2.0;
        let center_row = f32::from(placement.origin.row) + f32::from(placement.size.rows) / 2.0;
        let cx = center_col / f32::from(canvas.cells.cols);
        let cy = center_row / f32::from(canvas.cells.rows);
        assert_eq!(
            viewport
                .child_view_at_canvas_point(&mut cache, &catalog, ViewId::first(), cx, cy, canvas)
                .unwrap(),
            Some(ViewId::new(1))
        );
        assert_eq!(
            viewport
                .child_view_at_canvas_point(&mut cache, &catalog, ViewId::first(), 0.0, 0.0, canvas)
                .unwrap(),
            None
        );
    }

    fn legacy_crop_policy() -> PlacementPolicy {
        diagram_placement_policy(&PlacementChoiceConfig {
            scale_basis: crate::config::ScaleBasisChoice::Fit,
            overflow: crate::config::OverflowChoice::Crop,
        })
    }

    #[test]
    fn zoomed_placement_inflates_then_clips_viewport_under_crop_policy() {
        let raster = PixelSize::new(400, 200);
        let canvas = canvas();
        let (catalog, mut cache, mut viewport) = cached_parts(raster, legacy_crop_policy());

        viewport
            .zoom_view(&mut cache, &catalog, ViewId::first(), 2.0, canvas)
            .unwrap();
        let placement = viewport
            .placement(&mut cache, &catalog, ViewId::first(), canvas)
            .unwrap();

        assert!(placement.effective_scale > placement.fit_scale);
        assert!(placement.unclipped_display_pixels.width > placement.visible_pixels.width);
        assert!(placement.unclipped_display_pixels.height > placement.visible_pixels.height);
        assert_eq!(placement.size.cols, canvas.cells.cols);
        assert_eq!(placement.size.rows, canvas.cells.rows);
    }

    #[test]
    fn panning_moves_viewport_without_changing_zoom_scale() {
        let raster = PixelSize::new(400, 200);
        let canvas = canvas();
        let (catalog, mut cache, mut viewport) = cached_parts(raster, legacy_crop_policy());

        viewport
            .zoom_view(&mut cache, &catalog, ViewId::first(), 2.0, canvas)
            .unwrap();
        let before = viewport
            .placement(&mut cache, &catalog, ViewId::first(), canvas)
            .unwrap();
        viewport
            .pan_view(&mut cache, &catalog, ViewId::first(), 0.25, 0.25, canvas)
            .unwrap();
        let after = viewport
            .placement(&mut cache, &catalog, ViewId::first(), canvas)
            .unwrap();

        assert_eq!(after.effective_scale, before.effective_scale);
        assert!(after.source.x > before.source.x);
        assert!(after.source.y > before.source.y);
    }

    #[test]
    fn reset_after_zoom_and_pan_restores_fit_transform() {
        let raster = PixelSize::new(400, 200);
        let canvas = canvas();
        let (catalog, mut cache, mut viewport) = cached_parts(raster, legacy_crop_policy());
        let initial = viewport
            .placement(&mut cache, &catalog, ViewId::first(), canvas)
            .unwrap();

        viewport
            .zoom_view(&mut cache, &catalog, ViewId::first(), 2.0, canvas)
            .unwrap();
        viewport
            .pan_view(&mut cache, &catalog, ViewId::first(), 0.25, 0.25, canvas)
            .unwrap();
        viewport
            .reset_viewport(&mut cache, &catalog, ViewId::first(), canvas)
            .unwrap();

        assert_eq!(viewport.transform(ViewId::first()), ViewTransform::fit());
        assert_eq!(
            viewport
                .placement(&mut cache, &catalog, ViewId::first(), canvas)
                .unwrap(),
            initial
        );
    }

    #[test]
    fn materialized_widget_matches_canonical_placement() {
        let raster = PixelSize::new(400, 200);
        let canvas = canvas();
        let (catalog, mut cache, mut viewport) = cached_parts(raster, legacy_crop_policy());

        viewport
            .zoom_view(&mut cache, &catalog, ViewId::first(), 2.0, canvas)
            .unwrap();
        viewport
            .pan_view(&mut cache, &catalog, ViewId::first(), 0.25, 0.25, canvas)
            .unwrap();
        let placement = viewport
            .placement(&mut cache, &catalog, ViewId::first(), canvas)
            .unwrap();
        let rendered = cache.rendered_view(&catalog, ViewId::first()).unwrap();
        let widget = widget_from_canonical_placement(
            rendered.raster_size,
            &rendered.rgba,
            canvas,
            placement,
        )
        .unwrap();
        let widget_placement = widget.placement().unwrap().unwrap();

        assert_eq!(widget_placement.source, placement.source);
        assert_eq!(widget_placement.origin, placement.origin);
        assert_eq!(widget_placement.cell_cols, placement.size.cols);
        assert_eq!(widget_placement.cell_rows, placement.size.rows);
    }

    #[test]
    fn zoomed_hit_testing_uses_viewport_source_crop() {
        let raster = PixelSize::new(400, 200);
        let canvas = canvas();
        let mut rendered = rendered_view(raster);
        rendered.bboxes.push(ElementBBox {
            element_id: ElementId::new("center"),
            x: 190.0,
            y: 90.0,
            width: 20.0,
            height: 20.0,
        });
        let catalog = ViewCatalog::new(vec![view_info()]).unwrap();
        let mut cache = RenderCache::new(budget());
        cache.insert_rendered(ViewId::first(), rendered);
        let mut viewport = ViewportState::new(legacy_crop_policy());

        viewport
            .zoom_view(&mut cache, &catalog, ViewId::first(), 2.0, canvas)
            .unwrap();

        assert_eq!(
            viewport
                .element_at_canvas_point(&mut cache, &catalog, ViewId::first(), 0.5, 0.5, canvas)
                .unwrap(),
            Some(ElementId::new("center"))
        );
    }
}

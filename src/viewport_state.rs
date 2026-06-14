use crate::config::PlacementChoiceConfig;
use crate::ids::{ElementId, ViewId};
use crate::render_cache::RenderCache;
use crate::view_catalog::ViewCatalog;
use anyhow::{anyhow, bail, Result};
use std::collections::HashMap;
use tui_kit::layout::{
    CanvasMetrics, CellRect, CellRoundingPolicy, ClippedSides, ImageAnchorPolicy, ImagePoint,
    ImageScaleBasis, ImageZoomLimitPolicy, PixelSize, Placement, PlacementAnchor, PlacementPolicy,
    ViewTransform, MAX_SCALE, MIN_SCALE,
};
use tui_kit::widgets::image_viewport::{
    ImageScale, ImageViewportPlacement, ImageViewportWidget, PixelDistance, ResizePolicy,
    ScaledPixelOffset, StepDirection, ViewportAxis, ViewportImage, ZoomDirection, ZoomFactor,
};

#[derive(Debug)]
pub struct ViewportState {
    transforms: HashMap<ViewId, ViewTransform>,
    viewports: HashMap<ViewId, ImageViewportWidget>,
    placement_policy: PlacementPolicy,
}

pub struct ViewportRenderParts<'a> {
    pub png: &'a [u8],
    pub widget: &'a mut ImageViewportWidget,
}

impl ViewportState {
    pub fn new(placement_policy: PlacementPolicy) -> Self {
        Self {
            transforms: HashMap::new(),
            viewports: HashMap::new(),
            placement_policy,
        }
    }

    pub fn set_placement_policy(&mut self, policy: PlacementPolicy) {
        self.placement_policy = policy;
        self.viewports.clear();
        self.transforms.clear();
    }

    pub fn invalidate(&mut self, id: ViewId) {
        self.viewports.remove(&id);
        self.transforms.remove(&id);
    }

    pub fn transform(&self, id: ViewId) -> ViewTransform {
        self.transforms.get(&id).copied().unwrap_or_default()
    }

    pub fn placement(
        &mut self,
        cache: &mut RenderCache,
        catalog: &ViewCatalog,
        id: ViewId,
        canvas: CanvasMetrics,
    ) -> Result<Placement> {
        self.ensure_viewport(cache, catalog, id, canvas)?;
        let Some(widget) = self.viewports.get(&id) else {
            bail!("viewport was not cached after insertion");
        };
        let placement = widget
            .placement()?
            .ok_or_else(|| anyhow!("viewport has no visible placement"))?;
        Ok(status_placement(widget, placement))
    }

    pub fn viewport_render_parts<'a>(
        &'a mut self,
        cache: &'a mut RenderCache,
        catalog: &ViewCatalog,
        id: ViewId,
        canvas: CanvasMetrics,
    ) -> Result<ViewportRenderParts<'a>> {
        self.ensure_viewport(cache, catalog, id, canvas)?;
        let png = cache
            .cached_rendered_view(id)
            .ok_or_else(|| anyhow!("rendered view was not cached after viewport insertion"))?
            .png
            .as_slice();
        let widget = self
            .viewports
            .get_mut(&id)
            .ok_or_else(|| anyhow!("viewport was not cached after insertion"))?;
        Ok(ViewportRenderParts { png, widget })
    }

    pub fn reset_viewport(
        &mut self,
        cache: &mut RenderCache,
        catalog: &ViewCatalog,
        id: ViewId,
        canvas: CanvasMetrics,
    ) -> Result<()> {
        self.ensure_viewport(cache, catalog, id, canvas)?;
        let scale_basis = self.placement_policy.scale_basis;
        let Some(widget) = self.viewports.get_mut(&id) else {
            bail!("viewport was not cached after insertion");
        };
        reset_widget_to_canvas(widget, scale_basis)?;
        self.sync_transform_from_viewport(id);
        Ok(())
    }

    pub fn zoom_view(
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
        self.ensure_viewport(cache, catalog, id, canvas)?;
        let Some(widget) = self.viewports.get_mut(&id) else {
            bail!("viewport was not cached after insertion");
        };
        let (direction, factor) = if factor > 1.0 {
            (ZoomDirection::In, factor as f64)
        } else {
            (ZoomDirection::Out, (1.0 / factor.max(f32::EPSILON)) as f64)
        };
        widget.set_zoom(ZoomFactor::new(factor)?);
        widget.apply_zoom(direction)?;
        self.sync_transform_from_viewport(id);
        Ok(())
    }

    pub fn pan_view(
        &mut self,
        cache: &mut RenderCache,
        catalog: &ViewCatalog,
        id: ViewId,
        dx_fraction: f32,
        dy_fraction: f32,
        canvas: CanvasMetrics,
    ) -> Result<()> {
        self.ensure_viewport(cache, catalog, id, canvas)?;
        let Some(widget) = self.viewports.get_mut(&id) else {
            bail!("viewport was not cached after insertion");
        };
        let pixels = canvas.pixels();
        apply_pan_step(widget, ViewportAxis::X, dx_fraction * pixels.width as f32);
        apply_pan_step(widget, ViewportAxis::Y, dy_fraction * pixels.height as f32);
        self.sync_transform_from_viewport(id);
        Ok(())
    }

    pub fn element_at_canvas_point(
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
        self.ensure_viewport(cache, catalog, id, canvas)?;
        self.viewports
            .get(&id)
            .ok_or_else(|| anyhow!("viewport was not cached after insertion"))?
            .normalized_to_image(canvas_x, canvas_y)
            .map_err(Into::into)
    }

    fn ensure_viewport(
        &mut self,
        cache: &mut RenderCache,
        catalog: &ViewCatalog,
        id: ViewId,
        canvas: CanvasMetrics,
    ) -> Result<()> {
        if !self.viewports.contains_key(&id) {
            let (raster_size, rgba) = {
                let rendered = cache.rendered_view(catalog, id)?;
                (rendered.raster_size, rendered.rgba.clone())
            };
            let image = ViewportImage::new(raster_size, rgba)?;
            let mut widget = ImageViewportWidget::from_image(image, canvas);
            widget.set_resize_policy(ResizePolicy::PreserveCenter);
            reset_widget_to_canvas(&mut widget, self.placement_policy.scale_basis)?;
            self.viewports.insert(id, widget);
        }

        let Some(widget) = self.viewports.get_mut(&id) else {
            bail!("viewport was not cached after insertion");
        };
        widget.update_canvas(canvas);
        self.sync_transform_from_viewport(id);
        Ok(())
    }

    fn sync_transform_from_viewport(&mut self, id: ViewId) {
        let Some(widget) = self.viewports.get(&id) else {
            return;
        };
        self.transforms.insert(id, transform_from_widget(widget));
    }

    #[cfg(test)]
    pub fn child_view_at_canvas_point(
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
    pub fn child_views_at_canvas_point(
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

fn reset_widget_to_canvas(
    widget: &mut ImageViewportWidget,
    scale_basis: ImageScaleBasis,
) -> Result<()> {
    let image = widget.viewport().image().size();
    let canvas = widget.widget_pixels();
    let scale = initial_scale(image, canvas, scale_basis);
    widget.set_scale(ImageScale::new(scale)?);
    let theoretical = widget.viewport().theoretical_size()?;
    widget.set_offset(centered_offset(theoretical, canvas));
    Ok(())
}

fn initial_scale(image: PixelSize, canvas: PixelSize, scale_basis: ImageScaleBasis) -> f64 {
    match scale_basis {
        ImageScaleBasis::FitToArea => fit_scale(image, canvas),
        ImageScaleBasis::NativePixels => 1.0,
        ImageScaleBasis::FillArea => {
            let width = canvas.width.max(1) as f64 / image.width.max(1) as f64;
            let height = canvas.height.max(1) as f64 / image.height.max(1) as f64;
            width.max(height).max(f64::EPSILON)
        }
        ImageScaleBasis::ExplicitScale(scale) => f64::from(scale).max(f64::EPSILON),
        _ => fit_scale(image, canvas),
    }
}

fn fit_scale(image: PixelSize, canvas: PixelSize) -> f64 {
    let width = canvas.width.max(1) as f64 / image.width.max(1) as f64;
    let height = canvas.height.max(1) as f64 / image.height.max(1) as f64;
    width.min(height).max(f64::EPSILON)
}

fn centered_offset(theoretical: PixelSize, canvas: PixelSize) -> ScaledPixelOffset {
    ScaledPixelOffset::new(
        (i64::from(theoretical.width) - i64::from(canvas.width)) / 2,
        (i64::from(theoretical.height) - i64::from(canvas.height)) / 2,
    )
}

fn apply_pan_step(widget: &mut ImageViewportWidget, axis: ViewportAxis, delta: f32) {
    let pixels = delta.abs().round() as u32;
    if pixels == 0 {
        return;
    }
    widget.set_step(axis, PixelDistance::new(pixels));
    let direction = if delta < 0.0 {
        StepDirection::Negative
    } else {
        StepDirection::Positive
    };
    widget.apply_step(axis, direction);
}

fn transform_from_widget(widget: &ImageViewportWidget) -> ViewTransform {
    let image = widget.viewport().image().size();
    let canvas = widget.widget_pixels();
    let fit = fit_scale(image, canvas);
    let center = widget.normalized_to_image(0.5, 0.5).unwrap_or(ImagePoint {
        x: image.width as f32 / 2.0,
        y: image.height as f32 / 2.0,
        inside: false,
    });
    ViewTransform {
        scale: (widget.viewport().scale().get() / fit.max(f64::EPSILON)) as f32,
        center_x: (center.x / image.width.max(1) as f32).clamp(0.0, 1.0),
        center_y: (center.y / image.height.max(1) as f32).clamp(0.0, 1.0),
    }
}

fn status_placement(widget: &ImageViewportWidget, placement: ImageViewportPlacement) -> Placement {
    let image = widget.viewport().image().size();
    let canvas = widget.widget_pixels();
    let fit = fit_scale(image, canvas) as f32;
    let effective_scale = widget.viewport().scale().get() as f32;
    let transform = transform_from_widget(widget);
    Placement {
        source: placement.source,
        size: CellRect {
            cols: placement.cell_cols,
            rows: placement.cell_rows,
        },
        origin: placement.origin,
        effective_scale,
        fit_scale: fit,
        visible_pixels: placement.visible_pixels,
        unclipped_display_pixels: placement.theoretical_pixels,
        clipped_sides: ClippedSides {
            left: placement.source.x > 0,
            right: placement.source.x.saturating_add(placement.source.width) < image.width,
            top: placement.source.y > 0,
            bottom: placement.source.y.saturating_add(placement.source.height) < image.height,
        },
        anchor: PlacementAnchor {
            image_x: transform.center_x * image.width as f32,
            image_y: transform.center_y * image.height as f32,
            normalized_x: transform.center_x,
            normalized_y: transform.center_y,
        },
    }
}

pub fn diagram_placement_policy(choice: &PlacementChoiceConfig) -> PlacementPolicy {
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

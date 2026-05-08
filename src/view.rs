use crate::ids::{ElementId, ViewId};
use crate::render::{render_svg, RasterBudget, RenderedView};
use crate::workspace::{ElementMetadata, ExportedWorkspace, ViewInfo, WorkspaceModel};
use anyhow::{anyhow, bail, Result};
use std::collections::HashMap;
use tui_kit::layout::{
    CanvasMetrics, CellRoundingPolicy, ImageAnchorPolicy, ImageOverflowPolicy, ImagePoint,
    ImageScaleBasis, ImageZoomLimitPolicy, PixelSize, Placement, PlacementEngine, PlacementPolicy,
    ViewTransform, MAX_SCALE, MIN_SCALE,
};

#[derive(Debug)]
pub struct ViewStore {
    pub views: Vec<ViewInfo>,
    pub model: WorkspaceModel,
    rendered: HashMap<ViewId, RenderedView>,
    transforms: HashMap<ViewId, ViewTransform>,
    budget: RasterBudget,
    export_guard: Option<ExportedWorkspace>,
}

impl ViewStore {
    pub fn new(views: Vec<ViewInfo>, budget: RasterBudget) -> Result<Self> {
        if views.is_empty() {
            bail!("no exported SVG views were found");
        }

        Ok(Self {
            views,
            model: WorkspaceModel::default(),
            rendered: HashMap::new(),
            transforms: HashMap::new(),
            budget,
            export_guard: None,
        })
    }

    pub fn with_model(mut self, model: WorkspaceModel) -> Self {
        self.model = model;
        self
    }

    pub fn with_export(mut self, export: ExportedWorkspace) -> Self {
        self.export_guard = Some(export);
        self
    }

    pub fn element_metadata(&self, id: &ElementId) -> Option<&ElementMetadata> {
        self.model.elements.get(id)
    }

    pub fn element_at_canvas_point(
        &mut self,
        id: ViewId,
        canvas_x: f32,
        canvas_y: f32,
        canvas: CanvasMetrics,
    ) -> Result<Option<ElementId>> {
        let transform = self.transform(id);
        let rendered = self.rendered_view(id)?;
        let image_point: ImagePoint =
            canvas_to_image(transform, canvas_x, canvas_y, rendered.raster_size, canvas);
        if !image_point.inside {
            return Ok(None);
        }
        Ok(rendered
            .bboxes
            .iter()
            .filter(|bbox| bbox.contains(image_point.x, image_point.y))
            .min_by(|left, right| left.area().total_cmp(&right.area()))
            .map(|bbox| bbox.element_id.clone()))
    }

    pub const fn len(&self) -> usize {
        self.views.len()
    }

    pub fn view(&self, id: ViewId) -> &ViewInfo {
        &self.views[id.index()]
    }

    pub fn rendered_view(&mut self, id: ViewId) -> Result<&RenderedView> {
        if !self.rendered.contains_key(&id) {
            let rendered = render_svg(&self.views[id.index()].svg_path, self.budget)?;
            self.rendered.insert(id, rendered);
        }

        self.rendered
            .get(&id)
            .ok_or_else(|| anyhow!("rendered view was not cached after insertion"))
    }

    pub fn cached_rendered_view(&self, id: ViewId) -> Option<&RenderedView> {
        self.rendered.get(&id)
    }

    pub fn has_rendered(&self, id: ViewId) -> bool {
        self.rendered.contains_key(&id)
    }

    pub fn insert_rendered(&mut self, id: ViewId, rendered: RenderedView) {
        self.rendered.insert(id, rendered);
    }

    pub fn budget(&self) -> RasterBudget {
        self.budget
    }

    pub fn render_jobs(&self) -> Vec<(ViewId, std::path::PathBuf)> {
        self.views
            .iter()
            .enumerate()
            .filter(|(idx, _)| !self.rendered.contains_key(&ViewId::new(*idx)))
            .map(|(idx, info)| (ViewId::new(idx), info.svg_path.clone()))
            .collect()
    }

    pub fn transform(&self, id: ViewId) -> ViewTransform {
        self.transforms.get(&id).copied().unwrap_or_default()
    }

    pub fn set_transform(&mut self, id: ViewId, transform: ViewTransform) {
        self.transforms.insert(id, transform);
    }

    #[cfg(test)]
    pub fn placement(&mut self, id: ViewId, canvas: CanvasMetrics) -> Result<Placement> {
        let raster = self.rendered_view(id)?.raster_size;
        Ok(diagram_placement(self.transform(id), raster, canvas))
    }

    pub fn child_view_at_canvas_point(
        &mut self,
        id: ViewId,
        canvas_x: f32,
        canvas_y: f32,
        canvas: CanvasMetrics,
    ) -> Result<Option<ViewId>> {
        let transform = self.transform(id);
        let rendered = self.rendered_view(id)?;
        let image_point: ImagePoint =
            canvas_to_image(transform, canvas_x, canvas_y, rendered.raster_size, canvas);
        if !image_point.inside {
            return Ok(None);
        }
        let hit_element = rendered
            .bboxes
            .iter()
            .filter(|bbox| bbox.contains(image_point.x, image_point.y))
            .min_by(|left, right| left.area().total_cmp(&right.area()))
            .map(|bbox| bbox.element_id.clone());

        let Some(hit_element) = hit_element else {
            return Ok(None);
        };
        let Some(child_key) = self.views[id.index()]
            .child_view_by_element_id
            .get(&hit_element)
        else {
            return Ok(None);
        };

        Ok(self
            .views
            .iter()
            .position(|view| &view.key == child_key)
            .map(ViewId::new))
    }
}

pub fn image_id_for_view(id: ViewId) -> u32 {
    (id.index() as u32) + 1
}

pub fn diagram_placement(
    transform: ViewTransform,
    raster: PixelSize,
    canvas: CanvasMetrics,
) -> Placement {
    PlacementEngine::new(diagram_placement_policy())
        .expect("diagram placement policy is valid")
        .place(raster, canvas, transform)
}

fn diagram_placement_policy() -> PlacementPolicy {
    PlacementPolicy {
        scale_basis: ImageScaleBasis::FitToArea,
        zoom_limit: ImageZoomLimitPolicy::ClampScale {
            min: MIN_SCALE,
            max: MAX_SCALE,
        },
        // Kitty clips an overflowing image by receiving the visible source
        // rectangle. Logically the image is still fit once, inflated by zoom,
        // and viewed through the canvas.
        overflow: ImageOverflowPolicy::CropSourceToArea,
        anchor: ImageAnchorPolicy::Center,
        min_visible_pixels: PixelSize::new(1, 1),
        cell_rounding: CellRoundingPolicy::Nearest,
    }
}

fn canvas_to_image(
    transform: ViewTransform,
    canvas_x: f32,
    canvas_y: f32,
    raster: PixelSize,
    canvas: CanvasMetrics,
) -> ImagePoint {
    let placement = diagram_placement(transform, raster, canvas);
    let cell_pixel = canvas.cell_pixel.or_fallback();
    let canvas_x = canvas_x.clamp(0.0, 1.0);
    let canvas_y = canvas_y.clamp(0.0, 1.0);
    let canvas_pixels = canvas.pixels();
    let cursor_pixel_x = canvas_x * canvas_pixels.width as f32;
    let cursor_pixel_y = canvas_y * canvas_pixels.height as f32;
    let origin_pixel_x = f32::from(placement.origin.col) * f32::from(cell_pixel.width);
    let origin_pixel_y = f32::from(placement.origin.row) * f32::from(cell_pixel.height);
    let target_pixel_w = f32::from(placement.size.cols) * f32::from(cell_pixel.width);
    let target_pixel_h = f32::from(placement.size.rows) * f32::from(cell_pixel.height);
    let local_x = (cursor_pixel_x - origin_pixel_x) / target_pixel_w.max(1.0);
    let local_y = (cursor_pixel_y - origin_pixel_y) / target_pixel_h.max(1.0);
    let inside = (0.0..=1.0).contains(&local_x) && (0.0..=1.0).contains(&local_y);
    let local_x = local_x.clamp(0.0, 1.0);
    let local_y = local_y.clamp(0.0, 1.0);

    ImagePoint {
        x: placement.source.x as f32 + local_x * placement.source.width as f32,
        y: placement.source.y as f32 + local_y * placement.source.height as f32,
        inside,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn creates_view_store_for_non_empty_views() {
        let dir = tempfile::tempdir().unwrap();
        let svg = dir.path().join("landscape.svg");
        fs::write(&svg, "<svg />").unwrap();
        let store = ViewStore::new(
            vec![ViewInfo {
                key: "landscape".to_owned(),
                name: "Landscape".to_owned(),
                kind: crate::workspace::ViewKind::SystemLandscape,
                description: None,
                svg_path: svg,
                element_ids: std::collections::HashSet::new(),
                child_view_by_element_id: std::collections::HashMap::new(),
                primary_view_key: None,
                key_view_key: None,
            }],
            budget(),
        )
        .unwrap();

        assert_eq!(store.len(), 1);
        assert_eq!(image_id_for_view(ViewId::first()), 1);
    }

    #[test]
    fn rejects_empty_view_store() {
        assert!(ViewStore::new(Vec::new(), budget()).is_err());
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
        let mut child_view_by_element_id = std::collections::HashMap::new();
        child_view_by_element_id.insert(crate::ids::ElementId::new("1"), "child".to_owned());
        let mut store = ViewStore::new(
            vec![
                ViewInfo {
                    key: "parent".to_owned(),
                    name: "Parent".to_owned(),
                    kind: crate::workspace::ViewKind::SystemContext,
                    description: None,
                    svg_path: parent_svg,
                    element_ids: std::collections::HashSet::from([crate::ids::ElementId::new("1")]),
                    child_view_by_element_id,
                    primary_view_key: None,
                    key_view_key: None,
                },
                ViewInfo {
                    key: "child".to_owned(),
                    name: "Child".to_owned(),
                    kind: crate::workspace::ViewKind::Container,
                    description: None,
                    svg_path: child_svg,
                    element_ids: std::collections::HashSet::new(),
                    child_view_by_element_id: std::collections::HashMap::new(),
                    primary_view_key: None,
                    key_view_key: None,
                },
            ],
            budget(),
        )
        .unwrap();

        let canvas = canvas();
        let placement = store.placement(ViewId::first(), canvas).unwrap();
        let center_col = f32::from(placement.origin.col) + f32::from(placement.size.cols) / 2.0;
        let center_row = f32::from(placement.origin.row) + f32::from(placement.size.rows) / 2.0;
        let cx = center_col / f32::from(canvas.cells.cols);
        let cy = center_row / f32::from(canvas.cells.rows);
        assert_eq!(
            store
                .child_view_at_canvas_point(ViewId::first(), cx, cy, canvas)
                .unwrap(),
            Some(ViewId::new(1))
        );
        assert_eq!(
            store
                .child_view_at_canvas_point(ViewId::first(), 0.0, 0.0, canvas)
                .unwrap(),
            None
        );
    }

    #[test]
    fn zoomed_placement_inflates_then_clips_viewport() {
        let raster = tui_kit::layout::PixelSize::new(1000, 800);
        let canvas = canvas();
        let placement = diagram_placement(ViewTransform::fit().with_scale(2.0), raster, canvas);

        assert!(placement.effective_scale > placement.fit_scale);
        assert!(placement.unclipped_display_pixels.width > placement.visible_pixels.width);
        assert!(placement.unclipped_display_pixels.height > placement.visible_pixels.height);
        assert_eq!(placement.size.cols, canvas.cells.cols);
        assert_eq!(placement.size.rows, canvas.cells.rows);
    }

    #[test]
    fn panning_moves_viewport_without_changing_zoom_scale() {
        let raster = tui_kit::layout::PixelSize::new(2000, 1000);
        let canvas = canvas();
        let zoomed = ViewTransform::fit().with_scale(2.0);
        let panned = zoomed.panned(0.25, 0.25, raster, canvas);
        let before = diagram_placement(zoomed, raster, canvas);
        let after = diagram_placement(panned, raster, canvas);

        assert_eq!(after.effective_scale, before.effective_scale);
        assert!(after.source.x > before.source.x);
        assert!(after.source.y > before.source.y);
    }
}

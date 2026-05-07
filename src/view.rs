use crate::ids::{ElementId, ViewId};
use tui_kit::layout::{CanvasMetrics, ImagePoint, ViewTransform};
#[cfg(test)]
use tui_kit::layout::Placement;
use crate::render::{render_svg, RasterBudget, RenderedView};
use crate::workspace::{ElementMetadata, ExportedWorkspace, ViewInfo, WorkspaceModel};
use anyhow::{anyhow, bail, Result};
use std::collections::HashMap;

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
            transform.canvas_to_image(canvas_x, canvas_y, rendered.raster_size, canvas);
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
        Ok(self.transform(id).place(raster, canvas))
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
            transform.canvas_to_image(canvas_x, canvas_y, rendered.raster_size, canvas);
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

#[cfg(test)]
mod tests {
    use super::*;
    use tui_kit::layout::{CellPixel, CellSize};
    use std::fs;

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
}

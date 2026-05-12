use crate::config::PlacementChoiceConfig;
use crate::ids::{ElementId, RelationshipId, ViewId};
use crate::render::{render_svg, RasterBudget, RenderedView};
use crate::workspace::{
    ConnectionCounts, ElementMetadata, ExportedWorkspace, ViewInfo, WorkspaceModel,
};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionDirection {
    Outgoing,
    Incoming,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionNavigationCandidate {
    pub relationship_id: RelationshipId,
    pub direction: ConnectionDirection,
    pub connected_element_id: ElementId,
    pub view_id: ViewId,
}

#[derive(Debug)]
pub struct ViewStore {
    pub views: Vec<ViewInfo>,
    pub model: WorkspaceModel,
    rendered: HashMap<ViewId, RenderedView>,
    transforms: HashMap<ViewId, ViewTransform>,
    viewports: HashMap<ViewId, ImageViewportWidget>,
    budget: RasterBudget,
    placement_policy: PlacementPolicy,
    export_guard: Option<ExportedWorkspace>,
}

pub struct ViewportRenderParts<'a> {
    pub png: &'a [u8],
    pub widget: &'a mut ImageViewportWidget,
}

fn direction_rank(direction: ConnectionDirection) -> u8 {
    match direction {
        ConnectionDirection::Outgoing => 0,
        ConnectionDirection::Incoming => 1,
    }
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
            viewports: HashMap::new(),
            budget,
            placement_policy: diagram_placement_policy(&PlacementChoiceConfig::default()),
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

    pub fn with_placement_policy(mut self, policy: PlacementPolicy) -> Self {
        self.placement_policy = policy;
        self
    }

    pub fn set_placement_policy(&mut self, policy: PlacementPolicy) {
        self.placement_policy = policy;
        self.viewports.clear();
        self.transforms.clear();
    }

    pub fn element_metadata(&self, id: &ElementId) -> Option<&ElementMetadata> {
        self.model.elements.get(id)
    }

    pub fn connection_candidates_for_element(
        &self,
        current: ViewId,
        element_id: &ElementId,
    ) -> Vec<ConnectionNavigationCandidate> {
        let outgoing = self
            .model
            .outgoing_relationships(element_id)
            .flat_map(|rel| {
                self.candidate_views_for_element(current, &rel.destination_id)
                    .into_iter()
                    .map(move |view_id| ConnectionNavigationCandidate {
                        relationship_id: rel.id.clone(),
                        direction: ConnectionDirection::Outgoing,
                        connected_element_id: rel.destination_id.clone(),
                        view_id,
                    })
            });
        let incoming = self
            .model
            .incoming_relationships(element_id)
            .flat_map(|rel| {
                self.candidate_views_for_element(current, &rel.source_id)
                    .into_iter()
                    .map(move |view_id| ConnectionNavigationCandidate {
                        relationship_id: rel.id.clone(),
                        direction: ConnectionDirection::Incoming,
                        connected_element_id: rel.source_id.clone(),
                        view_id,
                    })
            });

        let mut candidates = outgoing.chain(incoming).collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            direction_rank(left.direction)
                .cmp(&direction_rank(right.direction))
                .then_with(|| {
                    self.element_label(&left.connected_element_id)
                        .cmp(self.element_label(&right.connected_element_id))
                })
                .then_with(|| {
                    self.view(left.view_id)
                        .name
                        .cmp(&self.view(right.view_id).name)
                })
                .then_with(|| left.relationship_id.cmp(&right.relationship_id))
        });
        candidates.dedup_by(|left, right| {
            left.direction == right.direction
                && left.connected_element_id == right.connected_element_id
                && left.view_id == right.view_id
        });
        candidates
    }

    pub fn connection_candidate_counts_for_element(
        &self,
        current: ViewId,
        element_id: &ElementId,
    ) -> ConnectionCounts {
        self.connection_candidates_for_element(current, element_id)
            .into_iter()
            .fold(ConnectionCounts::default(), |mut counts, candidate| {
                match candidate.direction {
                    ConnectionDirection::Outgoing => counts.outgoing += 1,
                    ConnectionDirection::Incoming => counts.incoming += 1,
                }
                counts
            })
    }

    fn candidate_views_for_element(&self, current: ViewId, element_id: &ElementId) -> Vec<ViewId> {
        self.views
            .iter()
            .enumerate()
            .filter_map(|(index, view)| {
                let view_id = ViewId::new(index);
                (view_id != current
                    && !view.kind.is_legend()
                    && view.element_ids.contains(element_id))
                .then_some(view_id)
            })
            .collect()
    }

    fn element_label<'a>(&'a self, element_id: &'a ElementId) -> &'a str {
        self.model
            .elements
            .get(element_id)
            .map(|element| element.name.as_str())
            .unwrap_or_else(|| element_id.as_str())
    }

    pub fn element_at_canvas_point(
        &mut self,
        id: ViewId,
        canvas_x: f32,
        canvas_y: f32,
        canvas: CanvasMetrics,
    ) -> Result<Option<ElementId>> {
        let image_point = self.image_point_at_canvas(id, canvas_x, canvas_y, canvas)?;
        if !image_point.inside {
            return Ok(None);
        }
        let rendered = self.rendered_view(id)?;
        Ok(rendered
            .bboxes
            .iter()
            .filter(|bbox| bbox.contains(image_point.x, image_point.y))
            .min_by(|left, right| left.area().total_cmp(&right.area()))
            .map(|bbox| bbox.element_id.clone()))
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
        self.viewports.remove(&id);
        self.transforms.remove(&id);
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

    pub fn placement(&mut self, id: ViewId, canvas: CanvasMetrics) -> Result<Placement> {
        self.ensure_viewport(id, canvas)?;
        let Some(widget) = self.viewports.get(&id) else {
            bail!("viewport was not cached after insertion");
        };
        let placement = widget
            .placement()?
            .ok_or_else(|| anyhow!("viewport has no visible placement"))?;
        Ok(status_placement(widget, placement))
    }

    pub fn viewport_render_parts(
        &mut self,
        id: ViewId,
        canvas: CanvasMetrics,
    ) -> Result<ViewportRenderParts<'_>> {
        self.ensure_viewport(id, canvas)?;
        let png = self
            .rendered
            .get(&id)
            .ok_or_else(|| anyhow!("rendered view was not cached after viewport insertion"))?
            .png
            .as_slice();
        let widget = self
            .viewports
            .get_mut(&id)
            .ok_or_else(|| anyhow!("viewport was not cached after insertion"))?;
        Ok(ViewportRenderParts { png, widget })
    }

    pub fn reset_viewport(&mut self, id: ViewId, canvas: CanvasMetrics) -> Result<()> {
        self.ensure_viewport(id, canvas)?;
        let scale_basis = self.placement_policy.scale_basis;
        let Some(widget) = self.viewports.get_mut(&id) else {
            bail!("viewport was not cached after insertion");
        };
        reset_widget_to_canvas(widget, scale_basis)?;
        self.sync_transform_from_viewport(id);
        Ok(())
    }

    pub fn zoom_view(&mut self, id: ViewId, factor: f32, canvas: CanvasMetrics) -> Result<()> {
        if (factor - 1.0).abs() < f32::EPSILON {
            return Ok(());
        }
        self.ensure_viewport(id, canvas)?;
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
        id: ViewId,
        dx_fraction: f32,
        dy_fraction: f32,
        canvas: CanvasMetrics,
    ) -> Result<()> {
        self.ensure_viewport(id, canvas)?;
        let Some(widget) = self.viewports.get_mut(&id) else {
            bail!("viewport was not cached after insertion");
        };
        let pixels = canvas.pixels();
        apply_pan_step(widget, ViewportAxis::X, dx_fraction * pixels.width as f32);
        apply_pan_step(widget, ViewportAxis::Y, dy_fraction * pixels.height as f32);
        self.sync_transform_from_viewport(id);
        Ok(())
    }

    fn image_point_at_canvas(
        &mut self,
        id: ViewId,
        canvas_x: f32,
        canvas_y: f32,
        canvas: CanvasMetrics,
    ) -> Result<ImagePoint> {
        self.ensure_viewport(id, canvas)?;
        self.viewports
            .get(&id)
            .ok_or_else(|| anyhow!("viewport was not cached after insertion"))?
            .normalized_to_image(canvas_x, canvas_y)
            .map_err(Into::into)
    }

    fn ensure_viewport(&mut self, id: ViewId, canvas: CanvasMetrics) -> Result<()> {
        if !self.viewports.contains_key(&id) {
            let rendered = self.rendered_view(id)?;
            let image = ViewportImage::new(rendered.raster_size, rendered.rgba.clone())?;
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
        id: ViewId,
        canvas_x: f32,
        canvas_y: f32,
        canvas: CanvasMetrics,
    ) -> Result<Option<ViewId>> {
        Ok(self
            .child_views_at_canvas_point(id, canvas_x, canvas_y, canvas)?
            .into_iter()
            .next())
    }

    #[cfg(test)]
    pub fn child_views_at_canvas_point(
        &mut self,
        id: ViewId,
        canvas_x: f32,
        canvas_y: f32,
        canvas: CanvasMetrics,
    ) -> Result<Vec<ViewId>> {
        let image_point = self.image_point_at_canvas(id, canvas_x, canvas_y, canvas)?;
        if !image_point.inside {
            return Ok(Vec::new());
        }
        let rendered = self.rendered_view(id)?;
        let hit_element = rendered
            .bboxes
            .iter()
            .filter(|bbox| bbox.contains(image_point.x, image_point.y))
            .min_by(|left, right| left.area().total_cmp(&right.area()))
            .map(|bbox| bbox.element_id.clone());

        let Some(hit_element) = hit_element else {
            return Ok(Vec::new());
        };
        Ok(self.child_view_ids_for_element(id, &hit_element))
    }

    pub fn child_view_ids_for_element(
        &self,
        current: ViewId,
        element_id: &ElementId,
    ) -> Vec<ViewId> {
        self.views[current.index()]
            .child_view_keys_by_element_id
            .get(element_id)
            .into_iter()
            .flatten()
            .filter_map(|child_key| {
                self.views
                    .iter()
                    .position(|view| &view.key == child_key)
                    .map(ViewId::new)
            })
            .collect()
    }
}

pub fn image_id_for_view(id: ViewId) -> u32 {
    (id.index() as u32) + 1
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
    use crate::render::ElementBBox;
    use crate::workspace::RelationshipMetadata;
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
            kind: crate::workspace::ViewKind::SystemLandscape,
            description: None,
            svg_path: std::path::PathBuf::from("unused.svg"),
            element_ids: std::collections::HashSet::new(),
            child_view_keys_by_element_id: std::collections::HashMap::new(),
            primary_view_key: None,
            key_view_key: None,
        }
    }

    fn view_with_elements(key: &str, kind: crate::workspace::ViewKind, ids: &[&str]) -> ViewInfo {
        ViewInfo {
            key: key.to_owned(),
            name: key.to_owned(),
            kind,
            description: None,
            svg_path: std::path::PathBuf::from(format!("{key}.svg")),
            element_ids: ids.iter().copied().map(ElementId::new).collect(),
            child_view_keys_by_element_id: HashMap::new(),
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

    fn store_with_rendered(raster: PixelSize, policy: PlacementPolicy) -> ViewStore {
        let mut store = ViewStore::new(vec![view_info()], budget())
            .unwrap()
            .with_placement_policy(policy);
        store.insert_rendered(ViewId::first(), rendered_view(raster));
        store
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
                child_view_keys_by_element_id: std::collections::HashMap::new(),
                primary_view_key: None,
                key_view_key: None,
            }],
            budget(),
        )
        .unwrap();

        assert_eq!(store.views.len(), 1);
        assert_eq!(image_id_for_view(ViewId::first()), 1);
    }

    #[test]
    fn rejects_empty_view_store() {
        assert!(ViewStore::new(Vec::new(), budget()).is_err());
    }

    #[test]
    fn connection_candidates_follow_outgoing_and_incoming_relationships_to_views() {
        let mut model = WorkspaceModel::default();
        model.relationships.insert(
            RelationshipId::new("r1"),
            relationship("r1", "api", "database"),
        );
        model.relationships.insert(
            RelationshipId::new("r2"),
            relationship("r2", "browser", "api"),
        );
        model.relationships.insert(
            RelationshipId::new("r3"),
            relationship("r3", "api", "cache"),
        );
        model.relationships.insert(
            RelationshipId::new("r4"),
            relationship("r4", "api", "database"),
        );
        model.outgoing_relationships_by_element.insert(
            ElementId::new("api"),
            vec![
                RelationshipId::new("r1"),
                RelationshipId::new("r3"),
                RelationshipId::new("r4"),
            ],
        );
        model
            .incoming_relationships_by_element
            .insert(ElementId::new("api"), vec![RelationshipId::new("r2")]);

        let store = ViewStore::new(
            vec![
                view_with_elements(
                    "current",
                    crate::workspace::ViewKind::Container,
                    &["api", "database", "browser"],
                ),
                view_with_elements(
                    "database-view",
                    crate::workspace::ViewKind::Component,
                    &["database"],
                ),
                view_with_elements(
                    "browser-view",
                    crate::workspace::ViewKind::SystemContext,
                    &["browser"],
                ),
                view_with_elements(
                    "database-key",
                    crate::workspace::ViewKind::Key,
                    &["database"],
                ),
            ],
            budget(),
        )
        .unwrap()
        .with_model(model);

        let candidates =
            store.connection_candidates_for_element(ViewId::first(), &ElementId::new("api"));

        assert_eq!(
            candidates,
            vec![
                ConnectionNavigationCandidate {
                    relationship_id: RelationshipId::new("r1"),
                    direction: ConnectionDirection::Outgoing,
                    connected_element_id: ElementId::new("database"),
                    view_id: ViewId::new(1),
                },
                ConnectionNavigationCandidate {
                    relationship_id: RelationshipId::new("r2"),
                    direction: ConnectionDirection::Incoming,
                    connected_element_id: ElementId::new("browser"),
                    view_id: ViewId::new(2),
                },
            ]
        );
        assert_eq!(
            store.connection_candidate_counts_for_element(ViewId::first(), &ElementId::new("api")),
            ConnectionCounts {
                outgoing: 1,
                incoming: 1
            }
        );
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
        let mut child_view_keys_by_element_id = std::collections::HashMap::new();
        child_view_keys_by_element_id
            .insert(crate::ids::ElementId::new("1"), vec!["child".to_owned()]);
        let mut store = ViewStore::new(
            vec![
                ViewInfo {
                    key: "parent".to_owned(),
                    name: "Parent".to_owned(),
                    kind: crate::workspace::ViewKind::SystemContext,
                    description: None,
                    svg_path: parent_svg,
                    element_ids: std::collections::HashSet::from([crate::ids::ElementId::new("1")]),
                    child_view_keys_by_element_id,
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
                    child_view_keys_by_element_id: std::collections::HashMap::new(),
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
        let mut store = store_with_rendered(raster, legacy_crop_policy());

        store.zoom_view(ViewId::first(), 2.0, canvas).unwrap();
        let placement = store.placement(ViewId::first(), canvas).unwrap();

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
        let mut store = store_with_rendered(raster, legacy_crop_policy());

        store.zoom_view(ViewId::first(), 2.0, canvas).unwrap();
        let before = store.placement(ViewId::first(), canvas).unwrap();
        store.pan_view(ViewId::first(), 0.25, 0.25, canvas).unwrap();
        let after = store.placement(ViewId::first(), canvas).unwrap();

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
        let mut store = ViewStore::new(vec![view_info()], budget())
            .unwrap()
            .with_placement_policy(legacy_crop_policy());
        store.insert_rendered(ViewId::first(), rendered);

        store.zoom_view(ViewId::first(), 2.0, canvas).unwrap();

        assert_eq!(
            store
                .element_at_canvas_point(ViewId::first(), 0.5, 0.5, canvas)
                .unwrap(),
            Some(ElementId::new("center"))
        );
    }
}

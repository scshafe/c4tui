use crate::event::{Command, ZoomAnchor};
use crate::ids::{ElementId, ViewId};
use crate::link_directory::LinkCandidate;
use crate::render_cache::RenderCache;
use crate::view_catalog::ViewCatalog;
use crate::viewport_state::ViewportState;
use anyhow::Result;
use tui_kit::layout::{CanvasMetrics, ViewTransform};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppState {
    current: ViewId,
    breadcrumbs: Vec<ViewId>,
    last_drag: Option<(u16, u16)>,
    pinned_element: Option<ElementId>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            current: ViewId::first(),
            breadcrumbs: Vec::new(),
            last_drag: None,
            pinned_element: None,
        }
    }
}

impl AppState {
    pub const fn current(&self) -> ViewId {
        self.current
    }

    pub fn pinned_element(&self) -> Option<&ElementId> {
        self.pinned_element.as_ref()
    }

    pub fn render_frame(&self) -> RenderFrame {
        RenderFrame {
            current: self.current,
            breadcrumbs: self.breadcrumbs.clone(),
            pinned_element: self.pinned_element.clone(),
            render_progress: None,
        }
    }

    pub fn apply(
        &mut self,
        command: Command,
        catalog: &ViewCatalog,
        cache: &mut RenderCache,
        viewport: &mut ViewportState,
        canvas: CanvasMetrics,
    ) -> Result<UpdateResult> {
        let mut result = UpdateResult {
            canvas,
            ..UpdateResult::default()
        };

        match command {
            Command::Quit => {
                result.effect = Some(Effect::Quit);
                result.render = false;
            }
            Command::OpenPicker => {
                result.effect = Some(Effect::OpenModal(ModalSpec::View));
                result.render = false;
            }
            Command::OpenLinkDirectory => {
                result.effect = Some(Effect::OpenModal(ModalSpec::Link));
                result.render = false;
            }
            Command::SelectView(next) => {
                self.current = next;
                self.breadcrumbs.clear();
                self.pinned_element = None;
            }
            Command::SelectChildView(next) => {
                self.navigate_child_view(next);
            }
            Command::SelectLink(candidate) => {
                self.navigate_link(candidate);
            }
            Command::Reload => {
                result.effect = Some(Effect::ReloadWorkspace);
                result.render = false;
            }
            Command::ReloadSucceeded => {
                self.reset_navigation();
                result.effect = Some(Effect::ClearImageCache);
            }
            Command::ReloadFailed => {
                self.last_drag = None;
            }
            Command::Help => {
                result.effect = Some(Effect::ShowHelp);
                result.render = false;
            }
            Command::ToggleLog => {
                result.effect = Some(Effect::ToggleLogView);
                result.render = false;
            }
            Command::CycleScaleBasis | Command::CycleOverflow | Command::CycleZoomStep => {
                unreachable!("cycling commands are short-circuited in App::handle_input");
            }
            Command::Back => {
                if let Some(previous) = self.breadcrumbs.pop() {
                    self.current = previous;
                    self.pinned_element = None;
                }
            }
            Command::ShowLegend => {
                let Some(legend_key) = catalog.view(self.current).key_view_key.clone() else {
                    result.render = false;
                    return Ok(result);
                };
                let Some(target) = catalog.view_id_by_key(&legend_key) else {
                    result.render = false;
                    return Ok(result);
                };
                self.breadcrumbs.push(self.current);
                self.current = target;
                self.last_drag = None;
                self.pinned_element = None;
            }
            Command::DrillAt { canvas_x, canvas_y } => {
                if let Some(element) = viewport.element_at_canvas_point(
                    cache,
                    catalog,
                    self.current,
                    canvas_x,
                    canvas_y,
                    canvas,
                )? {
                    match catalog
                        .child_view_ids_for_element(self.current, &element)
                        .as_slice()
                    {
                        [] => {
                            self.pinned_element = Some(element);
                        }
                        [child] => {
                            self.navigate_child_view(*child);
                        }
                        children => {
                            result.effect = Some(Effect::OpenModal(ModalSpec::ChildView {
                                target_view_ids: children.to_vec(),
                            }));
                            result.render = false;
                        }
                    }
                } else {
                    result.render = false;
                }
            }
            Command::InspectAt { canvas_x, canvas_y } => {
                self.pinned_element = viewport.element_at_canvas_point(
                    cache,
                    catalog,
                    self.current,
                    canvas_x,
                    canvas_y,
                    canvas,
                )?;
                if self.pinned_element.is_none() {
                    result.render = false;
                }
            }
            Command::ClearOrQuit => {
                if self.pinned_element.is_some() {
                    self.pinned_element = None;
                } else {
                    result.effect = Some(Effect::Quit);
                    result.render = false;
                }
            }
            Command::Zoom { factor, anchor } => {
                self.zoom_current(catalog, cache, viewport, factor, anchor, canvas)?;
            }
            Command::ResetView => {
                viewport.reset_viewport(cache, catalog, self.current, canvas)?;
            }
            Command::Pan {
                dx_fraction,
                dy_fraction,
            } => {
                self.pan_current(catalog, cache, viewport, dx_fraction, dy_fraction, canvas)?;
            }
            Command::DragTo {
                x,
                y,
                canvas: drag_canvas,
            } => {
                if let Some((last_x, last_y)) = self.last_drag {
                    let canvas_cols = drag_canvas.cells.cols.max(1);
                    let canvas_rows = drag_canvas.cells.rows.max(1);
                    let dx = (f32::from(last_x) - f32::from(x)) / f32::from(canvas_cols);
                    let dy = (f32::from(last_y) - f32::from(y)) / f32::from(canvas_rows);
                    self.pan_current(catalog, cache, viewport, dx, dy, drag_canvas)?;
                } else {
                    result.render = false;
                }
                self.last_drag = Some((x, y));
            }
            Command::EndDrag => {
                self.last_drag = None;
                result.render = false;
            }
            Command::Noop => {
                result.render = false;
            }
        }

        Ok(result)
    }

    fn navigate_link(&mut self, candidate: LinkCandidate) {
        self.breadcrumbs.push(self.current);
        self.current = candidate.target_view;
        self.last_drag = None;
        self.pinned_element = candidate.pinned_element_after_navigation;
    }

    fn navigate_child_view(&mut self, next: ViewId) {
        self.breadcrumbs.push(self.current);
        self.current = next;
        self.last_drag = None;
        self.pinned_element = None;
    }

    fn reset_navigation(&mut self) {
        self.current = ViewId::first();
        self.breadcrumbs.clear();
        self.last_drag = None;
        self.pinned_element = None;
    }

    fn zoom_current(
        &self,
        catalog: &ViewCatalog,
        cache: &mut RenderCache,
        viewport: &mut ViewportState,
        factor: f32,
        _anchor: ZoomAnchor,
        canvas: CanvasMetrics,
    ) -> Result<()> {
        let before_transform = viewport.transform(self.current);
        let before_placement = viewport.placement(cache, catalog, self.current, canvas)?;
        viewport.zoom_view(cache, catalog, self.current, factor, canvas)?;
        let after_transform = viewport.transform(self.current);
        let after_placement = viewport.placement(cache, catalog, self.current, canvas)?;

        if let Some(reason) = zoom_no_op_reason(
            factor,
            &before_transform,
            &after_transform,
            &before_placement,
            &after_placement,
        ) {
            log::warn!(
                "zoom factor {factor:.3} produced no visible change: {reason} \
                 (scale {:.3} -> {:.3}, effective {:.4} -> {:.4}, \
                 src {}x{}@{},{} -> {}x{}@{},{}, target {}x{} -> {}x{})",
                before_transform.scale,
                after_transform.scale,
                before_placement.effective_scale,
                after_placement.effective_scale,
                before_placement.source.width,
                before_placement.source.height,
                before_placement.source.x,
                before_placement.source.y,
                after_placement.source.width,
                after_placement.source.height,
                after_placement.source.x,
                after_placement.source.y,
                before_placement.size.cols,
                before_placement.size.rows,
                after_placement.size.cols,
                after_placement.size.rows,
            );
        }
        Ok(())
    }

    fn pan_current(
        &self,
        catalog: &ViewCatalog,
        cache: &mut RenderCache,
        viewport: &mut ViewportState,
        horizontal: f32,
        vertical: f32,
        canvas: CanvasMetrics,
    ) -> Result<()> {
        viewport.pan_view(cache, catalog, self.current, horizontal, vertical, canvas)
    }
}

/// Classify a zoom call that produced no observable change. Returns the
/// likely reason as a short string, or `None` if the zoom did change something.
///
/// Compares the placements before and after applying the new transform: if the
/// effective scale, source crop, and target cell rect are all unchanged, the
/// user will perceive no change at all. The reason string distinguishes the
/// common causes so the c4tui log can surface specifics.
fn zoom_no_op_reason(
    factor: f32,
    before_transform: &ViewTransform,
    after_transform: &ViewTransform,
    before_placement: &tui_kit::layout::Placement,
    after_placement: &tui_kit::layout::Placement,
) -> Option<&'static str> {
    if !before_placement.is_visually_equivalent(after_placement) {
        return None;
    }
    let scale_unchanged = (before_transform.scale - after_transform.scale).abs() < f32::EPSILON;

    if scale_unchanged && factor > 1.0 {
        Some(
            "transform.scale was already at MAX_SCALE; tui-kit's clamp_scale \
             refused to grow it further (try the `Z` key to lower the zoom step \
             so smaller increments still register)",
        )
    } else if scale_unchanged && factor < 1.0 {
        Some(
            "transform.scale was already at MIN_SCALE; tui-kit's clamp_scale \
             refused to shrink it further",
        )
    } else if !scale_unchanged && factor > 1.0 {
        // Scale changed but nothing visible did. Two common causes under the
        // default `crop` policy: (a) the source rect is already a single pixel
        // wide/tall, so smaller windows can't be sampled; (b) the visible rect
        // is locked to image aspect under `letterbox`, in which case visible
        // doesn't grow past the image-aspect rectangle.
        Some(
            "scale advanced but the placement engine produced an identical \
             placement. Common causes: (1) the source crop already saturated \
             at single-pixel granularity (try cycling Z to a smaller zoom \
             step so finer increments register); (2) you're in `letterbox` \
             overflow which locks the visible rectangle to the image aspect \
             — press O to cycle to `crop` for canonical magnifier behaviour \
             where the visible region grows to viewport bounds.",
        )
    } else if !scale_unchanged && factor < 1.0 {
        Some(
            "scale advanced down but the placement engine produced an \
             identical placement; usually means the image was already \
             smaller than the canvas and the overflow policy is preventing \
             further shrink",
        )
    } else {
        Some("zoom factor was effectively 1.0 — the call was a true no-op")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderFrame {
    pub current: ViewId,
    pub breadcrumbs: Vec<ViewId>,
    pub pinned_element: Option<ElementId>,
    pub render_progress: Option<(usize, usize)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UpdateResult {
    pub effect: Option<Effect>,
    pub render: bool,
    pub canvas: CanvasMetrics,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Quit,
    OpenModal(ModalSpec),
    ReloadWorkspace,
    ClearImageCache,
    ShowHelp,
    ToggleLogView,
}

/// Per-modal payload carried by `Effect::OpenModal`. Each variant maps to one
/// picker-spawn body in `App::handle_input`. `ShowHelp` and `ToggleLogView`
/// stay separate from this enum — they don't share the picker shape and
/// folding them in would force a fake-payload variant here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModalSpec {
    View,
    ChildView { target_view_ids: Vec<ViewId> },
    Link,
}

impl Default for UpdateResult {
    fn default() -> Self {
        Self {
            effect: None,
            render: true,
            canvas: CanvasMetrics::new(
                tui_kit::layout::CellSize::new(80, 24),
                tui_kit::layout::CellPixel::FALLBACK,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{ElementId, RelationshipId};
    use crate::link_directory::LinkKind;
    use crate::render::RasterBudget;
    use crate::workspace::{RelationshipMetadata, ViewInfo, WorkspaceModel};
    use std::collections::{HashMap, HashSet};
    use std::fs;
    use tui_kit::layout::{CellPixel, CellSize};

    fn canvas() -> CanvasMetrics {
        CanvasMetrics::new(CellSize::new(80, 24), CellPixel::new(8, 16))
    }

    fn budget() -> RasterBudget {
        RasterBudget {
            quality: 1.0,
            ..RasterBudget::default()
        }
    }

    fn test_parts() -> (ViewCatalog, RenderCache, ViewportState) {
        let dir = std::env::temp_dir().join(format!(
            "c4tui-state-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let parent_svg = dir.join("parent.svg");
        let child_svg = dir.join("child.svg");
        fs::write(
            &parent_svg,
            r#"<svg width="100" height="100"><g id="1"><rect x="10" y="10" width="80" height="80"/></g></svg>"#,
        )
        .unwrap();
        fs::write(&child_svg, r#"<svg width="100" height="100" />"#).unwrap();

        let mut child_view_keys_by_element_id = HashMap::new();
        child_view_keys_by_element_id.insert(ElementId::new("1"), vec!["child".to_owned()]);

        (
            ViewCatalog::new(vec![
                ViewInfo {
                    key: "parent".to_owned(),
                    name: "Parent".to_owned(),
                    kind: crate::workspace::ViewKind::SystemContext,
                    description: None,
                    svg_path: parent_svg,
                    element_ids: HashSet::from([ElementId::new("1")]),
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
                    element_ids: HashSet::new(),
                    child_view_keys_by_element_id: HashMap::new(),
                    primary_view_key: None,
                    key_view_key: None,
                },
            ])
            .unwrap(),
            RenderCache::new(budget()),
            ViewportState::default(),
        )
    }

    fn connection_parts() -> (ViewCatalog, RenderCache, ViewportState) {
        let views = vec![
            ViewInfo {
                key: "api".to_owned(),
                name: "API".to_owned(),
                kind: crate::workspace::ViewKind::Container,
                description: None,
                svg_path: std::path::PathBuf::from("api.svg"),
                element_ids: HashSet::from([ElementId::new("api")]),
                child_view_keys_by_element_id: HashMap::new(),
                primary_view_key: None,
                key_view_key: None,
            },
            ViewInfo {
                key: "database".to_owned(),
                name: "Database".to_owned(),
                kind: crate::workspace::ViewKind::Component,
                description: None,
                svg_path: std::path::PathBuf::from("database.svg"),
                element_ids: HashSet::from([ElementId::new("database")]),
                child_view_keys_by_element_id: HashMap::new(),
                primary_view_key: None,
                key_view_key: None,
            },
        ];
        let relationship = RelationshipMetadata {
            id: RelationshipId::new("r1"),
            source_id: ElementId::new("api"),
            destination_id: ElementId::new("database"),
            description: Some("Reads from".to_owned()),
            technology: None,
            tags: Vec::new(),
        };
        let mut model = WorkspaceModel::default();
        model
            .outgoing_relationships_by_element
            .insert(ElementId::new("api"), vec![relationship.id.clone()]);
        model
            .relationships
            .insert(relationship.id.clone(), relationship);

        (
            ViewCatalog::new(views).unwrap().with_model(model),
            RenderCache::new(budget()),
            ViewportState::default(),
        )
    }

    #[test]
    fn select_view_clears_breadcrumbs() {
        let (catalog, mut cache, mut viewport) = test_parts();
        let mut state = AppState::default();
        state
            .apply(
                Command::DrillAt {
                    canvas_x: 0.5,
                    canvas_y: 0.5,
                },
                &catalog,
                &mut cache,
                &mut viewport,
                canvas(),
            )
            .unwrap();

        state
            .apply(
                Command::SelectView(ViewId::first()),
                &catalog,
                &mut cache,
                &mut viewport,
                canvas(),
            )
            .unwrap();

        assert_eq!(state.current(), ViewId::first());
        assert!(state.render_frame().breadcrumbs.is_empty());
    }

    #[test]
    fn drill_and_back_update_navigation_without_tty() {
        let (catalog, mut cache, mut viewport) = test_parts();
        let mut state = AppState::default();

        state
            .apply(
                Command::DrillAt {
                    canvas_x: 0.5,
                    canvas_y: 0.5,
                },
                &catalog,
                &mut cache,
                &mut viewport,
                canvas(),
            )
            .unwrap();
        assert_eq!(state.current(), ViewId::new(1));
        assert_eq!(state.render_frame().breadcrumbs, &[ViewId::first()]);

        state
            .apply(Command::Back, &catalog, &mut cache, &mut viewport, canvas())
            .unwrap();
        assert_eq!(state.current(), ViewId::first());
        assert!(state.render_frame().breadcrumbs.is_empty());
    }

    #[test]
    fn open_link_directory_requests_modal_without_navigating() {
        let (catalog, mut cache, mut viewport) = connection_parts();
        let mut state = AppState {
            pinned_element: Some(ElementId::new("api")),
            ..Default::default()
        };

        let update = state
            .apply(
                Command::OpenLinkDirectory,
                &catalog,
                &mut cache,
                &mut viewport,
                canvas(),
            )
            .unwrap();

        assert_eq!(update.effect, Some(Effect::OpenModal(ModalSpec::Link)));
        assert!(!update.render);
        assert_eq!(state.current(), ViewId::first());
        assert!(state.render_frame().breadcrumbs.is_empty());
        assert_eq!(
            state.render_frame().pinned_element,
            Some(ElementId::new("api"))
        );
    }

    #[test]
    fn select_link_navigates_and_sets_optional_pin() {
        let (catalog, mut cache, mut viewport) = connection_parts();
        let mut state = AppState::default();
        let candidate = LinkCandidate {
            kind: LinkKind::ConnectionOutgoing,
            target_view: ViewId::new(1),
            source_element: Some(ElementId::new("api")),
            label: "Database".to_owned(),
            detail: Some("to Database".to_owned()),
            pinned_element_after_navigation: Some(ElementId::new("database")),
            secondary_tokens: Vec::new(),
        };

        state
            .apply(
                Command::SelectLink(candidate),
                &catalog,
                &mut cache,
                &mut viewport,
                canvas(),
            )
            .unwrap();

        assert_eq!(state.current(), ViewId::new(1));
        assert_eq!(state.render_frame().breadcrumbs, &[ViewId::first()]);
        assert_eq!(
            state.render_frame().pinned_element,
            Some(ElementId::new("database"))
        );
    }

    #[test]
    fn zoom_pan_and_reset_update_transform_without_tty() {
        let (catalog, mut cache, mut viewport) = test_parts();
        let mut state = AppState::default();

        state
            .apply(
                Command::Zoom {
                    factor: 2.0,
                    anchor: ZoomAnchor::Center,
                },
                &catalog,
                &mut cache,
                &mut viewport,
                canvas(),
            )
            .unwrap();
        let scale_after_zoom = viewport.transform(ViewId::first()).scale;
        assert!((scale_after_zoom - 2.0).abs() < 0.01);

        state
            .apply(
                Command::Pan {
                    dx_fraction: 0.1,
                    dy_fraction: 0.1,
                },
                &catalog,
                &mut cache,
                &mut viewport,
                canvas(),
            )
            .unwrap();
        assert!(viewport.transform(ViewId::first()).center_x > 0.5);

        state
            .apply(
                Command::ResetView,
                &catalog,
                &mut cache,
                &mut viewport,
                canvas(),
            )
            .unwrap();
        assert_eq!(viewport.transform(ViewId::first()), ViewTransform::fit());
    }

    #[test]
    fn zoom_below_one_keeps_image_visible_below_fit() {
        let (catalog, mut cache, mut viewport) = test_parts();
        let mut state = AppState::default();
        state
            .apply(
                Command::Zoom {
                    factor: 0.5,
                    anchor: ZoomAnchor::Center,
                },
                &catalog,
                &mut cache,
                &mut viewport,
                canvas(),
            )
            .unwrap();
        assert!(viewport.transform(ViewId::first()).scale < 1.0);
    }

    #[test]
    fn zoom_anchor_is_still_center_anchored() {
        let (catalog, mut center_cache, mut center_viewport) = test_parts();
        let mut center_state = AppState::default();
        center_state
            .apply(
                Command::Zoom {
                    factor: 2.0,
                    anchor: ZoomAnchor::Center,
                },
                &catalog,
                &mut center_cache,
                &mut center_viewport,
                canvas(),
            )
            .unwrap();

        let (_catalog, mut cursor_cache, mut cursor_viewport) = test_parts();
        let mut cursor_state = AppState::default();
        cursor_state
            .apply(
                Command::Zoom {
                    factor: 2.0,
                    anchor: ZoomAnchor::Canvas {
                        canvas_x: 0.1,
                        canvas_y: 0.9,
                    },
                },
                &catalog,
                &mut cursor_cache,
                &mut cursor_viewport,
                canvas(),
            )
            .unwrap();

        assert_eq!(
            center_viewport.transform(ViewId::first()),
            cursor_viewport.transform(ViewId::first())
        );
    }

    #[test]
    fn reload_success_resets_navigation_and_requests_cache_clear() {
        let (catalog, mut cache, mut viewport) = test_parts();
        let mut state = AppState::default();
        state
            .apply(
                Command::DrillAt {
                    canvas_x: 0.5,
                    canvas_y: 0.5,
                },
                &catalog,
                &mut cache,
                &mut viewport,
                canvas(),
            )
            .unwrap();

        let result = state
            .apply(
                Command::ReloadSucceeded,
                &catalog,
                &mut cache,
                &mut viewport,
                canvas(),
            )
            .unwrap();

        assert_eq!(state.current(), ViewId::first());
        assert!(state.render_frame().breadcrumbs.is_empty());
        assert_eq!(result.effect, Some(Effect::ClearImageCache));
    }

    #[test]
    fn reload_failure_keeps_current_view() {
        let (catalog, mut cache, mut viewport) = test_parts();
        let mut state = AppState::default();
        state
            .apply(
                Command::DrillAt {
                    canvas_x: 0.5,
                    canvas_y: 0.5,
                },
                &catalog,
                &mut cache,
                &mut viewport,
                canvas(),
            )
            .unwrap();

        state
            .apply(
                Command::ReloadFailed,
                &catalog,
                &mut cache,
                &mut viewport,
                canvas(),
            )
            .unwrap();

        assert_eq!(state.current(), ViewId::new(1));
    }

    #[test]
    fn help_and_reload_commands_request_effects_without_tty() {
        let (catalog, mut cache, mut viewport) = test_parts();
        let mut state = AppState::default();

        let help = state
            .apply(Command::Help, &catalog, &mut cache, &mut viewport, canvas())
            .unwrap();
        assert_eq!(help.effect, Some(Effect::ShowHelp));
        assert!(!help.render);

        let reload = state
            .apply(
                Command::Reload,
                &catalog,
                &mut cache,
                &mut viewport,
                canvas(),
            )
            .unwrap();
        assert_eq!(reload.effect, Some(Effect::ReloadWorkspace));
        assert!(!reload.render);
    }
}

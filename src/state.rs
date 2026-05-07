use crate::event::{Command, ZoomAnchor};
use crate::ids::{ElementId, ViewId};
use tui_kit::layout::{CanvasMetrics, ViewTransform};
use crate::view::ViewStore;
use anyhow::Result;

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
        store: &mut ViewStore,
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
                result.effect = Some(Effect::OpenPicker);
                result.render = false;
            }
            Command::SelectView(next) => {
                self.current = next;
                self.breadcrumbs.clear();
                self.pinned_element = None;
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
            Command::Back => {
                if let Some(previous) = self.breadcrumbs.pop() {
                    self.current = previous;
                    self.pinned_element = None;
                }
            }
            Command::ShowLegend => {
                let Some(legend_key) = store.view(self.current).key_view_key.clone() else {
                    result.render = false;
                    return Ok(result);
                };
                let Some(target) = store
                    .views
                    .iter()
                    .position(|v| v.key == legend_key)
                    .map(ViewId::new)
                else {
                    result.render = false;
                    return Ok(result);
                };
                self.breadcrumbs.push(self.current);
                self.current = target;
                self.last_drag = None;
                self.pinned_element = None;
            }
            Command::DrillAt { canvas_x, canvas_y } => {
                if let Some(child) = store.child_view_at_canvas_point(
                    self.current,
                    canvas_x,
                    canvas_y,
                    canvas,
                )? {
                    self.breadcrumbs.push(self.current);
                    self.current = child;
                    self.last_drag = None;
                } else if let Some(element) =
                    store.element_at_canvas_point(self.current, canvas_x, canvas_y, canvas)?
                {
                    self.pinned_element = Some(element);
                } else {
                    result.render = false;
                }
            }
            Command::InspectAt { canvas_x, canvas_y } => {
                self.pinned_element =
                    store.element_at_canvas_point(self.current, canvas_x, canvas_y, canvas)?;
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
                self.zoom_current(store, factor, anchor, canvas)?;
            }
            Command::ResetView => {
                store.set_transform(self.current, ViewTransform::fit());
            }
            Command::Pan {
                dx_fraction,
                dy_fraction,
            } => {
                self.pan_current(store, dx_fraction, dy_fraction, canvas)?;
            }
            Command::DragTo { x, y, canvas: drag_canvas } => {
                if let Some((last_x, last_y)) = self.last_drag {
                    let canvas_cols = drag_canvas.cells.cols.max(1);
                    let canvas_rows = drag_canvas.cells.rows.max(1);
                    let dx = (f32::from(last_x) - f32::from(x)) / f32::from(canvas_cols);
                    let dy = (f32::from(last_y) - f32::from(y)) / f32::from(canvas_rows);
                    self.pan_current(store, dx, dy, drag_canvas)?;
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

    fn reset_navigation(&mut self) {
        self.current = ViewId::first();
        self.breadcrumbs.clear();
        self.last_drag = None;
        self.pinned_element = None;
    }

    fn zoom_current(
        &self,
        store: &mut ViewStore,
        factor: f32,
        anchor: ZoomAnchor,
        canvas: CanvasMetrics,
    ) -> Result<()> {
        let raster = store.rendered_view(self.current)?.raster_size;
        let (anchor_x, anchor_y) = anchor.coordinates();
        let transform = store
            .transform(self.current)
            .zoomed_at(factor, anchor_x, anchor_y, raster, canvas);
        store.set_transform(self.current, transform);
        Ok(())
    }

    fn pan_current(
        &self,
        store: &mut ViewStore,
        horizontal: f32,
        vertical: f32,
        canvas: CanvasMetrics,
    ) -> Result<()> {
        let raster = store.rendered_view(self.current)?.raster_size;
        let transform = store
            .transform(self.current)
            .panned(horizontal, vertical, raster, canvas);
        store.set_transform(self.current, transform);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderFrame {
    pub current: ViewId,
    pub breadcrumbs: Vec<ViewId>,
    pub pinned_element: Option<ElementId>,
    pub render_progress: Option<(usize, usize)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UpdateResult {
    pub effect: Option<Effect>,
    pub render: bool,
    pub canvas: CanvasMetrics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Quit,
    OpenPicker,
    ReloadWorkspace,
    ClearImageCache,
    ShowHelp,
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
    use crate::ids::ElementId;
    use tui_kit::layout::{CellPixel, CellSize};
    use crate::render::RasterBudget;
    use crate::workspace::ViewInfo;
    use std::collections::{HashMap, HashSet};
    use std::fs;

    fn canvas() -> CanvasMetrics {
        CanvasMetrics::new(CellSize::new(80, 24), CellPixel::new(8, 16))
    }

    fn budget() -> RasterBudget {
        RasterBudget {
            quality: 1.0,
            ..RasterBudget::default()
        }
    }

    fn test_store() -> ViewStore {
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

        let mut child_view_by_element_id = HashMap::new();
        child_view_by_element_id.insert(ElementId::new("1"), "child".to_owned());

        ViewStore::new(
            vec![
                ViewInfo {
                    key: "parent".to_owned(),
                    name: "Parent".to_owned(),
                    kind: crate::workspace::ViewKind::SystemContext,
                    description: None,
                    svg_path: parent_svg,
                    element_ids: HashSet::from([ElementId::new("1")]),
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
                    element_ids: HashSet::new(),
                    child_view_by_element_id: HashMap::new(),
                    primary_view_key: None,
                    key_view_key: None,
                },
            ],
            budget(),
        )
        .unwrap()
    }

    #[test]
    fn select_view_clears_breadcrumbs() {
        let mut store = test_store();
        let mut state = AppState::default();
        state
            .apply(
                Command::DrillAt {
                    canvas_x: 0.5,
                    canvas_y: 0.5,
                },
                &mut store,
                canvas(),
            )
            .unwrap();

        state
            .apply(Command::SelectView(ViewId::first()), &mut store, canvas())
            .unwrap();

        assert_eq!(state.current(), ViewId::first());
        assert!(state.render_frame().breadcrumbs.is_empty());
    }

    #[test]
    fn drill_and_back_update_navigation_without_tty() {
        let mut store = test_store();
        let mut state = AppState::default();

        state
            .apply(
                Command::DrillAt {
                    canvas_x: 0.5,
                    canvas_y: 0.5,
                },
                &mut store,
                canvas(),
            )
            .unwrap();
        assert_eq!(state.current(), ViewId::new(1));
        assert_eq!(state.render_frame().breadcrumbs, &[ViewId::first()]);

        state.apply(Command::Back, &mut store, canvas()).unwrap();
        assert_eq!(state.current(), ViewId::first());
        assert!(state.render_frame().breadcrumbs.is_empty());
    }

    #[test]
    fn zoom_pan_and_reset_update_transform_without_tty() {
        let mut store = test_store();
        let mut state = AppState::default();

        state
            .apply(
                Command::Zoom {
                    factor: 2.0,
                    anchor: ZoomAnchor::Center,
                },
                &mut store,
                canvas(),
            )
            .unwrap();
        let scale_after_zoom = store.transform(ViewId::first()).scale;
        assert!((scale_after_zoom - 2.0).abs() < 0.01);

        state
            .apply(
                Command::Pan {
                    dx_fraction: 0.1,
                    dy_fraction: 0.1,
                },
                &mut store,
                canvas(),
            )
            .unwrap();
        assert!(store.transform(ViewId::first()).center_x > 0.5);

        state
            .apply(Command::ResetView, &mut store, canvas())
            .unwrap();
        assert_eq!(store.transform(ViewId::first()), ViewTransform::fit());
    }

    #[test]
    fn zoom_below_one_keeps_image_visible_below_fit() {
        let mut store = test_store();
        let mut state = AppState::default();
        state
            .apply(
                Command::Zoom {
                    factor: 0.5,
                    anchor: ZoomAnchor::Center,
                },
                &mut store,
                canvas(),
            )
            .unwrap();
        assert!(store.transform(ViewId::first()).scale < 1.0);
    }

    #[test]
    fn reload_success_resets_navigation_and_requests_cache_clear() {
        let mut store = test_store();
        let mut state = AppState::default();
        state
            .apply(
                Command::DrillAt {
                    canvas_x: 0.5,
                    canvas_y: 0.5,
                },
                &mut store,
                canvas(),
            )
            .unwrap();

        let result = state
            .apply(Command::ReloadSucceeded, &mut store, canvas())
            .unwrap();

        assert_eq!(state.current(), ViewId::first());
        assert!(state.render_frame().breadcrumbs.is_empty());
        assert_eq!(result.effect, Some(Effect::ClearImageCache));
    }

    #[test]
    fn reload_failure_keeps_current_view() {
        let mut store = test_store();
        let mut state = AppState::default();
        state
            .apply(
                Command::DrillAt {
                    canvas_x: 0.5,
                    canvas_y: 0.5,
                },
                &mut store,
                canvas(),
            )
            .unwrap();

        state
            .apply(Command::ReloadFailed, &mut store, canvas())
            .unwrap();

        assert_eq!(state.current(), ViewId::new(1));
    }

    #[test]
    fn help_and_reload_commands_request_effects_without_tty() {
        let mut store = test_store();
        let mut state = AppState::default();

        let help = state.apply(Command::Help, &mut store, canvas()).unwrap();
        assert_eq!(help.effect, Some(Effect::ShowHelp));
        assert!(!help.render);

        let reload = state
            .apply(Command::Reload, &mut store, canvas())
            .unwrap();
        assert_eq!(reload.effect, Some(Effect::ReloadWorkspace));
        assert!(!reload.render);
    }
}

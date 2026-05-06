use crate::event::Command;
use crate::ids::ViewId;
use crate::view::{ViewStore, ViewTransform};
use anyhow::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppState {
    current: ViewId,
    breadcrumbs: Vec<ViewId>,
    last_drag: Option<(u16, u16)>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            current: ViewId::first(),
            breadcrumbs: Vec::new(),
            last_drag: None,
        }
    }
}

impl AppState {
    pub const fn current(&self) -> ViewId {
        self.current
    }

    pub fn render_frame(&self) -> RenderFrame {
        RenderFrame {
            current: self.current,
            breadcrumbs: self.breadcrumbs.clone(),
        }
    }

    pub fn apply(&mut self, command: Command, store: &mut ViewStore) -> Result<UpdateResult> {
        let mut result = UpdateResult::default();

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
                }
            }
            Command::DrillAt { canvas_x, canvas_y } => {
                if let Some(child) =
                    store.child_view_at_canvas_point(self.current, canvas_x, canvas_y)?
                {
                    self.breadcrumbs.push(self.current);
                    self.current = child;
                    self.last_drag = None;
                } else {
                    result.render = false;
                }
            }
            Command::Zoom { factor, center } => {
                self.zoom_current(store, factor, center)?;
            }
            Command::ResetView => {
                store.set_transform(self.current, ViewTransform::reset());
            }
            Command::Pan {
                dx_fraction,
                dy_fraction,
            } => {
                self.pan_current(store, dx_fraction, dy_fraction)?;
            }
            Command::DragTo {
                x,
                y,
                canvas_cols,
                canvas_rows,
            } => {
                if let Some((last_x, last_y)) = self.last_drag {
                    let dx = (f32::from(last_x) - f32::from(x)) / f32::from(canvas_cols.max(1));
                    let dy = (f32::from(last_y) - f32::from(y)) / f32::from(canvas_rows.max(1));
                    self.pan_current(store, dx, dy)?;
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
    }

    fn zoom_current(&self, store: &mut ViewStore, factor: f32, center: (f32, f32)) -> Result<()> {
        let (width, height) = {
            let rendered = store.rendered_view(self.current)?;
            (rendered.width, rendered.height)
        };
        let transform = store
            .transform(self.current)
            .zoomed(factor, center.0, center.1, width, height);
        store.set_transform(self.current, transform);
        Ok(())
    }

    fn pan_current(&self, store: &mut ViewStore, horizontal: f32, vertical: f32) -> Result<()> {
        let (width, height) = {
            let rendered = store.rendered_view(self.current)?;
            (rendered.width, rendered.height)
        };
        let transform = store.transform(self.current);
        let rect = transform.source_rect(width, height);
        let dx = rect.width as f32 * horizontal;
        let dy = rect.height as f32 * vertical;
        store.set_transform(self.current, transform.panned(dx, dy, width, height));
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderFrame {
    pub current: ViewId,
    pub breadcrumbs: Vec<ViewId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateResult {
    pub effect: Option<Effect>,
    pub render: bool,
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ElementId;
    use crate::workspace::ViewInfo;
    use std::collections::{HashMap, HashSet};
    use std::fs;

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
            r#"<svg width="100" height="100"><g id="1"><rect x="10" y="10" width="40" height="40"/></g></svg>"#,
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
                    view_type: "SystemContext".to_owned(),
                    svg_path: parent_svg,
                    element_ids: HashSet::from([ElementId::new("1")]),
                    child_view_by_element_id,
                },
                ViewInfo {
                    key: "child".to_owned(),
                    name: "Child".to_owned(),
                    view_type: "Container".to_owned(),
                    svg_path: child_svg,
                    element_ids: HashSet::new(),
                    child_view_by_element_id: HashMap::new(),
                },
            ],
            1.0,
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
                    canvas_x: 0.2,
                    canvas_y: 0.2,
                },
                &mut store,
            )
            .unwrap();

        state
            .apply(Command::SelectView(ViewId::first()), &mut store)
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
                    canvas_x: 0.2,
                    canvas_y: 0.2,
                },
                &mut store,
            )
            .unwrap();
        assert_eq!(state.current(), ViewId::new(1));
        assert_eq!(state.render_frame().breadcrumbs, &[ViewId::first()]);

        state.apply(Command::Back, &mut store).unwrap();
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
                    center: (0.5, 0.5),
                },
                &mut store,
            )
            .unwrap();
        assert_eq!(store.transform(ViewId::first()).scale, 2.0);

        state
            .apply(
                Command::Pan {
                    dx_fraction: 0.1,
                    dy_fraction: 0.1,
                },
                &mut store,
            )
            .unwrap();
        assert!(store.transform(ViewId::first()).offset_x > 0.0);

        state.apply(Command::ResetView, &mut store).unwrap();
        assert_eq!(store.transform(ViewId::first()), ViewTransform::reset());
    }

    #[test]
    fn reload_success_resets_navigation_and_requests_cache_clear() {
        let mut store = test_store();
        let mut state = AppState::default();
        state
            .apply(
                Command::DrillAt {
                    canvas_x: 0.2,
                    canvas_y: 0.2,
                },
                &mut store,
            )
            .unwrap();

        let result = state.apply(Command::ReloadSucceeded, &mut store).unwrap();

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
                    canvas_x: 0.2,
                    canvas_y: 0.2,
                },
                &mut store,
            )
            .unwrap();

        state.apply(Command::ReloadFailed, &mut store).unwrap();

        assert_eq!(state.current(), ViewId::new(1));
    }

    #[test]
    fn help_and_reload_commands_request_effects_without_tty() {
        let mut store = test_store();
        let mut state = AppState::default();

        let help = state.apply(Command::Help, &mut store).unwrap();
        assert_eq!(help.effect, Some(Effect::ShowHelp));
        assert!(!help.render);

        let reload = state.apply(Command::Reload, &mut store).unwrap();
        assert_eq!(reload.effect, Some(Effect::ReloadWorkspace));
        assert!(!reload.render);
    }
}

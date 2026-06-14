use crate::backend::TerminalBackend;
use crate::config::AppConfig;
use crate::event::Command;
use crate::log_view::LogView;
use crate::logger::SharedLogBuffer;
use crate::modal::ActiveModal;
use crate::modal_factory::{spawn_modal, SCOPE_LINK_DIRECTORY, SCOPE_PICKER};
use crate::render_cache::RenderCache;
use crate::render_pool::{RenderPriority, RenderScheduler};
use crate::state::{AppState, Effect, RenderFrame, UpdateResult};
use crate::view_catalog::ViewCatalog;
use crate::viewport_state::{diagram_placement_policy, ViewportState};
use crate::workspace::{discover_views, export_workspace, load_workspace_model, WorkspaceSource};
use anyhow::Result;
use log::{error, info};
use tui_kit::focus::{FocusId, FocusManager, FocusNode, FocusScopeKind};
use tui_kit::layout::CanvasMetrics;

pub(crate) const SCOPE_ROOT: &str = "root";
pub(crate) const SCOPE_DIALOG: &str = "dialog";
pub(crate) const SCOPE_LOG: &str = "log";

#[derive(Debug)]
pub(crate) struct DialogSlot {
    pub(crate) dismissable: bool,
}

pub(crate) struct EffectRunner<'a, B: TerminalBackend> {
    pub terminal: &'a mut B,
    pub catalog: &'a mut ViewCatalog,
    pub render_cache: &'a mut RenderCache,
    pub viewport: &'a mut ViewportState,
    pub state: &'a mut AppState,
    pub scheduler: &'a mut RenderScheduler,
    pub focus: &'a mut FocusManager,
    pub active_modal: &'a mut Option<ActiveModal>,
    pub dialog_slot: &'a mut Option<DialogSlot>,
    pub config: &'a AppConfig,
    pub workspace: &'a WorkspaceSource,
    pub svg_format: &'a str,
    pub log_buffer: SharedLogBuffer,
    pub quit: &'a mut bool,
}

impl<B: TerminalBackend> EffectRunner<'_, B> {
    pub(crate) fn handle_update(&mut self, update: UpdateResult) -> Result<()> {
        self.request_active_render();

        match update.effect {
            Some(Effect::Quit) => {
                *self.quit = true;
            }
            Some(Effect::OpenModal(spec)) => {
                self.open_modal(spec)?;
            }
            Some(Effect::ReloadWorkspace) => {
                self.reload_workspace(update.canvas)?;
            }
            Some(Effect::ClearImageCache) => {
                self.terminal.clear_image_cache()?;
                self.render_root()?;
            }
            Some(Effect::ToggleLogView) => {
                toggle_log_view(
                    self.focus,
                    self.active_modal,
                    self.dialog_slot,
                    self.log_buffer.clone(),
                );
                self.redraw_for_mode()?;
            }
            Some(Effect::ShowHelp) => {
                self.terminal.show_help(&self.config.keys)?;
                enter_dialog(self.focus, self.active_modal, self.dialog_slot, true);
            }
            None if update.render => {
                self.render_root()?;
            }
            None => {}
        }

        Ok(())
    }

    fn open_modal(&mut self, spec: crate::state::ModalSpec) -> Result<()> {
        self.terminal
            .teardown_image_viewport(self.state.current())?;

        let spawn = spawn_modal(spec, self.catalog, self.state);
        if let Some(view_id) = spawn.initial_hover {
            self.request_hover_render(view_id);
        }

        self.focus
            .push_scope(
                spawn.scope_id,
                FocusScopeKind::Modal,
                vec![FocusNode::new(spawn.focus_node_id)],
            )
            .expect("modal scope is well-formed");
        *self.active_modal = Some(spawn.active_modal);

        if let Some(active) = self.active_modal.as_mut() {
            self.terminal
                .render_modal(active.as_modal_mut(), self.catalog, self.render_cache)?;
        }
        Ok(())
    }

    fn reload_workspace(&mut self, canvas: CanvasMetrics) -> Result<()> {
        self.terminal
            .show_message("Reloading workspace...", "Re-running Structurizr export.")?;
        match self.reload_store() {
            Ok(()) => {
                let update = self.state.apply(
                    Command::ReloadSucceeded,
                    self.catalog,
                    self.render_cache,
                    self.viewport,
                    canvas,
                )?;
                if update.effect == Some(Effect::ClearImageCache) {
                    self.terminal.clear_image_cache()?;
                }
            }
            Err(error) => {
                error!("reload failed: {error:#}");
                self.state.apply(
                    Command::ReloadFailed,
                    self.catalog,
                    self.render_cache,
                    self.viewport,
                    canvas,
                )?;
                self.terminal
                    .show_error("Reload failed", &format!("{error:#}"))?;
                enter_dialog(self.focus, self.active_modal, self.dialog_slot, true);
                return Ok(());
            }
        }
        self.render_root()
    }

    fn reload_store(&mut self) -> Result<()> {
        info!("reloading workspace {}", self.workspace.path.display());
        let exported = export_workspace(self.workspace, self.svg_format)?;
        let views = discover_views(&exported)?;
        let model = load_workspace_model(&exported);
        *self.catalog = ViewCatalog::new(views)?
            .with_model(model)
            .with_export(exported);
        *self.render_cache = RenderCache::new(self.config.raster_budget);
        *self.viewport = ViewportState::new(diagram_placement_policy(&self.config.placement));
        self.scheduler.invalidate_all();
        let render_jobs = self.render_cache.render_jobs(self.catalog);
        let render_budget = self.render_cache.budget();
        self.scheduler
            .request_all(render_jobs, RenderPriority::Background, render_budget);
        Ok(())
    }

    fn redraw_for_mode(&mut self) -> Result<()> {
        match active_scope(self.focus) {
            SCOPE_PICKER | SCOPE_LINK_DIRECTORY | SCOPE_LOG => {
                if let Some(active) = self.active_modal.as_mut() {
                    self.terminal.render_modal(
                        active.as_modal_mut(),
                        self.catalog,
                        self.render_cache,
                    )?;
                }
                Ok(())
            }
            SCOPE_DIALOG => Ok(()),
            _ => self.render_root(),
        }
    }

    fn render_root(&mut self) -> Result<()> {
        let frame = self.frame_with_progress();
        self.terminal
            .render(&frame, self.catalog, self.render_cache, self.viewport)
    }

    fn frame_with_progress(&self) -> RenderFrame {
        let mut frame = self.state.render_frame();
        let progress = self.scheduler.progress();
        if progress.pending > 0 {
            frame.render_progress =
                Some((progress.completed, progress.completed + progress.pending));
        }
        frame
    }

    fn request_hover_render(&mut self, view_id: crate::ids::ViewId) {
        if !self.render_cache.has_rendered(view_id) {
            let path = self.catalog.view(view_id).svg_path.clone();
            self.scheduler.request(
                view_id,
                RenderPriority::Hover,
                path,
                self.render_cache.budget(),
            );
        }
    }

    fn request_active_render(&mut self) {
        let view_id = self.state.current();
        if !self.render_cache.has_rendered(view_id) {
            let path = self.catalog.view(view_id).svg_path.clone();
            self.scheduler.request(
                view_id,
                RenderPriority::Active,
                path,
                self.render_cache.budget(),
            );
        }
    }
}

pub(crate) fn active_scope(focus: &FocusManager) -> &str {
    focus
        .active_scope_id()
        .map(FocusId::as_str)
        .unwrap_or(SCOPE_ROOT)
}

pub(crate) fn return_to_root(
    focus: &mut FocusManager,
    active_modal: &mut Option<ActiveModal>,
    dialog_slot: &mut Option<DialogSlot>,
) {
    while focus.active_scope_id().map(FocusId::as_str) != Some(SCOPE_ROOT) {
        focus.pop_scope();
    }
    *active_modal = None;
    *dialog_slot = None;
}

pub(crate) fn enter_dialog(
    focus: &mut FocusManager,
    active_modal: &mut Option<ActiveModal>,
    dialog_slot: &mut Option<DialogSlot>,
    dismissable: bool,
) {
    return_to_root(focus, active_modal, dialog_slot);
    focus
        .push_scope(
            SCOPE_DIALOG,
            FocusScopeKind::Modal,
            vec![FocusNode::new("dialog-ok")],
        )
        .expect("dialog scope is well-formed");
    *dialog_slot = Some(DialogSlot { dismissable });
}

fn toggle_log_view(
    focus: &mut FocusManager,
    active_modal: &mut Option<ActiveModal>,
    dialog_slot: &mut Option<DialogSlot>,
    log_buffer: SharedLogBuffer,
) {
    if matches!(active_modal, Some(ActiveModal::Log { .. })) {
        return_to_root(focus, active_modal, dialog_slot);
        return;
    }
    return_to_root(focus, active_modal, dialog_slot);
    focus
        .push_scope(
            SCOPE_LOG,
            FocusScopeKind::Modal,
            vec![FocusNode::new("log-pane")],
        )
        .expect("log scope is well-formed");
    *active_modal = Some(ActiveModal::Log {
        modal: Box::new(LogView::new(log_buffer)),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::fake::{FakeTerminalBackend, FakeTerminalCall};
    use crate::logger::LogBuffer;
    use crate::render::RasterBudget;
    use crate::workspace::{ViewInfo, ViewKind};
    use std::collections::{HashMap, HashSet};
    use std::num::NonZeroUsize;
    use std::path::PathBuf;
    use std::sync::{mpsc, Arc, Mutex};
    use tui_kit::focus::FocusConfig;

    struct Harness {
        terminal: FakeTerminalBackend,
        catalog: ViewCatalog,
        render_cache: RenderCache,
        viewport: ViewportState,
        state: AppState,
        scheduler: RenderScheduler,
        focus: FocusManager,
        active_modal: Option<ActiveModal>,
        dialog_slot: Option<DialogSlot>,
        config: AppConfig,
        workspace: WorkspaceSource,
        svg_format: String,
        log_buffer: SharedLogBuffer,
        quit: bool,
        _dir: tempfile::TempDir,
    }

    impl Harness {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let svg_path = dir.path().join("view.svg");
            std::fs::write(&svg_path, r#"<svg width="100" height="100"/>"#).unwrap();
            let catalog = ViewCatalog::new(vec![ViewInfo {
                key: "view".to_owned(),
                name: "View".to_owned(),
                kind: ViewKind::SystemContext,
                description: None,
                svg_path,
                element_ids: HashSet::new(),
                child_view_keys_by_element_id: HashMap::new(),
                primary_view_key: None,
                key_view_key: None,
            }])
            .unwrap();
            let (tx, _rx) = mpsc::channel();
            let focus = FocusManager::new(
                FocusConfig {
                    restore_on_scope_pop: true,
                    require_initial_focus: false,
                },
                vec![FocusNode::new("canvas")],
            )
            .unwrap();

            Self {
                terminal: FakeTerminalBackend::new(),
                catalog,
                render_cache: RenderCache::new(RasterBudget::default()),
                viewport: ViewportState::default(),
                state: AppState::default(),
                scheduler: RenderScheduler::new(NonZeroUsize::new(1).unwrap(), tx),
                focus,
                active_modal: None,
                dialog_slot: None,
                config: AppConfig::default(),
                workspace: WorkspaceSource {
                    path: PathBuf::from("workspace.dsl"),
                },
                svg_format: "svg".to_owned(),
                log_buffer: Arc::new(Mutex::new(LogBuffer::with_capacity(64))),
                quit: false,
                _dir: dir,
            }
        }

        fn handle_update(&mut self, update: UpdateResult) {
            let log_buffer = self.log_buffer.clone();
            let mut runner = EffectRunner {
                terminal: &mut self.terminal,
                catalog: &mut self.catalog,
                render_cache: &mut self.render_cache,
                viewport: &mut self.viewport,
                state: &mut self.state,
                scheduler: &mut self.scheduler,
                focus: &mut self.focus,
                active_modal: &mut self.active_modal,
                dialog_slot: &mut self.dialog_slot,
                config: &self.config,
                workspace: &self.workspace,
                svg_format: &self.svg_format,
                log_buffer,
                quit: &mut self.quit,
            };
            runner.handle_update(update).unwrap();
        }
    }

    #[test]
    fn clear_image_cache_effect_clears_and_renders_root() {
        let mut harness = Harness::new();

        harness.handle_update(UpdateResult {
            effect: Some(Effect::ClearImageCache),
            render: false,
            ..UpdateResult::default()
        });

        assert_eq!(
            harness.terminal.calls,
            vec![
                FakeTerminalCall::ClearImageCache,
                FakeTerminalCall::Render(crate::ids::ViewId::first()),
            ]
        );
    }

    #[test]
    fn show_help_effect_enters_dismissable_dialog_without_root_render() {
        let mut harness = Harness::new();

        harness.handle_update(UpdateResult {
            effect: Some(Effect::ShowHelp),
            render: false,
            ..UpdateResult::default()
        });

        assert_eq!(harness.terminal.calls, vec![FakeTerminalCall::ShowHelp]);
        assert_eq!(active_scope(&harness.focus), SCOPE_DIALOG);
        assert!(harness.dialog_slot.as_ref().unwrap().dismissable);
    }

    #[test]
    fn toggle_log_effect_opens_log_modal_and_renders_it() {
        let mut harness = Harness::new();

        harness.handle_update(UpdateResult {
            effect: Some(Effect::ToggleLogView),
            render: false,
            ..UpdateResult::default()
        });

        assert_eq!(harness.terminal.calls, vec![FakeTerminalCall::RenderModal]);
        assert_eq!(active_scope(&harness.focus), SCOPE_LOG);
        assert!(matches!(
            harness.active_modal,
            Some(ActiveModal::Log { .. })
        ));
    }
}

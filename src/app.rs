use crate::backend::TerminalBackend;
use crate::clipboard::Clipboard;
use crate::config::AppConfig;
use crate::effect_runner::{
    active_scope, enter_dialog, return_to_root, DialogSlot, EffectRunner, SCOPE_DIALOG, SCOPE_LOG,
    SCOPE_ROOT,
};
use crate::event::Command;
use crate::ids::ViewId;
use crate::keymap::{KeyMap, KeyMapExt};
use crate::logger::SharedLogBuffer;
use crate::modal::{ActiveModal, LogModalOutcome, NavModalOutcome};
use crate::modal_factory::{SCOPE_LINK_DIRECTORY, SCOPE_PICKER};
use crate::nav_items::NavTarget;
use crate::render_cache::RenderCache;
use crate::render_pool::{RenderPriority, RenderScheduler};
use crate::state::{AppState, Effect};
use crate::view_catalog::ViewCatalog;
use crate::viewport_state::{diagram_placement_policy, ViewportState};
use crate::workspace::{discover_views, export_workspace, load_workspace_model, WorkspaceSource};
use anyhow::Result;
use log::{error, info};
use std::collections::VecDeque;
use std::sync::mpsc::TryRecvError;
use tui_kit::events::{
    AppEvent, AppEventReceiver, AppEventSender, SchedulerEvent, TerminalEvent, WatcherEvent,
};
use tui_kit::focus::{FocusConfig, FocusManager, FocusNode};
use tui_kit::input::{InputEvent, KeyEvent};

pub struct App {
    catalog: ViewCatalog,
    render_cache: RenderCache,
    viewport: ViewportState,
    state: AppState,
    workspace: WorkspaceSource,
    svg_format: String,
    config: AppConfig,
    keymap: KeyMap,
    scheduler: RenderScheduler,
    quit: bool,
    focus: FocusManager,
    dialog_slot: Option<DialogSlot>,
    active_modal: Option<ActiveModal>,
    log_buffer: SharedLogBuffer,
    clipboard: Box<dyn Clipboard>,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("active_scope", &self.focus.active_scope_id())
            .field("quit", &self.quit)
            .finish_non_exhaustive()
    }
}

impl App {
    pub fn new(
        view_parts: (ViewCatalog, RenderCache, ViewportState),
        workspace: WorkspaceSource,
        svg_format: String,
        config: AppConfig,
        sink: AppEventSender,
        log_buffer: SharedLogBuffer,
        clipboard: Box<dyn Clipboard>,
    ) -> Self {
        let (catalog, render_cache, viewport) = view_parts;
        let keymap = <KeyMap as KeyMapExt>::from_app_config(&config);
        let mut scheduler = RenderScheduler::new(std::num::NonZeroUsize::new(1).unwrap(), sink);
        let render_jobs = render_cache.render_jobs(&catalog);
        let render_budget = render_cache.budget();
        scheduler.request_all(render_jobs, RenderPriority::Background, render_budget);
        let focus = FocusManager::new(
            FocusConfig {
                restore_on_scope_pop: true,
                require_initial_focus: false,
            },
            vec![FocusNode::new("canvas")],
        )
        .expect("root focus scope is well-formed");
        Self {
            catalog,
            render_cache,
            viewport,
            state: AppState::default(),
            workspace,
            svg_format,
            config,
            keymap,
            scheduler,
            quit: false,
            focus,
            dialog_slot: None,
            active_modal: None,
            log_buffer,
            clipboard,
        }
    }

    fn active_scope(&self) -> &str {
        active_scope(&self.focus)
    }

    /// Pop any active modal scope and clear its slot data. No-op at root.
    fn return_to_root(&mut self) {
        return_to_root(
            &mut self.focus,
            &mut self.active_modal,
            &mut self.dialog_slot,
        );
    }

    /// Recompute the placement policy from the current config and clear any
    /// placement state derived from the previous policy.
    fn apply_placement_change(&mut self) {
        let policy = diagram_placement_policy(&self.config.placement);
        self.viewport.set_placement_policy(policy);
    }

    /// Enter a dismissable dialog scope, replacing any current modal.
    fn enter_dialog(&mut self, dismissable: bool) {
        enter_dialog(
            &mut self.focus,
            &mut self.active_modal,
            &mut self.dialog_slot,
            dismissable,
        );
    }

    pub fn run(
        &mut self,
        terminal: &mut impl TerminalBackend,
        events: AppEventReceiver,
    ) -> Result<()> {
        terminal.render(
            &self.frame_with_progress(),
            &self.catalog,
            &mut self.render_cache,
            &mut self.viewport,
        )?;
        self.request_active_render();

        let mut pending_events = VecDeque::new();
        loop {
            let event = match pending_events.pop_front() {
                Some(event) => event,
                None => match events.recv() {
                    Ok(event) => event,
                    Err(_) => break,
                },
            };
            self.handle_event(event, terminal, &events, &mut pending_events)?;
            if self.quit {
                break;
            }
        }
        Ok(())
    }

    fn handle_event(
        &mut self,
        event: AppEvent,
        terminal: &mut impl TerminalBackend,
        events: &AppEventReceiver,
        pending_events: &mut VecDeque<AppEvent>,
    ) -> Result<()> {
        match event {
            AppEvent::Input(input) => self.handle_input_event(input, terminal),
            AppEvent::Scheduler(SchedulerEvent::Complete) => {
                let updated = self
                    .scheduler
                    .drain_into(&mut self.render_cache, &mut self.viewport);
                if !updated.is_empty() {
                    self.redraw_for_mode(terminal)?;
                }
                Ok(())
            }
            AppEvent::Watcher(WatcherEvent::WorkspaceChanged) => {
                terminal.show_message("Workspace changed", "Re-running Structurizr export.")?;
                match self.reload_store() {
                    Ok(()) => {
                        let canvas = terminal.canvas_metrics();
                        let update = self.state.apply(
                            Command::ReloadSucceeded,
                            &self.catalog,
                            &mut self.render_cache,
                            &mut self.viewport,
                            canvas,
                        )?;
                        if update.effect == Some(Effect::ClearImageCache) {
                            terminal.clear_image_cache()?;
                        }
                    }
                    Err(error) => {
                        error!("auto-reload failed: {error:#}");
                        let canvas = terminal.canvas_metrics();
                        self.state.apply(
                            Command::ReloadFailed,
                            &self.catalog,
                            &mut self.render_cache,
                            &mut self.viewport,
                            canvas,
                        )?;
                        terminal.show_error("Auto-reload failed", &format!("{error:#}"))?;
                        self.enter_dialog(true);
                        return Ok(());
                    }
                }
                self.return_to_root();
                self.redraw_for_mode(terminal)?;
                Ok(())
            }
            AppEvent::Terminal(TerminalEvent::Resize { cols, rows }) => {
                let (cols, rows, skipped) =
                    coalesce_resize_events(cols, rows, events, pending_events);
                if skipped > 0 {
                    log::info!("terminal resized to {cols}×{rows} after coalescing {skipped} resize events");
                } else {
                    log::info!("terminal resized to {cols}×{rows}");
                }
                self.redraw_for_mode(terminal)?;
                Ok(())
            }
            // The categorized AppEvent enum is non-exhaustive; ignore the
            // User variant since c4tui doesn't define a domain command type yet.
            _ => Ok(()),
        }
    }

    fn handle_input_event(
        &mut self,
        input: InputEvent,
        terminal: &mut impl TerminalBackend,
    ) -> Result<()> {
        match input {
            InputEvent::Key(key) => self.handle_key_event(key, terminal),
            InputEvent::Mouse(_) => {
                // Modal scopes (picker, link directory, log, dialog) absorb
                // mouse events without action — same behaviour as the pre-rename
                // `_ => Continue` arms in each modal's key handler. Only the
                // root scope routes mouse input to the root handler where the
                // keymap converts it into commands.
                if self.active_scope() == SCOPE_ROOT {
                    self.handle_input(input, terminal)
                } else {
                    Ok(())
                }
            }
            InputEvent::Resize { .. } => self.handle_input(input, terminal),
        }
    }

    fn handle_key_event(
        &mut self,
        key: KeyEvent,
        terminal: &mut impl TerminalBackend,
    ) -> Result<()> {
        match self.active_scope() {
            SCOPE_PICKER | SCOPE_LINK_DIRECTORY | SCOPE_LOG => self.handle_modal_key(key, terminal),
            SCOPE_DIALOG => {
                if self
                    .dialog_slot
                    .as_ref()
                    .map(|d| d.dismissable)
                    .unwrap_or(false)
                {
                    self.dialog_slot = None;
                    self.focus.pop_scope();
                    self.redraw_for_mode(terminal)?;
                }
                Ok(())
            }
            _ => self.handle_input(InputEvent::Key(key), terminal),
        }
    }

    /// Drive the currently-active modal through one key event. Replaces the
    /// pre-Task-6 trio (`handle_key_picker`, `handle_key_connection_picker`,
    /// `handle_key_log`); the variant inside `ActiveModal` is what
    /// discriminates instead of the focus scope.
    fn handle_modal_key(
        &mut self,
        key: KeyEvent,
        terminal: &mut impl TerminalBackend,
    ) -> Result<()> {
        // Capture the modal's response while the borrow on `active_modal`
        // is still alive, then drop it before touching other `self` fields.
        // `Step` records what the outer transition needs to do.
        enum Step {
            Idle,
            NavContinue { hover: Option<ViewId> },
            NavSelect(NavTarget),
            NavCancel,
            LogContinue,
            LogClose,
        }
        let step = match self.active_modal.as_mut() {
            None => Step::Idle,
            Some(ActiveModal::Nav { modal, .. }) => match modal.handle_key(key) {
                NavModalOutcome::Continue => Step::NavContinue {
                    hover: modal.currently_hovered_view(),
                },
                NavModalOutcome::Select(target) => Step::NavSelect(target),
                NavModalOutcome::Cancel => Step::NavCancel,
            },
            Some(ActiveModal::Log { modal }) => {
                match modal.handle_key(key, self.clipboard.as_ref())? {
                    LogModalOutcome::Continue => Step::LogContinue,
                    LogModalOutcome::Close => Step::LogClose,
                }
            }
        };

        match step {
            Step::Idle => Ok(()),
            Step::NavContinue { hover } => {
                if let Some(hover) = hover {
                    if !self.render_cache.has_rendered(hover) {
                        let path = self.catalog.view(hover).svg_path.clone();
                        self.scheduler.request(
                            hover,
                            RenderPriority::Hover,
                            path,
                            self.render_cache.budget(),
                        );
                    }
                }
                if let Some(active) = self.active_modal.as_mut() {
                    terminal.render_modal(
                        active.as_modal_mut(),
                        &self.catalog,
                        &self.render_cache,
                    )?;
                }
                Ok(())
            }
            Step::NavSelect(target) => {
                let modal = self
                    .active_modal
                    .take()
                    .expect("active modal present when handling Select");
                let ActiveModal::Nav { on_select, .. } = modal else {
                    unreachable!("Step::NavSelect implies Nav variant")
                };
                terminal.close_modal(&self.catalog)?;
                self.focus.pop_scope();
                let canvas = terminal.canvas_metrics();
                if let Some(command) = on_select(target, &mut self.state) {
                    self.state.apply(
                        command,
                        &self.catalog,
                        &mut self.render_cache,
                        &mut self.viewport,
                        canvas,
                    )?;
                }
                self.request_active_render();
                terminal.render(
                    &self.frame_with_progress(),
                    &self.catalog,
                    &mut self.render_cache,
                    &mut self.viewport,
                )?;
                Ok(())
            }
            Step::NavCancel => {
                self.active_modal = None;
                terminal.close_modal(&self.catalog)?;
                self.focus.pop_scope();
                terminal.render(
                    &self.frame_with_progress(),
                    &self.catalog,
                    &mut self.render_cache,
                    &mut self.viewport,
                )?;
                Ok(())
            }
            Step::LogContinue => {
                if let Some(active) = self.active_modal.as_mut() {
                    terminal.render_modal(
                        active.as_modal_mut(),
                        &self.catalog,
                        &self.render_cache,
                    )?;
                }
                Ok(())
            }
            Step::LogClose => {
                self.return_to_root();
                terminal.render(
                    &self.frame_with_progress(),
                    &self.catalog,
                    &mut self.render_cache,
                    &mut self.viewport,
                )?;
                Ok(())
            }
        }
    }

    fn redraw_for_mode(&mut self, terminal: &mut impl TerminalBackend) -> Result<()> {
        let frame = self.frame_with_progress();
        match self.active_scope() {
            SCOPE_PICKER | SCOPE_LINK_DIRECTORY | SCOPE_LOG => {
                if let Some(active) = self.active_modal.as_mut() {
                    terminal.render_modal(
                        active.as_modal_mut(),
                        &self.catalog,
                        &self.render_cache,
                    )?;
                }
                Ok(())
            }
            SCOPE_DIALOG => Ok(()),
            _ => terminal.render(
                &frame,
                &self.catalog,
                &mut self.render_cache,
                &mut self.viewport,
            ),
        }
    }

    fn handle_input(
        &mut self,
        input: InputEvent,
        terminal: &mut impl TerminalBackend,
    ) -> Result<()> {
        let canvas = terminal.canvas_metrics();
        let command = self.keymap.resolve(input, canvas);

        match command {
            Command::CycleScaleBasis => {
                self.config.placement.scale_basis = self.config.placement.scale_basis.cycle_next();
                self.apply_placement_change();
                self.request_active_render();
                terminal.render(
                    &self.frame_with_progress(),
                    &self.catalog,
                    &mut self.render_cache,
                    &mut self.viewport,
                )?;
                return Ok(());
            }
            Command::CycleOverflow => {
                self.config.placement.overflow = self.config.placement.overflow.cycle_next();
                self.apply_placement_change();
                self.request_active_render();
                terminal.render(
                    &self.frame_with_progress(),
                    &self.catalog,
                    &mut self.render_cache,
                    &mut self.viewport,
                )?;
                return Ok(());
            }
            Command::CycleZoomStep => {
                self.config.zoom = self.config.zoom.cycle_next();
                self.keymap = <KeyMap as KeyMapExt>::from_app_config(&self.config);
                self.request_active_render();
                terminal.render(
                    &self.frame_with_progress(),
                    &self.catalog,
                    &mut self.render_cache,
                    &mut self.viewport,
                )?;
                return Ok(());
            }
            _ => {}
        }

        let update = self.state.apply(
            command,
            &self.catalog,
            &mut self.render_cache,
            &mut self.viewport,
            canvas,
        )?;

        let log_buffer = self.log_buffer.clone();
        let mut runner = EffectRunner {
            terminal,
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
        runner.handle_update(update)
    }

    fn frame_with_progress(&self) -> crate::state::RenderFrame {
        let mut frame = self.state.render_frame();
        let progress = self.scheduler.progress();
        if progress.pending > 0 {
            frame.render_progress =
                Some((progress.completed, progress.completed + progress.pending));
        }
        frame
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

    fn reload_store(&mut self) -> Result<()> {
        info!("reloading workspace {}", self.workspace.path.display());
        let exported = export_workspace(&self.workspace, &self.svg_format)?;
        let views = discover_views(&exported)?;
        let model = load_workspace_model(&exported);
        self.catalog = ViewCatalog::new(views)?
            .with_model(model)
            .with_export(exported);
        self.render_cache = RenderCache::new(self.config.raster_budget);
        self.viewport = ViewportState::new(diagram_placement_policy(&self.config.placement));
        self.scheduler.invalidate_all();
        let render_jobs = self.render_cache.render_jobs(&self.catalog);
        let render_budget = self.render_cache.budget();
        self.scheduler
            .request_all(render_jobs, RenderPriority::Background, render_budget);
        Ok(())
    }
}

fn coalesce_resize_events(
    mut cols: u16,
    mut rows: u16,
    events: &AppEventReceiver,
    pending_events: &mut VecDeque<AppEvent>,
) -> (u16, u16, usize) {
    let mut skipped = 0;
    loop {
        match events.try_recv() {
            Ok(AppEvent::Terminal(TerminalEvent::Resize {
                cols: next_cols,
                rows: next_rows,
            })) => {
                cols = next_cols;
                rows = next_rows;
                skipped += 1;
            }
            Ok(other) => {
                pending_events.push_back(other);
                break;
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
        }
    }
    (cols, rows, skipped)
}

#[cfg(test)]
mod tests;

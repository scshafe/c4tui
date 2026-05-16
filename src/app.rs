use crate::backend::TerminalBackend;
use crate::clipboard::Clipboard;
use crate::config::AppConfig;
use crate::event::Command;
use crate::ids::ViewId;
use crate::keymap::{KeyMap, KeyMapExt};
use crate::log_view::LogView;
use crate::logger::SharedLogBuffer;
use crate::modal::{ActiveModal, LogModalOutcome, NavModalOutcome, NavPickerModal};
use crate::nav_items::{ConnectionNavItem, NavTarget, ViewNavItem};
use crate::nav_picker::{
    NavPicker, NavPickerConfig, NavPickerMode, NavRenderArtifact, ThumbnailCellArea,
};
use crate::render_pool::{RenderPriority, RenderScheduler};
use crate::state::{AppState, Effect};
use crate::view::{diagram_placement_policy, ViewStore};
use crate::workspace::{discover_views, export_workspace, load_workspace_model, WorkspaceSource};
use anyhow::Result;
use log::{error, info};
use std::collections::VecDeque;
use std::sync::mpsc::TryRecvError;
use tui_kit::component::{Cached, ComponentId};
use tui_kit::events::{
    AppEvent, AppEventReceiver, AppEventSender, SchedulerEvent, TerminalEvent, WatcherEvent,
};
use tui_kit::focus::{FocusConfig, FocusId, FocusManager, FocusNode, FocusScopeKind};
use tui_kit::input::{InputEvent, KeyEvent};

// Modal scope identifiers. c4tui's modes (picker, dialog, log viewer) push
// focus scopes with these IDs; routing reads `focus.active_scope_id()` to
// decide which path handles input or a redraw.
const SCOPE_ROOT: &str = "root";
const SCOPE_PICKER: &str = "picker";
const SCOPE_CONNECTION_PICKER: &str = "connection-picker";
const SCOPE_DIALOG: &str = "dialog";
const SCOPE_LOG: &str = "log";

#[derive(Debug)]
struct DialogSlot {
    dismissable: bool,
}

pub struct App {
    store: ViewStore,
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
        store: ViewStore,
        workspace: WorkspaceSource,
        svg_format: String,
        config: AppConfig,
        sink: AppEventSender,
        log_buffer: SharedLogBuffer,
        clipboard: Box<dyn Clipboard>,
    ) -> Self {
        let keymap = <KeyMap as KeyMapExt>::from_app_config(&config);
        let mut scheduler = RenderScheduler::new(std::num::NonZeroUsize::new(1).unwrap(), sink);
        scheduler.request_all(
            store.render_jobs(),
            RenderPriority::Background,
            store.budget(),
        );
        let focus = FocusManager::new(
            FocusConfig {
                restore_on_scope_pop: true,
                require_initial_focus: false,
            },
            vec![FocusNode::new("canvas")],
        )
        .expect("root focus scope is well-formed");
        Self {
            store,
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
        self.focus
            .active_scope_id()
            .map(FocusId::as_str)
            .unwrap_or(SCOPE_ROOT)
    }

    /// Pop any active modal scope and clear its slot data. No-op at root.
    fn return_to_root(&mut self) {
        while self.focus.active_scope_id().map(FocusId::as_str) != Some(SCOPE_ROOT) {
            self.focus.pop_scope();
        }
        self.active_modal = None;
        self.dialog_slot = None;
    }

    /// Recompute the placement policy from the current config and store it on
    /// `ViewStore`. Called after `B` / `O` cycles a placement option.
    fn apply_placement_change(&mut self) {
        let policy = diagram_placement_policy(&self.config.placement);
        // ViewStore exposes a `with_placement_policy` setter on a moved
        // value; use direct field access via a small helper to avoid moving.
        self.store.set_placement_policy(policy);
    }

    /// Toggle the log viewer scope. Closes if already open; opens otherwise.
    fn toggle_log_view(&mut self) {
        if matches!(self.active_modal, Some(ActiveModal::Log { .. })) {
            self.return_to_root();
            return;
        }
        self.return_to_root();
        self.focus
            .push_scope(
                SCOPE_LOG,
                FocusScopeKind::Modal,
                vec![FocusNode::new("log-pane")],
            )
            .expect("log scope is well-formed");
        self.active_modal = Some(ActiveModal::Log {
            modal: Box::new(LogView::new(self.log_buffer.clone())),
        });
    }

    /// Enter a dismissable dialog scope, replacing any current modal.
    fn enter_dialog(&mut self, dismissable: bool) {
        self.return_to_root();
        self.focus
            .push_scope(
                SCOPE_DIALOG,
                FocusScopeKind::Modal,
                vec![FocusNode::new("dialog-ok")],
            )
            .expect("dialog scope is well-formed");
        self.dialog_slot = Some(DialogSlot { dismissable });
    }

    #[allow(dead_code)]
    pub fn with_keymap(mut self, keymap: KeyMap) -> Self {
        self.keymap = keymap;
        self
    }

    pub fn run(
        &mut self,
        terminal: &mut impl TerminalBackend,
        events: AppEventReceiver,
    ) -> Result<()> {
        terminal.render(&self.frame_with_progress(), &mut self.store)?;
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
                let updated = self.scheduler.drain_into(&mut self.store);
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
                        let update =
                            self.state
                                .apply(Command::ReloadSucceeded, &mut self.store, canvas)?;
                        if update.effect == Some(Effect::ClearImageCache) {
                            terminal.clear_image_cache()?;
                        }
                    }
                    Err(error) => {
                        error!("auto-reload failed: {error:#}");
                        let canvas = terminal.canvas_metrics();
                        self.state
                            .apply(Command::ReloadFailed, &mut self.store, canvas)?;
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
                // Modal scopes (picker, connection picker, log, dialog) absorb
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
            SCOPE_PICKER | SCOPE_CONNECTION_PICKER | SCOPE_LOG => {
                self.handle_modal_key(key, terminal)
            }
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
                    if !self.store.has_rendered(hover) {
                        let path = self.store.view(hover).svg_path.clone();
                        self.scheduler.request(
                            hover,
                            RenderPriority::Hover,
                            path,
                            self.store.budget(),
                        );
                    }
                }
                if let Some(active) = self.active_modal.as_mut() {
                    terminal.render_modal(active.as_modal_mut(), &self.store)?;
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
                terminal.close_modal(&self.store)?;
                self.focus.pop_scope();
                let canvas = terminal.canvas_metrics();
                if let Some(command) = on_select(target, &mut self.state) {
                    self.state.apply(command, &mut self.store, canvas)?;
                }
                self.request_active_render();
                terminal.render(&self.frame_with_progress(), &mut self.store)?;
                Ok(())
            }
            Step::NavCancel => {
                self.active_modal = None;
                terminal.close_modal(&self.store)?;
                self.focus.pop_scope();
                terminal.render(&self.frame_with_progress(), &mut self.store)?;
                Ok(())
            }
            Step::LogContinue => {
                if let Some(active) = self.active_modal.as_mut() {
                    terminal.render_modal(active.as_modal_mut(), &self.store)?;
                }
                Ok(())
            }
            Step::LogClose => {
                self.return_to_root();
                terminal.render(&self.frame_with_progress(), &mut self.store)?;
                Ok(())
            }
        }
    }

    fn redraw_for_mode(&mut self, terminal: &mut impl TerminalBackend) -> Result<()> {
        let frame = self.frame_with_progress();
        match self.active_scope() {
            SCOPE_PICKER | SCOPE_CONNECTION_PICKER | SCOPE_LOG => {
                if let Some(active) = self.active_modal.as_mut() {
                    terminal.render_modal(active.as_modal_mut(), &self.store)?;
                }
                Ok(())
            }
            SCOPE_DIALOG => Ok(()),
            _ => terminal.render(&frame, &mut self.store),
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
                terminal.render(&self.frame_with_progress(), &mut self.store)?;
                return Ok(());
            }
            Command::CycleOverflow => {
                self.config.placement.overflow = self.config.placement.overflow.cycle_next();
                self.apply_placement_change();
                self.request_active_render();
                terminal.render(&self.frame_with_progress(), &mut self.store)?;
                return Ok(());
            }
            Command::CycleZoomStep => {
                self.config.zoom = self.config.zoom.cycle_next();
                self.keymap = <KeyMap as KeyMapExt>::from_app_config(&self.config);
                self.request_active_render();
                terminal.render(&self.frame_with_progress(), &mut self.store)?;
                return Ok(());
            }
            _ => {}
        }

        let update = self.state.apply(command, &mut self.store, canvas)?;
        self.request_active_render();

        match update.effect {
            Some(Effect::Quit) => {
                self.quit = true;
            }
            Some(Effect::OpenPicker) => {
                let current = self.state.current();
                terminal.teardown_image_viewport(current)?;
                let items = ViewNavItem::collect_all(&self.store.views, &self.store.model);
                let initial = items
                    .iter()
                    .position(|item| item.view_id == current)
                    .unwrap_or(0);
                let picker_inner = NavPicker::new(
                    NavPickerConfig {
                        id: ComponentId::new("c4tui-view-picker"),
                        title: " View Picker ".into(),
                        footer_hint:
                            " type → filter | Tab → legends | Enter → select | Esc → cancel "
                                .into(),
                        default_header: "Pick a view  —  type to filter, Enter to select, Esc to cancel, Tab to toggle key views".into(),
                        min_cell_cols: 22,
                        cell_rows: 8,
                        mode: NavPickerMode::Filterable {
                            allows_secondary_toggle: true,
                            secondary_label: "legends",
                        },
                    },
                    items,
                    initial,
                );
                let initial_hover = picker_inner
                    .selected()
                    .map(|item| item.view_id)
                    .unwrap_or(current);
                if !self.store.has_rendered(initial_hover) {
                    let path = self.store.view(initial_hover).svg_path.clone();
                    self.scheduler.request(
                        initial_hover,
                        RenderPriority::Hover,
                        path,
                        self.store.budget(),
                    );
                }
                self.focus
                    .push_scope(
                        SCOPE_PICKER,
                        FocusScopeKind::Modal,
                        vec![FocusNode::new("picker-list")],
                    )
                    .expect("picker scope is well-formed");
                let modal = NavPickerModal {
                    picker: Cached::new(picker_inner),
                    into_target: Box::new(NavTarget::View),
                    hovered_view: Box::new(|p| p.selected().map(|i| i.view_id)),
                    thumbnails: Box::new(thumbnails_from_view_picker),
                };
                self.active_modal = Some(ActiveModal::Nav {
                    modal: Box::new(modal),
                    on_select: Box::new(|target, _state| match target {
                        NavTarget::View(view_id) => Some(Command::SelectView(view_id)),
                        _ => None,
                    }),
                });
                if let Some(active) = self.active_modal.as_mut() {
                    terminal.render_modal(active.as_modal_mut(), &self.store)?;
                }
            }
            Some(Effect::OpenChildViewPicker { target_view_ids }) => {
                let current = self.state.current();
                terminal.teardown_image_viewport(current)?;
                let items = ViewNavItem::collect_for_view_ids(
                    &self.store.views,
                    &self.store.model,
                    &target_view_ids,
                );
                let initial_view = target_view_ids.first().copied().unwrap_or(current);
                let initial = items
                    .iter()
                    .position(|item| item.view_id == initial_view)
                    .unwrap_or(0);
                let picker_inner = NavPicker::new(
                    NavPickerConfig {
                        id: ComponentId::new("c4tui-child-view-picker"),
                        title: " Related Views ".into(),
                        footer_hint: " type → filter | Enter → drill | Esc → cancel ".into(),
                        default_header: "Pick a child view to drill into".into(),
                        min_cell_cols: 22,
                        cell_rows: 8,
                        mode: NavPickerMode::Filterable {
                            allows_secondary_toggle: false,
                            secondary_label: "",
                        },
                    },
                    items,
                    initial,
                );
                let initial_hover = picker_inner
                    .selected()
                    .map(|item| item.view_id)
                    .unwrap_or(initial_view);
                if !self.store.has_rendered(initial_hover) {
                    let path = self.store.view(initial_hover).svg_path.clone();
                    self.scheduler.request(
                        initial_hover,
                        RenderPriority::Hover,
                        path,
                        self.store.budget(),
                    );
                }
                self.focus
                    .push_scope(
                        SCOPE_PICKER,
                        FocusScopeKind::Modal,
                        vec![FocusNode::new("picker-list")],
                    )
                    .expect("picker scope is well-formed");
                let modal = NavPickerModal {
                    picker: Cached::new(picker_inner),
                    into_target: Box::new(NavTarget::ChildView),
                    hovered_view: Box::new(|p| p.selected().map(|i| i.view_id)),
                    thumbnails: Box::new(thumbnails_from_view_picker),
                };
                self.active_modal = Some(ActiveModal::Nav {
                    modal: Box::new(modal),
                    on_select: Box::new(|target, _state| match target {
                        NavTarget::ChildView(view_id) => Some(Command::SelectChildView(view_id)),
                        _ => None,
                    }),
                });
                if let Some(active) = self.active_modal.as_mut() {
                    terminal.render_modal(active.as_modal_mut(), &self.store)?;
                }
            }
            Some(Effect::OpenConnectionPicker { source_element_id }) => {
                let current = self.state.current();
                let candidates = self
                    .store
                    .connection_candidates_for_element(current, &source_element_id);
                let source_element_name = self
                    .store
                    .model
                    .elements
                    .get(&source_element_id)
                    .map(|e| e.name.clone())
                    .unwrap_or_else(|| source_element_id.to_string());
                terminal.teardown_image_viewport(current)?;
                let items = ConnectionNavItem::collect_from_candidates(candidates, &self.store);
                let picker_inner = NavPicker::new(
                    NavPickerConfig {
                        id: ComponentId::new("c4tui-connection-picker"),
                        title: " Connection Picker ".into(),
                        footer_hint: " Enter → navigate | Esc → cancel ".into(),
                        default_header: format!(
                            "Connections for {}  -  Enter to navigate, Esc to cancel",
                            source_element_name
                        ),
                        min_cell_cols: 34,
                        cell_rows: 5,
                        mode: NavPickerMode::Flat,
                    },
                    items,
                    0,
                );
                self.focus
                    .push_scope(
                        SCOPE_CONNECTION_PICKER,
                        FocusScopeKind::Modal,
                        vec![FocusNode::new("connection-picker-list")],
                    )
                    .expect("connection picker scope is well-formed");
                let modal: NavPickerModal<ConnectionNavItem> = NavPickerModal {
                    picker: Cached::new(picker_inner),
                    into_target: Box::new(NavTarget::Connection),
                    hovered_view: Box::new(|_| None),
                    thumbnails: Box::new(|_| Vec::new()),
                };
                self.active_modal = Some(ActiveModal::Nav {
                    modal: Box::new(modal),
                    on_select: Box::new(|target, _state| match target {
                        NavTarget::Connection(candidate) => {
                            Some(Command::SelectConnection(candidate))
                        }
                        _ => None,
                    }),
                });
                if let Some(active) = self.active_modal.as_mut() {
                    terminal.render_modal(active.as_modal_mut(), &self.store)?;
                }
            }
            Some(Effect::ReloadWorkspace) => {
                terminal
                    .show_message("Reloading workspace...", "Re-running Structurizr export.")?;
                match self.reload_store() {
                    Ok(()) => {
                        let update =
                            self.state
                                .apply(Command::ReloadSucceeded, &mut self.store, canvas)?;
                        if update.effect == Some(Effect::ClearImageCache) {
                            terminal.clear_image_cache()?;
                        }
                    }
                    Err(error) => {
                        error!("reload failed: {error:#}");
                        self.state
                            .apply(Command::ReloadFailed, &mut self.store, canvas)?;
                        terminal.show_error("Reload failed", &format!("{error:#}"))?;
                        self.enter_dialog(true);
                        return Ok(());
                    }
                }
                terminal.render(&self.frame_with_progress(), &mut self.store)?;
            }
            Some(Effect::ClearImageCache) => {
                terminal.clear_image_cache()?;
                terminal.render(&self.frame_with_progress(), &mut self.store)?;
            }
            Some(Effect::ToggleLogView) => {
                self.toggle_log_view();
                self.redraw_for_mode(terminal)?;
            }
            Some(Effect::ShowHelp) => {
                terminal.show_help(&self.config.keys)?;
                self.enter_dialog(true);
            }
            None if update.render => {
                terminal.render(&self.frame_with_progress(), &mut self.store)?;
            }
            None => {}
        }
        Ok(())
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
        if !self.store.has_rendered(view_id) {
            let path = self.store.view(view_id).svg_path.clone();
            self.scheduler
                .request(view_id, RenderPriority::Active, path, self.store.budget());
        }
    }

    fn reload_store(&mut self) -> Result<()> {
        info!("reloading workspace {}", self.workspace.path.display());
        let exported = export_workspace(&self.workspace, &self.svg_format)?;
        let views = discover_views(&exported)?;
        let model = load_workspace_model(&exported);
        self.store = ViewStore::new(views, self.config.raster_budget)?
            .with_model(model)
            .with_export(exported);
        self.scheduler.invalidate_all();
        self.scheduler.request_all(
            self.store.render_jobs(),
            RenderPriority::Background,
            self.store.budget(),
        );
        Ok(())
    }
}

/// Convert a `ViewNavItem` picker's per-frame artifacts into the thumbnail
/// anchors the terminal layer paints after rendering. Same shape as the
/// pre-Task-6 inline loop in `TerminalSession::draw_picker`.
fn thumbnails_from_view_picker(picker: &NavPicker<ViewNavItem>) -> Vec<ThumbnailCellArea> {
    picker
        .last_artifacts()
        .iter()
        .map(|artifact| match artifact {
            NavRenderArtifact::Thumbnail { id, area } => ThumbnailCellArea {
                view_id: id.view_id(),
                area: *area,
            },
        })
        .collect()
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
mod tests {
    use super::*;
    use crate::backend::fake::{FakeTerminalBackend, FakeTerminalCall};
    use crate::ids::{ElementId, RelationshipId, ViewId};
    use crate::render::RasterBudget;
    use crate::workspace::{
        ElementKind, ElementMetadata, RelationshipMetadata, ViewInfo, WorkspaceModel,
        WorkspaceSource,
    };
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;
    use std::sync::mpsc;
    use tui_kit::input::{InputEvent, KeyEvent, MouseEvent};

    fn budget() -> RasterBudget {
        RasterBudget {
            quality: 1.0,
            ..RasterBudget::default()
        }
    }

    fn test_app_with_real_svgs(dir: &std::path::Path, sink: AppEventSender) -> App {
        let parent = dir.join("parent.svg");
        let child = dir.join("child.svg");
        std::fs::write(&parent, r#"<svg width="100" height="100"/>"#).unwrap();
        std::fs::write(&child, r#"<svg width="100" height="100"/>"#).unwrap();
        let views = vec![
            ViewInfo {
                key: "parent".to_owned(),
                name: "Parent".to_owned(),
                kind: crate::workspace::ViewKind::SystemContext,
                description: None,
                svg_path: parent,
                element_ids: HashSet::new(),
                child_view_keys_by_element_id: HashMap::new(),
                primary_view_key: None,
                key_view_key: None,
            },
            ViewInfo {
                key: "child".to_owned(),
                name: "Child".to_owned(),
                kind: crate::workspace::ViewKind::Container,
                description: None,
                svg_path: child,
                element_ids: HashSet::new(),
                child_view_keys_by_element_id: HashMap::new(),
                primary_view_key: None,
                key_view_key: None,
            },
        ];

        let log_buffer = std::sync::Arc::new(std::sync::Mutex::new(
            crate::logger::LogBuffer::with_capacity(64),
        ));
        let clipboard: Box<dyn crate::clipboard::Clipboard> =
            Box::new(crate::clipboard::DefaultClipboard);
        App::new(
            ViewStore::new(views, budget()).unwrap(),
            WorkspaceSource {
                path: PathBuf::from("workspace.dsl"),
            },
            "svg".to_owned(),
            AppConfig::default(),
            sink,
            log_buffer,
            clipboard,
        )
    }

    fn test_app_with_connection_svgs(dir: &std::path::Path, sink: AppEventSender) -> App {
        let api_svg = dir.join("api.svg");
        let database_svg = dir.join("database.svg");
        std::fs::write(
            &api_svg,
            r#"<svg width="100" height="100"><g id="api"><rect x="10" y="10" width="80" height="80"/></g></svg>"#,
        )
        .unwrap();
        std::fs::write(&database_svg, r#"<svg width="100" height="100"/>"#).unwrap();

        let views = vec![
            ViewInfo {
                key: "api".to_owned(),
                name: "API".to_owned(),
                kind: crate::workspace::ViewKind::Container,
                description: None,
                svg_path: api_svg,
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
                svg_path: database_svg,
                element_ids: HashSet::from([ElementId::new("database")]),
                child_view_keys_by_element_id: HashMap::new(),
                primary_view_key: None,
                key_view_key: None,
            },
        ];
        let mut model = WorkspaceModel::default();
        model.elements.insert(
            ElementId::new("api"),
            ElementMetadata {
                id: ElementId::new("api"),
                name: "API".to_owned(),
                description: None,
                technology: None,
                tags: Vec::new(),
                kind: ElementKind::Container,
            },
        );
        model.elements.insert(
            ElementId::new("database"),
            ElementMetadata {
                id: ElementId::new("database"),
                name: "Database".to_owned(),
                description: None,
                technology: Some("PostgreSQL".to_owned()),
                tags: Vec::new(),
                kind: ElementKind::Container,
            },
        );
        let relationship = RelationshipMetadata {
            id: RelationshipId::new("r1"),
            source_id: ElementId::new("api"),
            destination_id: ElementId::new("database"),
            description: Some("Reads from".to_owned()),
            technology: None,
            tags: Vec::new(),
        };
        model
            .outgoing_relationships_by_element
            .insert(ElementId::new("api"), vec![relationship.id.clone()]);
        model
            .relationships
            .insert(relationship.id.clone(), relationship);

        let log_buffer = std::sync::Arc::new(std::sync::Mutex::new(
            crate::logger::LogBuffer::with_capacity(64),
        ));
        let clipboard: Box<dyn crate::clipboard::Clipboard> =
            Box::new(crate::clipboard::DefaultClipboard);
        App::new(
            ViewStore::new(views, budget()).unwrap().with_model(model),
            WorkspaceSource {
                path: PathBuf::from("workspace.dsl"),
            },
            "svg".to_owned(),
            AppConfig::default(),
            sink,
            log_buffer,
            clipboard,
        )
    }

    fn test_app_with_related_view_svgs(dir: &std::path::Path, sink: AppEventSender) -> App {
        let parent_svg = dir.join("system.svg");
        let containers_svg = dir.join("containers.svg");
        let flow_svg = dir.join("flow.svg");
        std::fs::write(
            &parent_svg,
            r#"<svg width="100" height="100"><g id="system"><rect x="10" y="10" width="80" height="80"/></g></svg>"#,
        )
        .unwrap();
        std::fs::write(&containers_svg, r#"<svg width="100" height="100"/>"#).unwrap();
        std::fs::write(&flow_svg, r#"<svg width="100" height="100"/>"#).unwrap();

        let mut related = HashMap::new();
        related.insert(
            ElementId::new("system"),
            vec!["containers".to_owned(), "flow".to_owned()],
        );
        let views = vec![
            ViewInfo {
                key: "system".to_owned(),
                name: "System Context".to_owned(),
                kind: crate::workspace::ViewKind::SystemContext,
                description: None,
                svg_path: parent_svg,
                element_ids: HashSet::from([ElementId::new("system")]),
                child_view_keys_by_element_id: related,
                primary_view_key: None,
                key_view_key: None,
            },
            ViewInfo {
                key: "containers".to_owned(),
                name: "Containers".to_owned(),
                kind: crate::workspace::ViewKind::Container,
                description: None,
                svg_path: containers_svg,
                element_ids: HashSet::new(),
                child_view_keys_by_element_id: HashMap::new(),
                primary_view_key: None,
                key_view_key: None,
            },
            ViewInfo {
                key: "flow".to_owned(),
                name: "Flow".to_owned(),
                kind: crate::workspace::ViewKind::Dynamic,
                description: None,
                svg_path: flow_svg,
                element_ids: HashSet::new(),
                child_view_keys_by_element_id: HashMap::new(),
                primary_view_key: None,
                key_view_key: None,
            },
        ];

        let log_buffer = std::sync::Arc::new(std::sync::Mutex::new(
            crate::logger::LogBuffer::with_capacity(64),
        ));
        let clipboard: Box<dyn crate::clipboard::Clipboard> =
            Box::new(crate::clipboard::DefaultClipboard);
        App::new(
            ViewStore::new(views, budget()).unwrap(),
            WorkspaceSource {
                path: PathBuf::from("workspace.dsl"),
            },
            "svg".to_owned(),
            AppConfig::default(),
            sink,
            log_buffer,
            clipboard,
        )
    }

    fn run_with_keys(app: &mut App, terminal: &mut FakeTerminalBackend, keys: &[KeyEvent]) {
        let (tx, rx) = mpsc::channel();
        for key in keys {
            tx.send(AppEvent::input_key(*key)).unwrap();
        }
        drop(tx);
        let _ = app.run(terminal, rx);
    }

    #[test]
    fn picker_navigation_selects_view_by_keystrokes() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, _rx) = mpsc::channel();
        let mut app = test_app_with_real_svgs(dir.path(), tx);
        let mut terminal = FakeTerminalBackend::new();

        run_with_keys(
            &mut app,
            &mut terminal,
            &[
                KeyEvent::Char('o'),
                KeyEvent::Down,
                KeyEvent::Enter,
                KeyEvent::Char('q'),
            ],
        );

        assert_eq!(app.state.current(), ViewId::new(1));
        assert_eq!(terminal.viewport_teardowns, vec![ViewId::first()]);
        assert_eq!(
            terminal.calls,
            vec![
                FakeTerminalCall::Render(ViewId::first()),
                FakeTerminalCall::TeardownImageViewport(ViewId::first()),
                FakeTerminalCall::RenderModal,
                FakeTerminalCall::RenderModal,
                FakeTerminalCall::CloseModal,
                FakeTerminalCall::Render(ViewId::new(1)),
            ]
        );
        assert!(terminal.modal_renders >= 1);
    }

    #[test]
    fn picker_cancel_runs_full_image_lifecycle_back_to_main_view() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, _rx) = mpsc::channel();
        let mut app = test_app_with_real_svgs(dir.path(), tx);
        let mut terminal = FakeTerminalBackend::new();

        run_with_keys(
            &mut app,
            &mut terminal,
            &[KeyEvent::Char('o'), KeyEvent::Esc, KeyEvent::Char('q')],
        );

        assert_eq!(app.state.current(), ViewId::first());
        assert_eq!(
            terminal.calls,
            vec![
                FakeTerminalCall::Render(ViewId::first()),
                FakeTerminalCall::TeardownImageViewport(ViewId::first()),
                FakeTerminalCall::RenderModal,
                FakeTerminalCall::CloseModal,
                FakeTerminalCall::Render(ViewId::first()),
            ]
        );
    }

    #[test]
    fn click_on_element_with_multiple_related_views_opens_picker_and_drills() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, _rx) = mpsc::channel();
        let mut app = test_app_with_related_view_svgs(dir.path(), tx);
        let mut terminal = FakeTerminalBackend::new();

        app.handle_input(
            InputEvent::Mouse(MouseEvent::Click { x: 41, y: 14 }),
            &mut terminal,
        )
        .unwrap();
        app.handle_key_event(KeyEvent::Down, &mut terminal).unwrap();
        app.handle_key_event(KeyEvent::Enter, &mut terminal)
            .unwrap();

        assert_eq!(app.state.current(), ViewId::new(2));
        assert_eq!(app.state.render_frame().breadcrumbs, &[ViewId::first()]);
        assert_eq!(
            terminal.calls,
            vec![
                FakeTerminalCall::TeardownImageViewport(ViewId::first()),
                FakeTerminalCall::RenderModal,
                FakeTerminalCall::RenderModal,
                FakeTerminalCall::CloseModal,
                FakeTerminalCall::Render(ViewId::new(2)),
            ]
        );
    }

    #[test]
    fn related_view_picker_cancel_preserves_current_view() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, _rx) = mpsc::channel();
        let mut app = test_app_with_related_view_svgs(dir.path(), tx);
        let mut terminal = FakeTerminalBackend::new();

        app.handle_input(
            InputEvent::Mouse(MouseEvent::Click { x: 41, y: 14 }),
            &mut terminal,
        )
        .unwrap();
        app.handle_key_event(KeyEvent::Esc, &mut terminal).unwrap();

        assert_eq!(app.state.current(), ViewId::first());
        assert!(app.state.render_frame().breadcrumbs.is_empty());
        assert_eq!(
            terminal.calls,
            vec![
                FakeTerminalCall::TeardownImageViewport(ViewId::first()),
                FakeTerminalCall::RenderModal,
                FakeTerminalCall::CloseModal,
                FakeTerminalCall::Render(ViewId::first()),
            ]
        );
    }

    #[test]
    fn connection_picker_selects_connection_by_keystrokes() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, _rx) = mpsc::channel();
        let mut app = test_app_with_connection_svgs(dir.path(), tx);
        let mut terminal = FakeTerminalBackend::new();

        run_with_keys(
            &mut app,
            &mut terminal,
            &[KeyEvent::Enter, KeyEvent::Enter, KeyEvent::Char('q')],
        );

        assert_eq!(app.state.current(), ViewId::new(1));
        assert_eq!(
            app.state.render_frame().pinned_element,
            Some(ElementId::new("database"))
        );
        assert_eq!(
            terminal.calls,
            vec![
                FakeTerminalCall::Render(ViewId::first()),
                FakeTerminalCall::TeardownImageViewport(ViewId::first()),
                FakeTerminalCall::RenderModal,
                FakeTerminalCall::CloseModal,
                FakeTerminalCall::Render(ViewId::new(1)),
            ]
        );
    }

    #[test]
    fn connection_picker_cancel_returns_to_current_view() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, _rx) = mpsc::channel();
        let mut app = test_app_with_connection_svgs(dir.path(), tx);
        let mut terminal = FakeTerminalBackend::new();

        run_with_keys(
            &mut app,
            &mut terminal,
            &[KeyEvent::Enter, KeyEvent::Esc, KeyEvent::Char('q')],
        );

        assert_eq!(app.state.current(), ViewId::first());
        assert_eq!(app.state.render_frame().pinned_element, None);
        assert_eq!(
            terminal.calls,
            vec![
                FakeTerminalCall::Render(ViewId::first()),
                FakeTerminalCall::TeardownImageViewport(ViewId::first()),
                FakeTerminalCall::RenderModal,
                FakeTerminalCall::CloseModal,
                FakeTerminalCall::Render(ViewId::first()),
            ]
        );
    }

    #[test]
    fn connection_picker_opens_empty_without_pinning_source() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, _rx) = mpsc::channel();
        let mut app = test_app_with_connection_svgs(dir.path(), tx);
        app.store.model.relationships.clear();
        app.store.model.outgoing_relationships_by_element.clear();
        app.store.model.incoming_relationships_by_element.clear();
        let mut terminal = FakeTerminalBackend::new();

        run_with_keys(
            &mut app,
            &mut terminal,
            &[KeyEvent::Enter, KeyEvent::Esc, KeyEvent::Char('q')],
        );

        assert_eq!(app.state.current(), ViewId::first());
        assert_eq!(app.state.render_frame().pinned_element, None);
        assert_eq!(
            terminal.calls,
            vec![
                FakeTerminalCall::Render(ViewId::first()),
                FakeTerminalCall::TeardownImageViewport(ViewId::first()),
                FakeTerminalCall::RenderModal,
                FakeTerminalCall::CloseModal,
                FakeTerminalCall::Render(ViewId::first()),
            ]
        );
    }

    #[test]
    fn run_uses_backend_for_help_dialog() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, _rx) = mpsc::channel();
        let mut app = test_app_with_real_svgs(dir.path(), tx);
        let mut terminal = FakeTerminalBackend::new();

        run_with_keys(
            &mut app,
            &mut terminal,
            &[
                KeyEvent::Char('?'),
                KeyEvent::Char(' '),
                KeyEvent::Char('q'),
            ],
        );

        assert_eq!(terminal.help_count, 1);
    }

    #[test]
    fn run_handles_hjkl_panning() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, _rx) = mpsc::channel();
        let mut app = test_app_with_real_svgs(dir.path(), tx);
        let mut terminal = FakeTerminalBackend::new();
        run_with_keys(
            &mut app,
            &mut terminal,
            &[
                KeyEvent::Char('+'),
                KeyEvent::Char('+'),
                KeyEvent::Char('+'),
                KeyEvent::Char('l'),
                KeyEvent::Char('q'),
            ],
        );
        assert!(app.store.transform(ViewId::first()).scale > 1.0);
        assert!(app.store.transform(ViewId::first()).center_x > 0.5);
    }

    #[test]
    fn resize_event_redraws_without_key_input() {
        let dir = tempfile::tempdir().unwrap();
        let (scheduler_tx, _scheduler_rx) = mpsc::channel();
        let mut app = test_app_with_real_svgs(dir.path(), scheduler_tx);
        let mut terminal = FakeTerminalBackend::new();
        let (event_tx, event_rx) = mpsc::channel();

        event_tx.send(AppEvent::terminal_resize(120, 40)).unwrap();
        event_tx.send(AppEvent::input_key(KeyEvent::CtrlC)).unwrap();
        drop(event_tx);

        app.run(&mut terminal, event_rx).unwrap();

        assert_eq!(terminal.rendered_frames.len(), 2);
    }

    #[test]
    fn resize_burst_coalesces_to_single_redraw() {
        let dir = tempfile::tempdir().unwrap();
        let (scheduler_tx, _scheduler_rx) = mpsc::channel();
        let mut app = test_app_with_real_svgs(dir.path(), scheduler_tx);
        let mut terminal = FakeTerminalBackend::new();
        let (event_tx, event_rx) = mpsc::channel();

        event_tx.send(AppEvent::terminal_resize(100, 30)).unwrap();
        event_tx.send(AppEvent::terminal_resize(120, 40)).unwrap();
        event_tx.send(AppEvent::terminal_resize(140, 50)).unwrap();
        event_tx.send(AppEvent::input_key(KeyEvent::CtrlC)).unwrap();
        drop(event_tx);

        app.run(&mut terminal, event_rx).unwrap();

        assert_eq!(terminal.rendered_frames.len(), 2);
        assert!(app.quit);
    }
}

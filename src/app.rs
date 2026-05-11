use crate::backend::TerminalBackend;
use crate::clipboard::Clipboard;
use crate::config::AppConfig;
use crate::event::{Command, InputEvent};
use crate::ids::ViewId;
use crate::keymap::{KeyMap, KeyMapExt};
use crate::log_view::{LogView, LogViewOutcome};
use crate::logger::SharedLogBuffer;
use crate::picker::{PickerOutcome, ViewPicker};
use crate::render_pool::{RenderPriority, RenderScheduler};
use crate::state::{AppState, Effect};
use crate::view::{diagram_placement_policy, ViewStore};
use crate::workspace::{discover_views, export_workspace, load_workspace_model, WorkspaceSource};
use anyhow::Result;
use log::{error, info};
use std::collections::VecDeque;
use std::sync::mpsc::TryRecvError;
use tui_kit::component::{Cached, Component, ComponentOutcome};
use tui_kit::events::{
    AppEvent, AppEventReceiver, AppEventSender, InputEvent as TuiKitInputEvent, SchedulerEvent,
    TerminalEvent, WatcherEvent,
};
use tui_kit::focus::{FocusConfig, FocusId, FocusManager, FocusNode, FocusScopeKind};
use tui_kit::input::Key;

// Modal scope identifiers. c4tui's modes (picker, dialog, log viewer) push
// focus scopes with these IDs; routing reads `focus.active_scope_id()` to
// decide which path handles input or a redraw.
const SCOPE_ROOT: &str = "root";
const SCOPE_PICKER: &str = "picker";
const SCOPE_DIALOG: &str = "dialog";
const SCOPE_LOG: &str = "log";

#[derive(Debug)]
struct PickerSlot {
    picker: Cached<ViewPicker>,
    last_hover: ViewId,
}

#[derive(Debug)]
struct DialogSlot {
    dismissable: bool,
}

#[derive(Debug)]
struct LogSlot {
    view: LogView,
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
    picker_slot: Option<PickerSlot>,
    dialog_slot: Option<DialogSlot>,
    log_slot: Option<LogSlot>,
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
            picker_slot: None,
            dialog_slot: None,
            log_slot: None,
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
        self.picker_slot = None;
        self.dialog_slot = None;
        self.log_slot = None;
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
        if self.log_slot.is_some() {
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
        self.log_slot = Some(LogSlot {
            view: LogView::new(self.log_buffer.clone()),
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
            AppEvent::Input(TuiKitInputEvent::Key(key)) => self.handle_key(key, terminal),
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

    fn handle_key(&mut self, key: Key, terminal: &mut impl TerminalBackend) -> Result<()> {
        match self.active_scope() {
            SCOPE_PICKER => self.handle_key_picker(key, terminal),
            SCOPE_LOG => self.handle_key_log(key, terminal),
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
            _ => {
                let input = terminal.translate_key(key);
                self.handle_input(input, terminal)
            }
        }
    }

    fn handle_key_log(&mut self, key: Key, terminal: &mut impl TerminalBackend) -> Result<()> {
        let outcome = {
            let Some(slot) = self.log_slot.as_mut() else {
                return Ok(());
            };
            slot.view.handle_key(key, self.clipboard.as_ref())?
        };
        match outcome {
            LogViewOutcome::Continue => {
                if let Some(slot) = self.log_slot.as_mut() {
                    terminal.draw_log_view(&mut slot.view)?;
                }
                Ok(())
            }
            LogViewOutcome::Close => {
                self.return_to_root();
                terminal.render(&self.frame_with_progress(), &mut self.store)?;
                Ok(())
            }
        }
    }

    fn handle_key_picker(&mut self, key: Key, terminal: &mut impl TerminalBackend) -> Result<()> {
        let outcome = {
            let Some(slot) = self.picker_slot.as_mut() else {
                return Ok(());
            };
            let outcome = match slot.picker.handle_event(&key)? {
                ComponentOutcome::Message(m) => m,
                _ => PickerOutcome::Continue,
            };
            let now = slot.picker.inner().selected_view_id();
            if now != slot.last_hover {
                if !self.store.has_rendered(now) {
                    let path = self.store.view(now).svg_path.clone();
                    self.scheduler
                        .request(now, RenderPriority::Hover, path, self.store.budget());
                }
                slot.last_hover = now;
            }
            outcome
        };
        match outcome {
            PickerOutcome::Continue => {
                if let Some(slot) = self.picker_slot.as_mut() {
                    terminal.draw_picker(&mut slot.picker, &self.store)?;
                }
                Ok(())
            }
            PickerOutcome::Select(view_id) => {
                terminal.close_picker(&self.store)?;
                self.picker_slot = None;
                self.focus.pop_scope();
                let canvas = terminal.canvas_metrics();
                self.state
                    .apply(Command::SelectView(view_id), &mut self.store, canvas)?;
                self.request_active_render();
                terminal.render(&self.frame_with_progress(), &mut self.store)?;
                Ok(())
            }
            PickerOutcome::Cancel => {
                terminal.close_picker(&self.store)?;
                self.picker_slot = None;
                self.focus.pop_scope();
                terminal.render(&self.frame_with_progress(), &mut self.store)?;
                Ok(())
            }
        }
    }

    fn redraw_for_mode(&mut self, terminal: &mut impl TerminalBackend) -> Result<()> {
        let frame = self.frame_with_progress();
        match self.active_scope() {
            SCOPE_PICKER => {
                if let Some(slot) = self.picker_slot.as_mut() {
                    terminal.draw_picker(&mut slot.picker, &self.store)?;
                }
                Ok(())
            }
            SCOPE_LOG => {
                if let Some(slot) = self.log_slot.as_mut() {
                    terminal.draw_log_view(&mut slot.view)?;
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
        let pending = self.keymap.resolve(input);
        let command = pending.resolve(canvas);
        let update = self.state.apply(command, &mut self.store, canvas)?;
        self.request_active_render();

        match update.effect {
            Some(Effect::Quit) => {
                self.quit = true;
            }
            Some(Effect::OpenPicker) => {
                let current = self.state.current();
                terminal.teardown_image_viewport(current)?;
                let picker_inner = ViewPicker::new(&self.store.views, &self.store.model, current);
                let last_hover = picker_inner.selected_view_id();
                if !self.store.has_rendered(last_hover) {
                    let path = self.store.view(last_hover).svg_path.clone();
                    self.scheduler.request(
                        last_hover,
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
                self.picker_slot = Some(PickerSlot {
                    picker: Cached::new(picker_inner),
                    last_hover,
                });
                if let Some(slot) = self.picker_slot.as_mut() {
                    terminal.draw_picker(&mut slot.picker, &self.store)?;
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
            Some(Effect::CycleScaleBasis) => {
                self.config.placement.scale_basis = self.config.placement.scale_basis.cycle_next();
                self.apply_placement_change();
                terminal.render(&self.frame_with_progress(), &mut self.store)?;
            }
            Some(Effect::CycleOverflow) => {
                self.config.placement.overflow = self.config.placement.overflow.cycle_next();
                self.apply_placement_change();
                terminal.render(&self.frame_with_progress(), &mut self.store)?;
            }
            Some(Effect::CycleZoomStep) => {
                self.config.zoom = self.config.zoom.cycle_next();
                self.keymap = <KeyMap as KeyMapExt>::from_app_config(&self.config);
                terminal.render(&self.frame_with_progress(), &mut self.store)?;
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
    use crate::ids::ViewId;
    use crate::render::RasterBudget;
    use crate::workspace::{ViewInfo, WorkspaceSource};
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;
    use std::sync::mpsc;
    use tui_kit::input::Key;

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
                child_view_by_element_id: HashMap::new(),
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
                child_view_by_element_id: HashMap::new(),
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

    fn run_with_keys(app: &mut App, terminal: &mut FakeTerminalBackend, keys: &[Key]) {
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
            &[Key::Char('o'), Key::Down, Key::Enter, Key::Char('q')],
        );

        assert_eq!(app.state.current(), ViewId::new(1));
        assert_eq!(terminal.viewport_teardowns, vec![ViewId::first()]);
        assert_eq!(
            terminal.calls,
            vec![
                FakeTerminalCall::Render(ViewId::first()),
                FakeTerminalCall::TeardownImageViewport(ViewId::first()),
                FakeTerminalCall::DrawPicker,
                FakeTerminalCall::DrawPicker,
                FakeTerminalCall::ClosePicker,
                FakeTerminalCall::Render(ViewId::new(1)),
            ]
        );
        assert!(terminal.picker_draws >= 1);
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
            &[Key::Char('o'), Key::Esc, Key::Char('q')],
        );

        assert_eq!(app.state.current(), ViewId::first());
        assert_eq!(
            terminal.calls,
            vec![
                FakeTerminalCall::Render(ViewId::first()),
                FakeTerminalCall::TeardownImageViewport(ViewId::first()),
                FakeTerminalCall::DrawPicker,
                FakeTerminalCall::ClosePicker,
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
            &[Key::Char('?'), Key::Char(' '), Key::Char('q')],
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
                Key::Char('+'),
                Key::Char('+'),
                Key::Char('+'),
                Key::Char('l'),
                Key::Char('q'),
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
        event_tx.send(AppEvent::input_key(Key::CtrlC)).unwrap();
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
        event_tx.send(AppEvent::input_key(Key::CtrlC)).unwrap();
        drop(event_tx);

        app.run(&mut terminal, event_rx).unwrap();

        assert_eq!(terminal.rendered_frames.len(), 2);
        assert!(app.quit);
    }
}

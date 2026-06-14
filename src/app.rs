use crate::backend::TerminalBackend;
use crate::clipboard::Clipboard;
use crate::config::AppConfig;
use crate::keymap::{KeyMap, KeyMapExt};
use crate::logger::SharedLogBuffer;
use crate::modal::ActiveModal;
use crate::render_cache::RenderCache;
use crate::render_pool::{RenderPriority, RenderScheduler};
use crate::runtime::{AppRuntime, DialogSlot};
use crate::state::AppState;
use crate::view_catalog::ViewCatalog;
use crate::viewport_state::ViewportState;
use crate::workspace::WorkspaceSource;
use anyhow::Result;
use std::collections::VecDeque;
use std::sync::mpsc::TryRecvError;
use tui_kit::events::{
    AppEvent, AppEventReceiver, AppEventSender, SchedulerEvent, TerminalEvent, WatcherEvent,
};
use tui_kit::focus::{FocusConfig, FocusManager, FocusNode};
use tui_kit::input::InputEvent;

pub(crate) struct App {
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
    pub(crate) fn new(
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

    pub(crate) fn run(
        &mut self,
        terminal: &mut impl TerminalBackend,
        events: AppEventReceiver,
    ) -> Result<()> {
        self.runtime(terminal).render_startup()?;

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
                self.runtime(terminal).handle_scheduler_complete()
            }
            AppEvent::Watcher(WatcherEvent::WorkspaceChanged) => {
                self.runtime(terminal).handle_watcher_reload()
            }
            AppEvent::Terminal(TerminalEvent::Resize { cols, rows }) => {
                let (cols, rows, skipped) =
                    coalesce_resize_events(cols, rows, events, pending_events);
                if skipped > 0 {
                    log::info!("terminal resized to {cols}×{rows} after coalescing {skipped} resize events");
                } else {
                    log::info!("terminal resized to {cols}×{rows}");
                }
                self.runtime(terminal).redraw_for_mode()
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
        self.runtime(terminal).handle_input_event(input)
    }

    fn runtime<'a, B: TerminalBackend>(&'a mut self, terminal: &'a mut B) -> AppRuntime<'a, B> {
        AppRuntime {
            terminal,
            catalog: &mut self.catalog,
            render_cache: &mut self.render_cache,
            viewport: &mut self.viewport,
            state: &mut self.state,
            config: &mut self.config,
            keymap: &mut self.keymap,
            scheduler: &mut self.scheduler,
            focus: &mut self.focus,
            active_modal: &mut self.active_modal,
            dialog_slot: &mut self.dialog_slot,
            workspace: &self.workspace,
            svg_format: &self.svg_format,
            log_buffer: self.log_buffer.clone(),
            clipboard: self.clipboard.as_ref(),
            quit: &mut self.quit,
        }
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

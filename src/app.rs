use crate::backend::TerminalBackend;
use crate::config::AppConfig;
use crate::event::{Command, InputEvent};
use tui_kit::events::{AppEvent, AppEventReceiver, AppEventSender};
use crate::ids::ViewId;
use tui_kit::input::Key;
use crate::keymap::KeyMap;
use crate::picker::{PickerOutcome, ViewPicker};
use crate::render_pool::{RenderPriority, RenderScheduler};
use crate::state::{AppState, Effect};
use crate::view::ViewStore;
use crate::workspace::{discover_views, export_workspace, load_workspace_model, WorkspaceSource};
use anyhow::Result;
use log::{error, info};
use std::path::PathBuf;

#[derive(Debug)]
enum AppMode {
    Normal,
    Picker {
        picker: ViewPicker,
        last_hover: ViewId,
    },
    Dialog {
        dismissable: bool,
    },
}

pub struct App {
    store: ViewStore,
    state: AppState,
    workspace: WorkspaceSource,
    structurizr_cli: PathBuf,
    svg_format: String,
    config: AppConfig,
    keymap: KeyMap,
    scheduler: RenderScheduler,
    quit: bool,
    mode: AppMode,
}

impl App {
    pub fn new(
        store: ViewStore,
        workspace: WorkspaceSource,
        structurizr_cli: PathBuf,
        svg_format: String,
        config: AppConfig,
        sink: AppEventSender,
    ) -> Self {
        let keymap = KeyMap::defaults(&config.keys);
        let mut scheduler = RenderScheduler::new(1, sink);
        scheduler.request_all(
            store.render_jobs(),
            RenderPriority::Background,
            store.budget(),
        );
        Self {
            store,
            state: AppState::default(),
            workspace,
            structurizr_cli,
            svg_format,
            config,
            keymap,
            scheduler,
            quit: false,
            mode: AppMode::Normal,
        }
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

        while let Ok(event) = events.recv() {
            self.handle_event(event, terminal)?;
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
    ) -> Result<()> {
        match event {
            AppEvent::Key(key) => self.handle_key(key, terminal),
            AppEvent::SchedulerComplete => {
                let updated = self.scheduler.drain_into(&mut self.store);
                if !updated.is_empty() {
                    self.redraw_for_mode(terminal)?;
                }
                Ok(())
            }
            AppEvent::WorkspaceChanged => {
                terminal.show_message(
                    "Workspace changed",
                    "Re-running Structurizr export.",
                )?;
                match self.reload_store() {
                    Ok(()) => {
                        let canvas = terminal.canvas_metrics();
                        let update = self.state.apply(
                            Command::ReloadSucceeded,
                            &mut self.store,
                            canvas,
                        )?;
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
                        self.mode = AppMode::Dialog { dismissable: true };
                        return Ok(());
                    }
                }
                self.mode = AppMode::Normal;
                self.redraw_for_mode(terminal)?;
                Ok(())
            }
            AppEvent::Resize { cols, rows } => {
                log::info!("terminal resized to {cols}×{rows}");
                self.redraw_for_mode(terminal)?;
                Ok(())
            }
            AppEvent::Heartbeat => {
                self.redraw_for_mode(terminal)?;
                Ok(())
            }
        }
    }

    fn handle_key(&mut self, key: Key, terminal: &mut impl TerminalBackend) -> Result<()> {
        match &mut self.mode {
            AppMode::Normal => {
                let input = terminal.translate_key(key);
                self.handle_input(input, terminal)
            }
            AppMode::Dialog { dismissable } => {
                if *dismissable {
                    self.mode = AppMode::Normal;
                    self.redraw_for_mode(terminal)?;
                }
                Ok(())
            }
            AppMode::Picker { .. } => self.handle_key_picker(key, terminal),
        }
    }

    fn handle_key_picker(
        &mut self,
        key: Key,
        terminal: &mut impl TerminalBackend,
    ) -> Result<()> {
        let outcome = {
            let AppMode::Picker { picker, last_hover } = &mut self.mode else {
                return Ok(());
            };
            let outcome = picker.handle_key(key);
            let now = picker.selected_view_id();
            if now != *last_hover {
                if !self.store.has_rendered(now) {
                    let path = self.store.view(now).svg_path.clone();
                    self.scheduler.request(
                        now,
                        RenderPriority::Hover,
                        path,
                        self.store.budget(),
                    );
                }
                *last_hover = now;
            }
            outcome
        };
        match outcome {
            PickerOutcome::Continue => {
                if let AppMode::Picker { picker, .. } = &self.mode {
                    terminal.draw_picker(picker, &self.store)?;
                }
                Ok(())
            }
            PickerOutcome::Select(view_id) => {
                terminal.close_picker(&self.store)?;
                self.mode = AppMode::Normal;
                let canvas = terminal.canvas_metrics();
                self.state
                    .apply(Command::SelectView(view_id), &mut self.store, canvas)?;
                self.request_active_render();
                terminal.render(&self.frame_with_progress(), &mut self.store)?;
                Ok(())
            }
            PickerOutcome::Cancel => {
                terminal.close_picker(&self.store)?;
                self.mode = AppMode::Normal;
                terminal.render(&self.frame_with_progress(), &mut self.store)?;
                Ok(())
            }
        }
    }

    fn redraw_for_mode(&mut self, terminal: &mut impl TerminalBackend) -> Result<()> {
        match &self.mode {
            AppMode::Normal => terminal.render(&self.frame_with_progress(), &mut self.store),
            AppMode::Picker { picker, .. } => terminal.draw_picker(picker, &self.store),
            AppMode::Dialog { .. } => Ok(()),
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
                let picker = ViewPicker::new(
                    &self.store.views,
                    &self.store.model,
                    self.state.current(),
                );
                let last_hover = picker.selected_view_id();
                if !self.store.has_rendered(last_hover) {
                    let path = self.store.view(last_hover).svg_path.clone();
                    self.scheduler.request(
                        last_hover,
                        RenderPriority::Hover,
                        path,
                        self.store.budget(),
                    );
                }
                self.mode = AppMode::Picker { picker, last_hover };
                if let AppMode::Picker { picker, .. } = &self.mode {
                    terminal.draw_picker(picker, &self.store)?;
                }
            }
            Some(Effect::ReloadWorkspace) => {
                terminal.show_message(
                    "Reloading workspace...",
                    "Re-running Structurizr export.",
                )?;
                match self.reload_store() {
                    Ok(()) => {
                        let update = self.state.apply(
                            Command::ReloadSucceeded,
                            &mut self.store,
                            canvas,
                        )?;
                        if update.effect == Some(Effect::ClearImageCache) {
                            terminal.clear_image_cache()?;
                        }
                    }
                    Err(error) => {
                        error!("reload failed: {error:#}");
                        self.state
                            .apply(Command::ReloadFailed, &mut self.store, canvas)?;
                        terminal.show_error("Reload failed", &format!("{error:#}"))?;
                        self.mode = AppMode::Dialog { dismissable: true };
                        return Ok(());
                    }
                }
                terminal.render(&self.frame_with_progress(), &mut self.store)?;
            }
            Some(Effect::ClearImageCache) => {
                terminal.clear_image_cache()?;
                terminal.render(&self.frame_with_progress(), &mut self.store)?;
            }
            Some(Effect::ShowHelp) => {
                terminal.show_help(&self.config.keys)?;
                self.mode = AppMode::Dialog { dismissable: true };
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
            frame.render_progress = Some((
                progress.completed,
                progress.completed + progress.pending,
            ));
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
        let exported = export_workspace(&self.workspace, &self.structurizr_cli, &self.svg_format)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::fake::FakeTerminalBackend;
    use crate::ids::ViewId;
    use tui_kit::input::Key;
    use crate::render::RasterBudget;
    use crate::workspace::{ViewInfo, WorkspaceSource};
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;
    use std::sync::mpsc;

    fn budget() -> RasterBudget {
        RasterBudget {
            quality: 1.0,
            ..RasterBudget::default()
        }
    }

    fn test_app_with_real_svgs(
        dir: &std::path::Path,
        sink: AppEventSender,
    ) -> App {
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

        App::new(
            ViewStore::new(views, budget()).unwrap(),
            WorkspaceSource {
                path: PathBuf::from("workspace.dsl"),
            },
            PathBuf::from("structurizr-cli"),
            "svg".to_owned(),
            AppConfig::default(),
            sink,
        )
    }

    fn run_with_keys(app: &mut App, terminal: &mut FakeTerminalBackend, keys: &[Key]) {
        let (tx, rx) = mpsc::channel();
        for key in keys {
            tx.send(AppEvent::Key(*key)).unwrap();
        }
        drop(tx);
        let _ = app.run(terminal, rx);
    }

    #[test]
    fn picker_navigation_selects_view_by_keystrokes() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, _rx) = mpsc::channel();
        let mut app = test_app_with_real_svgs(dir.path(), tx);
        let mut terminal = FakeTerminalBackend::new([]);

        run_with_keys(
            &mut app,
            &mut terminal,
            &[Key::Char('o'), Key::Down, Key::Enter, Key::Char('q')],
        );

        assert_eq!(app.state.current(), ViewId::new(1));
        assert!(terminal.picker_draws >= 1);
    }

    #[test]
    fn run_uses_backend_for_help_dialog() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, _rx) = mpsc::channel();
        let mut app = test_app_with_real_svgs(dir.path(), tx);
        let mut terminal = FakeTerminalBackend::new([]);

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
        let mut terminal = FakeTerminalBackend::new([]);
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
}

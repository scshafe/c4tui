use crate::event::{Command, InputEvent};
use crate::input::{read_key, Key};
use crate::state::{AppState, Effect};
use crate::terminal::TerminalSession;
use crate::view::ViewStore;
use crate::{
    config::AppConfig,
    workspace::{discover_views, export_workspace, WorkspaceSource},
};
use anyhow::Result;
use log::{error, info};
use std::path::PathBuf;

#[derive(Debug)]
pub struct App {
    store: ViewStore,
    state: AppState,
    workspace: WorkspaceSource,
    structurizr_cli: PathBuf,
    svg_format: String,
    config: AppConfig,
}

impl App {
    pub fn new(
        store: ViewStore,
        workspace: WorkspaceSource,
        structurizr_cli: PathBuf,
        svg_format: String,
        config: AppConfig,
    ) -> Self {
        Self {
            store,
            state: AppState::default(),
            workspace,
            structurizr_cli,
            svg_format,
            config,
        }
    }

    pub fn run(&mut self, terminal: &mut TerminalSession) -> Result<()> {
        terminal.display_frame(&self.state.render_frame(), &mut self.store)?;

        loop {
            let input = self.read_input_event(terminal)?;
            let command = Command::from_input(input, &self.config)
                .with_canvas_size(terminal.canvas_cols(), terminal.canvas_rows());
            let update = self.state.apply(command, &mut self.store)?;

            match update.effect {
                Some(Effect::Quit) => break,
                Some(Effect::OpenPicker) => {
                    if let Some(next) =
                        terminal.open_view_picker(&self.store, self.state.current())?
                    {
                        self.state
                            .apply(Command::SelectView(next), &mut self.store)?;
                    }
                    terminal.display_frame(&self.state.render_frame(), &mut self.store)?;
                }
                Some(Effect::ReloadWorkspace) => {
                    terminal
                        .show_message("Reloading workspace...", "Re-running Structurizr export.")?;
                    match self.reload_store() {
                        Ok(()) => {
                            let update = self
                                .state
                                .apply(Command::ReloadSucceeded, &mut self.store)?;
                            if update.effect == Some(Effect::ClearImageCache) {
                                terminal.clear_image_cache()?;
                            }
                        }
                        Err(error) => {
                            error!("reload failed: {error:#}");
                            self.state.apply(Command::ReloadFailed, &mut self.store)?;
                            terminal.show_error("Reload failed", &format!("{error:#}"))?;
                        }
                    }
                    terminal.display_frame(&self.state.render_frame(), &mut self.store)?;
                }
                Some(Effect::ClearImageCache) => {
                    terminal.clear_image_cache()?;
                    terminal.display_frame(&self.state.render_frame(), &mut self.store)?;
                }
                Some(Effect::ShowHelp) => {
                    terminal.show_help(&self.config.keys)?;
                    terminal.display_frame(&self.state.render_frame(), &mut self.store)?;
                }
                None if update.render => {
                    terminal.display_frame(&self.state.render_frame(), &mut self.store)?;
                }
                None => {}
            }
        }

        Ok(())
    }

    fn read_input_event(&self, terminal: &TerminalSession) -> Result<InputEvent> {
        Ok(match read_key()? {
            Key::MouseClick { x, y } => {
                let (canvas_x, canvas_y) = terminal.mouse_canvas_point(x, y);
                InputEvent::MouseClick { canvas_x, canvas_y }
            }
            Key::MouseWheelUp { x, y } => {
                let (canvas_x, canvas_y) = terminal.mouse_canvas_point(x, y);
                InputEvent::MouseWheelUp { canvas_x, canvas_y }
            }
            Key::MouseWheelDown { x, y } => {
                let (canvas_x, canvas_y) = terminal.mouse_canvas_point(x, y);
                InputEvent::MouseWheelDown { canvas_x, canvas_y }
            }
            key => InputEvent::from(key),
        })
    }

    fn reload_store(&mut self) -> Result<()> {
        info!("reloading workspace {}", self.workspace.path.display());
        let exported = export_workspace(&self.workspace, &self.structurizr_cli, &self.svg_format)?;
        let views = discover_views(&exported)?;
        self.store = ViewStore::new(views, self.config.dpi_scale)?.with_export(exported);
        Ok(())
    }
}

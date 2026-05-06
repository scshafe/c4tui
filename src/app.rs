use crate::backend::TerminalBackend;
use crate::event::Command;
use crate::state::{AppState, Effect};
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

    pub fn run(&mut self, terminal: &mut impl TerminalBackend) -> Result<()> {
        terminal.render(&self.state.render_frame(), &mut self.store)?;

        loop {
            let input = terminal.read_input()?;
            let size = terminal.size();
            let command = Command::from_input(input, &self.config)
                .with_canvas_size(size.canvas_cols(), size.canvas_rows());
            let update = self.state.apply(command, &mut self.store)?;

            match update.effect {
                Some(Effect::Quit) => break,
                Some(Effect::OpenPicker) => {
                    if let Some(next) = terminal.choose_view(&self.store, self.state.current())? {
                        self.state
                            .apply(Command::SelectView(next), &mut self.store)?;
                    }
                    terminal.render(&self.state.render_frame(), &mut self.store)?;
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
                    terminal.render(&self.state.render_frame(), &mut self.store)?;
                }
                Some(Effect::ClearImageCache) => {
                    terminal.clear_image_cache()?;
                    terminal.render(&self.state.render_frame(), &mut self.store)?;
                }
                Some(Effect::ShowHelp) => {
                    terminal.show_help(&self.config.keys)?;
                    terminal.render(&self.state.render_frame(), &mut self.store)?;
                }
                None if update.render => {
                    terminal.render(&self.state.render_frame(), &mut self.store)?;
                }
                None => {}
            }
        }

        Ok(())
    }

    fn reload_store(&mut self) -> Result<()> {
        info!("reloading workspace {}", self.workspace.path.display());
        let exported = export_workspace(&self.workspace, &self.structurizr_cli, &self.svg_format)?;
        let views = discover_views(&exported)?;
        self.store = ViewStore::new(views, self.config.dpi_scale)?.with_export(exported);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::fake::FakeTerminalBackend;
    use crate::event::InputEvent;
    use crate::ids::ViewId;
    use crate::input::Key;
    use crate::workspace::{ViewInfo, WorkspaceSource};
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;

    fn test_app() -> App {
        let views = vec![
            ViewInfo {
                key: "parent".to_owned(),
                name: "Parent".to_owned(),
                view_type: "SystemContext".to_owned(),
                svg_path: PathBuf::from("parent.svg"),
                element_ids: HashSet::new(),
                child_view_by_element_id: HashMap::new(),
            },
            ViewInfo {
                key: "child".to_owned(),
                name: "Child".to_owned(),
                view_type: "Container".to_owned(),
                svg_path: PathBuf::from("child.svg"),
                element_ids: HashSet::new(),
                child_view_by_element_id: HashMap::new(),
            },
        ];

        App::new(
            ViewStore::new(views, 1.0).unwrap(),
            WorkspaceSource {
                path: PathBuf::from("workspace.dsl"),
            },
            PathBuf::from("structurizr-cli"),
            "svg".to_owned(),
            AppConfig::default(),
        )
    }

    #[test]
    fn run_uses_backend_for_picker_and_rendering() {
        let mut app = test_app();
        let mut terminal = FakeTerminalBackend::new([
            InputEvent::Key(Key::Char('o')),
            InputEvent::Key(Key::Char('q')),
        ])
        .with_view_choices([Some(ViewId::new(1))]);

        app.run(&mut terminal).unwrap();

        assert_eq!(terminal.rendered_frames.len(), 2);
        assert_eq!(terminal.rendered_frames[0].current, ViewId::first());
        assert_eq!(terminal.rendered_frames[1].current, ViewId::new(1));
    }

    #[test]
    fn run_uses_backend_for_help_dialog() {
        let mut app = test_app();
        let mut terminal = FakeTerminalBackend::new([
            InputEvent::Key(Key::Char('?')),
            InputEvent::Key(Key::Char('q')),
        ]);

        app.run(&mut terminal).unwrap();

        assert_eq!(terminal.help_count, 1);
        assert_eq!(terminal.rendered_frames.len(), 2);
    }
}

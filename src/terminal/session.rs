use crate::backend::{TerminalBackend, TerminalCommand};
use crate::config::AppConfig;
use crate::statusbar::{default_footer_bar, default_status_bar, StatusBar};
use anyhow::Result;
use tui_kit::layout::{CanvasMetrics, CellSize};
use tui_kit::terminal::TerminalConfig;

use super::{dialogs, images, modal, root};

const STATUS_ROWS: u16 = 1;
const FOOTER_ROWS: u16 = 1;

pub struct TerminalSession {
    pub(super) inner: tui_kit::terminal::Terminal,
    pub(super) status_bar: StatusBar,
    pub(super) footer_bar: StatusBar,
    pub(super) workspace_path: Option<std::path::PathBuf>,
    pub(super) config: AppConfig,
}

impl TerminalSession {
    pub fn enter(config: AppConfig) -> Result<Self> {
        let inner =
            tui_kit::terminal::Terminal::enter_with_config(TerminalConfig::strict_wezterm_kitty())?;
        Ok(Self {
            inner,
            status_bar: default_status_bar(),
            footer_bar: default_footer_bar(),
            workspace_path: None,
            config,
        })
    }

    pub fn set_workspace_path(&mut self, path: std::path::PathBuf) {
        self.workspace_path = Some(path);
    }

    pub fn canvas(&self) -> CanvasMetrics {
        let metrics = self.inner.metrics();
        let reserved = STATUS_ROWS + FOOTER_ROWS;
        let cells = CellSize::new(
            metrics.cells.cols,
            metrics.cells.rows.saturating_sub(reserved).max(1),
        );
        CanvasMetrics::new(cells, metrics.cell_pixel.or_fallback())
    }
}

impl TerminalBackend for TerminalSession {
    fn canvas_metrics(&self) -> CanvasMetrics {
        Self::canvas(self)
    }

    fn execute(&mut self, command: TerminalCommand<'_>) -> Result<()> {
        match command {
            TerminalCommand::RenderRoot(request) => root::render_view(
                self,
                request.frame,
                request.catalog,
                request.cache,
                request.viewport,
            ),
            TerminalCommand::RenderModal(request) => {
                modal::render_modal(self, request.modal, request.catalog, request.cache)
            }
            TerminalCommand::CloseModal(request) => modal::close_modal(self, request.catalog),
            TerminalCommand::TeardownRootViewport { view_id } => {
                images::teardown_root_viewport(self, view_id)
            }
            TerminalCommand::ClearImageCache => images::clear_image_cache(self),
            TerminalCommand::ShowMessage { title, message } => {
                dialogs::show_message(self, title, message)
            }
            TerminalCommand::ShowError { title, message } => {
                dialogs::show_error(self, title, message)
            }
            TerminalCommand::ShowHelp { keys } => dialogs::show_help(self, keys),
        }
    }
}

// Drop handled by tui_kit::terminal::Terminal: leaves alt-screen, disables
// mouse capture, restores cursor, exits raw mode, shuts down image registry.

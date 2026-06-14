use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tempfile::TempDir;
use tui_kit::input::KeyEvent;

use crate::app::App;
use crate::backend::fake::FakeTerminalBackend;
use crate::clipboard::{Clipboard, DefaultClipboard};
use crate::config::AppConfig;
use crate::logger::LogBuffer;
use crate::test_support::events;
use crate::test_support::render;
use crate::view_catalog::ViewCatalog;
use crate::workspace::WorkspaceSource;
use tui_kit::events::AppEventReceiver;

#[derive(Debug)]
pub(crate) struct AppHarness {
    pub(crate) app: App,
    pub(crate) terminal: FakeTerminalBackend,
    pub(crate) _dir: TempDir,
    _scheduler_rx: AppEventReceiver,
}

impl AppHarness {
    pub(crate) fn new(dir: TempDir, catalog: ViewCatalog) -> Self {
        let (scheduler_tx, scheduler_rx) = events::channel();
        let log_buffer = Arc::new(Mutex::new(LogBuffer::with_capacity(64)));
        let clipboard: Box<dyn Clipboard> = Box::new(DefaultClipboard);
        let app = App::new(
            render::render_parts(catalog),
            WorkspaceSource {
                path: PathBuf::from("workspace.dsl"),
            },
            "svg".to_owned(),
            AppConfig::default(),
            scheduler_tx,
            log_buffer,
            clipboard,
        );
        Self {
            app,
            terminal: FakeTerminalBackend::new(),
            _dir: dir,
            _scheduler_rx: scheduler_rx,
        }
    }

    pub(crate) fn run_keys(&mut self, keys: &[KeyEvent]) {
        let _ = self
            .app
            .run(&mut self.terminal, events::receiver_for_keys(keys));
    }
}

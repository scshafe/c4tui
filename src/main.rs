mod app;
mod backend;
mod capabilities;
mod cli;
mod clipboard;
mod config;
mod connection_picker;
mod event;
mod ids;
mod keymap;
mod log_view;
mod logger;
mod picker;
mod render;
mod render_pool;
mod state;
mod statusbar;
mod terminal;
mod view;
mod workspace;

use anyhow::{bail, Context, Result};
use app::App;
use capabilities::{detect_capabilities, Support};
use clap::Parser;
use cli::Cli;
use clipboard::DefaultClipboard;
use config::load_config;
use log::info;
use logger::SharedLogBuffer;
use render::RasterBudget;
use std::time::Duration;
use terminal::TerminalSession;
use view::ViewStore;
use workspace::{
    discover_views, export_workspace, load_workspace_model, resolve_workspace, WorkspaceSource,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    // Always log to a file. If --log-file isn't passed, default to a fixed
    // path so diagnostics are always captured for the most recent launch.
    // File::create truncates on open, so each launch starts fresh.
    let default_log_path = std::path::PathBuf::from("/tmp/c4tui.log");
    let log_path = cli.log_file.as_deref().unwrap_or(&default_log_path);
    let log_buffer = logger::install(Some(log_path))?;
    info!("c4tui session log: {}", log_path.display());
    let config = load_config(cli.config.as_deref())?;
    info!(
        "starting c4tui with raster_quality={}, max_raster_pixels={}",
        config.raster_budget.quality, config.raster_budget.max_pixels
    );
    let timeout = Duration::from_millis(cli.capability_timeout_ms);
    let capabilities = detect_capabilities(timeout, cli.force_probe)
        .context("failed to detect terminal capabilities")?;

    if cli.workspace.is_none() {
        println!("Kitty graphics: {}", capabilities.kitty_graphics);
        println!("Pixel mouse: {}", capabilities.pixel_mouse);
        println!("Truecolor: {}", capabilities.truecolor);

        if capabilities.kitty_graphics != Support::Yes {
            bail!("c4tui requires a terminal with Kitty graphics support (Kitty, WezTerm, or Ghostty)");
        }

        return Ok(());
    }

    if capabilities.kitty_graphics != Support::Yes {
        bail!("c4tui requires a terminal with Kitty graphics support (Kitty, WezTerm, or Ghostty)");
    }

    let workspace = resolve_workspace(cli.workspace.as_deref().expect("checked above"))?;
    let mut terminal = TerminalSession::enter(config.clone())?;
    terminal.set_workspace_path(workspace.path.clone());
    let view_store = match load_view_store(
        &workspace,
        &cli.svg_format,
        config.raster_budget,
        &config.placement,
    ) {
        Ok(store) => store,
        Err(error) => {
            terminal.show_error("Startup failed", &format!("{error:#}"))?;
            return Err(error);
        }
    };
    let (event_tx, event_rx) = std::sync::mpsc::channel();
    tui_kit::input_thread::spawn(event_tx.clone());
    let watcher = if config.watch_workspace {
        let workspace_path = workspace.path.clone();
        match tui_kit::watcher::WorkspaceWatcher::spawn(
            [workspace_path.as_path()],
            Duration::from_millis(config.watch_debounce_ms),
            event_tx.clone(),
        ) {
            Ok(w) => Some(w),
            Err(error) => {
                log::warn!("file watcher disabled: {error:#}");
                None
            }
        }
    } else {
        None
    };
    let _watcher = watcher;
    let clipboard: Box<dyn clipboard::Clipboard> = Box::new(DefaultClipboard);
    let mut app = App::new(
        view_store,
        workspace,
        cli.svg_format,
        config,
        event_tx,
        log_buffer,
        clipboard,
    );
    app.run(&mut terminal, event_rx)?;

    Ok(())
}

#[allow(dead_code)]
fn _expose_log_buffer_type(_: SharedLogBuffer) {}

fn load_view_store(
    workspace: &WorkspaceSource,
    svg_format: &str,
    budget: RasterBudget,
    placement: &config::PlacementChoiceConfig,
) -> Result<ViewStore> {
    let exported = export_workspace(workspace, svg_format)?;
    let views = discover_views(&exported)?;
    let model = load_workspace_model(&exported);
    let policy = view::diagram_placement_policy(placement);
    ViewStore::new(views, budget).map(|store| {
        store
            .with_model(model)
            .with_export(exported)
            .with_placement_policy(policy)
    })
}

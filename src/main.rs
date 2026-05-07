mod app;
mod backend;
mod capabilities;
mod cli;
mod config;
mod event;
mod ids;
mod keymap;
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
use config::load_config;
use env_logger::Env;
use log::info;
use render::RasterBudget;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
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
    init_logging(cli.log_file.as_deref())?;
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
    let structurizr_cli = cli
        .structurizr_cli
        .unwrap_or_else(|| PathBuf::from("structurizr-cli"));
    let mut terminal = TerminalSession::enter(config.clone())?;
    terminal.set_workspace_path(workspace.path.clone());
    let view_store = match load_view_store(
        &workspace,
        &structurizr_cli,
        &cli.svg_format,
        config.raster_budget,
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
            &[workspace_path.as_path()],
            event_tx.clone(),
            Duration::from_millis(config.watch_debounce_ms),
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
    let mut app = App::new(
        view_store,
        workspace,
        structurizr_cli,
        cli.svg_format,
        config,
        event_tx,
    );
    app.run(&mut terminal, event_rx)?;

    Ok(())
}

fn load_view_store(
    workspace: &WorkspaceSource,
    structurizr_cli: &std::path::Path,
    svg_format: &str,
    budget: RasterBudget,
) -> Result<ViewStore> {
    let exported = export_workspace(workspace, structurizr_cli, svg_format)?;
    let views = discover_views(&exported)?;
    let model = load_workspace_model(&exported);
    ViewStore::new(views, budget).map(|store| store.with_model(model).with_export(exported))
}

fn init_logging(log_file: Option<&std::path::Path>) -> Result<()> {
    let env = Env::default().filter_or("RUST_LOG", "warn");
    let mut builder = env_logger::Builder::from_env(env);
    builder.format_timestamp_secs();
    if let Some(path) = log_file {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create log directory {}", parent.display()))?;
        }
        let file = File::create(path)
            .with_context(|| format!("failed to create log file {}", path.display()))?;
        builder.target(env_logger::Target::Pipe(Box::new(file)));
    }
    builder.try_init().ok();
    std::io::stderr().flush().ok();
    Ok(())
}


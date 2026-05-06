mod app;
mod capabilities;
mod cli;
mod input;
mod render;
mod terminal;
mod tty;
mod view;
mod workspace;

use anyhow::{bail, Context, Result};
use app::App;
use capabilities::{detect_capabilities, Support};
use clap::Parser;
use cli::Cli;
use std::path::PathBuf;
use std::time::Duration;
use terminal::TerminalSession;
use view::ViewStore;
use workspace::{discover_views, export_workspace, resolve_workspace};

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
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
    let exported = export_workspace(&workspace, &structurizr_cli, &cli.svg_format)?;
    let views = discover_views(&exported)?;
    let view_store = ViewStore::new(views)?;

    let mut terminal = TerminalSession::enter()?;
    let mut app = App::new(view_store);
    app.run(&mut terminal)?;

    Ok(())
}

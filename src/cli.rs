use clap::Parser;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(author, version, about)]
pub struct Cli {
    /// Path to workspace.dsl, workspace.json, or a directory containing one.
    #[arg(long)]
    pub workspace: Option<PathBuf>,

    /// Path to the structurizr-cli executable.
    #[arg(long)]
    pub structurizr_cli: Option<PathBuf>,

    /// Structurizr CLI export format used to produce SVG files.
    #[arg(long, default_value = "svg")]
    pub svg_format: String,

    /// Path to a c4tui config.toml file. Defaults to ~/.config/c4tui/config.toml when present.
    #[arg(long)]
    pub config: Option<PathBuf>,

    /// Write logs to this file. Logging level is controlled by RUST_LOG.
    #[arg(long)]
    pub log_file: Option<PathBuf>,

    /// Timeout for terminal capability probes, in milliseconds.
    #[arg(long, default_value_t = 200)]
    pub capability_timeout_ms: u64,

    /// Print capability results even when stdout is not attached to a terminal.
    #[arg(long)]
    pub force_probe: bool,
}

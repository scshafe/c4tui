use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use clap::Parser;
use resvg::{tiny_skia, usvg};
use serde::Deserialize;
use std::env;
use std::fmt;
use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use tempfile::TempDir;

#[derive(Debug, Parser)]
#[command(author, version, about)]
struct Cli {
    /// Path to workspace.dsl, workspace.json, or a directory containing one.
    #[arg(long)]
    workspace: Option<PathBuf>,

    /// Path to the structurizr-cli executable.
    #[arg(long)]
    structurizr_cli: Option<PathBuf>,

    /// Structurizr CLI export format used to produce SVG files.
    #[arg(long, default_value = "svg")]
    svg_format: String,

    /// Timeout for terminal capability probes, in milliseconds.
    #[arg(long, default_value_t = 200)]
    capability_timeout_ms: u64,

    /// Print capability results even when stdout is not attached to a terminal.
    #[arg(long)]
    force_probe: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Support {
    Yes,
    No,
    Unknown,
}

impl fmt::Display for Support {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Support::Yes => f.write_str("yes"),
            Support::No => f.write_str("no"),
            Support::Unknown => f.write_str("unknown"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Capabilities {
    kitty_graphics: Support,
    pixel_mouse: Support,
    truecolor: Support,
}

#[derive(Debug)]
struct WorkspaceSource {
    path: PathBuf,
}

#[derive(Debug)]
struct ExportedWorkspace {
    _temp_dir: TempDir,
    output_dir: PathBuf,
    workspace_json: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct ViewInfo {
    key: String,
    name: String,
    view_type: String,
    svg_path: PathBuf,
}

#[derive(Debug)]
struct RenderedView {
    width: u32,
    height: u32,
    png: Vec<u8>,
}

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
    let first_view = views.first().context("no exported SVG views were found")?;
    let rendered = render_svg(&first_view.svg_path)?;

    let mut terminal = TerminalSession::enter()?;
    terminal.display_view(first_view, &rendered)?;
    terminal.wait_for_quit()?;

    Ok(())
}

fn resolve_workspace(input: &Path) -> Result<WorkspaceSource> {
    let path = if input.is_dir() {
        let dsl = input.join("workspace.dsl");
        let json = input.join("workspace.json");
        if dsl.exists() {
            dsl
        } else if json.exists() {
            json
        } else {
            bail!(
                "workspace directory {} contains neither workspace.dsl nor workspace.json",
                input.display()
            );
        }
    } else {
        input.to_path_buf()
    };

    if !path.exists() {
        bail!("workspace file not found: {}", path.display());
    }

    Ok(WorkspaceSource { path })
}

fn export_workspace(
    workspace: &WorkspaceSource,
    structurizr_cli: &Path,
    svg_format: &str,
) -> Result<ExportedWorkspace> {
    let temp_dir = TempDir::new().context("failed to create temporary export directory")?;
    let output_dir = temp_dir.path().to_path_buf();

    let output = Command::new(structurizr_cli)
        .arg("export")
        .arg("-workspace")
        .arg(&workspace.path)
        .arg("-format")
        .arg(svg_format)
        .arg("-output")
        .arg(&output_dir)
        .output()
        .with_context(|| format!("failed to run {}", structurizr_cli.display()))?;

    if !output.status.success() {
        bail!(
            "structurizr-cli export failed with status {}\n{}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let workspace_json = find_workspace_json(&workspace.path, &output_dir);

    Ok(ExportedWorkspace {
        _temp_dir: temp_dir,
        output_dir,
        workspace_json,
    })
}

fn find_workspace_json(workspace_path: &Path, output_dir: &Path) -> Option<PathBuf> {
    let candidates = [
        output_dir.join("workspace.json"),
        workspace_path.with_extension("json"),
        workspace_path.to_path_buf(),
    ];

    candidates.into_iter().find(|path| {
        path.file_name()
            .is_some_and(|name| name == "workspace.json")
            && path.exists()
    })
}

fn discover_views(exported: &ExportedWorkspace) -> Result<Vec<ViewInfo>> {
    let mut svg_files = list_svg_files(&exported.output_dir)?;
    svg_files.sort();

    let metadata = exported
        .workspace_json
        .as_deref()
        .and_then(|path| ViewMetadata::load(path).ok());

    let mut views = Vec::with_capacity(svg_files.len());
    for svg_path in svg_files {
        let stem = svg_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("view")
            .to_owned();
        let meta = metadata.as_ref().and_then(|m| m.find_for_svg_stem(&stem));
        views.push(ViewInfo {
            key: meta.map(|m| m.key.clone()).unwrap_or_else(|| stem.clone()),
            name: meta
                .map(|m| m.name.clone())
                .unwrap_or_else(|| stem.replace('_', " ")),
            view_type: meta
                .map(|m| m.view_type.clone())
                .unwrap_or_else(|| "Unknown".to_owned()),
            svg_path,
        });
    }

    Ok(views)
}

fn list_svg_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut svg_files = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "svg") {
            svg_files.push(path);
        }
    }
    Ok(svg_files)
}

#[derive(Debug)]
struct ViewMetadata {
    views: Vec<ViewMetadataEntry>,
}

#[derive(Debug)]
struct ViewMetadataEntry {
    key: String,
    name: String,
    view_type: String,
}

#[derive(Debug, Deserialize)]
struct StructurizrWorkspaceJson {
    views: Option<StructurizrViewsJson>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StructurizrViewsJson {
    system_landscape_views: Option<Vec<StructurizrViewJson>>,
    system_context_views: Option<Vec<StructurizrViewJson>>,
    container_views: Option<Vec<StructurizrViewJson>>,
    component_views: Option<Vec<StructurizrViewJson>>,
    dynamic_views: Option<Vec<StructurizrViewJson>>,
    deployment_views: Option<Vec<StructurizrViewJson>>,
    filtered_views: Option<Vec<StructurizrViewJson>>,
    custom_views: Option<Vec<StructurizrViewJson>>,
    image_views: Option<Vec<StructurizrViewJson>>,
}

#[derive(Debug, Deserialize)]
struct StructurizrViewJson {
    key: Option<String>,
    name: Option<String>,
    title: Option<String>,
}

impl ViewMetadata {
    fn load(path: &Path) -> Result<Self> {
        let json = fs::read_to_string(path).with_context(|| {
            format!("failed to read workspace metadata from {}", path.display())
        })?;
        let workspace: StructurizrWorkspaceJson =
            serde_json::from_str(&json).with_context(|| {
                format!("failed to parse workspace metadata from {}", path.display())
            })?;
        let Some(views) = workspace.views else {
            return Ok(Self { views: Vec::new() });
        };

        let mut entries = Vec::new();
        push_views(
            &mut entries,
            "SystemLandscape",
            views.system_landscape_views,
        );
        push_views(&mut entries, "SystemContext", views.system_context_views);
        push_views(&mut entries, "Container", views.container_views);
        push_views(&mut entries, "Component", views.component_views);
        push_views(&mut entries, "Dynamic", views.dynamic_views);
        push_views(&mut entries, "Deployment", views.deployment_views);
        push_views(&mut entries, "Filtered", views.filtered_views);
        push_views(&mut entries, "Custom", views.custom_views);
        push_views(&mut entries, "Image", views.image_views);

        Ok(Self { views: entries })
    }

    fn find_for_svg_stem(&self, stem: &str) -> Option<&ViewMetadataEntry> {
        let normalized_stem = normalize_view_key(stem);
        self.views.iter().find(|view| {
            normalize_view_key(&view.key) == normalized_stem
                || normalize_view_key(&view.name) == normalized_stem
        })
    }
}

fn push_views(
    entries: &mut Vec<ViewMetadataEntry>,
    view_type: &str,
    views: Option<Vec<StructurizrViewJson>>,
) {
    let Some(views) = views else {
        return;
    };

    for view in views {
        let key = view
            .key
            .or_else(|| view.name.clone())
            .or_else(|| view.title.clone())
            .unwrap_or_else(|| "view".to_owned());
        let name = view.title.or(view.name).unwrap_or_else(|| key.clone());
        entries.push(ViewMetadataEntry {
            key,
            name,
            view_type: view_type.to_owned(),
        });
    }
}

fn normalize_view_key(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn render_svg(svg_path: &Path) -> Result<RenderedView> {
    let svg =
        fs::read(svg_path).with_context(|| format!("failed to read {}", svg_path.display()))?;
    let options = usvg::Options::default();
    let tree = usvg::Tree::from_data(&svg, &options)
        .with_context(|| format!("failed to parse SVG {}", svg_path.display()))?;
    let size = tree.size().to_int_size();
    let mut pixmap = tiny_skia::Pixmap::new(size.width(), size.height())
        .ok_or_else(|| anyhow!("SVG view has an invalid size"))?;

    resvg::render(
        &tree,
        tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    let png = encode_png(pixmap.width(), pixmap.height(), pixmap.data())?;

    Ok(RenderedView {
        width: pixmap.width(),
        height: pixmap.height(),
        png,
    })
}

fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(rgba)?;
    }
    Ok(png)
}

struct TerminalSession;

impl TerminalSession {
    fn enter() -> Result<Self> {
        write_stdout_all(b"\x1b[?1049h\x1b[?25l")?;
        Ok(Self)
    }

    fn display_view(&mut self, view: &ViewInfo, rendered: &RenderedView) -> Result<()> {
        let title = format!(
            "c4tui | {} ({}) | {} | {}x{} | press q to quit",
            view.name, view.key, view.view_type, rendered.width, rendered.height
        );
        write_stdout_all(b"\x1b[2J\x1b[H")?;
        write_stdout_all(title.as_bytes())?;
        write_stdout_all(b"\r\n")?;
        transmit_kitty_png(1, &rendered.png)?;
        write_stdout_all(b"\x1b_Ga=p,i=1,p=1,q=2;\x1b\\")?;
        Ok(())
    }

    fn wait_for_quit(&mut self) -> Result<()> {
        let mut stdin = io::stdin();
        let mut byte = [0_u8; 1];
        loop {
            stdin.read_exact(&mut byte)?;
            if matches!(byte[0], b'q' | b'Q' | 0x03 | 0x04 | 0x1b) {
                break;
            }
        }
        Ok(())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = write_stdout_all(b"\x1b_Ga=d,i=1;\x1b\\");
        let _ = write_stdout_all(b"\x1b[?25h\x1b[?1049l");
    }
}

fn transmit_kitty_png(image_id: u32, png: &[u8]) -> Result<()> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(png);
    let mut chunks = encoded.as_bytes().chunks(4096).peekable();

    while let Some(chunk) = chunks.next() {
        let more = if chunks.peek().is_some() { 1 } else { 0 };
        write!(
            io::stdout().lock(),
            "\x1b_Ga=t,f=100,t=f,i={image_id},m={more};{}\x1b\\",
            std::str::from_utf8(chunk)?
        )?;
        io::stdout().flush()?;
    }

    Ok(())
}

fn detect_capabilities(timeout: Duration, force_probe: bool) -> io::Result<Capabilities> {
    let truecolor = detect_truecolor();

    if !force_probe && (!io::stdout().is_terminal() || !io::stdin().is_terminal()) {
        return Ok(Capabilities {
            kitty_graphics: Support::Unknown,
            pixel_mouse: Support::Unknown,
            truecolor,
        });
    }

    let mut session = TerminalProbeSession::new()?;
    let kitty_graphics = session.query_kitty_graphics(timeout)?;
    let pixel_mouse = session.query_sgr_pixel_mouse(timeout)?;

    Ok(Capabilities {
        kitty_graphics,
        pixel_mouse,
        truecolor,
    })
}

fn detect_truecolor() -> Support {
    let colorterm = env::var("COLORTERM")
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(colorterm.as_str(), "truecolor" | "24bit") {
        return Support::Yes;
    }

    let term = env::var("TERM").unwrap_or_default().to_ascii_lowercase();
    if term.contains("truecolor") || term.contains("24bit") {
        return Support::Yes;
    }

    match env::var("TERM_PROGRAM").unwrap_or_default().as_str() {
        "WezTerm" | "ghostty" | "Ghostty" | "iTerm.app" | "kitty" => Support::Yes,
        _ => Support::Unknown,
    }
}

struct TerminalProbeSession {
    stdin_fd: libc::c_int,
    original_termios: libc::termios,
    original_flags: libc::c_int,
}

impl TerminalProbeSession {
    fn new() -> io::Result<Self> {
        let stdin_fd = libc::STDIN_FILENO;
        let original_termios = get_termios(stdin_fd)?;
        let original_flags = get_fd_flags(stdin_fd)?;

        let mut raw = original_termios;
        make_raw(&mut raw);
        set_termios(stdin_fd, &raw)?;
        set_fd_flags(stdin_fd, original_flags | libc::O_NONBLOCK)?;

        Ok(Self {
            stdin_fd,
            original_termios,
            original_flags,
        })
    }

    fn query_kitty_graphics(&mut self, timeout: Duration) -> io::Result<Support> {
        self.drain_input();

        write_stdout_all(b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\")?;
        let response = self.read_until(timeout, |bytes| bytes.windows(4).any(|w| w == b"\x1b_G"));

        Ok(parse_kitty_graphics_response(&response))
    }

    fn query_sgr_pixel_mouse(&mut self, timeout: Duration) -> io::Result<Support> {
        self.drain_input();

        write_stdout_all(b"\x1b[?1016h\x1b[?1016$p")?;
        let response = self.read_until(timeout, |bytes| {
            let text = String::from_utf8_lossy(bytes);
            text.contains("\x1b[?1016;") && text.contains("$y")
        });
        write_stdout_all(b"\x1b[?1016l")?;

        Ok(parse_decrpm_mode_response(&response, 1016))
    }

    fn drain_input(&mut self) {
        let mut buf = [0_u8; 1024];
        loop {
            match unsafe { libc::read(self.stdin_fd, buf.as_mut_ptr().cast(), buf.len()) } {
                n if n > 0 => continue,
                _ => break,
            }
        }
    }

    fn read_until(&mut self, timeout: Duration, done: impl Fn(&[u8]) -> bool) -> Vec<u8> {
        let deadline = Instant::now() + timeout;
        let mut out = Vec::new();
        let mut buf = [0_u8; 1024];

        while Instant::now() < deadline {
            match unsafe { libc::read(self.stdin_fd, buf.as_mut_ptr().cast(), buf.len()) } {
                n if n > 0 => {
                    out.extend_from_slice(&buf[..n as usize]);
                    if done(&out) {
                        break;
                    }
                }
                -1 => {
                    let err = io::Error::last_os_error();
                    if err.kind() != io::ErrorKind::WouldBlock {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                _ => break,
            }
        }

        out
    }
}

impl Drop for TerminalProbeSession {
    fn drop(&mut self) {
        let _ = write_stdout_all(b"\x1b[?1016l");
        let _ = set_fd_flags(self.stdin_fd, self.original_flags);
        let _ = set_termios(self.stdin_fd, &self.original_termios);
    }
}

fn parse_kitty_graphics_response(response: &[u8]) -> Support {
    let text = String::from_utf8_lossy(response);
    if !text.contains("\x1b_G") || !text.contains("i=31") {
        return Support::No;
    }

    if text.contains(";OK") || text.contains(",OK") || text.contains(";ok") || text.contains(",ok")
    {
        Support::Yes
    } else if text.contains("EINVAL") || text.contains("error") || text.contains("ERROR") {
        Support::No
    } else {
        Support::Yes
    }
}

fn parse_decrpm_mode_response(response: &[u8], mode: u16) -> Support {
    let text = String::from_utf8_lossy(response);
    let prefix = format!("\x1b[?{mode};");
    let Some(start) = text.find(&prefix) else {
        return Support::No;
    };
    let rest = &text[start + prefix.len()..];
    let Some(end) = rest.find("$y") else {
        return Support::Unknown;
    };
    let value = &rest[..end];

    match value.chars().next() {
        Some('1') | Some('3') => Support::Yes,
        Some('2') | Some('4') => Support::No,
        _ => Support::Unknown,
    }
}

fn write_stdout_all(bytes: &[u8]) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    stdout.write_all(bytes)?;
    stdout.flush()
}

fn get_termios(fd: libc::c_int) -> io::Result<libc::termios> {
    let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
    if unsafe { libc::tcgetattr(fd, termios.as_mut_ptr()) } == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { termios.assume_init() })
    }
}

fn set_termios(fd: libc::c_int, termios: &libc::termios) -> io::Result<()> {
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, termios) } == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn get_fd_flags(fd: libc::c_int) -> io::Result<libc::c_int> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(flags)
    }
}

fn set_fd_flags(fd: libc::c_int, flags: libc::c_int) -> io::Result<()> {
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags) } == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn make_raw(termios: &mut libc::termios) {
    termios.c_iflag &=
        !(libc::BRKINT | libc::ICRNL | libc::INPCK | libc::ISTRIP | libc::IXON | libc::PARMRK);
    termios.c_oflag &= !libc::OPOST;
    termios.c_cflag |= libc::CS8;
    termios.c_lflag &= !(libc::ECHO | libc::ICANON | libc::IEXTEN | libc::ISIG);
    termios.c_cc[libc::VMIN] = 0;
    termios.c_cc[libc::VTIME] = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_workspace_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspace.dsl");
        fs::write(&path, "workspace {}\n").unwrap();
        assert_eq!(resolve_workspace(&path).unwrap().path, path);
    }

    #[test]
    fn prefers_dsl_over_json_in_workspace_directory() {
        let dir = tempfile::tempdir().unwrap();
        let dsl = dir.path().join("workspace.dsl");
        let json = dir.path().join("workspace.json");
        fs::write(&dsl, "workspace {}\n").unwrap();
        fs::write(json, "{}\n").unwrap();
        assert_eq!(resolve_workspace(dir.path()).unwrap().path, dsl);
    }

    #[test]
    fn parses_workspace_view_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let json = dir.path().join("workspace.json");
        fs::write(
            &json,
            r#"{
              "views": {
                "systemLandscapeViews": [{"key":"landscape", "title":"Landscape"}],
                "containerViews": [{"key":"containers", "name":"Containers"}]
              }
            }"#,
        )
        .unwrap();

        let metadata = ViewMetadata::load(&json).unwrap();
        assert_eq!(metadata.views.len(), 2);
        assert_eq!(
            metadata.find_for_svg_stem("landscape").unwrap().name,
            "Landscape"
        );
        assert_eq!(
            metadata.find_for_svg_stem("containers").unwrap().view_type,
            "Container"
        );
    }

    #[test]
    fn parses_positive_kitty_response() {
        assert_eq!(
            parse_kitty_graphics_response(b"\x1b_Gi=31;OK\x1b\\"),
            Support::Yes
        );
    }

    #[test]
    fn parses_missing_kitty_response_as_no() {
        assert_eq!(parse_kitty_graphics_response(b""), Support::No);
    }

    #[test]
    fn parses_decrpm_set_as_yes() {
        assert_eq!(
            parse_decrpm_mode_response(b"\x1b[?1016;1$y", 1016),
            Support::Yes
        );
    }

    #[test]
    fn parses_decrpm_reset_as_no() {
        assert_eq!(
            parse_decrpm_mode_response(b"\x1b[?1016;2$y", 1016),
            Support::No
        );
    }

    #[test]
    fn parses_absent_decrpm_as_no() {
        assert_eq!(parse_decrpm_mode_response(b"", 1016), Support::No);
    }
}

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use clap::Parser;
use resvg::{tiny_skia, usvg};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fmt;
use std::fs;
use std::io::{self, IsTerminal, Write};
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
    let view_store = ViewStore::new(views)?;

    let mut terminal = TerminalSession::enter()?;
    let mut app = App::new(view_store);
    app.run(&mut terminal)?;

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

#[derive(Debug)]
struct ViewStore {
    views: Vec<ViewInfo>,
    rendered: HashMap<usize, RenderedView>,
    transforms: HashMap<usize, ViewTransform>,
}

impl ViewStore {
    fn new(views: Vec<ViewInfo>) -> Result<Self> {
        if views.is_empty() {
            bail!("no exported SVG views were found");
        }

        Ok(Self {
            views,
            rendered: HashMap::new(),
            transforms: HashMap::new(),
        })
    }

    fn len(&self) -> usize {
        self.views.len()
    }

    fn view(&self, index: usize) -> &ViewInfo {
        &self.views[index]
    }

    fn rendered_view(&mut self, index: usize) -> Result<&RenderedView> {
        if !self.rendered.contains_key(&index) {
            let rendered = render_svg(&self.views[index].svg_path)?;
            self.rendered.insert(index, rendered);
        }

        Ok(self.rendered.get(&index).expect("rendered view inserted"))
    }

    fn transform(&self, index: usize) -> ViewTransform {
        self.transforms.get(&index).copied().unwrap_or_default()
    }

    fn set_transform(&mut self, index: usize, transform: ViewTransform) {
        self.transforms.insert(index, transform);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ViewTransform {
    scale: f32,
    offset_x: f32,
    offset_y: f32,
}

impl Default for ViewTransform {
    fn default() -> Self {
        Self {
            scale: 1.0,
            offset_x: 0.0,
            offset_y: 0.0,
        }
    }
}

impl ViewTransform {
    fn zoomed(
        self,
        factor: f32,
        center_x: f32,
        center_y: f32,
        image_width: u32,
        image_height: u32,
    ) -> Self {
        let old_scale = self.scale;
        let scale = (self.scale * factor).clamp(1.0, 4.0);
        if (scale - old_scale).abs() < f32::EPSILON {
            return self;
        }

        let image_width = image_width as f32;
        let image_height = image_height as f32;
        let old_width = image_width / old_scale;
        let old_height = image_height / old_scale;
        let new_width = image_width / scale;
        let new_height = image_height / scale;
        let center_image_x = self.offset_x + old_width * center_x.clamp(0.0, 1.0);
        let center_image_y = self.offset_y + old_height * center_y.clamp(0.0, 1.0);

        Self {
            scale,
            offset_x: center_image_x - new_width * center_x.clamp(0.0, 1.0),
            offset_y: center_image_y - new_height * center_y.clamp(0.0, 1.0),
        }
        .clamped(image_width as u32, image_height as u32)
    }

    fn panned(self, dx: f32, dy: f32, image_width: u32, image_height: u32) -> Self {
        Self {
            offset_x: self.offset_x + dx,
            offset_y: self.offset_y + dy,
            ..self
        }
        .clamped(image_width, image_height)
    }

    fn reset() -> Self {
        Self::default()
    }

    fn source_rect(self, image_width: u32, image_height: u32) -> SourceRect {
        let clamped = self.clamped(image_width, image_height);
        let width = ((image_width as f32 / clamped.scale).round() as u32).clamp(1, image_width);
        let height = ((image_height as f32 / clamped.scale).round() as u32).clamp(1, image_height);
        let max_x = image_width.saturating_sub(width);
        let max_y = image_height.saturating_sub(height);
        SourceRect {
            x: (clamped.offset_x.round() as u32).min(max_x),
            y: (clamped.offset_y.round() as u32).min(max_y),
            width,
            height,
        }
    }

    fn clamped(self, image_width: u32, image_height: u32) -> Self {
        let scale = self.scale.clamp(1.0, 4.0);
        let source_width = image_width as f32 / scale;
        let source_height = image_height as f32 / scale;
        let max_x = (image_width as f32 - source_width).max(0.0);
        let max_y = (image_height as f32 - source_height).max(0.0);
        Self {
            scale,
            offset_x: self.offset_x.clamp(0.0, max_x),
            offset_y: self.offset_y.clamp(0.0, max_y),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SourceRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[derive(Debug)]
struct App {
    store: ViewStore,
    current: usize,
    last_drag: Option<(u16, u16)>,
}

impl App {
    fn new(store: ViewStore) -> Self {
        Self {
            store,
            current: 0,
            last_drag: None,
        }
    }

    fn run(&mut self, terminal: &mut TerminalSession) -> Result<()> {
        terminal.display_view(self.current, &mut self.store)?;

        loop {
            match read_key()? {
                Key::Char('q') | Key::Char('Q') | Key::CtrlC | Key::Esc => break,
                Key::Char('o') | Key::Char('O') => {
                    if let Some(next) = terminal.open_view_picker(&self.store, self.current)? {
                        self.current = next;
                    }
                    terminal.display_view(self.current, &mut self.store)?;
                }
                Key::Char('+') | Key::Char('=') => {
                    self.zoom_current(1.25, (0.5, 0.5))?;
                    terminal.display_view(self.current, &mut self.store)?;
                }
                Key::Char('-') | Key::Char('_') => {
                    self.zoom_current(0.8, (0.5, 0.5))?;
                    terminal.display_view(self.current, &mut self.store)?;
                }
                Key::MouseWheelUp { x, y } => {
                    self.zoom_current(1.25, terminal.mouse_canvas_point(x, y))?;
                    terminal.display_view(self.current, &mut self.store)?;
                }
                Key::MouseWheelDown { x, y } => {
                    self.zoom_current(0.8, terminal.mouse_canvas_point(x, y))?;
                    terminal.display_view(self.current, &mut self.store)?;
                }
                Key::Char('0') => {
                    self.store
                        .set_transform(self.current, ViewTransform::reset());
                    terminal.display_view(self.current, &mut self.store)?;
                }
                Key::Char('f') | Key::Char('F') => {
                    self.store
                        .set_transform(self.current, ViewTransform::reset());
                    terminal.display_view(self.current, &mut self.store)?;
                }
                Key::Left => {
                    self.pan_current(-0.10, 0.0)?;
                    terminal.display_view(self.current, &mut self.store)?;
                }
                Key::Right => {
                    self.pan_current(0.10, 0.0)?;
                    terminal.display_view(self.current, &mut self.store)?;
                }
                Key::Up => {
                    self.pan_current(0.0, -0.10)?;
                    terminal.display_view(self.current, &mut self.store)?;
                }
                Key::Down => {
                    self.pan_current(0.0, 0.10)?;
                    terminal.display_view(self.current, &mut self.store)?;
                }
                Key::MouseDrag { x, y } => {
                    if let Some((last_x, last_y)) = self.last_drag {
                        let dx = (last_x as f32 - x as f32) / terminal.canvas_cols() as f32;
                        let dy = (last_y as f32 - y as f32) / terminal.canvas_rows() as f32;
                        self.pan_current(dx, dy)?;
                        terminal.display_view(self.current, &mut self.store)?;
                    }
                    self.last_drag = Some((x, y));
                }
                Key::MouseRelease => {
                    self.last_drag = None;
                }
                _ => {}
            }
        }

        Ok(())
    }

    fn zoom_current(&mut self, factor: f32, center: (f32, f32)) -> Result<()> {
        let (width, height) = {
            let rendered = self.store.rendered_view(self.current)?;
            (rendered.width, rendered.height)
        };
        let transform = self
            .store
            .transform(self.current)
            .zoomed(factor, center.0, center.1, width, height);
        self.store.set_transform(self.current, transform);
        Ok(())
    }

    fn pan_current(&mut self, dx_fraction: f32, dy_fraction: f32) -> Result<()> {
        let (width, height) = {
            let rendered = self.store.rendered_view(self.current)?;
            (rendered.width, rendered.height)
        };
        let transform = self.store.transform(self.current);
        let rect = transform.source_rect(width, height);
        let dx = rect.width as f32 * dx_fraction;
        let dy = rect.height as f32 * dy_fraction;
        self.store
            .set_transform(self.current, transform.panned(dx, dy, width, height));
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Key {
    Char(char),
    Up,
    Down,
    Left,
    Right,
    Enter,
    Esc,
    CtrlC,
    MouseWheelUp { x: u16, y: u16 },
    MouseWheelDown { x: u16, y: u16 },
    MouseDrag { x: u16, y: u16 },
    MouseRelease,
    Unknown,
}

fn read_key() -> Result<Key> {
    let byte = read_stdin_byte_blocking()?;

    Ok(match byte {
        b'\r' | b'\n' => Key::Enter,
        0x03 => Key::CtrlC,
        0x1b => parse_escape_sequence()?,
        byte if byte.is_ascii() && !byte.is_ascii_control() => Key::Char(byte as char),
        _ => Key::Unknown,
    })
}

fn read_stdin_byte_blocking() -> io::Result<u8> {
    loop {
        let mut byte = [0_u8; 1];
        match unsafe { libc::read(libc::STDIN_FILENO, byte.as_mut_ptr().cast(), 1) } {
            1 => return Ok(byte[0]),
            -1 => {
                let err = io::Error::last_os_error();
                if err.kind() != io::ErrorKind::WouldBlock {
                    return Err(err);
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            _ => std::thread::sleep(Duration::from_millis(2)),
        }
    }
}

fn parse_escape_sequence() -> io::Result<Key> {
    let mut bytes = Vec::new();
    let deadline = Instant::now() + Duration::from_millis(25);
    while Instant::now() < deadline {
        let mut byte = [0_u8; 1];
        match unsafe { libc::read(libc::STDIN_FILENO, byte.as_mut_ptr().cast(), 1) } {
            1 => {
                bytes.push(byte[0]);
                if matches!(byte[0], b'A' | b'B' | b'C' | b'D' | b'M' | b'm') {
                    break;
                }
            }
            -1 => {
                let err = io::Error::last_os_error();
                if err.kind() != io::ErrorKind::WouldBlock {
                    return Err(err);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            _ => std::thread::sleep(Duration::from_millis(1)),
        }
    }

    Ok(parse_escape_bytes(&bytes))
}

fn parse_escape_bytes(bytes: &[u8]) -> Key {
    match bytes {
        [b'[', b'A'] => Key::Up,
        [b'[', b'B'] => Key::Down,
        [b'[', b'C'] => Key::Right,
        [b'[', b'D'] => Key::Left,
        _ => parse_sgr_mouse(bytes).unwrap_or(Key::Esc),
    }
}

fn parse_sgr_mouse(bytes: &[u8]) -> Option<Key> {
    let text = std::str::from_utf8(bytes).ok()?;
    if !text.starts_with("[<") || !(text.ends_with('M') || text.ends_with('m')) {
        return None;
    }
    let released = text.ends_with('m');
    let body = &text[2..text.len() - 1];
    let mut parts = body.split(';');
    let code = parts.next()?.parse::<u16>().ok()?;
    let x = parts.next()?.parse::<u16>().ok()?;
    let y = parts.next()?.parse::<u16>().ok()?;

    if released {
        return Some(Key::MouseRelease);
    }

    match code {
        64 => Some(Key::MouseWheelUp { x, y }),
        65 => Some(Key::MouseWheelDown { x, y }),
        32..=63 => Some(Key::MouseDrag { x, y }),
        _ => None,
    }
}

struct TerminalSession {
    original_termios: libc::termios,
    original_flags: libc::c_int,
    transmitted_images: HashSet<u32>,
    cols: u16,
    rows: u16,
}

impl TerminalSession {
    fn enter() -> Result<Self> {
        let original_termios = get_termios(libc::STDIN_FILENO)?;
        let original_flags = get_fd_flags(libc::STDIN_FILENO)?;
        let mut raw = original_termios;
        make_raw(&mut raw);
        raw.c_cc[libc::VMIN] = 0;
        raw.c_cc[libc::VTIME] = 0;
        set_termios(libc::STDIN_FILENO, &raw)?;
        set_fd_flags(libc::STDIN_FILENO, original_flags | libc::O_NONBLOCK)?;

        let (cols, rows) = terminal_size();
        write_stdout_all(b"\x1b[?1049h\x1b[?25l\x1b[?1000h\x1b[?1002h\x1b[?1006h\x1b[?1016h")?;
        Ok(Self {
            original_termios,
            original_flags,
            transmitted_images: HashSet::new(),
            cols,
            rows,
        })
    }

    fn display_view(&mut self, index: usize, store: &mut ViewStore) -> Result<()> {
        let image_id = image_id_for_view(index);
        let view = store.view(index).clone();
        let (width, height) = {
            let rendered = store.rendered_view(index)?;
            if !self.transmitted_images.contains(&image_id) {
                transmit_kitty_png(image_id, &rendered.png)?;
                self.transmitted_images.insert(image_id);
            }
            (rendered.width, rendered.height)
        };

        let transform = store.transform(index);
        let rect = transform.source_rect(width, height);
        let title = format!(
            "c4tui | {} ({}) | {} | zoom {:.0}% | arrows pan | +/- zoom | 0/f fit | o views | q quit",
            view.name, view.key, view.view_type, transform.scale * 100.0
        );
        write_stdout_all(b"\x1b[2J\x1b[H")?;
        write_stdout_all(title.as_bytes())?;
        write_stdout_all(b"\r\n")?;
        write!(
            io::stdout().lock(),
            "\x1b_Ga=p,i={image_id},p=1,q=2,X={},Y={},W={},H={},c={},r={};\x1b\\",
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            self.cols,
            self.canvas_rows()
        )?;
        io::stdout().flush()?;
        Ok(())
    }

    fn canvas_cols(&self) -> u16 {
        self.cols.max(1)
    }

    fn canvas_rows(&self) -> u16 {
        self.rows.saturating_sub(1).max(1)
    }

    fn mouse_canvas_point(&self, x: u16, y: u16) -> (f32, f32) {
        (
            (x.saturating_sub(1) as f32 / self.canvas_cols() as f32).clamp(0.0, 1.0),
            (y.saturating_sub(2) as f32 / self.canvas_rows() as f32).clamp(0.0, 1.0),
        )
    }

    fn open_view_picker(&mut self, store: &ViewStore, current: usize) -> Result<Option<usize>> {
        let mut selected = current;
        loop {
            self.draw_view_picker(store, selected)?;
            match read_key()? {
                Key::Up => {
                    selected = selected.saturating_sub(1);
                }
                Key::Down => {
                    selected = (selected + 1).min(store.len() - 1);
                }
                Key::Enter => return Ok(Some(selected)),
                Key::Esc | Key::Char('q') | Key::Char('Q') => return Ok(None),
                _ => {}
            }
        }
    }

    fn draw_view_picker(&mut self, store: &ViewStore, selected: usize) -> Result<()> {
        write_stdout_all(b"\x1b[2J\x1b[H")?;
        write_stdout_all(b"Select a view (Up/Down, Enter, Esc)\r\n\r\n")?;

        for (index, view) in store.views.iter().enumerate() {
            let marker = if index == selected { ">" } else { " " };
            writeln!(
                io::stdout().lock(),
                "{marker} {} ({}) [{}]\r",
                view.name,
                view.key,
                view.view_type
            )?;
        }

        io::stdout().flush()?;
        Ok(())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        for image_id in &self.transmitted_images {
            let _ = write!(io::stdout().lock(), "\x1b_Ga=d,i={image_id};\x1b\\");
        }
        let _ = io::stdout().flush();
        let _ =
            write_stdout_all(b"\x1b[?1016l\x1b[?1006l\x1b[?1002l\x1b[?1000l\x1b[?25h\x1b[?1049l");
        let _ = set_fd_flags(libc::STDIN_FILENO, self.original_flags);
        let _ = set_termios(libc::STDIN_FILENO, &self.original_termios);
    }
}

fn terminal_size() -> (u16, u16) {
    let mut size = std::mem::MaybeUninit::<libc::winsize>::zeroed();
    if unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, size.as_mut_ptr()) } == -1 {
        return (80, 24);
    }
    let size = unsafe { size.assume_init() };
    (size.ws_col.max(1), size.ws_row.max(2))
}

fn image_id_for_view(index: usize) -> u32 {
    (index as u32) + 1
}

fn transmit_kitty_png(image_id: u32, png: &[u8]) -> Result<()> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(png);
    let mut chunks = encoded.as_bytes().chunks(4096).peekable();

    while let Some(chunk) = chunks.next() {
        let more = if chunks.peek().is_some() { 1 } else { 0 };
        write!(
            io::stdout().lock(),
            "\x1b_Ga=t,f=100,i={image_id},m={more};{}\x1b\\",
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
    fn creates_view_store_for_non_empty_views() {
        let dir = tempfile::tempdir().unwrap();
        let svg = dir.path().join("landscape.svg");
        fs::write(&svg, "<svg />").unwrap();
        let store = ViewStore::new(vec![ViewInfo {
            key: "landscape".to_owned(),
            name: "Landscape".to_owned(),
            view_type: "SystemLandscape".to_owned(),
            svg_path: svg,
        }])
        .unwrap();

        assert_eq!(store.len(), 1);
        assert_eq!(image_id_for_view(0), 1);
    }

    #[test]
    fn rejects_empty_view_store() {
        assert!(ViewStore::new(Vec::new()).is_err());
    }

    #[test]
    fn view_transform_zoom_pan_and_reset_source_rect() {
        let transform = ViewTransform::default()
            .zoomed(2.0, 0.5, 0.5, 1000, 800)
            .panned(100.0, 50.0, 1000, 800);
        let rect = transform.source_rect(1000, 800);

        assert_eq!(rect.width, 500);
        assert_eq!(rect.height, 400);
        assert_eq!(rect.x, 350);
        assert_eq!(rect.y, 250);
        assert_eq!(ViewTransform::reset(), ViewTransform::default());
    }

    #[test]
    fn view_transform_clamps_to_image_bounds() {
        let transform = ViewTransform {
            scale: 10.0,
            offset_x: 10_000.0,
            offset_y: 10_000.0,
        }
        .clamped(1000, 800);
        let rect = transform.source_rect(1000, 800);

        assert_eq!(transform.scale, 4.0);
        assert_eq!(rect.width, 250);
        assert_eq!(rect.height, 200);
        assert_eq!(rect.x, 750);
        assert_eq!(rect.y, 600);
    }

    #[test]
    fn parses_sgr_mouse_wheel_and_drag() {
        assert_eq!(
            parse_sgr_mouse(b"[<64;10;20M"),
            Some(Key::MouseWheelUp { x: 10, y: 20 })
        );
        assert_eq!(
            parse_sgr_mouse(b"[<65;10;20M"),
            Some(Key::MouseWheelDown { x: 10, y: 20 })
        );
        assert_eq!(
            parse_sgr_mouse(b"[<32;12;24M"),
            Some(Key::MouseDrag { x: 12, y: 24 })
        );
        assert_eq!(parse_sgr_mouse(b"[<0;12;24m"), Some(Key::MouseRelease));
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

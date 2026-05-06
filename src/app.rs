use crate::input::{read_key, Key};
use crate::terminal::TerminalSession;
use crate::view::{ViewStore, ViewTransform};
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
    current: usize,
    breadcrumbs: Vec<usize>,
    last_drag: Option<(u16, u16)>,
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
            current: 0,
            breadcrumbs: Vec::new(),
            last_drag: None,
            workspace,
            structurizr_cli,
            svg_format,
            config,
        }
    }

    pub fn run(&mut self, terminal: &mut TerminalSession) -> Result<()> {
        terminal.display_view(self.current, &self.breadcrumbs, &mut self.store)?;

        loop {
            match read_key()? {
                Key::Char(ch) if self.is_key(ch, self.config.keys.quit) => break,
                Key::CtrlC | Key::Esc => break,
                Key::Char(ch) if self.is_key(ch, self.config.keys.open_picker) => {
                    if let Some(next) = terminal.open_view_picker(&self.store, self.current)? {
                        self.current = next;
                        self.breadcrumbs.clear();
                    }
                    terminal.display_view(self.current, &self.breadcrumbs, &mut self.store)?;
                }
                Key::Char(ch) if self.is_key(ch, self.config.keys.reload) => {
                    terminal
                        .show_message("Reloading workspace...", "Re-running Structurizr export.")?;
                    match self.reload() {
                        Ok(()) => {
                            terminal.clear_image_cache()?;
                            terminal.display_view(
                                self.current,
                                &self.breadcrumbs,
                                &mut self.store,
                            )?;
                        }
                        Err(error) => {
                            error!("reload failed: {error:#}");
                            terminal.show_error("Reload failed", &format!("{error:#}"))?;
                            terminal.display_view(
                                self.current,
                                &self.breadcrumbs,
                                &mut self.store,
                            )?;
                        }
                    }
                }
                Key::Char(ch) if self.is_key(ch, self.config.keys.help) => {
                    terminal.show_help(&self.config.keys)?;
                    terminal.display_view(self.current, &self.breadcrumbs, &mut self.store)?;
                }
                Key::Back => {
                    if let Some(previous) = self.breadcrumbs.pop() {
                        self.current = previous;
                    }
                    terminal.display_view(self.current, &self.breadcrumbs, &mut self.store)?;
                }
                Key::MouseClick { x, y } => {
                    let point = terminal.mouse_canvas_point(x, y);
                    if let Some(child) =
                        self.store
                            .child_view_at_canvas_point(self.current, point.0, point.1)?
                    {
                        self.breadcrumbs.push(self.current);
                        self.current = child;
                        self.last_drag = None;
                        terminal.display_view(self.current, &self.breadcrumbs, &mut self.store)?;
                    }
                }
                Key::Char(ch) if ch == self.config.keys.zoom_in || ch == '=' => {
                    self.zoom_current(1.25, (0.5, 0.5))?;
                    terminal.display_view(self.current, &self.breadcrumbs, &mut self.store)?;
                }
                Key::Char(ch) if ch == self.config.keys.zoom_out || ch == '_' => {
                    self.zoom_current(0.8, (0.5, 0.5))?;
                    terminal.display_view(self.current, &self.breadcrumbs, &mut self.store)?;
                }
                Key::MouseWheelUp { x, y } => {
                    self.zoom_current(1.25, terminal.mouse_canvas_point(x, y))?;
                    terminal.display_view(self.current, &self.breadcrumbs, &mut self.store)?;
                }
                Key::MouseWheelDown { x, y } => {
                    self.zoom_current(0.8, terminal.mouse_canvas_point(x, y))?;
                    terminal.display_view(self.current, &self.breadcrumbs, &mut self.store)?;
                }
                Key::Char(ch)
                    if self.is_key(ch, self.config.keys.reset)
                        || self.is_key(ch, self.config.keys.fit) =>
                {
                    self.store
                        .set_transform(self.current, ViewTransform::reset());
                    terminal.display_view(self.current, &self.breadcrumbs, &mut self.store)?;
                }
                Key::Left => {
                    self.pan_current(-0.10, 0.0)?;
                    terminal.display_view(self.current, &self.breadcrumbs, &mut self.store)?;
                }
                Key::Right => {
                    self.pan_current(0.10, 0.0)?;
                    terminal.display_view(self.current, &self.breadcrumbs, &mut self.store)?;
                }
                Key::Up => {
                    self.pan_current(0.0, -0.10)?;
                    terminal.display_view(self.current, &self.breadcrumbs, &mut self.store)?;
                }
                Key::Down => {
                    self.pan_current(0.0, 0.10)?;
                    terminal.display_view(self.current, &self.breadcrumbs, &mut self.store)?;
                }
                Key::MouseDrag { x, y } => {
                    if let Some((last_x, last_y)) = self.last_drag {
                        let dx = (last_x as f32 - x as f32) / terminal.canvas_cols() as f32;
                        let dy = (last_y as f32 - y as f32) / terminal.canvas_rows() as f32;
                        self.pan_current(dx, dy)?;
                        terminal.display_view(self.current, &self.breadcrumbs, &mut self.store)?;
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

    fn reload(&mut self) -> Result<()> {
        info!("reloading workspace {}", self.workspace.path.display());
        let exported = export_workspace(&self.workspace, &self.structurizr_cli, &self.svg_format)?;
        let views = discover_views(&exported)?;
        self.store = ViewStore::new(views, self.config.dpi_scale)?.with_export(exported);
        self.current = 0;
        self.breadcrumbs.clear();
        self.last_drag = None;
        Ok(())
    }

    fn is_key(&self, actual: char, configured: char) -> bool {
        actual == configured || actual.eq_ignore_ascii_case(&configured)
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

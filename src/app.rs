use crate::input::{read_key, Key};
use crate::terminal::TerminalSession;
use crate::view::{ViewStore, ViewTransform};
use anyhow::Result;

#[derive(Debug)]
pub struct App {
    store: ViewStore,
    current: usize,
    last_drag: Option<(u16, u16)>,
}

impl App {
    pub fn new(store: ViewStore) -> Self {
        Self {
            store,
            current: 0,
            last_drag: None,
        }
    }

    pub fn run(&mut self, terminal: &mut TerminalSession) -> Result<()> {
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
                Key::Char('0') | Key::Char('f') | Key::Char('F') => {
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

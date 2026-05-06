use crate::backend::{TerminalBackend, TerminalSize};
use crate::config::KeyBindings;
use crate::event::InputEvent;
use crate::ids::ViewId;
use crate::input::{read_key, Key};
use crate::state::RenderFrame;
use crate::tty::{
    get_fd_flags, get_termios, make_raw, set_fd_flags, set_termios, terminal_size, write_stdout_all,
};
use crate::view::{image_id_for_view, ViewStore};
use anyhow::Result;
use base64::Engine;
use std::collections::HashSet;
use std::io::{self, Write};

pub struct TerminalSession {
    original_termios: libc::termios,
    original_flags: libc::c_int,
    transmitted_images: HashSet<u32>,
    cols: u16,
    rows: u16,
}

impl TerminalSession {
    pub fn enter() -> Result<Self> {
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

    pub fn display_view(
        &mut self,
        index: ViewId,
        breadcrumbs: &[ViewId],
        store: &mut ViewStore,
    ) -> Result<()> {
        let image_id = image_id_for_view(index);
        let view = store.view(index).clone();
        let (width, height) = {
            let rendered = store.rendered_view(index)?;
            if self.transmitted_images.insert(image_id) {
                transmit_kitty_png(image_id, &rendered.png)?;
            }
            (rendered.width, rendered.height)
        };

        let transform = store.transform(index);
        let rect = transform.source_rect(width, height);
        let breadcrumb = format_breadcrumb(store, breadcrumbs, index);
        let title = format!(
            "c4tui | {} | {} | zoom {:.0}% | click drill | Backspace back | ? help | q quit",
            breadcrumb,
            view.view_type,
            transform.scale * 100.0
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

    pub fn display_frame(&mut self, frame: &RenderFrame, store: &mut ViewStore) -> Result<()> {
        self.display_view(frame.current, &frame.breadcrumbs, store)
    }

    pub fn canvas_cols(&self) -> u16 {
        self.cols.max(1)
    }

    pub fn canvas_rows(&self) -> u16 {
        self.rows.saturating_sub(1).max(1)
    }

    pub const fn size(&self) -> TerminalSize {
        TerminalSize {
            cols: self.cols,
            rows: self.rows,
        }
    }

    pub fn mouse_canvas_point(&self, x: u16, y: u16) -> (f32, f32) {
        (
            (f32::from(x.saturating_sub(1)) / f32::from(self.canvas_cols())).clamp(0.0, 1.0),
            (f32::from(y.saturating_sub(2)) / f32::from(self.canvas_rows())).clamp(0.0, 1.0),
        )
    }

    pub fn read_input_event(&self) -> Result<InputEvent> {
        Ok(match read_key()? {
            Key::MouseClick { x, y } => {
                let (canvas_x, canvas_y) = self.mouse_canvas_point(x, y);
                InputEvent::MouseClick { canvas_x, canvas_y }
            }
            Key::MouseWheelUp { x, y } => {
                let (canvas_x, canvas_y) = self.mouse_canvas_point(x, y);
                InputEvent::MouseWheelUp { canvas_x, canvas_y }
            }
            Key::MouseWheelDown { x, y } => {
                let (canvas_x, canvas_y) = self.mouse_canvas_point(x, y);
                InputEvent::MouseWheelDown { canvas_x, canvas_y }
            }
            key => InputEvent::from(key),
        })
    }

    pub fn open_view_picker(
        &mut self,
        store: &ViewStore,
        current: ViewId,
    ) -> Result<Option<ViewId>> {
        let mut selected = current;
        loop {
            Self::draw_view_picker(store, selected)?;
            match read_key()? {
                Key::Up => {
                    selected = ViewId::new(selected.index().saturating_sub(1));
                }
                Key::Down => {
                    selected = ViewId::new((selected.index() + 1).min(store.len() - 1));
                }
                Key::Enter => return Ok(Some(selected)),
                Key::Esc | Key::Char('q' | 'Q') => return Ok(None),
                _ => {}
            }
        }
    }

    pub fn clear_image_cache(&mut self) -> Result<()> {
        for image_id in &self.transmitted_images {
            write!(io::stdout().lock(), "\x1b_Ga=d,i={image_id};\x1b\\")?;
        }
        io::stdout().flush()?;
        self.transmitted_images.clear();
        Ok(())
    }

    pub fn show_error(&mut self, title: &str, message: &str) -> Result<()> {
        Self::show_dialog(title, message, "Press any key to continue")
    }

    pub fn show_message(&mut self, title: &str, message: &str) -> Result<()> {
        Self::show_dialog(title, message, "")
    }

    pub fn show_help(&mut self, keys: &KeyBindings) -> Result<()> {
        let help = format!(
            "Keys\n\n  {quit}  Quit\n  {open}  Open view picker\n  {reload}  Reload workspace/export\n  Backspace  Go back through breadcrumbs\n  {zoom_in}/=  Zoom in\n  {zoom_out}/_  Zoom out\n  {reset} or {fit}  Reset/fit view\n  Arrow keys  Pan\n  Mouse wheel  Zoom around cursor\n  Mouse drag  Pan\n  Click element  Drill into child view\n\nConfig: ~/.config/c4tui/config.toml\nLogging: --log-file <path>, level via RUST_LOG",
            quit = keys.quit,
            open = keys.open_picker,
            reload = keys.reload,
            zoom_in = keys.zoom_in,
            zoom_out = keys.zoom_out,
            reset = keys.reset,
            fit = keys.fit,
        );
        Self::show_dialog("c4tui help", &help, "Press any key to continue")
    }

    fn show_dialog(title: &str, message: &str, footer: &str) -> Result<()> {
        write_stdout_all(b"\x1b[2J\x1b[H")?;
        writeln!(io::stdout().lock(), "{title}\r")?;
        writeln!(io::stdout().lock(), "{}\r", "=".repeat(title.len().max(1)))?;
        for line in message.lines() {
            writeln!(io::stdout().lock(), "{line}\r")?;
        }
        if footer.is_empty() {
            io::stdout().flush()?;
        } else {
            writeln!(io::stdout().lock(), "\r\n{footer}\r")?;
            io::stdout().flush()?;
            let _ = read_key()?;
        }
        Ok(())
    }

    fn draw_view_picker(store: &ViewStore, selected: ViewId) -> Result<()> {
        write_stdout_all(b"\x1b[2J\x1b[H")?;
        write_stdout_all(b"Select a view (Up/Down, Enter, Esc)\r\n\r\n")?;

        for (index, view) in store.views.iter().enumerate() {
            let marker = if ViewId::new(index) == selected {
                ">"
            } else {
                " "
            };
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

impl TerminalBackend for TerminalSession {
    fn size(&self) -> TerminalSize {
        Self::size(self)
    }

    fn read_input(&mut self) -> Result<InputEvent> {
        Self::read_input_event(self)
    }

    fn render(&mut self, frame: &RenderFrame, store: &mut ViewStore) -> Result<()> {
        Self::display_frame(self, frame, store)
    }

    fn choose_view(&mut self, store: &ViewStore, current: ViewId) -> Result<Option<ViewId>> {
        Self::open_view_picker(self, store, current)
    }

    fn clear_image_cache(&mut self) -> Result<()> {
        Self::clear_image_cache(self)
    }

    fn show_message(&mut self, title: &str, message: &str) -> Result<()> {
        Self::show_message(self, title, message)
    }

    fn show_error(&mut self, title: &str, message: &str) -> Result<()> {
        Self::show_error(self, title, message)
    }

    fn show_help(&mut self, keys: &KeyBindings) -> Result<()> {
        Self::show_help(self, keys)
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

fn format_breadcrumb(store: &ViewStore, breadcrumbs: &[ViewId], current: ViewId) -> String {
    breadcrumbs
        .iter()
        .copied()
        .chain(std::iter::once(current))
        .map(|index| store.view(index).name.as_str())
        .collect::<Vec<_>>()
        .join(" > ")
}

fn transmit_kitty_png(image_id: u32, png: &[u8]) -> Result<()> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(png);
    let mut chunks = encoded.as_bytes().chunks(4096).peekable();

    while let Some(chunk) = chunks.next() {
        let more = u8::from(chunks.peek().is_some());
        write!(
            io::stdout().lock(),
            "\x1b_Ga=t,f=100,i={image_id},m={more};{}\x1b\\",
            std::str::from_utf8(chunk)?
        )?;
        io::stdout().flush()?;
    }

    Ok(())
}

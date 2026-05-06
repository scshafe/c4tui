use crate::input::{read_key, Key};
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

    pub fn display_view(&mut self, index: usize, store: &mut ViewStore) -> Result<()> {
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

    pub fn canvas_cols(&self) -> u16 {
        self.cols.max(1)
    }

    pub fn canvas_rows(&self) -> u16 {
        self.rows.saturating_sub(1).max(1)
    }

    pub fn mouse_canvas_point(&self, x: u16, y: u16) -> (f32, f32) {
        (
            (x.saturating_sub(1) as f32 / self.canvas_cols() as f32).clamp(0.0, 1.0),
            (y.saturating_sub(2) as f32 / self.canvas_rows() as f32).clamp(0.0, 1.0),
        )
    }

    pub fn open_view_picker(&mut self, store: &ViewStore, current: usize) -> Result<Option<usize>> {
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

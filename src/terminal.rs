use crate::backend::TerminalBackend;
use crate::config::{AppConfig, KeyBindings};
use crate::event::InputEvent;
use crate::ids::ViewId;
use tui_kit::input::Key;
use tui_kit::image::{
    picker_placement_id, ImageSurface, KittyImageRegistry, PlaceOptions, MAIN_PLACEMENT_ID,
};
use tui_kit::layout::{CanvasMetrics, CellSize};
use crate::picker::{PickerLine, ViewPicker};
use crate::state::RenderFrame;
use crate::statusbar::{default_footer_bar, default_status_bar, StatusBar, StatusContext};
use tui_kit::tty::terminal_metrics;
use crate::view::{image_id_for_view, ViewStore};
use anyhow::Result;
use ratatui::backend::CrosstermBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap};
use std::io::{self, Stdout, Write};

const STATUS_ROWS: u16 = 1;
const FOOTER_ROWS: u16 = 1;

type Term = ratatui::Terminal<CrosstermBackend<Stdout>>;

pub struct TerminalSession {
    terminal: Option<Term>,
    images: KittyImageRegistry,
    status_bar: StatusBar,
    footer_bar: StatusBar,
    workspace_path: Option<std::path::PathBuf>,
    config: AppConfig,
}

impl TerminalSession {
    pub fn enter(config: AppConfig) -> Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        let mut stdout = io::stdout();
        crossterm::execute!(
            stdout,
            crossterm::terminal::EnterAlternateScreen,
            crossterm::cursor::Hide,
            crossterm::event::EnableMouseCapture,
        )?;
        let backend = CrosstermBackend::new(io::stdout());
        let terminal = ratatui::Terminal::new(backend)?;
        Ok(Self {
            terminal: Some(terminal),
            images: KittyImageRegistry::default(),
            status_bar: default_status_bar(),
            footer_bar: default_footer_bar(),
            workspace_path: None,
            config,
        })
    }

    pub fn set_workspace_path(&mut self, path: std::path::PathBuf) {
        self.workspace_path = Some(path);
    }

    #[allow(dead_code)]
    pub fn set_footer_bar(&mut self, bar: StatusBar) {
        self.footer_bar = bar;
    }

    #[allow(dead_code)]
    pub fn with_status_bar(mut self, status_bar: StatusBar) -> Self {
        self.status_bar = status_bar;
        self
    }

    pub fn canvas(&self) -> CanvasMetrics {
        let metrics = terminal_metrics();
        let reserved = STATUS_ROWS + FOOTER_ROWS;
        let cells = CellSize::new(
            metrics.cells.cols,
            metrics.cells.rows.saturating_sub(reserved).max(1),
        );
        CanvasMetrics::new(cells, metrics.cell_pixel.or_fallback())
    }

    fn full_terminal_metrics(&self) -> CanvasMetrics {
        terminal_metrics()
    }

    fn render_view(
        &mut self,
        view_id: ViewId,
        breadcrumbs: &[ViewId],
        pinned_element: Option<&crate::ids::ElementId>,
        render_progress: Option<(usize, usize)>,
        store: &mut ViewStore,
    ) -> Result<()> {
        let canvas = self.canvas();
        let raster = store.rendered_view(view_id)?.raster_size;
        let transform = store.transform(view_id);
        let placement = transform.place(raster, canvas);
        let image_id = image_id_for_view(view_id);

        {
            let png = &store.rendered_view(view_id)?.png;
            self.images.ensure_loaded(image_id, png)?;
        }

        let view = store.view(view_id).clone();
        let breadcrumb_names: Vec<String> = breadcrumbs
            .iter()
            .map(|id| store.view(*id).name.clone())
            .collect();
        let pinned_meta = pinned_element.and_then(|id| store.element_metadata(id)).cloned();
        let rendered = store.rendered_view(view_id)?;
        let breadcrumb_refs: Vec<&str> = breadcrumb_names.iter().map(String::as_str).collect();
        let workspace_path = self.workspace_path.as_deref();
        let context = StatusContext {
            view: &view,
            view_id,
            breadcrumb_names: breadcrumb_refs,
            transform,
            placement,
            canvas,
            rendered,
            config: &self.config,
            pinned_element: pinned_meta.as_ref(),
            render_progress,
            workspace_path,
        };
        let status_text = self.status_bar.render(&context, canvas.cells.cols);
        let footer_text = self.footer_bar.render(&context, canvas.cells.cols);

        let mut canvas_rect = Rect::default();
        let terminal = self
            .terminal
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("terminal session not initialised"))?;
        terminal.draw(|frame| {
            let chunks = Layout::vertical([
                Constraint::Length(1),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(frame.area());
            Paragraph::new(status_text).render(chunks[0], frame.buffer_mut());
            Clear.render(chunks[1], frame.buffer_mut());
            Paragraph::new(footer_text).render(chunks[2], frame.buffer_mut());
            canvas_rect = chunks[1];
        })?;

        let cursor_row = canvas_rect.y + placement.origin.row + 1;
        let cursor_col = canvas_rect.x + placement.origin.col + 1;
        position_cursor(cursor_row, cursor_col)?;
        self.images.place(PlaceOptions {
            image_id,
            placement_id: MAIN_PLACEMENT_ID,
            source: placement.source,
            cell_cols: placement.size.cols,
            cell_rows: placement.size.rows,
        })?;
        self.images.flush()?;
        Ok(())
    }

    pub fn close_picker(&mut self, store: &ViewStore) -> Result<()> {
        self.images
            .delete_placements_in((0..store.views.len()).map(picker_placement_id))?;
        self.images.flush()?;
        Ok(())
    }

    pub fn draw_picker(&mut self, picker: &ViewPicker, store: &ViewStore) -> Result<()> {
        const ITEM_ROW_SPAN: u16 = 3;
        const THUMB_COLS: u16 = 12;
        const THUMB_ROWS: u16 = 3;
        let rendered = picker.render();

        self.images.delete_placement(MAIN_PLACEMENT_ID)?;
        let mut thumbs: Vec<(ViewId, u16, u16)> = Vec::new();
        let terminal = self
            .terminal
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("terminal session not initialised"))?;
        terminal.draw(|frame| {
            let widget = PickerWidget {
                rendered: &rendered,
                item_row_span: ITEM_ROW_SPAN,
                thumb_cols: THUMB_COLS,
                thumb_rows: THUMB_ROWS,
                thumbnails: &mut thumbs,
            };
            widget.render(frame.area(), frame.buffer_mut());
        })?;

        let placements_to_clear: Vec<u32> = (0..store.views.len())
            .map(picker_placement_id)
            .filter(|id| !thumbs.iter().any(|(vid, _, _)| picker_placement_id(vid.index()) == *id))
            .collect();
        self.images.delete_placements_in(placements_to_clear)?;

        for (view_id, row, col) in &thumbs {
            self.draw_thumbnail(*view_id, *row, *col, THUMB_COLS, THUMB_ROWS, store)?;
        }
        self.images.flush()?;
        Ok(())
    }

    fn draw_thumbnail(
        &mut self,
        view_id: ViewId,
        row: u16,
        col: u16,
        cols: u16,
        rows: u16,
        store: &ViewStore,
    ) -> Result<bool> {
        let image_id = image_id_for_view(view_id);
        let Some(rendered) = store.cached_rendered_view(view_id) else {
            return Ok(false);
        };
        self.images.ensure_loaded(image_id, &rendered.png)?;
        position_cursor(row, col)?;
        self.images.place(PlaceOptions {
            image_id,
            placement_id: picker_placement_id(view_id.index()),
            source: tui_kit::layout::PixelRect {
                x: 0,
                y: 0,
                width: rendered.raster_size.width,
                height: rendered.raster_size.height,
            },
            cell_cols: cols,
            cell_rows: rows,
        })?;
        Ok(true)
    }

    fn clear_image_cache_inner(&mut self) -> Result<()> {
        self.images.forget_all()?;
        self.images.flush()?;
        Ok(())
    }

    pub fn show_error(&mut self, title: &str, message: &str) -> Result<()> {
        self.show_dialog(title, message, " press any key to continue ")
    }

    pub fn show_message(&mut self, title: &str, message: &str) -> Result<()> {
        self.show_dialog(title, message, "")
    }

    pub fn help_text(keys: &KeyBindings) -> String {
        format!(
            "Keys\n\n  {quit}  Quit\n  Esc  Clear pinned element / quit if none\n  {open}  Open view picker (type to filter, Tab toggles legends)\n  {reload}  Reload workspace/export\n  K  Jump to legend for current view\n  i  Inspect element at viewport center\n  Backspace  Go back through breadcrumbs\n  Arrows or hjkl  Pan\n  {zoom_in}/=  Zoom in\n  {zoom_out}/_  Zoom out\n  {reset} or {fit}  Reset/fit view\n  Mouse wheel  Zoom around cursor\n  Mouse drag  Pan\n  Click element  Drill into child view, else pin\n\nConfig: ~/.config/c4tui/config.toml\nLogging: --log-file <path>, level via RUST_LOG",
            quit = keys.quit,
            open = keys.open_picker,
            reload = keys.reload,
            zoom_in = keys.zoom_in,
            zoom_out = keys.zoom_out,
            reset = keys.reset,
            fit = keys.fit,
        )
    }

    fn show_help_inner(&mut self, keys: &KeyBindings) -> Result<()> {
        let text = Self::help_text(keys);
        self.show_dialog("c4tui help", &text, " press any key to continue ")
    }

    pub fn show_dialog(&mut self, title: &str, message: &str, footer: &str) -> Result<()> {
        let title_owned = title.to_owned();
        let message_owned = message.to_owned();
        let footer_owned = footer.to_owned();
        self.images.delete_placement(MAIN_PLACEMENT_ID)?;
        let terminal = self
            .terminal
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("terminal session not initialised"))?;
        terminal.draw(|frame| {
            let area = frame.area();
            let block = Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} ", title_owned))
                .title_bottom(footer_owned.clone());
            let inner = block.inner(area);
            block.render(area, frame.buffer_mut());
            Paragraph::new(message_owned.clone())
                .wrap(Wrap { trim: false })
                .render(inner, frame.buffer_mut());
        })?;
        self.images.flush()?;
        Ok(())
    }

    fn mouse_canvas_point(&self, x: u16, y: u16) -> (f32, f32) {
        let canvas = self.canvas();
        let cols = f32::from(canvas.cells.cols.max(1));
        let rows = f32::from(canvas.cells.rows.max(1));
        let canvas_x = f32::from(x.saturating_sub(1)) / cols;
        let canvas_y = f32::from(y.saturating_sub(1 + STATUS_ROWS)) / rows;
        (canvas_x.clamp(0.0, 1.0), canvas_y.clamp(0.0, 1.0))
    }

    pub fn translate_key(&self, key: Key) -> InputEvent {
        match key {
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
            other => InputEvent::from(other),
        }
    }
}

impl TerminalBackend for TerminalSession {
    fn canvas_metrics(&self) -> CanvasMetrics {
        Self::canvas(self)
    }

    fn translate_key(&self, key: Key) -> InputEvent {
        Self::translate_key(self, key)
    }

    fn render(&mut self, frame: &RenderFrame, store: &mut ViewStore) -> Result<()> {
        Self::render_view(
            self,
            frame.current,
            &frame.breadcrumbs,
            frame.pinned_element.as_ref(),
            frame.render_progress,
            store,
        )
    }

    fn draw_picker(&mut self, picker: &ViewPicker, store: &ViewStore) -> Result<()> {
        Self::draw_picker(self, picker, store)
    }

    fn close_picker(&mut self, store: &ViewStore) -> Result<()> {
        Self::close_picker(self, store)
    }

    fn clear_image_cache(&mut self) -> Result<()> {
        Self::clear_image_cache_inner(self)
    }

    fn show_message(&mut self, title: &str, message: &str) -> Result<()> {
        Self::show_message(self, title, message)
    }

    fn show_error(&mut self, title: &str, message: &str) -> Result<()> {
        Self::show_error(self, title, message)
    }

    fn show_help(&mut self, keys: &KeyBindings) -> Result<()> {
        Self::show_help_inner(self, keys)
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        self.terminal.take();
        self.images.shutdown();
        let _ = io::stdout().flush();
        let _ = crossterm::execute!(
            io::stdout(),
            crossterm::event::DisableMouseCapture,
            crossterm::cursor::Show,
            crossterm::terminal::LeaveAlternateScreen,
        );
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

fn position_cursor(row: u16, col: u16) -> Result<()> {
    write!(io::stdout().lock(), "\x1b[{};{}H", row.max(1), col.max(1))?;
    Ok(())
}

struct PickerWidget<'a> {
    rendered: &'a crate::picker::RenderedPicker,
    item_row_span: u16,
    thumb_cols: u16,
    thumb_rows: u16,
    thumbnails: &'a mut Vec<(ViewId, u16, u16)>,
}

#[derive(Debug)]
struct LineSpan {
    index: usize,
    virtual_row: u16,
    span: u16,
    selected_item: bool,
}

impl<'a> Widget for PickerWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" View Picker ")
            .title_bottom(" type → filter | Tab → legends | Enter → select | Esc → cancel ");
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.height < 3 || inner.width < 8 {
            return;
        }

        let header_avail = inner.width.saturating_sub(1) as usize;
        Paragraph::new(truncate(&self.rendered.header, header_avail))
            .render(Rect { x: inner.x, y: inner.y, width: inner.width, height: 1 }, buf);

        let body = Rect {
            x: inner.x,
            y: inner.y + 2,
            width: inner.width,
            height: inner.height.saturating_sub(2),
        };

        let layouts = compute_line_spans(&self.rendered.lines, self.item_row_span);
        let total_virtual = layouts.last().map(|l| l.virtual_row + l.span).unwrap_or(0);
        let mut scroll: u16 = 0;
        if let Some(sel) = layouts.iter().find(|l| l.selected_item) {
            let sel_end = sel.virtual_row + sel.span;
            if total_virtual > body.height {
                if sel_end > scroll + body.height {
                    scroll = sel_end - body.height;
                }
                if sel.virtual_row < scroll {
                    scroll = sel.virtual_row;
                }
            }
        }

        for layout in &layouts {
            if layout.virtual_row + layout.span <= scroll {
                continue;
            }
            if layout.virtual_row >= scroll + body.height {
                break;
            }
            let screen_row = body.y + layout.virtual_row.saturating_sub(scroll);
            if screen_row >= body.y + body.height {
                break;
            }
            match &self.rendered.lines[layout.index] {
                PickerLine::Header(text) => {
                    let avail = body.width.saturating_sub(1) as usize;
                    buf.set_string(
                        body.x,
                        screen_row,
                        truncate(text, avail),
                        Style::default().add_modifier(Modifier::BOLD),
                    );
                }
                PickerLine::Item {
                    view_id,
                    marker,
                    primary,
                    detail,
                    selected,
                } => {
                    self.thumbnails
                        .push((*view_id, screen_row + 1, body.x + 1 + 1));
                    let text_col = body.x + self.thumb_cols + 2;
                    let text_avail = body.width.saturating_sub(self.thumb_cols + 2 + 1) as usize;
                    let text = format!("{} {}", marker, truncate(primary, text_avail.saturating_sub(2)));
                    let style = if *selected {
                        Style::default().add_modifier(Modifier::REVERSED)
                    } else {
                        Style::default()
                    };
                    let visible_len = text.chars().count();
                    buf.set_string(text_col, screen_row, &text, style);
                    if *selected {
                        let pad = text_avail.saturating_sub(visible_len);
                        if pad > 0 {
                            buf.set_string(
                                text_col + visible_len as u16,
                                screen_row,
                                " ".repeat(pad),
                                style,
                            );
                        }
                    }
                    if let Some(d) = detail {
                        let detail_row = screen_row + 1;
                        if detail_row < body.y + body.height {
                            let avail = body.width.saturating_sub(self.thumb_cols + 4 + 1) as usize;
                            buf.set_string(
                                text_col + 2,
                                detail_row,
                                truncate(d, avail),
                                Style::default().add_modifier(Modifier::DIM),
                            );
                        }
                    }
                }
                PickerLine::Empty(text) => {
                    buf.set_string(body.x, screen_row, text, Style::default());
                }
            }
        }

        if scroll > 0 {
            buf.set_string(
                inner.x + inner.width.saturating_sub(1),
                body.y,
                "▲",
                Style::default(),
            );
        }
        if scroll + body.height < total_virtual {
            buf.set_string(
                inner.x + inner.width.saturating_sub(1),
                body.y + body.height.saturating_sub(1),
                "▼",
                Style::default(),
            );
        }
    }
}

fn compute_line_spans(lines: &[PickerLine], item_span: u16) -> Vec<LineSpan> {
    let mut layouts = Vec::with_capacity(lines.len());
    let mut row = 0u16;
    for (idx, line) in lines.iter().enumerate() {
        let (span, selected_item) = match line {
            PickerLine::Header(_) => (1, false),
            PickerLine::Item { selected, .. } => (item_span, *selected),
            PickerLine::Empty(_) => (1, false),
        };
        layouts.push(LineSpan {
            index: idx,
            virtual_row: row,
            span,
            selected_item,
        });
        row = row.saturating_add(span);
    }
    layouts
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        text.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
    }
}

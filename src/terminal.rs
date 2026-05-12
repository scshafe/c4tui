use crate::backend::TerminalBackend;
use crate::config::{AppConfig, KeyBindings};
use crate::event::InputEvent;
use crate::ids::ViewId;
use crate::log_view::LogView;
use crate::picker::ViewPicker;
use crate::state::RenderFrame;
use crate::statusbar::{default_footer_bar, default_status_bar, StatusBar, StatusContext};
use crate::view::{image_id_for_view, ViewStore};
use anyhow::Result;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};
use tui_kit::component::Cached;
use tui_kit::image::{picker_placement_id, ImageSurface, MAIN_PLACEMENT_ID};
use tui_kit::input::Key;
use tui_kit::layout::{CanvasMetrics, CellArea, CellSize};
use tui_kit::terminal::TerminalConfig;
use tui_kit::widgets::dialog::Dialog;
use tui_kit::widgets::image_viewport::{
    ImageViewportInitialScale, ImageViewportOptions, ResizePolicy, ViewportImage,
};

const STATUS_ROWS: u16 = 1;
const FOOTER_ROWS: u16 = 1;

pub struct TerminalSession {
    inner: tui_kit::terminal::Terminal,
    status_bar: StatusBar,
    footer_bar: StatusBar,
    workspace_path: Option<std::path::PathBuf>,
    config: AppConfig,
}

impl TerminalSession {
    pub fn enter(config: AppConfig) -> Result<Self> {
        let inner =
            tui_kit::terminal::Terminal::enter_with_config(TerminalConfig::strict_wezterm_kitty())?;
        Ok(Self {
            inner,
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
        let metrics = self.inner.metrics();
        let reserved = STATUS_ROWS + FOOTER_ROWS;
        let cells = CellSize::new(
            metrics.cells.cols,
            metrics.cells.rows.saturating_sub(reserved).max(1),
        );
        CanvasMetrics::new(cells, metrics.cell_pixel.or_fallback())
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
        let image_id = image_id_for_view(view_id);
        let placement = store.placement(view_id, canvas)?;
        let transform = store.transform(view_id);

        let view = store.view(view_id).clone();
        let breadcrumb_names: Vec<String> = breadcrumbs
            .iter()
            .map(|id| store.view(*id).name.clone())
            .collect();
        let pinned_meta = pinned_element
            .and_then(|id| store.element_metadata(id))
            .cloned();
        let pinned_connection_counts = pinned_element
            .map(|id| store.model.connection_counts(id))
            .filter(|connections| connections.total() > 0);
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
            pinned_element_connections: pinned_connection_counts,
            render_progress,
            workspace_path,
        };
        let status_text = self.status_bar.render(&context, canvas.cells.cols);
        let footer_text = self.footer_bar.render(&context, canvas.cells.cols);

        let mut canvas_rect = Rect::default();
        self.inner.draw(|frame| {
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

        let screen_canvas = CanvasMetrics::new(
            CellSize::new(canvas_rect.width, canvas_rect.height),
            canvas.cell_pixel.or_fallback(),
        );
        let target = CellArea::new(
            canvas_rect.x,
            canvas_rect.y,
            canvas_rect.width,
            canvas_rect.height,
        );
        let parts = store.viewport_render_parts(view_id, screen_canvas)?;
        self.inner.render_image_viewport(
            parts.widget,
            image_id,
            MAIN_PLACEMENT_ID,
            parts.png,
            target,
        )?;
        self.inner.images().flush()?;
        Ok(())
    }

    pub fn close_picker(&mut self, store: &ViewStore) -> Result<()> {
        self.inner
            .teardown_image_viewports((0..store.views.len()).map(|index| {
                (
                    image_id_for_view(ViewId::new(index)),
                    picker_placement_id(index),
                )
            }))?;
        self.inner.images().flush()?;
        Ok(())
    }

    pub fn teardown_image_viewport(&mut self, view_id: ViewId) -> Result<()> {
        self.inner
            .teardown_image_viewport(image_id_for_view(view_id), MAIN_PLACEMENT_ID)?;
        self.inner.images().flush()?;
        Ok(())
    }

    pub fn draw_picker(
        &mut self,
        picker: &mut Cached<ViewPicker>,
        store: &ViewStore,
    ) -> Result<()> {
        let mut render_result: Result<()> = Ok(());
        self.inner.draw(|frame| {
            let area = frame.area();
            render_result = picker.render_to_buffer(area, frame.buffer_mut());
        })?;
        render_result?;

        let thumbs = picker.inner().thumbnails().to_vec();
        let placements_to_clear: Vec<(u32, u32)> = (0..store.views.len())
            .map(|index| {
                (
                    image_id_for_view(ViewId::new(index)),
                    picker_placement_id(index),
                )
            })
            .filter(|(_, placement_id)| {
                !thumbs
                    .iter()
                    .any(|thumb| picker_placement_id(thumb.view_id.index()) == *placement_id)
            })
            .collect();
        self.inner.teardown_image_viewports(placements_to_clear)?;

        for thumb in &thumbs {
            self.draw_thumbnail(thumb.view_id, thumb.area, store)?;
        }
        self.inner.images().flush()?;
        Ok(())
    }

    fn draw_thumbnail(
        &mut self,
        view_id: ViewId,
        area: CellArea,
        store: &ViewStore,
    ) -> Result<bool> {
        let image_id = image_id_for_view(view_id);
        let Some(rendered) = store.cached_rendered_view(view_id) else {
            return Ok(false);
        };
        let image = ViewportImage::new(rendered.raster_size, rendered.rgba.clone())?;
        self.inner.render_viewport_image(
            image,
            image_id,
            picker_placement_id(view_id.index()),
            &rendered.png,
            area,
            ImageViewportOptions {
                initial_scale: ImageViewportInitialScale::FitToBox,
                resize_policy: ResizePolicy::PreserveTopLeft,
            },
        )?;
        Ok(true)
    }

    fn clear_image_cache_inner(&mut self) -> Result<()> {
        self.inner.images().forget_all()?;
        self.inner.images().flush()?;
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
            "Keys\n\n  {quit}  Quit\n  Esc  Clear pinned element / quit if none\n  {open}  Open view picker (type to filter, Tab toggles legends)\n  {reload}  Reload workspace/export\n  K  Jump to legend for current view\n  i  Inspect element at viewport center\n  Enter  Follow first connection from pinned/center element\n  Backspace  Go back through breadcrumbs\n  Arrows or hjkl  Pan\n  {zoom_in}/=  Zoom in\n  {zoom_out}/_  Zoom out\n  {reset} or {fit}  Reset/fit view\n  Mouse wheel  Zoom around cursor\n  Mouse drag  Pan\n  Click element  Drill into child view, else pin\n\nConfig: ~/.config/c4tui/config.toml\nLogging: --log-file <path>, level via RUST_LOG",
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
        let dialog = Dialog::new(title, message).with_footer(footer);
        self.inner.images().delete_placement(MAIN_PLACEMENT_ID)?;
        self.inner.draw(|frame| {
            let area = frame.area();
            dialog.render(area, frame.buffer_mut());
        })?;
        self.inner.images().flush()?;
        Ok(())
    }

    pub fn draw_log_view(&mut self, log_view: &mut LogView) -> Result<()> {
        // Clear any diagram image so the log pane gets a clean text surface
        // and clipboard selection is unobstructed by graphic cells.
        self.inner.images().delete_placement(MAIN_PLACEMENT_ID)?;
        self.inner
            .images()
            .delete_all_placements()
            .or_else(|_| Ok::<_, anyhow::Error>(()))
            .ok();
        let mut render_result: Result<()> = Ok(());
        self.inner.draw(|frame| {
            let area = frame.area();
            render_result = render_log_view(area, frame.buffer_mut(), log_view);
        })?;
        render_result?;
        self.inner.images().flush()?;
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

    fn teardown_image_viewport(&mut self, view_id: ViewId) -> Result<()> {
        Self::teardown_image_viewport(self, view_id)
    }

    fn draw_picker(&mut self, picker: &mut Cached<ViewPicker>, store: &ViewStore) -> Result<()> {
        Self::draw_picker(self, picker, store)
    }

    fn close_picker(&mut self, store: &ViewStore) -> Result<()> {
        Self::close_picker(self, store)
    }

    fn draw_log_view(&mut self, log_view: &mut LogView) -> Result<()> {
        Self::draw_log_view(self, log_view)
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

// Drop handled by tui_kit::terminal::Terminal: leaves alt-screen, disables
// mouse capture, restores cursor, exits raw mode, shuts down image registry.

fn render_log_view(
    area: Rect,
    buf: &mut ratatui::buffer::Buffer,
    log_view: &mut LogView,
) -> Result<()> {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" c4tui logs ")
        .title_bottom(" j/k or ↑/↓ scroll · g/G top/bottom · y yank visible · Y yank all · c clear · Esc close ");
    let inner = block.inner(area);
    Clear.render(area, buf);
    block.render(area, buf);
    if inner.height < 3 || inner.width < 12 {
        return Ok(());
    }

    let body_height = inner.height.saturating_sub(1) as usize;
    log_view.note_body_height(body_height);
    let snap = log_view.snapshot(body_height);

    // Status / arrival info on the top row.
    let mut header = format!(
        "{} entries  ·  scroll back: {}",
        snap.total, snap.scroll_back
    );
    if snap.new_arrivals_since_last_snapshot {
        header.push_str("  ·  (new entries since last view)");
    }
    let avail = inner.width.saturating_sub(1) as usize;
    buf.set_string(
        inner.x,
        inner.y,
        truncate(&header, avail),
        Style::default().add_modifier(Modifier::DIM),
    );

    let body = Rect {
        x: inner.x,
        y: inner.y.saturating_add(1),
        width: inner.width,
        height: inner.height.saturating_sub(1),
    };

    if snap.visible.is_empty() {
        let placeholder = if snap.total == 0 {
            "no log entries yet"
        } else {
            "no entries in view"
        };
        buf.set_string(
            body.x,
            body.y,
            placeholder,
            Style::default().add_modifier(Modifier::DIM),
        );
    } else {
        for (idx, entry) in snap.visible.iter().enumerate() {
            let row = body.y + idx as u16;
            if row >= body.y + body.height {
                break;
            }
            let style = match entry.level {
                log::Level::Error => Style::default().add_modifier(Modifier::BOLD),
                log::Level::Warn => Style::default().add_modifier(Modifier::BOLD),
                log::Level::Info => Style::default(),
                log::Level::Debug | log::Level::Trace => {
                    Style::default().add_modifier(Modifier::DIM)
                }
            };
            let line = entry.render();
            let max_cols = body.width.saturating_sub(1) as usize;
            buf.set_string(body.x, row, truncate(&line, max_cols), style);
        }
    }

    // Scroll arrows on the right gutter of the body.
    if snap.can_scroll_up && body.height > 0 {
        buf.set_string(
            inner.x + inner.width.saturating_sub(1),
            body.y,
            "▲",
            Style::default(),
        );
    }
    if snap.can_scroll_down && body.height > 0 {
        buf.set_string(
            inner.x + inner.width.saturating_sub(1),
            body.y + body.height.saturating_sub(1),
            "▼",
            Style::default(),
        );
    }

    // Toast (last yank/clear status) overlaid on the bottom border row.
    if let Some(status) = log_view.last_status() {
        let avail = area.width.saturating_sub(2) as usize;
        let bottom_y = area.y + area.height.saturating_sub(1);
        buf.set_string(
            area.x + 1,
            bottom_y,
            truncate(status, avail),
            Style::default().add_modifier(Modifier::REVERSED),
        );
    }

    Ok(())
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        text.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
    }
}

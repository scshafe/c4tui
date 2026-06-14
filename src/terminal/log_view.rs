use crate::log_view::LogView;
use anyhow::Result;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Widget};

pub(crate) fn render_log_view(area: Rect, buf: &mut Buffer, log_view: &mut LogView) -> Result<()> {
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

use crate::config::KeyBindings;
use anyhow::Result;
use ratatui::widgets::Widget;
use tui_kit::image::{ImageSurface, MAIN_PLACEMENT_ID};
use tui_kit::widgets::dialog::Dialog;

use super::session::TerminalSession;

pub(super) fn show_error(session: &mut TerminalSession, title: &str, message: &str) -> Result<()> {
    show_dialog(session, title, message, " press any key to continue ")
}

pub(super) fn show_message(
    session: &mut TerminalSession,
    title: &str,
    message: &str,
) -> Result<()> {
    show_dialog(session, title, message, "")
}

pub(super) fn show_help(session: &mut TerminalSession, keys: &KeyBindings) -> Result<()> {
    let text = help_text(keys);
    show_dialog(session, "c4tui help", &text, " press any key to continue ")
}

pub(super) fn help_text(keys: &KeyBindings) -> String {
    format!(
        "Keys\n\n  {quit}  Quit\n  Esc  Clear pinned element / quit if none\n  {open}  Open view picker (type to filter, Tab toggles legends)\n  {reload}  Reload workspace/export\n  K  Jump to legend for current view\n  i  Inspect element at viewport center\n  Enter  Open link directory\n  Backspace  Go back through breadcrumbs\n  Arrows or hjkl  Pan\n  {zoom_in}/=  Zoom in\n  {zoom_out}/_  Zoom out\n  {reset} or {fit}  Reset/fit view\n  Mouse wheel  Zoom around cursor\n  Mouse drag  Pan\n  Click element  Drill into child view, else pin\n\nConfig: ~/.config/c4tui/config.toml\nLogging: --log-file <path>, level via RUST_LOG",
        quit = keys.quit,
        open = keys.open_picker,
        reload = keys.reload,
        zoom_in = keys.zoom_in,
        zoom_out = keys.zoom_out,
        reset = keys.reset,
        fit = keys.fit,
    )
}

fn show_dialog(
    session: &mut TerminalSession,
    title: &str,
    message: &str,
    footer: &str,
) -> Result<()> {
    let dialog = Dialog::new(title, message).with_footer(footer);
    session.inner.images().delete_placement(MAIN_PLACEMENT_ID)?;
    session.inner.draw(|frame| {
        let area = frame.area();
        dialog.render(area, frame.buffer_mut());
    })?;
    session.inner.images().flush()?;
    Ok(())
}

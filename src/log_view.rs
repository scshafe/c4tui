#![allow(dead_code)]

//! State and key handling for the in-app log viewer pane.
//!
//! The viewer is a modal scope (`SCOPE_LOG`) the user pushes with `L` from
//! the normal mode. It scrolls a snapshot of the [`SharedLogBuffer`] and
//! lets the user yank to the system clipboard via the [`Clipboard`] trait.

use crate::clipboard::{Clipboard, CopyOutcome};
use crate::logger::{LogEntry, SharedLogBuffer};
use anyhow::Result;
use tui_kit::input::KeyEvent;
use tui_kit::layout::TailViewport;

/// What the viewer wants the app shell to do after a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogViewOutcome {
    Continue,
    Close,
}

/// The yank target: just visible lines, or the whole buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YankRange {
    Visible,
    All,
}

#[derive(Debug)]
pub struct LogView {
    buffer: SharedLogBuffer,
    /// Number of lines from the bottom we've scrolled up. 0 means stuck to
    /// the tail.
    scroll_back: usize,
    /// Last visible body height, captured at render time. Used to clamp
    /// scroll on resize and to decide what "all visible" means for yank.
    last_body_height: usize,
    /// Last action result, surfaced as a single footer line. Cleared on the
    /// next key.
    last_status: Option<String>,
    /// Total entries pushed at last snapshot — used to detect new arrivals.
    last_seen_total: u64,
}

impl LogView {
    pub fn new(buffer: SharedLogBuffer) -> Self {
        Self {
            buffer,
            scroll_back: 0,
            last_body_height: 0,
            last_status: None,
            last_seen_total: 0,
        }
    }

    pub fn buffer(&self) -> &SharedLogBuffer {
        &self.buffer
    }

    pub fn last_status(&self) -> Option<&str> {
        self.last_status.as_deref()
    }

    /// Rendering captures the body height so future scroll/yank decisions
    /// can use it. Called by the terminal layer.
    pub fn note_body_height(&mut self, height: usize) {
        self.last_body_height = height.max(1);
    }

    /// Snapshot of entries to render in the current viewport.
    /// Returns `(visible_entries, total_entries, scroll_back, can_scroll_up, can_scroll_down)`.
    pub fn snapshot(&mut self, body_height: usize) -> LogSnapshot {
        let body = body_height.max(1);
        let buf = self.buffer.lock().expect("log buffer poisoned");
        let total = buf.len();
        let viewport = TailViewport::new(total, body, self.scroll_back);
        self.scroll_back = viewport.scroll_back;
        let visible: Vec<LogEntry> = buf
            .entries()
            .iter()
            .skip(viewport.start)
            .take(viewport.end - viewport.start)
            .cloned()
            .collect();
        let new_total = buf.total_pushed;
        drop(buf);
        let new_arrivals = new_total > self.last_seen_total;
        self.last_seen_total = new_total;
        LogSnapshot {
            visible,
            total,
            scroll_back: viewport.scroll_back,
            can_scroll_up: viewport.can_scroll_up,
            can_scroll_down: viewport.can_scroll_down,
            new_arrivals_since_last_snapshot: new_arrivals,
        }
    }

    /// Handle a key press. The clipboard is passed in so the viewer can
    /// implement `y` / `Y` without owning the impl.
    pub fn handle_key(
        &mut self,
        key: KeyEvent,
        clipboard: &dyn Clipboard,
    ) -> Result<LogViewOutcome> {
        // Any key clears a stale toast.
        self.last_status = None;
        match key {
            KeyEvent::Esc | KeyEvent::Char('q') | KeyEvent::Char('Q') => Ok(LogViewOutcome::Close),
            KeyEvent::Up | KeyEvent::Char('k') => {
                self.scroll_by(1);
                Ok(LogViewOutcome::Continue)
            }
            KeyEvent::Down | KeyEvent::Char('j') => {
                self.scroll_by(-1);
                Ok(LogViewOutcome::Continue)
            }
            KeyEvent::Char('g') => {
                self.scroll_to_top();
                Ok(LogViewOutcome::Continue)
            }
            KeyEvent::Char('G') => {
                self.scroll_to_bottom();
                Ok(LogViewOutcome::Continue)
            }
            KeyEvent::Char('y') => {
                let outcome = self.yank(YankRange::Visible, clipboard);
                self.last_status = Some(outcome);
                Ok(LogViewOutcome::Continue)
            }
            KeyEvent::Char('Y') => {
                let outcome = self.yank(YankRange::All, clipboard);
                self.last_status = Some(outcome);
                Ok(LogViewOutcome::Continue)
            }
            KeyEvent::Char('c') => {
                let cleared = {
                    let mut buf = self.buffer.lock().expect("log buffer poisoned");
                    let n = buf.len();
                    buf.clear();
                    n
                };
                self.scroll_back = 0;
                self.last_status = Some(format!("cleared {cleared} log entries"));
                Ok(LogViewOutcome::Continue)
            }
            _ => Ok(LogViewOutcome::Continue),
        }
    }

    fn scroll_by(&mut self, delta_up: isize) {
        let body = self.last_body_height.max(1);
        let total = self.buffer.lock().expect("log buffer poisoned").len();
        self.scroll_back = TailViewport::new(total, body, self.scroll_back).scroll_by(delta_up);
    }

    fn scroll_to_top(&mut self) {
        let body = self.last_body_height.max(1);
        let total = self.buffer.lock().expect("log buffer poisoned").len();
        self.scroll_back = TailViewport::new(total, body, self.scroll_back).scroll_to_top();
    }

    fn scroll_to_bottom(&mut self) {
        self.scroll_back = TailViewport::scroll_to_bottom();
    }

    fn yank(&self, range: YankRange, clipboard: &dyn Clipboard) -> String {
        let (text, line_count) = self.materialise(range);
        if text.is_empty() {
            return "log buffer is empty".to_owned();
        }
        match clipboard.copy_text(&text) {
            Ok(CopyOutcome::Native) => format!("copied {line_count} lines to clipboard"),
            Ok(CopyOutcome::Fallback { description, .. }) => {
                format!("copied {line_count} lines: {description}")
            }
            Err(error) => format!("clipboard error: {error}"),
        }
    }

    fn materialise(&self, range: YankRange) -> (String, usize) {
        let buf = self.buffer.lock().expect("log buffer poisoned");
        let entries: Vec<&LogEntry> = match range {
            YankRange::All => buf.entries().iter().collect(),
            YankRange::Visible => {
                let body = self.last_body_height.max(1);
                let total = buf.len();
                let viewport = TailViewport::new(total, body, self.scroll_back);
                buf.entries()
                    .iter()
                    .skip(viewport.start)
                    .take(viewport.end - viewport.start)
                    .collect()
            }
        };
        let line_count = entries.len();
        let text = entries
            .into_iter()
            .map(LogEntry::render)
            .collect::<Vec<_>>()
            .join("\n");
        (text, line_count)
    }
}

#[derive(Debug, Clone)]
pub struct LogSnapshot {
    pub visible: Vec<LogEntry>,
    pub total: usize,
    pub scroll_back: usize,
    pub can_scroll_up: bool,
    pub can_scroll_down: bool,
    pub new_arrivals_since_last_snapshot: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::{ClipboardCapabilities, ClipboardSupport, ImagePayload};
    use crate::logger::LogBuffer;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    struct RecordingClipboard {
        copies: AtomicUsize,
        last_text: Mutex<Option<String>>,
    }
    impl Clipboard for RecordingClipboard {
        fn capabilities(&self) -> ClipboardCapabilities {
            ClipboardCapabilities {
                text: ClipboardSupport::Native,
                image: ClipboardSupport::Unsupported,
            }
        }
        fn copy_text(&self, text: &str) -> Result<CopyOutcome> {
            self.copies.fetch_add(1, Ordering::SeqCst);
            *self.last_text.lock().unwrap() = Some(text.to_owned());
            Ok(CopyOutcome::Native)
        }
        fn copy_image(&self, _image: &ImagePayload) -> Result<CopyOutcome> {
            anyhow::bail!("unsupported")
        }
    }

    fn populated_buffer(n: usize) -> SharedLogBuffer {
        let buf = std::sync::Arc::new(Mutex::new(LogBuffer::with_capacity(1000)));
        for i in 0..n {
            buf.lock().unwrap().push(LogEntry {
                level: log::Level::Warn,
                target: "test".into(),
                message: format!("line {i}"),
            });
        }
        buf
    }

    #[test]
    fn snapshot_returns_tail_when_not_scrolled() {
        let mut view = LogView::new(populated_buffer(5));
        let snap = view.snapshot(3);
        assert_eq!(snap.visible.len(), 3);
        assert_eq!(snap.visible.last().unwrap().message, "line 4");
        assert!(snap.can_scroll_up);
        assert!(!snap.can_scroll_down);
    }

    #[test]
    fn scrolling_up_reveals_older_entries() {
        let mut view = LogView::new(populated_buffer(5));
        view.note_body_height(3);
        view.handle_key(KeyEvent::Up, &RecordingClipboard::default())
            .unwrap();
        view.handle_key(KeyEvent::Up, &RecordingClipboard::default())
            .unwrap();
        let snap = view.snapshot(3);
        assert_eq!(snap.visible.last().unwrap().message, "line 2");
        assert!(snap.can_scroll_down);
    }

    #[test]
    fn yank_visible_copies_only_visible_lines() {
        let mut view = LogView::new(populated_buffer(10));
        view.note_body_height(3);
        let cb = RecordingClipboard::default();
        view.handle_key(KeyEvent::Char('y'), &cb).unwrap();
        let copied = cb.last_text.lock().unwrap().clone().unwrap();
        let lines: Vec<&str> = copied.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[2].contains("line 9"));
    }

    #[test]
    fn yank_all_copies_full_buffer() {
        let mut view = LogView::new(populated_buffer(10));
        view.note_body_height(3);
        let cb = RecordingClipboard::default();
        view.handle_key(KeyEvent::Char('Y'), &cb).unwrap();
        let copied = cb.last_text.lock().unwrap().clone().unwrap();
        assert_eq!(copied.lines().count(), 10);
    }

    #[test]
    fn esc_closes_the_view() {
        let mut view = LogView::new(populated_buffer(2));
        let outcome = view
            .handle_key(KeyEvent::Esc, &RecordingClipboard::default())
            .unwrap();
        assert_eq!(outcome, LogViewOutcome::Close);
    }

    #[test]
    fn clear_empties_the_buffer() {
        let mut view = LogView::new(populated_buffer(3));
        view.handle_key(KeyEvent::Char('c'), &RecordingClipboard::default())
            .unwrap();
        let snap = view.snapshot(5);
        assert_eq!(snap.visible.len(), 0);
    }

    #[test]
    fn yank_on_empty_buffer_reports_empty() {
        let mut view = LogView::new(populated_buffer(0));
        let cb = RecordingClipboard::default();
        view.handle_key(KeyEvent::Char('y'), &cb).unwrap();
        assert_eq!(cb.copies.load(Ordering::SeqCst), 0);
        assert!(view.last_status().unwrap().contains("empty"));
    }
}

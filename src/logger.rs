#![allow(dead_code)]

//! In-memory + optional-file logger that never writes to stderr.
//!
//! While c4tui owns the alt screen, anything written to stderr corrupts the
//! UI (env_logger's default target). This module installs an [`AppLogger`]
//! that drains every log record into a bounded ring buffer (consumed by the
//! in-app log viewer) and, when `--log-file` is supplied, also into that
//! file. Stderr is never touched.
//!
//! Filtering still honours the `RUST_LOG` env var via [`env_logger::filter`].

use anyhow::{Context, Result};
use std::collections::VecDeque;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};

const DEFAULT_BUFFER_CAPACITY: usize = 1000;

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub level: log::Level,
    pub target: String,
    pub message: String,
}

impl LogEntry {
    /// Render the entry as the line that should appear in the log viewer or
    /// be copied to the clipboard.
    pub fn render(&self) -> String {
        format!("{:<5} {} — {}", self.level, self.target, self.message)
    }
}

#[derive(Debug, Default)]
pub struct LogBuffer {
    entries: VecDeque<LogEntry>,
    capacity: usize,
    /// Monotonic counter of entries pushed since process start (including
    /// ones evicted from the ring). The viewer uses this to know whether new
    /// entries arrived since the last snapshot.
    pub total_pushed: u64,
}

impl LogBuffer {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity.min(1024)),
            capacity,
            total_pushed: 0,
        }
    }

    pub fn push(&mut self, entry: LogEntry) {
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
        self.total_pushed = self.total_pushed.saturating_add(1);
    }

    pub fn entries(&self) -> &VecDeque<LogEntry> {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Render every entry as `level target — message` separated by `\n`.
    pub fn render_all(&self) -> String {
        let mut out = String::new();
        for (idx, entry) in self.entries.iter().enumerate() {
            if idx > 0 {
                out.push('\n');
            }
            out.push_str(&entry.render());
        }
        out
    }
}

pub type SharedLogBuffer = Arc<Mutex<LogBuffer>>;

#[derive(Debug)]
pub struct AppLogger {
    buffer: SharedLogBuffer,
    file: Option<Mutex<File>>,
    filter: env_filter::Filter,
}

impl log::Log for AppLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        self.filter.enabled(metadata)
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.filter.matches(record) {
            return;
        }
        let entry = LogEntry {
            level: record.level(),
            target: record.target().to_owned(),
            message: format!("{}", record.args()),
        };

        // Always write to the buffer first (the UI's source of truth).
        if let Ok(mut buf) = self.buffer.lock() {
            buf.push(entry.clone());
        }

        // Best-effort tee to file. Any error is silenced — we can't log it
        // without recursing into ourselves, and failing to tee shouldn't
        // affect runtime behaviour.
        if let Some(file_mu) = &self.file {
            if let Ok(mut file) = file_mu.lock() {
                let _ = writeln!(file, "{}", entry.render());
            }
        }
    }

    fn flush(&self) {
        if let Some(file_mu) = &self.file {
            if let Ok(mut file) = file_mu.lock() {
                let _ = file.flush();
            }
        }
    }
}

/// Install [`AppLogger`] as the global logger. Returns the shared buffer so
/// the app can read from it.
///
/// `RUST_LOG` controls filtering. The default level is `warn` if `RUST_LOG`
/// is unset.
pub fn install(log_file: Option<&Path>) -> Result<SharedLogBuffer> {
    let mut filter_builder = env_filter::Builder::from_env("RUST_LOG");
    if std::env::var("RUST_LOG").is_err() {
        filter_builder.parse("warn");
    }
    let filter = filter_builder.build();

    let buffer = Arc::new(Mutex::new(LogBuffer::with_capacity(
        DEFAULT_BUFFER_CAPACITY,
    )));
    let file = match log_file {
        Some(path) => {
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent).with_context(|| {
                        format!("failed to create log directory {}", parent.display())
                    })?;
                }
            }
            let f = File::create(path)
                .with_context(|| format!("failed to create log file {}", path.display()))?;
            Some(Mutex::new(f))
        }
        None => None,
    };

    let level = filter.filter();
    let logger = AppLogger {
        buffer: Arc::clone(&buffer),
        file,
        filter,
    };
    log::set_max_level(level);
    log::set_boxed_logger(Box::new(logger))
        .context("a global logger was already installed by something else")?;
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_evicts_oldest_when_full() {
        let mut buf = LogBuffer::with_capacity(2);
        buf.push(LogEntry {
            level: log::Level::Warn,
            target: "t".into(),
            message: "a".into(),
        });
        buf.push(LogEntry {
            level: log::Level::Warn,
            target: "t".into(),
            message: "b".into(),
        });
        buf.push(LogEntry {
            level: log::Level::Warn,
            target: "t".into(),
            message: "c".into(),
        });
        assert_eq!(buf.len(), 2);
        assert_eq!(buf.entries().front().unwrap().message, "b");
        assert_eq!(buf.entries().back().unwrap().message, "c");
        assert_eq!(buf.total_pushed, 3);
    }

    #[test]
    fn render_all_joins_with_newlines() {
        let mut buf = LogBuffer::with_capacity(4);
        buf.push(LogEntry {
            level: log::Level::Warn,
            target: "x".into(),
            message: "first".into(),
        });
        buf.push(LogEntry {
            level: log::Level::Info,
            target: "y".into(),
            message: "second".into(),
        });
        let s = buf.render_all();
        assert!(s.contains("first"));
        assert!(s.contains("second"));
        assert_eq!(s.lines().count(), 2);
    }
}

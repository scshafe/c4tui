#![allow(dead_code)]

//! Clipboard interface, capability reporting, and a built-in default
//! implementation.
//!
//! The trait is the seam for future native-clipboard extensions (e.g., a
//! macOS-specific impl that puts images on `NSPasteboard`, or a Linux impl
//! that talks to `wl-copy` / `xclip`). Those should ship as separate crates
//! or feature-gated modules and be swapped in at `App::new` time.
//!
//! [`DefaultClipboard`] is what c4tui ships out of the box. Text uses
//! `pbcopy` on macOS (system clipboard); other platforms and image payloads
//! fall through to a file under `/tmp` and report
//! [`CopyOutcome::Fallback`] so the UI can surface the path.

use anyhow::{Context, Result};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClipboardSupport {
    /// Lands directly in the OS clipboard.
    Native,
    /// Lands on disk; the user has to read or attach the file themselves.
    FileFallback,
    /// Not implemented at all — calling the corresponding method returns an error.
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipboardCapabilities {
    pub text: ClipboardSupport,
    pub image: ClipboardSupport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CopyOutcome {
    /// Content reached the system clipboard.
    Native,
    /// Content was written to a file as a fallback. `description` is a short
    /// human-readable explanation suitable for a status line / toast.
    Fallback { path: PathBuf, description: String },
}

#[derive(Debug, Clone)]
pub struct ImagePayload {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub trait Clipboard: Send + Sync + std::fmt::Debug {
    fn capabilities(&self) -> ClipboardCapabilities;
    fn copy_text(&self, text: &str) -> Result<CopyOutcome>;
    fn copy_image(&self, image: &ImagePayload) -> Result<CopyOutcome>;
}

/// Built-in clipboard implementation. Designed to be replaceable: future
/// platform-native crates implement [`Clipboard`] and get swapped in at app
/// construction.
#[derive(Debug, Default)]
pub struct DefaultClipboard;

const TEXT_FALLBACK_PATH: &str = "/tmp/c4tui-yank.txt";

impl Clipboard for DefaultClipboard {
    fn capabilities(&self) -> ClipboardCapabilities {
        ClipboardCapabilities {
            text: if cfg!(target_os = "macos") {
                ClipboardSupport::Native
            } else {
                ClipboardSupport::FileFallback
            },
            image: ClipboardSupport::FileFallback,
        }
    }

    fn copy_text(&self, text: &str) -> Result<CopyOutcome> {
        if cfg!(target_os = "macos") {
            match shell_pbcopy(text) {
                Ok(()) => return Ok(CopyOutcome::Native),
                Err(error) => {
                    log::warn!("pbcopy failed, falling back to file: {error:#}");
                }
            }
        }
        write_text_fallback(text)
    }

    fn copy_image(&self, image: &ImagePayload) -> Result<CopyOutcome> {
        // No native image clipboard support in the built-in. Always fallback.
        write_image_fallback(image)
    }
}

fn shell_pbcopy(text: &str) -> Result<()> {
    let mut child = Command::new("pbcopy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to spawn pbcopy")?;
    child
        .stdin
        .as_mut()
        .context("pbcopy stdin unavailable")?
        .write_all(text.as_bytes())
        .context("failed to write to pbcopy stdin")?;
    let output = child.wait_with_output().context("pbcopy did not finish")?;
    if !output.status.success() {
        anyhow::bail!(
            "pbcopy exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

fn write_text_fallback(text: &str) -> Result<CopyOutcome> {
    let path = PathBuf::from(TEXT_FALLBACK_PATH);
    write_to_path(&path, text.as_bytes())?;
    Ok(CopyOutcome::Fallback {
        description: format!("wrote text to {}", path.display()),
        path,
    })
}

fn write_image_fallback(image: &ImagePayload) -> Result<CopyOutcome> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = PathBuf::from(format!("/tmp/c4tui-yank-{stamp}.png"));
    write_to_path(&path, &image.png)?;
    Ok(CopyOutcome::Fallback {
        description: format!(
            "wrote {}x{} PNG to {}",
            image.width,
            image.height,
            path.display()
        ),
        path,
    })
}

fn write_to_path(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = std::fs::File::create(path)
        .with_context(|| format!("failed to create {}", path.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("failed to write {}", path.display()))?;
    file.flush().ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default)]
    struct CountingClipboard {
        text_calls: std::sync::atomic::AtomicUsize,
    }

    impl Clipboard for CountingClipboard {
        fn capabilities(&self) -> ClipboardCapabilities {
            ClipboardCapabilities {
                text: ClipboardSupport::Native,
                image: ClipboardSupport::Unsupported,
            }
        }
        fn copy_text(&self, _text: &str) -> Result<CopyOutcome> {
            self.text_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(CopyOutcome::Native)
        }
        fn copy_image(&self, _image: &ImagePayload) -> Result<CopyOutcome> {
            anyhow::bail!("unsupported")
        }
    }

    #[test]
    fn trait_is_dyn_compatible() {
        let cb: Box<dyn Clipboard> = Box::new(CountingClipboard::default());
        let outcome = cb.copy_text("hello").unwrap();
        assert_eq!(outcome, CopyOutcome::Native);
    }

    #[test]
    fn default_clipboard_text_fallback_writes_file_off_macos() {
        if cfg!(target_os = "macos") {
            return;
        }
        let cb = DefaultClipboard;
        let outcome = cb.copy_text("hello").unwrap();
        match outcome {
            CopyOutcome::Fallback { path, .. } => {
                assert!(
                    path.exists(),
                    "fallback file should exist at {}",
                    path.display()
                );
            }
            CopyOutcome::Native => panic!("expected fallback off macOS"),
        }
    }

    #[test]
    fn default_clipboard_image_always_falls_back_to_file() {
        let cb = DefaultClipboard;
        let payload = ImagePayload {
            png: vec![0x89, 0x50, 0x4E, 0x47],
            width: 1,
            height: 1,
        };
        let outcome = cb.copy_image(&payload).unwrap();
        match outcome {
            CopyOutcome::Fallback { path, .. } => assert!(path.exists()),
            CopyOutcome::Native => panic!("DefaultClipboard never reports Native for images"),
        }
    }

    #[test]
    fn capabilities_reflect_platform() {
        let caps = DefaultClipboard.capabilities();
        if cfg!(target_os = "macos") {
            assert_eq!(caps.text, ClipboardSupport::Native);
        } else {
            assert_eq!(caps.text, ClipboardSupport::FileFallback);
        }
        assert_eq!(caps.image, ClipboardSupport::FileFallback);
    }
}

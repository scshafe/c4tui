use std::env;
use std::fmt;
use std::io;
use std::time::{Duration, Instant};
use tui_kit::tty::{
    get_termios, make_raw, set_termios, stdin_is_terminal, stdout_is_terminal, write_stdout_all,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Support {
    Yes,
    No,
    Unknown,
}

impl fmt::Display for Support {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Yes => f.write_str("yes"),
            Self::No => f.write_str("no"),
            Self::Unknown => f.write_str("unknown"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Capabilities {
    pub kitty_graphics: Support,
    pub pixel_mouse: Support,
    pub truecolor: Support,
}

const KITTY_GRAPHICS_QUERY: &[u8] = b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\";
const SGR_PIXEL_MOUSE_QUERY: &[u8] = b"\x1b[?1016h\x1b[?1016$p";
const SGR_PIXEL_MOUSE_DISABLE: &[u8] = b"\x1b[?1016l";

pub(crate) fn detect_capabilities(
    timeout: Duration,
    force_probe: bool,
) -> io::Result<Capabilities> {
    let baseline = env_capabilities();

    let probe_possible = stdin_is_terminal() && stdout_is_terminal();
    if !probe_possible {
        return Ok(baseline);
    }

    let mut session = match TerminalProbeSession::new() {
        Ok(session) => session,
        Err(error) if force_probe => {
            log::warn!("capability probe unavailable ({error}); falling back to env detection");
            return Ok(baseline);
        }
        Err(error) => return Err(error),
    };
    let kitty_graphics = merge_support(
        baseline.kitty_graphics,
        session.query_kitty_graphics(timeout)?,
    );
    let pixel_mouse = merge_support(
        baseline.pixel_mouse,
        session.query_sgr_pixel_mouse(timeout)?,
    );

    Ok(Capabilities {
        kitty_graphics,
        pixel_mouse,
        truecolor: baseline.truecolor,
    })
}

fn merge_support(env: Support, probed: Support) -> Support {
    match probed {
        Support::Unknown => env,
        decided => decided,
    }
}

fn env_capabilities() -> Capabilities {
    let term_program = env::var("TERM_PROGRAM").unwrap_or_default();
    let term = env::var("TERM").unwrap_or_default().to_ascii_lowercase();
    Capabilities {
        kitty_graphics: detect_kitty_graphics_env(&term_program, &term),
        pixel_mouse: detect_pixel_mouse_env(&term_program, &term),
        truecolor: detect_truecolor_env(&term_program, &term),
    }
}

fn detect_truecolor_env(term_program: &str, term: &str) -> Support {
    let colorterm = env::var("COLORTERM")
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(colorterm.as_str(), "truecolor" | "24bit") {
        return Support::Yes;
    }
    if term.contains("truecolor") || term.contains("24bit") {
        return Support::Yes;
    }
    match term_program {
        "WezTerm" | "ghostty" | "Ghostty" | "iTerm.app" | "kitty" => Support::Yes,
        _ => Support::Unknown,
    }
}

fn detect_kitty_graphics_env(term_program: &str, term: &str) -> Support {
    if env::var_os("KITTY_WINDOW_ID").is_some() {
        return Support::Yes;
    }
    if term.contains("kitty") || term.contains("ghostty") {
        return Support::Yes;
    }
    match term_program {
        "WezTerm" | "ghostty" | "Ghostty" | "kitty" => Support::Yes,
        _ => Support::Unknown,
    }
}

fn detect_pixel_mouse_env(term_program: &str, _term: &str) -> Support {
    match term_program {
        "WezTerm" | "ghostty" | "Ghostty" | "kitty" | "iTerm.app" => Support::Yes,
        _ => Support::Unknown,
    }
}

trait CapabilityProbeIo {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()>;
    fn drain_input(&mut self) -> io::Result<()>;
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;
    fn restore(&mut self) -> io::Result<()>;
}

struct RawTerminalProbeIo {
    stdin_fd: libc::c_int,
    original_termios: libc::termios,
    restored: bool,
}

impl RawTerminalProbeIo {
    fn enter() -> io::Result<Self> {
        let stdin_fd = libc::STDIN_FILENO;
        let original_termios = get_termios(stdin_fd)?;

        let mut raw = original_termios;
        make_raw(&mut raw);
        set_termios(stdin_fd, &raw)?;

        Ok(Self {
            stdin_fd,
            original_termios,
            restored: false,
        })
    }
}

impl CapabilityProbeIo for RawTerminalProbeIo {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        write_stdout_all(bytes)
    }

    fn drain_input(&mut self) -> io::Result<()> {
        let mut buf = [0_u8; 1024];
        loop {
            match self.read(&mut buf) {
                Ok(0) => break,
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = unsafe { libc::read(self.stdin_fd, buf.as_mut_ptr().cast(), buf.len()) };
        if read < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(read as usize)
        }
    }

    fn restore(&mut self) -> io::Result<()> {
        if self.restored {
            return Ok(());
        }
        set_termios(self.stdin_fd, &self.original_termios)?;
        self.restored = true;
        Ok(())
    }
}

struct TerminalProbeSession<I: CapabilityProbeIo = RawTerminalProbeIo> {
    io: I,
}

impl TerminalProbeSession<RawTerminalProbeIo> {
    fn new() -> io::Result<Self> {
        Ok(Self::with_io(RawTerminalProbeIo::enter()?))
    }
}

impl<I: CapabilityProbeIo> TerminalProbeSession<I> {
    fn with_io(io: I) -> Self {
        Self { io }
    }

    fn query_kitty_graphics(&mut self, timeout: Duration) -> io::Result<Support> {
        self.io.drain_input()?;

        self.io.write_all(KITTY_GRAPHICS_QUERY)?;
        let response = self.read_until(timeout, |bytes| {
            bytes.windows(3).any(|w| w == b"\x1b_G") && bytes.windows(2).any(|w| w == b"\x1b\\")
        });

        Ok(parse_kitty_graphics_response(&response))
    }

    fn query_sgr_pixel_mouse(&mut self, timeout: Duration) -> io::Result<Support> {
        self.io.drain_input()?;

        self.io.write_all(SGR_PIXEL_MOUSE_QUERY)?;
        let response = self.read_until(timeout, |bytes| {
            let text = String::from_utf8_lossy(bytes);
            text.contains("\x1b[?1016;") && text.contains("$y")
        });
        self.io.write_all(SGR_PIXEL_MOUSE_DISABLE)?;

        Ok(parse_decrpm_mode_response(&response, 1016))
    }

    fn read_until(&mut self, timeout: Duration, done: impl Fn(&[u8]) -> bool) -> Vec<u8> {
        let deadline = Instant::now() + timeout;
        let mut out = Vec::new();
        let mut buf = [0_u8; 1024];

        while Instant::now() < deadline {
            match self.io.read(&mut buf) {
                Ok(n @ 1..) => {
                    out.extend_from_slice(&buf[..n]);
                    if done(&out) {
                        break;
                    }
                }
                Ok(0) => std::thread::sleep(Duration::from_millis(5)),
                Err(error) => {
                    if error.kind() != io::ErrorKind::WouldBlock {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }

        out
    }
}

impl<I: CapabilityProbeIo> Drop for TerminalProbeSession<I> {
    fn drop(&mut self) {
        let _ = self.io.write_all(SGR_PIXEL_MOUSE_DISABLE);
        let _ = self.io.restore();
    }
}

fn parse_kitty_graphics_response(response: &[u8]) -> Support {
    let text = String::from_utf8_lossy(response);
    if !text.contains("\x1b_G") || !text.contains("i=31") {
        return Support::No;
    }

    if text.contains(";OK") || text.contains(",OK") || text.contains(";ok") || text.contains(",ok")
    {
        Support::Yes
    } else if text.contains("EINVAL") || text.contains("error") || text.contains("ERROR") {
        Support::No
    } else {
        Support::Yes
    }
}

fn parse_decrpm_mode_response(response: &[u8], mode: u16) -> Support {
    let text = String::from_utf8_lossy(response);
    let prefix = format!("\x1b[?{mode};");
    let Some(start) = text.find(&prefix) else {
        return Support::No;
    };
    let rest = &text[start + prefix.len()..];
    let Some(end) = rest.find("$y") else {
        return Support::Unknown;
    };
    let value = &rest[..end];

    match value.chars().next() {
        Some('1' | '3') => Support::Yes,
        Some('2' | '4') => Support::No,
        _ => Support::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;
    use std::rc::Rc;

    #[derive(Clone, Default)]
    struct ScriptedProbeIo {
        state: Rc<RefCell<ScriptedProbeState>>,
        restored: Rc<Cell<usize>>,
        drains: Rc<Cell<usize>>,
    }

    #[derive(Default)]
    struct ScriptedProbeState {
        reads: VecDeque<Vec<u8>>,
        writes: Vec<u8>,
    }

    impl ScriptedProbeIo {
        fn with_reads(reads: impl IntoIterator<Item = &'static [u8]>) -> Self {
            let io = Self::default();
            io.state.borrow_mut().reads = reads.into_iter().map(Vec::from).collect();
            io
        }

        fn writes(&self) -> Vec<u8> {
            self.state.borrow().writes.clone()
        }

        fn restores(&self) -> usize {
            self.restored.get()
        }

        fn drain_calls(&self) -> usize {
            self.drains.get()
        }
    }

    impl CapabilityProbeIo for ScriptedProbeIo {
        fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
            self.state.borrow_mut().writes.extend_from_slice(bytes);
            Ok(())
        }

        fn drain_input(&mut self) -> io::Result<()> {
            self.drains.set(self.drains.get() + 1);
            Ok(())
        }

        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let Some(chunk) = self.state.borrow_mut().reads.pop_front() else {
                return Ok(0);
            };
            let len = chunk.len().min(buf.len());
            buf[..len].copy_from_slice(&chunk[..len]);
            Ok(len)
        }

        fn restore(&mut self) -> io::Result<()> {
            self.restored.set(self.restored.get() + 1);
            Ok(())
        }
    }

    #[test]
    fn parses_positive_kitty_response() {
        assert_eq!(
            parse_kitty_graphics_response(b"\x1b_Gi=31;OK\x1b\\"),
            Support::Yes
        );
    }

    #[test]
    fn parses_missing_kitty_response_as_no() {
        assert_eq!(parse_kitty_graphics_response(b""), Support::No);
    }

    #[test]
    fn parses_decrpm_set_as_yes() {
        assert_eq!(
            parse_decrpm_mode_response(b"\x1b[?1016;1$y", 1016),
            Support::Yes
        );
    }

    #[test]
    fn parses_decrpm_reset_as_no() {
        assert_eq!(
            parse_decrpm_mode_response(b"\x1b[?1016;2$y", 1016),
            Support::No
        );
    }

    #[test]
    fn parses_absent_decrpm_as_no() {
        assert_eq!(parse_decrpm_mode_response(b"", 1016), Support::No);
    }

    #[test]
    fn kitty_done_predicate_matches_three_byte_marker() {
        let predicate = |bytes: &[u8]| {
            bytes.windows(3).any(|w| w == b"\x1b_G") && bytes.windows(2).any(|w| w == b"\x1b\\")
        };
        assert!(predicate(b"\x1b_Gi=31;OK\x1b\\"));
        assert!(!predicate(b"\x1b_G"));
        assert!(!predicate(b""));
    }

    #[test]
    fn merge_support_prefers_probed_decision() {
        assert_eq!(merge_support(Support::Yes, Support::No), Support::No);
        assert_eq!(merge_support(Support::No, Support::Yes), Support::Yes);
        assert_eq!(merge_support(Support::Yes, Support::Unknown), Support::Yes);
        assert_eq!(
            merge_support(Support::Unknown, Support::Unknown),
            Support::Unknown
        );
    }

    #[test]
    fn probe_session_queries_kitty_graphics_through_injected_io() {
        let io = ScriptedProbeIo::with_reads([b"\x1b_Gi=31;OK\x1b\\" as &'static [u8]]);

        {
            let mut session = TerminalProbeSession::with_io(io.clone());
            assert_eq!(
                session
                    .query_kitty_graphics(Duration::from_millis(50))
                    .unwrap(),
                Support::Yes
            );
            assert_eq!(io.writes(), KITTY_GRAPHICS_QUERY);
            assert_eq!(io.drain_calls(), 1);
        }

        let mut expected = Vec::from(KITTY_GRAPHICS_QUERY);
        expected.extend_from_slice(SGR_PIXEL_MOUSE_DISABLE);
        assert_eq!(io.writes(), expected);
        assert_eq!(io.restores(), 1);
    }

    #[test]
    fn probe_session_queries_pixel_mouse_and_disables_mode() {
        let io = ScriptedProbeIo::with_reads([b"\x1b[?1016;1$y" as &'static [u8]]);

        {
            let mut session = TerminalProbeSession::with_io(io.clone());
            assert_eq!(
                session
                    .query_sgr_pixel_mouse(Duration::from_millis(50))
                    .unwrap(),
                Support::Yes
            );
            let mut expected = Vec::from(SGR_PIXEL_MOUSE_QUERY);
            expected.extend_from_slice(SGR_PIXEL_MOUSE_DISABLE);
            assert_eq!(io.writes(), expected);
            assert_eq!(io.drain_calls(), 1);
        }

        let mut expected = Vec::from(SGR_PIXEL_MOUSE_QUERY);
        expected.extend_from_slice(SGR_PIXEL_MOUSE_DISABLE);
        expected.extend_from_slice(SGR_PIXEL_MOUSE_DISABLE);
        assert_eq!(io.writes(), expected);
        assert_eq!(io.restores(), 1);
    }
}

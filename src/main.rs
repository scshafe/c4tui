use clap::Parser;
use std::env;
use std::fmt;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Debug, Parser)]
#[command(author, version, about)]
struct Cli {
    /// Path to workspace.dsl, workspace.json, or a directory containing one.
    #[arg(long)]
    workspace: Option<PathBuf>,

    /// Path to the structurizr-cli executable.
    #[arg(long)]
    structurizr_cli: Option<PathBuf>,

    /// Timeout for terminal capability probes, in milliseconds.
    #[arg(long, default_value_t = 200)]
    capability_timeout_ms: u64,

    /// Print capability results even when stdout is not attached to a terminal.
    #[arg(long)]
    force_probe: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Support {
    Yes,
    No,
    Unknown,
}

impl fmt::Display for Support {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Support::Yes => f.write_str("yes"),
            Support::No => f.write_str("no"),
            Support::Unknown => f.write_str("unknown"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Capabilities {
    kitty_graphics: Support,
    pixel_mouse: Support,
    truecolor: Support,
}

fn main() {
    let cli = Cli::parse();
    let _workspace = cli.workspace.as_deref();
    let _structurizr_cli = cli.structurizr_cli.as_deref();
    let timeout = Duration::from_millis(cli.capability_timeout_ms);

    match detect_capabilities(timeout, cli.force_probe) {
        Ok(capabilities) => {
            println!("Kitty graphics: {}", capabilities.kitty_graphics);
            println!("Pixel mouse: {}", capabilities.pixel_mouse);
            println!("Truecolor: {}", capabilities.truecolor);

            if capabilities.kitty_graphics != Support::Yes {
                eprintln!(
                    "error: c4tui requires a terminal with Kitty graphics support (Kitty, WezTerm, or Ghostty)."
                );
                std::process::exit(1);
            }
        }
        Err(error) => {
            eprintln!("error: failed to detect terminal capabilities: {error}");
            std::process::exit(1);
        }
    }
}

fn detect_capabilities(timeout: Duration, force_probe: bool) -> io::Result<Capabilities> {
    let truecolor = detect_truecolor();

    if !force_probe && (!stdout_is_tty() || !stdin_is_tty()) {
        return Ok(Capabilities {
            kitty_graphics: Support::Unknown,
            pixel_mouse: Support::Unknown,
            truecolor,
        });
    }

    let mut session = TerminalProbeSession::new()?;
    let kitty_graphics = session.query_kitty_graphics(timeout)?;
    let pixel_mouse = session.query_sgr_pixel_mouse(timeout)?;

    Ok(Capabilities {
        kitty_graphics,
        pixel_mouse,
        truecolor,
    })
}

fn detect_truecolor() -> Support {
    let colorterm = env::var("COLORTERM")
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(colorterm.as_str(), "truecolor" | "24bit") {
        return Support::Yes;
    }

    let term = env::var("TERM").unwrap_or_default().to_ascii_lowercase();
    if term.contains("truecolor") || term.contains("24bit") {
        return Support::Yes;
    }

    match env::var("TERM_PROGRAM").unwrap_or_default().as_str() {
        "WezTerm" | "ghostty" | "Ghostty" | "iTerm.app" | "kitty" => Support::Yes,
        _ => Support::Unknown,
    }
}

struct TerminalProbeSession {
    stdin_fd: libc::c_int,
    original_termios: libc::termios,
    original_flags: libc::c_int,
}

impl TerminalProbeSession {
    fn new() -> io::Result<Self> {
        let stdin_fd = libc::STDIN_FILENO;
        let original_termios = get_termios(stdin_fd)?;
        let original_flags = get_fd_flags(stdin_fd)?;

        let mut raw = original_termios;
        make_raw(&mut raw);
        set_termios(stdin_fd, &raw)?;
        set_fd_flags(stdin_fd, original_flags | libc::O_NONBLOCK)?;

        Ok(Self {
            stdin_fd,
            original_termios,
            original_flags,
        })
    }

    fn query_kitty_graphics(&mut self, timeout: Duration) -> io::Result<Support> {
        self.drain_input();

        // Kitty graphics protocol query: ask whether a tiny direct image transfer would be accepted.
        // Terminals that implement the protocol answer with an APC response containing `i=31`.
        write_stdout_all(b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\")?;
        let response = self.read_until(timeout, |bytes| bytes.windows(4).any(|w| w == b"\x1b_G"));

        Ok(parse_kitty_graphics_response(&response))
    }

    fn query_sgr_pixel_mouse(&mut self, timeout: Duration) -> io::Result<Support> {
        self.drain_input();

        // Enable SGR pixel mouse mode, then request its DEC private mode state.
        write_stdout_all(b"\x1b[?1016h\x1b[?1016$p")?;
        let response = self.read_until(timeout, |bytes| {
            let text = String::from_utf8_lossy(bytes);
            text.contains("\x1b[?1016;") && text.contains("$y")
        });
        write_stdout_all(b"\x1b[?1016l")?;

        Ok(parse_decrpm_mode_response(&response, 1016))
    }

    fn drain_input(&mut self) {
        let mut buf = [0_u8; 1024];
        loop {
            match unsafe { libc::read(self.stdin_fd, buf.as_mut_ptr().cast(), buf.len()) } {
                n if n > 0 => continue,
                _ => break,
            }
        }
    }

    fn read_until(&mut self, timeout: Duration, done: impl Fn(&[u8]) -> bool) -> Vec<u8> {
        let deadline = Instant::now() + timeout;
        let mut out = Vec::new();
        let mut buf = [0_u8; 1024];

        while Instant::now() < deadline {
            match unsafe { libc::read(self.stdin_fd, buf.as_mut_ptr().cast(), buf.len()) } {
                n if n > 0 => {
                    out.extend_from_slice(&buf[..n as usize]);
                    if done(&out) {
                        break;
                    }
                }
                -1 => {
                    let err = io::Error::last_os_error();
                    if err.kind() != io::ErrorKind::WouldBlock {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                _ => break,
            }
        }

        out
    }
}

impl Drop for TerminalProbeSession {
    fn drop(&mut self) {
        let _ = write_stdout_all(b"\x1b[?1016l");
        let _ = set_fd_flags(self.stdin_fd, self.original_flags);
        let _ = set_termios(self.stdin_fd, &self.original_termios);
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
        // A structured graphics response with the requested image id is enough to prove protocol support.
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
        Some('1') | Some('3') => Support::Yes,
        Some('2') | Some('4') => Support::No,
        _ => Support::Unknown,
    }
}

fn write_stdout_all(bytes: &[u8]) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    stdout.write_all(bytes)?;
    stdout.flush()
}

fn stdin_is_tty() -> bool {
    unsafe { libc::isatty(libc::STDIN_FILENO) == 1 }
}

fn stdout_is_tty() -> bool {
    unsafe { libc::isatty(libc::STDOUT_FILENO) == 1 }
}

fn get_termios(fd: libc::c_int) -> io::Result<libc::termios> {
    let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
    if unsafe { libc::tcgetattr(fd, termios.as_mut_ptr()) } == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { termios.assume_init() })
    }
}

fn set_termios(fd: libc::c_int, termios: &libc::termios) -> io::Result<()> {
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, termios) } == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn get_fd_flags(fd: libc::c_int) -> io::Result<libc::c_int> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(flags)
    }
}

fn set_fd_flags(fd: libc::c_int, flags: libc::c_int) -> io::Result<()> {
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags) } == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn make_raw(termios: &mut libc::termios) {
    termios.c_iflag &=
        !(libc::BRKINT | libc::ICRNL | libc::INPCK | libc::ISTRIP | libc::IXON | libc::PARMRK);
    termios.c_oflag &= !libc::OPOST;
    termios.c_cflag |= libc::CS8;
    termios.c_lflag &= !(libc::ECHO | libc::ICANON | libc::IEXTEN | libc::ISIG);
    termios.c_cc[libc::VMIN] = 0;
    termios.c_cc[libc::VTIME] = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

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
}

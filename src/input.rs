use anyhow::Result;
use std::io;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Key {
    Char(char),
    Up,
    Down,
    Left,
    Right,
    Enter,
    Esc,
    CtrlC,
    MouseWheelUp { x: u16, y: u16 },
    MouseWheelDown { x: u16, y: u16 },
    MouseDrag { x: u16, y: u16 },
    MouseRelease,
    Unknown,
}

pub fn read_key() -> Result<Key> {
    let byte = read_stdin_byte_blocking()?;

    Ok(match byte {
        b'\r' | b'\n' => Key::Enter,
        0x03 => Key::CtrlC,
        0x1b => parse_escape_sequence()?,
        byte if byte.is_ascii() && !byte.is_ascii_control() => Key::Char(byte as char),
        _ => Key::Unknown,
    })
}

fn read_stdin_byte_blocking() -> io::Result<u8> {
    loop {
        let mut byte = [0_u8; 1];
        match unsafe { libc::read(libc::STDIN_FILENO, byte.as_mut_ptr().cast(), 1) } {
            1 => return Ok(byte[0]),
            -1 => {
                let err = io::Error::last_os_error();
                if err.kind() != io::ErrorKind::WouldBlock {
                    return Err(err);
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            _ => std::thread::sleep(Duration::from_millis(2)),
        }
    }
}

fn parse_escape_sequence() -> io::Result<Key> {
    let mut bytes = Vec::new();
    let deadline = Instant::now() + Duration::from_millis(25);
    while Instant::now() < deadline {
        let mut byte = [0_u8; 1];
        match unsafe { libc::read(libc::STDIN_FILENO, byte.as_mut_ptr().cast(), 1) } {
            1 => {
                bytes.push(byte[0]);
                if matches!(byte[0], b'A' | b'B' | b'C' | b'D' | b'M' | b'm') {
                    break;
                }
            }
            -1 => {
                let err = io::Error::last_os_error();
                if err.kind() != io::ErrorKind::WouldBlock {
                    return Err(err);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            _ => std::thread::sleep(Duration::from_millis(1)),
        }
    }

    Ok(parse_escape_bytes(&bytes))
}

fn parse_escape_bytes(bytes: &[u8]) -> Key {
    match bytes {
        [b'[', b'A'] => Key::Up,
        [b'[', b'B'] => Key::Down,
        [b'[', b'C'] => Key::Right,
        [b'[', b'D'] => Key::Left,
        _ => parse_sgr_mouse(bytes).unwrap_or(Key::Esc),
    }
}

fn parse_sgr_mouse(bytes: &[u8]) -> Option<Key> {
    let text = std::str::from_utf8(bytes).ok()?;
    if !text.starts_with("[<") || !(text.ends_with('M') || text.ends_with('m')) {
        return None;
    }
    let released = text.ends_with('m');
    let body = &text[2..text.len() - 1];
    let mut parts = body.split(';');
    let code = parts.next()?.parse::<u16>().ok()?;
    let x = parts.next()?.parse::<u16>().ok()?;
    let y = parts.next()?.parse::<u16>().ok()?;

    if released {
        return Some(Key::MouseRelease);
    }

    match code {
        64 => Some(Key::MouseWheelUp { x, y }),
        65 => Some(Key::MouseWheelDown { x, y }),
        32..=63 => Some(Key::MouseDrag { x, y }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sgr_mouse_wheel_and_drag() {
        assert_eq!(
            parse_sgr_mouse(b"[<64;10;20M"),
            Some(Key::MouseWheelUp { x: 10, y: 20 })
        );
        assert_eq!(
            parse_sgr_mouse(b"[<65;10;20M"),
            Some(Key::MouseWheelDown { x: 10, y: 20 })
        );
        assert_eq!(
            parse_sgr_mouse(b"[<32;12;24M"),
            Some(Key::MouseDrag { x: 12, y: 24 })
        );
        assert_eq!(parse_sgr_mouse(b"[<0;12;24m"), Some(Key::MouseRelease));
    }
}

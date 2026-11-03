#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Key {
    Char(char),
    Ctrl(char),
    Esc,
    Enter,
    Backspace,
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Home,
    End,
    WheelUp,
    WheelDown,
}

pub fn parse(bytes: &[u8]) -> Vec<Key> {
    let mut keys = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            0x1b => i += parse_escape(&bytes[i..], &mut keys),
            b'\r' | b'\n' => {
                keys.push(Key::Enter);
                i += 1;
            }
            0x7f | 0x08 => {
                keys.push(Key::Backspace);
                i += 1;
            }
            control @ 1..=26 => {
                keys.push(Key::Ctrl((b'a' + control - 1) as char));
                i += 1;
            }
            byte if byte < 0x20 => i += 1,
            _ => i += parse_text(&bytes[i..], &mut keys),
        }
    }
    keys
}

fn parse_text(bytes: &[u8], keys: &mut Vec<Key>) -> usize {
    let len = match bytes[0] {
        0xf0..=0xf7 => 4,
        0xe0..=0xef => 3,
        0xc0..=0xdf => 2,
        _ => 1,
    };
    let end = len.min(bytes.len());
    if let Some(character) = std::str::from_utf8(&bytes[..end]).ok().and_then(|text| text.chars().next()) {
        keys.push(Key::Char(character));
    }
    end
}

fn parse_escape(bytes: &[u8], keys: &mut Vec<Key>) -> usize {
    match bytes.get(1) {
        Some(b'[') => {
            let Some(length) = bytes[2..].iter().position(|byte| (0x40..=0x7e).contains(byte)) else {
                return bytes.len();
            };
            let end = 2 + length;
            if let Some(key) = parse_csi(&bytes[2..end], bytes[end]) {
                keys.push(key);
            }
            end + 1
        }
        Some(b'O') => {
            if let Some(key) = bytes.get(2).and_then(|&last| parse_csi(b"", last)) {
                keys.push(key);
            }
            3.min(bytes.len())
        }
        _ => {
            keys.push(Key::Esc);
            1
        }
    }
}

fn parse_csi(parameters: &[u8], last: u8) -> Option<Key> {
    if let Some(mouse) = parameters.strip_prefix(b"<") {
        let button = mouse.split(|&byte| byte == b';').next()?;
        return match (button, last) {
            (b"64", b'M') => Some(Key::WheelUp),
            (b"65", b'M') => Some(Key::WheelDown),
            _ => None,
        };
    }
    match (parameters, last) {
        (_, b'A') => Some(Key::Up),
        (_, b'B') => Some(Key::Down),
        (_, b'C') => Some(Key::Right),
        (_, b'D') => Some(Key::Left),
        (_, b'H') | (b"1" | b"7", b'~') => Some(Key::Home),
        (_, b'F') | (b"4" | b"8", b'~') => Some(Key::End),
        (b"5", b'~') => Some(Key::PageUp),
        (b"6", b'~') => Some(Key::PageDown),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_and_shifted_letters() {
        assert_eq!(parse(b"jJ:"), [Key::Char('j'), Key::Char('J'), Key::Char(':')]);
    }

    #[test]
    fn control_keys() {
        assert_eq!(parse(b"\x04\x15"), [Key::Ctrl('d'), Key::Ctrl('u')]);
        assert_eq!(parse(b"\r\x7f"), [Key::Enter, Key::Backspace]);
    }

    #[test]
    fn lone_escape_is_esc() {
        assert_eq!(parse(b"\x1b"), [Key::Esc]);
        assert_eq!(parse(b"\x1bq"), [Key::Esc, Key::Char('q')]);
    }

    #[test]
    fn arrows_and_paging_in_both_encodings() {
        assert_eq!(parse(b"\x1b[A\x1bOB"), [Key::Up, Key::Down]);
        assert_eq!(parse(b"\x1b[5~\x1b[6~"), [Key::PageUp, Key::PageDown]);
        assert_eq!(parse(b"\x1b[H\x1b[4~"), [Key::Home, Key::End]);
    }

    #[test]
    fn wheel_is_reported_and_other_mouse_events_are_ignored() {
        assert_eq!(parse(b"\x1b[<64;10;10M\x1b[<65;10;10M"), [Key::WheelUp, Key::WheelDown]);
        assert_eq!(parse(b"\x1b[<0;10;10M\x1b[<0;10;10m"), []);
    }

    #[test]
    fn utf8_search_text_survives() {
        assert_eq!(parse("é∂".as_bytes()), [Key::Char('é'), Key::Char('∂')]);
    }

    #[test]
    fn truncated_sequences_do_not_panic() {
        assert_eq!(parse(b"\x1b["), []);
        assert_eq!(parse(b"\x1bO"), []);
        assert_eq!(parse(&[0xe2, 0x88]), []);
    }
}

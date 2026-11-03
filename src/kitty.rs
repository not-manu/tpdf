use std::ffi::{CStr, CString};
use std::io;
use std::io::Write;
use std::{process, ptr, slice};

const PLACEHOLDER: char = '\u{10EEEE}';
const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const DIACRITICS: [char; 64] = [
    '\u{0305}', '\u{030D}', '\u{030E}', '\u{0310}', '\u{0312}', '\u{033D}', '\u{033E}', '\u{033F}',
    '\u{0346}', '\u{034A}', '\u{034B}', '\u{034C}', '\u{0350}', '\u{0351}', '\u{0352}', '\u{0357}',
    '\u{035B}', '\u{0363}', '\u{0364}', '\u{0365}', '\u{0366}', '\u{0367}', '\u{0368}', '\u{0369}',
    '\u{036A}', '\u{036B}', '\u{036C}', '\u{036D}', '\u{036E}', '\u{036F}', '\u{0483}', '\u{0484}',
    '\u{0485}', '\u{0486}', '\u{0487}', '\u{0592}', '\u{0593}', '\u{0594}', '\u{0595}', '\u{0597}',
    '\u{0598}', '\u{0599}', '\u{059C}', '\u{059D}', '\u{059E}', '\u{059F}', '\u{05A0}', '\u{05A1}',
    '\u{05A8}', '\u{05A9}', '\u{05AB}', '\u{05AC}', '\u{05AF}', '\u{05C4}', '\u{0610}', '\u{0611}',
    '\u{0612}', '\u{0613}', '\u{0614}', '\u{0615}', '\u{0616}', '\u{0617}', '\u{0657}', '\u{0658}',
];

pub const MAX_GRID: i64 = DIACRITICS.len() as i64;

pub struct Image<'a> {
    pub id: u8,
    pub width: i32,
    pub height: i32,
    pub cols: i64,
    pub rows: i64,
    pub name: &'a CStr,
}

pub struct Graphics {
    tmux: bool,
    serial: u32,
}

pub struct Shared {
    name: CString,
    pointer: *mut u8,
    len: usize,
}

impl Graphics {
    pub fn new(tmux: bool) -> Graphics {
        Graphics { tmux, serial: 0 }
    }

    pub fn shared(&mut self, len: usize) -> io::Result<Shared> {
        self.serial += 1;
        let name = format!("/tpdf.{}.{}", process::id(), self.serial);
        Shared::create(CString::new(name).expect("shared memory name has no NUL"), len)
    }

    pub fn transmit(&self, out: &mut Vec<u8>, image: &Image) {
        let mut command = Vec::with_capacity(128);
        let bytes = image.width as usize * image.height as usize * 3;
        write!(
            command,
            "\x1b_Ga=T,U=1,q=2,f=24,t=s,i={},s={},v={},c={},r={},S={};",
            image.id, image.width, image.height, image.cols, image.rows, bytes
        )
        .expect("writing to a Vec cannot fail");
        base64(&mut command, image.name.to_bytes());
        command.extend_from_slice(b"\x1b\\");
        self.send(out, &command);
    }

    pub fn delete(&self, out: &mut Vec<u8>, id: u8) {
        let command = format!("\x1b_Ga=d,d=I,q=2,i={id}\x1b\\");
        self.send(out, command.as_bytes());
    }

    pub fn prevent_discard(&self, out: &mut Vec<u8>) {
        if self.tmux {
            self.send(out, b"");
        }
    }

    fn send(&self, out: &mut Vec<u8>, command: &[u8]) {
        if !self.tmux {
            out.extend_from_slice(command);
            return;
        }
        out.extend_from_slice(b"\x1bPtmux;");
        for &byte in command {
            if byte == 0x1b {
                out.push(0x1b);
            }
            out.push(byte);
        }
        out.extend_from_slice(b"\x1b\\");
    }
}

impl Shared {
    fn create(name: CString, len: usize) -> io::Result<Shared> {
        let flags = libc::O_CREAT | libc::O_EXCL | libc::O_RDWR;
        unsafe {
            let fd = libc::shm_open(name.as_ptr(), flags, 0o600 as libc::c_uint);
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            let mapped = if libc::ftruncate(fd, len as libc::off_t) == 0 {
                libc::mmap(
                    ptr::null_mut(),
                    len,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_SHARED,
                    fd,
                    0,
                )
            } else {
                libc::MAP_FAILED
            };
            let error = io::Error::last_os_error();
            libc::close(fd);
            if mapped == libc::MAP_FAILED {
                libc::shm_unlink(name.as_ptr());
                return Err(error);
            }
            Ok(Shared { name, pointer: mapped.cast(), len })
        }
    }

    pub fn pixels(&mut self) -> &mut [u8] {
        unsafe { slice::from_raw_parts_mut(self.pointer, self.len) }
    }

    pub fn into_name(self) -> CString {
        self.name.clone()
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.pointer.cast(), self.len);
        }
    }
}

pub fn unlink(name: &CStr) {
    unsafe {
        libc::shm_unlink(name.as_ptr());
    }
}

pub fn cell(out: &mut Vec<u8>, row: i64, col: i64) {
    let mut buffer = [0u8; 4];
    for character in [PLACEHOLDER, DIACRITICS[row as usize], DIACRITICS[col as usize]] {
        out.extend_from_slice(character.encode_utf8(&mut buffer).as_bytes());
    }
}

fn base64(out: &mut Vec<u8>, data: &[u8]) {
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = *chunk.get(1).unwrap_or(&0) as usize;
        let b2 = *chunk.get(2).unwrap_or(&0) as usize;
        out.push(BASE64[b0 >> 2]);
        out.push(BASE64[((b0 & 0x03) << 4) | (b1 >> 4)]);
        out.push(if chunk.len() > 1 { BASE64[((b1 & 0x0f) << 2) | (b2 >> 6)] } else { b'=' });
        out.push(if chunk.len() > 2 { BASE64[b2 & 0x3f] } else { b'=' });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(data: &[u8]) -> String {
        let mut out = Vec::new();
        base64(&mut out, data);
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(encoded(b"f"), "Zg==");
        assert_eq!(encoded(b"fo"), "Zm8=");
        assert_eq!(encoded(b"foo"), "Zm9v");
        assert_eq!(encoded(b"/tpdf.1.1"), "L3RwZGYuMS4x");
    }

    #[test]
    fn cell_is_placeholder_then_row_then_column() {
        let mut out = Vec::new();
        cell(&mut out, 0, 1);
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text.chars().collect::<Vec<_>>(), ['\u{10EEEE}', '\u{0305}', '\u{030D}']);
    }

    #[test]
    fn tmux_passthrough_doubles_every_escape() {
        let graphics = Graphics::new(true);
        let mut out = Vec::new();
        graphics.delete(&mut out, 16);
        let expected = b"\x1bPtmux;\x1b\x1b_Ga=d,d=I,q=2,i=16\x1b\x1b\\\x1b\\";
        assert_eq!(out, expected);
    }

    #[test]
    fn prevent_discard_is_an_empty_passthrough_only_in_tmux() {
        let mut out = Vec::new();
        Graphics::new(true).prevent_discard(&mut out);
        assert_eq!(out, b"\x1bPtmux;\x1b\\");

        let mut out = Vec::new();
        Graphics::new(false).prevent_discard(&mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn plain_terminal_gets_the_command_unwrapped() {
        let graphics = Graphics::new(false);
        let mut out = Vec::new();
        graphics.delete(&mut out, 16);
        assert_eq!(out, b"\x1b_Ga=d,d=I,q=2,i=16\x1b\\");
    }
}

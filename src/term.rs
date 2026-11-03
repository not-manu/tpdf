use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{env, mem, panic};

pub static RESIZED: AtomicBool = AtomicBool::new(false);
pub static STOP: AtomicBool = AtomicBool::new(false);
static SAVED: OnceLock<libc::termios> = OnceLock::new();

const ENTER: &[u8] = b"\x1b[?1049h\x1b[?25l\x1b[?1000h\x1b[?1006h\x1b[2J";
const LEAVE: &[u8] = b"\x1b[0m\x1b[?1006l\x1b[?1000l\x1b[?25h\x1b[?1049l";
const FALLBACK_CELL: (i64, i64) = (10, 20);

#[derive(Clone, Copy, PartialEq)]
pub struct Size {
    pub cols: i64,
    pub rows: i64,
    pub cell: (i64, i64),
}

pub struct Guard;

extern "C" fn on_resize(_: libc::c_int) {
    RESIZED.store(true, Ordering::Relaxed);
}

extern "C" fn on_stop(_: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}

fn handle(signal: libc::c_int, handler: extern "C" fn(libc::c_int)) {
    unsafe {
        let mut action: libc::sigaction = mem::zeroed();
        action.sa_sigaction = handler as *const () as usize;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(signal, &action, std::ptr::null_mut());
    }
}

fn restore() {
    write_all(LEAVE);
    if let Some(saved) = SAVED.get() {
        unsafe {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, saved);
        }
    }
}

pub fn enter() -> Result<Guard, String> {
    unsafe {
        if libc::isatty(libc::STDIN_FILENO) == 0 || libc::isatty(libc::STDOUT_FILENO) == 0 {
            return Err("tpdf must run in a terminal".to_string());
        }
        let mut raw: libc::termios = mem::zeroed();
        if libc::tcgetattr(libc::STDIN_FILENO, &mut raw) != 0 {
            return Err("could not read terminal settings".to_string());
        }
        SAVED.set(raw).ok();
        libc::cfmakeraw(&mut raw);
        if libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, &raw) != 0 {
            return Err("could not set terminal to raw mode".to_string());
        }
    }
    handle(libc::SIGWINCH, on_resize);
    handle(libc::SIGTERM, on_stop);
    handle(libc::SIGHUP, on_stop);
    handle(libc::SIGINT, on_stop);

    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        restore();
        default_hook(info);
    }));

    write_all(ENTER);
    Ok(Guard)
}

impl Drop for Guard {
    fn drop(&mut self) {
        restore();
    }
}

pub fn in_tmux() -> bool {
    env::var_os("TMUX").is_some_and(|value| !value.is_empty())
}

pub fn allow_passthrough() {
    let Ok(pane) = env::var("TMUX_PANE") else {
        return;
    };
    let _ = Command::new("tmux")
        .args(["set-option", "-p", "-t", &pane, "allow-passthrough", "all"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn tmux_format(format: &str) -> Option<String> {
    if !in_tmux() {
        return None;
    }
    let output = Command::new("tmux")
        .args(["display-message", "-p", format])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    String::from_utf8(output.stdout).ok()
}

pub fn tmux_discarded() -> u64 {
    tmux_format("#{client_discarded}")
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(0)
}

fn tmux_cell() -> Option<(i64, i64)> {
    let text = tmux_format("#{window_cell_width} #{window_cell_height}")?;
    let mut values = text.split_whitespace().map(str::parse::<i64>);
    let cell = (values.next()?.ok()?, values.next()?.ok()?);
    (cell.0 > 0 && cell.1 > 0).then_some(cell)
}

pub fn size() -> Size {
    let mut window: libc::winsize = unsafe { mem::zeroed() };
    unsafe {
        libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut window);
    }
    let cols = (window.ws_col as i64).max(1);
    let rows = (window.ws_row as i64).max(1);
    let (x_pixels, y_pixels) = (window.ws_xpixel as i64, window.ws_ypixel as i64);
    let cell = if x_pixels >= cols && y_pixels >= rows {
        (x_pixels / cols, y_pixels / rows)
    } else {
        tmux_cell().unwrap_or(FALLBACK_CELL)
    };
    Size { cols, rows, cell }
}

pub fn write_all(mut bytes: &[u8]) {
    while !bytes.is_empty() {
        let written =
            unsafe { libc::write(libc::STDOUT_FILENO, bytes.as_ptr().cast(), bytes.len()) };
        if written > 0 {
            bytes = &bytes[written as usize..];
        } else if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            return;
        }
    }
}

pub fn readable(timeout_ms: i32) -> bool {
    let mut descriptor = libc::pollfd { fd: libc::STDIN_FILENO, events: libc::POLLIN, revents: 0 };
    unsafe { libc::poll(&mut descriptor, 1, timeout_ms) > 0 }
}

pub fn read(buffer: &mut [u8]) -> Option<usize> {
    let count = unsafe { libc::read(libc::STDIN_FILENO, buffer.as_mut_ptr().cast(), buffer.len()) };
    if count > 0 {
        return Some(count as usize);
    }
    if count < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
        return Some(0);
    }
    None
}

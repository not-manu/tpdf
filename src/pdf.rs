use std::ffi::{CStr, CString, c_char, c_int};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

const STORE_BYTES: usize = 32 << 20;
const MAX_HITS: usize = 128;
const TEXT_ON_DARK: c_int = 0xCECDC3;
const PAPER_ON_DARK: c_int = 0x100F0F;

#[repr(C)]
struct Handle {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn tpdf_open(
        path: *const c_char,
        store: usize,
        error: *mut c_char,
        error_size: usize,
    ) -> *mut Handle;
    fn tpdf_close(handle: *mut Handle);
    fn tpdf_pages(handle: *mut Handle) -> c_int;
    fn tpdf_bounds(handle: *mut Handle, index: c_int, width: *mut f32, height: *mut f32) -> c_int;
    fn tpdf_render(
        handle: *mut Handle,
        index: c_int,
        scale: f32,
        x: c_int,
        y: c_int,
        width: c_int,
        height: c_int,
        rgb: *mut u8,
        dark: c_int,
        black: c_int,
        white: c_int,
    ) -> c_int;
    fn tpdf_search(
        handle: *mut Handle,
        index: c_int,
        needle: *const c_char,
        out: *mut f32,
        max: c_int,
    ) -> c_int;
}

pub struct Doc {
    handle: *mut Handle,
}

pub struct Tile {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Doc {
    pub fn open(path: &Path) -> Result<Doc, String> {
        let c_path = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| "path contains a NUL byte".to_string())?;
        let mut error = [0 as c_char; 256];
        let handle =
            unsafe { tpdf_open(c_path.as_ptr(), STORE_BYTES, error.as_mut_ptr(), error.len()) };
        if handle.is_null() {
            let message = unsafe { CStr::from_ptr(error.as_ptr()) };
            return Err(message.to_string_lossy().into_owned());
        }
        Ok(Doc { handle })
    }

    pub fn pages(&self) -> usize {
        unsafe { tpdf_pages(self.handle) as usize }
    }

    pub fn bounds(&self, index: usize) -> Option<(f32, f32)> {
        let (mut width, mut height) = (0.0, 0.0);
        let ok = unsafe { tpdf_bounds(self.handle, index as c_int, &mut width, &mut height) };
        (ok != 0 && width > 0.0 && height > 0.0).then_some((width, height))
    }

    pub fn render(&self, index: usize, scale: f32, tile: &Tile, dark: bool, rgb: &mut [u8]) -> bool {
        assert_eq!(rgb.len(), tile.width as usize * tile.height as usize * 3);
        let ok = unsafe {
            tpdf_render(
                self.handle,
                index as c_int,
                scale,
                tile.x,
                tile.y,
                tile.width,
                tile.height,
                rgb.as_mut_ptr(),
                dark as c_int,
                TEXT_ON_DARK,
                PAPER_ON_DARK,
            )
        };
        ok != 0
    }

    pub fn search(&self, index: usize, needle: &str) -> Vec<[f32; 4]> {
        let Ok(c_needle) = CString::new(needle) else {
            return Vec::new();
        };
        let mut raw = vec![0f32; MAX_HITS * 4];
        let count = unsafe {
            tpdf_search(
                self.handle,
                index as c_int,
                c_needle.as_ptr(),
                raw.as_mut_ptr(),
                MAX_HITS as c_int,
            )
        };
        raw.chunks_exact(4)
            .take(count.max(0) as usize)
            .map(|hit| [hit[0], hit[1], hit[2], hit[3]])
            .collect()
    }
}

impl Drop for Doc {
    fn drop(&mut self) {
        unsafe { tpdf_close(self.handle) }
    }
}

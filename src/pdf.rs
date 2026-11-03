use std::ffi::{CStr, CString, c_char, c_int};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

const STORE_BYTES: usize = 32 << 20;
const MAX_HITS: usize = 128;
const LABEL_BYTES: usize = 64;
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
    fn tpdf_label(handle: *mut Handle, index: c_int, label: *mut c_char, size: c_int) -> c_int;
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
    fn tpdf_text(handle: *mut Handle, index: c_int, x0: f32, y0: f32, x1: f32, y1: f32) -> *mut c_char;
    fn tpdf_free_text(handle: *mut Handle, text: *mut c_char);
    fn tpdf_save_png(
        handle: *mut Handle,
        path: *const c_char,
        width: c_int,
        height: c_int,
        rgb: *mut u8,
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

    fn label(&self, index: usize) -> Option<String> {
        let mut label = [0 as c_char; LABEL_BYTES];
        let ok = unsafe { tpdf_label(self.handle, index as c_int, label.as_mut_ptr(), LABEL_BYTES as c_int) };
        (ok != 0).then(|| unsafe { CStr::from_ptr(label.as_ptr()) }.to_string_lossy().into_owned())
    }

    pub fn find_page(&self, target: &str) -> Option<usize> {
        match target.strip_prefix('p') {
            Some(label) => (0..self.pages()).find(|&index| self.label(index).as_deref() == Some(label)),
            None => target.parse::<usize>().ok().map(|page| page.saturating_sub(1)),
        }
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

    pub fn text(&self, index: usize, area: [f32; 4]) -> String {
        unsafe {
            let raw = tpdf_text(self.handle, index as c_int, area[0], area[1], area[2], area[3]);
            if raw.is_null() {
                return String::new();
            }
            let text = CStr::from_ptr(raw).to_string_lossy().into_owned();
            tpdf_free_text(self.handle, raw);
            text
        }
    }

    pub fn save_png(&self, path: &Path, width: usize, height: usize, rgb: &mut [u8]) -> bool {
        assert_eq!(rgb.len(), width * height * 3);
        let Ok(c_path) = CString::new(path.as_os_str().as_bytes()) else {
            return false;
        };
        let ok = unsafe {
            tpdf_save_png(
                self.handle,
                c_path.as_ptr(),
                width as c_int,
                height as c_int,
                rgb.as_mut_ptr(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const PAGE: &str = "<</Type/Page/Parent 2 0 R/MediaBox[0 0 100 100]>>";

    fn open(name: &str, labels: &str) -> Doc {
        let pdf = format!(
            "%PDF-1.7\n1 0 obj\n<</Type/Catalog/Pages 2 0 R{labels}>>\nendobj\n2 0 obj\n<</Type/Pages/Count 3/Kids[3 0 R 4 0 R 5 0 R]>>\nendobj\n3 0 obj\n{PAGE}\nendobj\n4 0 obj\n{PAGE}\nendobj\n5 0 obj\n{PAGE}\nendobj\ntrailer\n<</Root 1 0 R>>\n"
        );
        let path = Path::new(env!("OUT_DIR")).join(name);
        fs::write(&path, pdf).unwrap();
        Doc::open(&path).unwrap()
    }

    #[test]
    fn p_finds_a_page_by_its_label_and_a_bare_number_by_its_position() {
        let doc = open("labeled.pdf", "/PageLabels<</Nums[0<</S/r>>2<</S/D>>]>>");
        assert_eq!(doc.find_page("p1"), Some(2));
        assert_eq!(doc.find_page("pii"), Some(1));
        assert_eq!(doc.find_page("1"), Some(0));
        assert_eq!(doc.find_page("0"), Some(0));
    }

    #[test]
    fn a_label_or_target_that_does_not_exist_finds_nothing() {
        let doc = open("missing.pdf", "/PageLabels<</Nums[0<</S/r>>2<</S/D>>]>>");
        assert_eq!(doc.find_page("p2"), None);
        assert_eq!(doc.find_page("p"), None);
        assert_eq!(doc.find_page("nope"), None);
    }

    #[test]
    fn without_labels_p_counts_from_one() {
        let doc = open("plain.pdf", "");
        assert_eq!(doc.find_page("p3"), Some(2));
    }
}

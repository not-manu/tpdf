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
    fn tpdf_link_at(
        handle: *mut Handle,
        index: c_int,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        uri: *mut *mut c_char,
        target: *mut c_int,
        target_y: *mut f32,
    );
    fn tpdf_fold(handle: *mut Handle, text: *const c_char) -> *mut c_char;
    fn tpdf_folded_page(handle: *mut Handle, index: c_int) -> *mut c_char;
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

#[derive(Debug, PartialEq)]
pub enum Link {
    Internal { page: usize, y: f32 },
    External(String),
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

    pub fn label(&self, index: usize) -> Option<String> {
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

    fn take_text(&self, raw: *mut c_char) -> String {
        if raw.is_null() {
            return String::new();
        }
        unsafe {
            let text = CStr::from_ptr(raw).to_string_lossy().into_owned();
            tpdf_free_text(self.handle, raw);
            text
        }
    }

    pub fn text(&self, index: usize, area: [f32; 4]) -> String {
        self.take_text(unsafe {
            tpdf_text(self.handle, index as c_int, area[0], area[1], area[2], area[3])
        })
    }

    pub fn fold(&self, text: &str) -> String {
        let Ok(c_text) = CString::new(text) else {
            return String::new();
        };
        self.take_text(unsafe { tpdf_fold(self.handle, c_text.as_ptr()) })
    }

    pub fn folded_page(&self, index: usize) -> String {
        self.take_text(unsafe { tpdf_folded_page(self.handle, index as c_int) })
    }

    pub fn link_at(&self, index: usize, area: [f32; 4]) -> Option<Link> {
        let mut uri = std::ptr::null_mut();
        let (mut page, mut y) = (-1, 0.0);
        unsafe {
            tpdf_link_at(
                self.handle,
                index as c_int,
                area[0],
                area[1],
                area[2],
                area[3],
                &mut uri,
                &mut page,
                &mut y,
            );
            if !uri.is_null() {
                let text = CStr::from_ptr(uri).to_string_lossy().into_owned();
                tpdf_free_text(self.handle, uri);
                return Some(Link::External(text));
            }
        }
        (page >= 0).then_some(Link::Internal { page: page as usize, y })
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
    const LINKS: &str = "<</Type/Page/Parent 2 0 R/MediaBox[0 0 100 100]/Annots[\
        <</Type/Annot/Subtype/Link/Rect[0 80 40 100]/A<</S/URI/URI(https://example.com/a)>>>>\
        <</Type/Annot/Subtype/Link/Rect[60 80 100 100]/Dest[4 0 R/XYZ 0 75 0]>>\
        <</Type/Annot/Subtype/Link/Rect[60 0 100 20]/Dest[5 0 R/Fit]>>\
        ]>>";

    fn open_with(name: &str, labels: &str, first_page: &str) -> Doc {
        let pdf = format!(
            "%PDF-1.7\n1 0 obj\n<</Type/Catalog/Pages 2 0 R{labels}>>\nendobj\n2 0 obj\n<</Type/Pages/Count 3/Kids[3 0 R 4 0 R 5 0 R]>>\nendobj\n3 0 obj\n{first_page}\nendobj\n4 0 obj\n{PAGE}\nendobj\n5 0 obj\n{PAGE}\nendobj\ntrailer\n<</Root 1 0 R>>\n"
        );
        let path = Path::new(env!("OUT_DIR")).join(name);
        fs::write(&path, pdf).unwrap();
        Doc::open(&path).unwrap()
    }

    fn open(name: &str, labels: &str) -> Doc {
        open_with(name, labels, PAGE)
    }

    #[test]
    fn a_web_link_comes_back_as_its_address() {
        let doc = open_with("web-link.pdf", "", LINKS);
        assert_eq!(
            doc.link_at(0, [10.0, 5.0, 12.0, 7.0]),
            Some(Link::External("https://example.com/a".to_string()))
        );
    }

    #[test]
    fn an_internal_link_gives_the_page_and_height_it_lands_on() {
        let doc = open_with("internal-link.pdf", "", LINKS);
        assert_eq!(
            doc.link_at(0, [70.0, 5.0, 72.0, 7.0]),
            Some(Link::Internal { page: 1, y: 25.0 })
        );
    }

    #[test]
    fn an_internal_link_with_no_height_lands_at_the_top() {
        let doc = open_with("fit-link.pdf", "", LINKS);
        assert_eq!(
            doc.link_at(0, [70.0, 90.0, 72.0, 92.0]),
            Some(Link::Internal { page: 2, y: 0.0 })
        );
    }

    #[test]
    fn a_cell_that_only_touches_a_link_still_counts() {
        let doc = open_with("touching-link.pdf", "", LINKS);
        assert!(matches!(
            doc.link_at(0, [39.0, 5.0, 45.0, 7.0]),
            Some(Link::External(_))
        ));
    }

    #[test]
    fn a_click_away_from_any_link_finds_nothing() {
        let doc = open_with("no-link.pdf", "", LINKS);
        assert_eq!(doc.link_at(0, [50.0, 50.0, 52.0, 52.0]), None);
        assert_eq!(doc.link_at(1, [10.0, 5.0, 12.0, 7.0]), None);
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

    fn open_text(name: &str, content: &str) -> Doc {
        let page = "<</Type/Page/Parent 2 0 R/MediaBox[0 0 400 400]/Contents 4 0 R/Resources<</Font<</F1<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>>>>>>>";
        let length = content.len();
        let pdf = format!(
            "%PDF-1.7\n1 0 obj\n<</Type/Catalog/Pages 2 0 R>>\nendobj\n2 0 obj\n<</Type/Pages/Count 1/Kids[3 0 R]>>\nendobj\n3 0 obj\n{page}\nendobj\n4 0 obj\n<</Length {length}>>\nstream\n{content}\nendstream\nendobj\ntrailer\n<</Root 1 0 R>>\n"
        );
        let path = Path::new(env!("OUT_DIR")).join(name);
        fs::write(&path, pdf).unwrap();
        Doc::open(&path).unwrap()
    }

    #[test]
    fn folding_ignores_case_spacing_hyphens_and_accents() {
        let doc = open("fold.pdf", "");
        assert_eq!(doc.fold("Well-Known  Fact"), "WELLKNOWNFACT");
        assert_eq!(doc.fold("caf\u{e9}"), "CAFE");
        assert_eq!(doc.fold("cafe\u{301}"), "CAFE");
    }

    #[test]
    fn folding_applies_compatibility_forms_like_mupdf_search_does() {
        let doc = open("fold-compat.pdf", "");
        assert_eq!(doc.fold("\u{fb01}nd"), "FIND");
        assert_eq!(doc.fold("\u{211d}"), "R");
        assert_eq!(doc.fold("\u{1d453}"), "F");
        assert_eq!(doc.fold("x\u{b2}"), "X2");
    }

    #[test]
    fn a_word_split_across_lines_is_found_by_search_and_by_the_folded_page() {
        let doc = open_text(
            "split.pdf",
            "BT /F1 12 Tf 20 300 Td (a para-) Tj 0 -14 Td (graph of Text) Tj ET",
        );
        let folded = doc.folded_page(0);
        for needle in ["paragraph", "PARAGRAPH", "graph of", "of  text"] {
            assert!(!doc.search(0, needle).is_empty(), "search missed {needle}");
            assert!(folded.contains(&doc.fold(needle)), "index missed {needle}");
        }
        assert!(doc.search(0, "absent").is_empty());
        assert!(!folded.contains(&doc.fold("absent")));
    }

    #[test]
    fn a_page_that_does_not_exist_folds_to_nothing() {
        let doc = open("fold-missing.pdf", "");
        assert_eq!(doc.folded_page(99), "");
    }
}

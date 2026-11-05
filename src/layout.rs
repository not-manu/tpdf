use std::ops::Range;

use crate::kitty::MAX_GRID;

pub const TILE_COLS: i64 = MAX_GRID;
pub const TILE_ROWS: i64 = 32;
const PAGE_GAP: i64 = 1;

#[derive(Debug, PartialEq)]
pub struct Span {
    pub page: usize,
    pub rows: Range<i64>,
    pub cols: Range<i64>,
}

pub struct Layout {
    pub tops: Vec<i64>,
    pub rows: Vec<i64>,
    pub cols: Vec<i64>,
    pub lefts: Vec<i64>,
    pub total_rows: i64,
    pub canvas_cols: i64,
}

impl Layout {
    pub fn new(sizes: &[(f32, f32)], scale: f32, cell: (i64, i64), view_cols: i64) -> Layout {
        let span = |points: f32, cell: i64| {
            (((points * scale) / cell as f32 - 0.01).ceil() as i64).max(1)
        };
        let cols: Vec<i64> = sizes.iter().map(|size| span(size.0, cell.0)).collect();
        let rows: Vec<i64> = sizes.iter().map(|size| span(size.1, cell.1)).collect();

        let mut tops = Vec::with_capacity(rows.len());
        let mut next = 0;
        for &height in &rows {
            tops.push(next);
            next += height + PAGE_GAP;
        }
        let total_rows = (next - PAGE_GAP).max(0);

        let canvas_cols = cols.iter().copied().max().unwrap_or(0).max(view_cols);
        let lefts = cols.iter().map(|width| (canvas_cols - width) / 2).collect();

        Layout { tops, rows, cols, lefts, total_rows, canvas_cols }
    }

    pub fn page_at(&self, row: i64) -> usize {
        self.tops.partition_point(|&top| top <= row).saturating_sub(1)
    }

    pub fn locate(&self, row: i64, col: i64) -> Option<(usize, i64, i64)> {
        let page = self.page_at(row);
        let (page_row, page_col) = (row - self.tops[page], col - self.lefts[page]);
        let inside = (0..self.rows[page]).contains(&page_row) && (0..self.cols[page]).contains(&page_col);
        inside.then_some((page, page_row, page_col))
    }

    pub fn spans(&self, from: i64, to: i64, scroll_col: i64, view_cols: i64) -> Vec<Span> {
        let (from, to) = (from.max(0), to.min(self.total_rows));
        let mut spans = Vec::new();
        if from >= to {
            return spans;
        }
        let mut page = self.page_at(from);
        while page < self.tops.len() && self.tops[page] < to {
            let top = self.tops[page];
            let left = self.lefts[page] - scroll_col;
            let rows = (from - top).max(0)..(to - top).min(self.rows[page]);
            let cols = (-left).max(0)..(view_cols - left).min(self.cols[page]);
            if !rows.is_empty() && !cols.is_empty() {
                spans.push(Span { page, rows, cols });
            }
            page += 1;
        }
        spans
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Layout {
        Layout::new(&[(100.0, 200.0), (200.0, 100.0)], 2.0, (10, 20), 30)
    }

    #[test]
    fn pages_stack_with_a_gap() {
        let layout = layout();
        assert_eq!(layout.rows, [20, 10]);
        assert_eq!(layout.tops, [0, 21]);
        assert_eq!(layout.total_rows, 31);
    }

    #[test]
    fn narrow_pages_center_on_the_canvas() {
        let layout = layout();
        assert_eq!(layout.cols, [20, 40]);
        assert_eq!(layout.canvas_cols, 40);
        assert_eq!(layout.lefts, [10, 0]);
    }

    #[test]
    fn view_wider_than_pages_sets_the_canvas() {
        let layout = Layout::new(&[(100.0, 100.0)], 1.0, (10, 20), 50);
        assert_eq!(layout.canvas_cols, 50);
        assert_eq!(layout.lefts, [20]);
    }

    #[test]
    fn exact_fit_does_not_round_up_a_row() {
        let scale = 620.0 / 792.0;
        let layout = Layout::new(&[(612.0, 792.0)], scale, (16, 31), 100);
        assert_eq!(layout.rows, [20]);
    }

    #[test]
    fn spans_cover_each_visible_page_and_skip_gaps() {
        let layout = layout();
        let spans = layout.spans(15, 25, 0, 40);
        assert_eq!(
            spans,
            [
                Span { page: 0, rows: 15..20, cols: 0..20 },
                Span { page: 1, rows: 0..4, cols: 0..40 },
            ]
        );
    }

    #[test]
    fn spans_clip_to_the_view_when_scrolled_sideways() {
        let layout = layout();
        let spans = layout.spans(21, 22, 10, 15);
        assert_eq!(spans, [Span { page: 1, rows: 0..1, cols: 10..25 }]);
    }

    #[test]
    fn spans_outside_the_document_are_empty() {
        let layout = layout();
        assert!(layout.spans(100, 110, 0, 40).is_empty());
        assert!(layout.spans(20, 21, 0, 40).is_empty());
    }

    #[test]
    fn locate_gives_the_cell_within_its_page() {
        let layout = layout();
        assert_eq!(layout.locate(5, 15), Some((0, 5, 5)));
        assert_eq!(layout.locate(22, 3), Some((1, 1, 3)));
    }

    #[test]
    fn locate_finds_nothing_off_the_page() {
        let layout = layout();
        assert_eq!(layout.locate(-1, 15), None);
        assert_eq!(layout.locate(20, 15), None);
        assert_eq!(layout.locate(5, 9), None);
        assert_eq!(layout.locate(5, 30), None);
        assert_eq!(layout.locate(100, 3), None);
    }

    #[test]
    fn page_at_treats_a_gap_as_the_page_above() {
        let layout = layout();
        assert_eq!(layout.page_at(0), 0);
        assert_eq!(layout.page_at(19), 0);
        assert_eq!(layout.page_at(20), 0);
        assert_eq!(layout.page_at(21), 1);
        assert_eq!(layout.page_at(1000), 1);
    }
}

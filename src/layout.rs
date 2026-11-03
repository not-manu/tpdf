use crate::kitty::MAX_GRID;

pub const TILE_COLS: i64 = MAX_GRID;
pub const TILE_ROWS: i64 = 32;
const PAGE_GAP: i64 = 1;

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
    fn page_at_treats_a_gap_as_the_page_above() {
        let layout = layout();
        assert_eq!(layout.page_at(0), 0);
        assert_eq!(layout.page_at(19), 0);
        assert_eq!(layout.page_at(20), 0);
        assert_eq!(layout.page_at(21), 1);
        assert_eq!(layout.page_at(1000), 1);
    }
}

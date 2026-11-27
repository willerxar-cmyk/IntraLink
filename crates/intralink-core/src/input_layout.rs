//! Deterministic spatial input pad geometry, independent of native hooks.
use crate::input::MAX_SLOTS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Zone {
    /// None is the local/safe zone; Some is a zero-based remote slot.
    pub target: Option<usize>,
    pub x: u16,
    pub y: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct Layout {
    slots: usize,
    columns: usize,
    rows: usize,
}

impl Layout {
    pub fn new(slots: usize) -> Option<Self> {
        if !(1..=MAX_SLOTS).contains(&slots) {
            return None;
        }
        let cells = slots + 1;
        let columns = if cells <= 4 { 2 } else { 3 };
        let rows = cells.div_ceil(columns);
        Some(Self {
            slots,
            columns,
            rows,
        })
    }

    pub fn cells(self) -> usize {
        self.slots + 1
    }

    /// Half-open rectangles partition the complete pad without overlap.
    pub fn bounds(
        self,
        cell: usize,
        width: usize,
        height: usize,
    ) -> Option<(usize, usize, usize, usize)> {
        if cell >= self.cells() || width < self.columns * 2 || height < self.rows * 2 {
            return None;
        }
        let col = cell % self.columns;
        let row = cell / self.columns;
        let left = width.checked_mul(col)? / self.columns;
        let right = width.checked_mul(col + 1)? / self.columns;
        let top = height.checked_mul(row)? / self.rows;
        let bottom = height.checked_mul(row + 1)? / self.rows;
        Some((left, top, right, bottom))
    }

    pub fn zone(self, x: usize, y: usize, width: usize, height: usize) -> Option<Zone> {
        if x >= width || y >= height {
            return None;
        }
        for cell in 0..self.cells() {
            let (left, top, right, bottom) = self.bounds(cell, width, height)?;
            if (left..right).contains(&x) && (top..bottom).contains(&y) {
                let normalize = |offset: usize, extent: usize| -> u16 {
                    ((offset as u128 * u16::MAX as u128) / (extent - 1) as u128) as u16
                };
                return Some(Zone {
                    target: cell.checked_sub(1),
                    x: normalize(x - left, right - left),
                    y: normalize(y - top, bottom - top),
                });
            }
        }
        // Empty cells can never select a remote host.
        Some(Zone {
            target: None,
            x: 0,
            y: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_eight_slots_have_disjoint_exact_edges_and_normalized_coordinates() {
        let layout = Layout::new(8).unwrap();
        for cell in 0..9 {
            let (l, t, r, b) = layout.bounds(cell, 961, 541).unwrap();
            let first = layout.zone(l, t, 961, 541).unwrap();
            let last = layout.zone(r - 1, b - 1, 961, 541).unwrap();
            assert_eq!(first.target, cell.checked_sub(1));
            assert_eq!(last.target, first.target);
            assert_eq!((first.x, first.y), (0, 0));
            assert_eq!((last.x, last.y), (65535, 65535));
        }
        assert!(layout.zone(961, 0, 961, 541).is_none());
        assert!(Layout::new(0).is_none());
        assert!(Layout::new(9).is_none());
    }

    #[test]
    fn unused_cells_are_local_and_tiny_dimensions_are_refused() {
        let layout = Layout::new(4).unwrap();
        assert_eq!(layout.zone(899, 599, 900, 600).unwrap().target, None);
        assert!(layout.zone(0, 0, 1, 1).is_none());
    }
}

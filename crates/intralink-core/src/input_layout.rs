//! Deterministic, rearrangeable spatial input pad geometry. No native hooks.
use crate::input::MAX_SLOTS;

const EMPTY: u8 = u8::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Zone {
    /// None is the local/safe zone; Some is a zero-based remote slot.
    pub target: Option<usize>,
    pub x: u16,
    pub y: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    slots: usize,
    columns: usize,
    rows: usize,
    order: [u8; 9],
}

impl Layout {
    pub fn new(slots: usize) -> Option<Self> {
        if !(1..=MAX_SLOTS).contains(&slots) {
            return None;
        }
        let columns = if slots < 4 { 2 } else { 3 };
        let rows = (slots + 1).div_ceil(columns);
        let mut order = [EMPTY; 9];
        for (i, cell) in order.iter_mut().take(slots + 1).enumerate() {
            *cell = i as u8;
        }
        Some(Self {
            slots,
            columns,
            rows,
            order,
        })
    }

    /// Local starts in the center, other screens placed around it. The
    /// configured slot identity never changes when its card is dragged.
    pub fn arranged(slots: usize) -> Option<Self> {
        let mut layout = Self::new(slots)?;
        layout.columns = 3;
        layout.rows = 3;
        layout.order.fill(EMPTY);
        for (i, cell) in [4, 3, 5, 1, 7, 0, 2, 6, 8]
            .into_iter()
            .take(slots + 1)
            .enumerate()
        {
            layout.order[cell] = i as u8;
        }
        Some(layout)
    }

    pub fn cells(self) -> usize {
        self.columns * self.rows
    }

    /// Display label 0 is local, 1-8 are immutable paired slot identities.
    pub fn label(self, cell: usize) -> Option<usize> {
        self.order
            .get(cell)
            .copied()
            .filter(|v| usize::from(*v) <= self.slots)
            .map(usize::from)
    }

    pub fn cell_of(self, target: Option<usize>) -> Option<usize> {
        let label = match target {
            None => 0,
            Some(slot) if slot < self.slots => slot + 1,
            _ => return None,
        };
        self.order.iter().position(|v| usize::from(*v) == label)
    }

    /// Swap a populated card with any valid cell, including an empty cell.
    /// The caller must permit editing only when remote input is disarmed.
    pub fn relocate(&mut self, from: usize, to: usize) -> bool {
        if from == to || self.label(from).is_none() || to >= self.cells() {
            return false;
        }
        self.order.swap(from, to);
        true
    }

    pub fn cell_at(self, x: usize, y: usize, width: usize, height: usize) -> Option<usize> {
        if x >= width || y >= height {
            return None;
        }
        (0..self.cells()).find(|&cell| {
            self.bounds(cell, width, height)
                .is_some_and(|(l, t, r, b)| (l..r).contains(&x) && (t..b).contains(&y))
        })
    }

    /// Only actual shared edges count, not a diagonal corner or a gap.
    pub fn adjacent(self, a: usize, b: usize) -> bool {
        a < self.cells()
            && b < self.cells()
            && (a % self.columns).abs_diff(b % self.columns)
                + (a / self.columns).abs_diff(b / self.columns)
                == 1
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
        Some((
            width.checked_mul(col)? / self.columns,
            height.checked_mul(row)? / self.rows,
            width.checked_mul(col + 1)? / self.columns,
            height.checked_mul(row + 1)? / self.rows,
        ))
    }

    pub fn zone(self, x: usize, y: usize, width: usize, height: usize) -> Option<Zone> {
        let cell = self.cell_at(x, y, width, height)?;
        let (left, top, right, bottom) = self.bounds(cell, width, height)?;
        let normalize = |offset: usize, extent: usize| -> u16 {
            ((offset as u128 * u16::MAX as u128) / (extent - 1) as u128) as u16
        };
        Some(Zone {
            target: self.label(cell).and_then(|label| label.checked_sub(1)),
            x: normalize(x - left, right - left),
            y: normalize(y - top, bottom - top),
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

    #[test]
    fn arranged_cards_keep_identity_and_empty_space_never_grants_input() {
        let mut layout = Layout::arranged(3).unwrap();
        assert_eq!(layout.cell_of(None), Some(4));
        assert_eq!(layout.cell_of(Some(0)), Some(3));
        assert!(layout.relocate(3, 8));
        assert_eq!(layout.cell_of(Some(0)), Some(8));
        assert_eq!(layout.label(3), None);
        assert!(!layout.relocate(3, 0));
        assert!(!layout.relocate(8, 9));
        assert!(layout.relocate(8, 4));
        assert_eq!(layout.cell_of(None), Some(8));
        assert_eq!(layout.cell_of(Some(0)), Some(4));
        assert!(layout.adjacent(4, 5));
        assert!(!layout.adjacent(4, 8));
        assert!(!layout.adjacent(3, 5));
        assert_eq!(layout.zone(480, 270, 960, 540).unwrap().target, Some(0));
    }
}

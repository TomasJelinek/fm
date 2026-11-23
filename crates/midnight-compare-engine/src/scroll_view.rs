//! The painted surface of a text or byte comparison: the view, and content painted around
//! it, so that the window moves the view at once by scroll input the worker has not painted
//! yet (`midnight-compare-ui` `text_view.rs`), as the text viewer moves over its prepared
//! strips. Pure geometry and row arithmetic.
use crate::ScrollView;

/// The farthest horizontal position, physical pixels.
pub(crate) const HORIZONTAL_LIMIT: f32 = 262_144.0;
/// Pixels of one painted surface at most (32 MiB of RGBA); the margins shrink to fit.
const SURFACE_PIXELS: u64 = 8 * 1024 * 1024;
/// Logical pixels painted left and right of each pane's view, at most half the pane.
const HORIZONTAL_MARGIN: f32 = 256.0;
/// Rows painted at most, a bound on one paint's work.
pub(crate) const MAX_ROWS: u64 = 768;

/// A view of two panes side by side and its painted margins, physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Surface {
    /// Each pane's view width and the view's height.
    pub panes: [u32; 2],
    pub height: u32,
    /// Painted beyond the view: [left, right] of each pane, [above, below] of both.
    pub margin: [[u32; 2]; 2],
    /// The view's position and the farthest position per axis.
    pub position: [u64; 2],
    pub extent: [u64; 2],
}

impl Surface {
    /// The surface of a `width` × `height` view split into two panes at `position` of
    /// `extent`, with rows of `row` pixels: one view above and below, and
    /// `HORIZONTAL_MARGIN` beside each pane, cut at the content's edges, the pixel budget
    /// and the row bound.
    pub fn plan(
        width: u32,
        height: u32,
        scale: f32,
        row: u32,
        position: [u64; 2],
        extent: [u64; 2],
    ) -> Self {
        let panes = [width / 2, width - width / 2];
        let position = [position[0].min(extent[0]), position[1].min(extent[1])];
        let beside = u64::from(((HORIZONTAL_MARGIN * scale).round() as u32).min(panes[0] / 2));
        let left = beside.min(position[0]);
        let right = beside.min(extent[0] - position[0]);
        let surface_width = u64::from(width) + 2 * (left + right);
        let height_64 = u64::from(height);
        let budget = (SURFACE_PIXELS / surface_width.max(1)).saturating_sub(height_64) / 2;
        let rows =
            (MAX_ROWS.saturating_sub(2) * u64::from(row.max(1))).saturating_sub(height_64) / 2;
        let around = height_64.min(budget).min(rows);
        let above = around.min(position[1]);
        let below = around.min(extent[1] - position[1]);
        Self {
            panes,
            height,
            margin: [[left as u32, right as u32], [above as u32, below as u32]],
            position,
            extent,
        }
    }

    /// One pane's painted region: its left edge in the surface and its width.
    pub fn region(&self, side: usize) -> (u32, u32) {
        let [left, right] = self.margin[0];
        let width = left + self.panes[side] + right;
        if side == 0 {
            (0, width)
        } else {
            (left + self.panes[0] + right, width)
        }
    }

    pub fn width(&self) -> u32 {
        self.region(1).0 + self.region(1).1
    }

    pub fn surface_height(&self) -> u32 {
        self.margin[1][0] + self.height + self.margin[1][1]
    }

    /// The vertical position of the surface's first pixel row.
    pub fn top(&self) -> u64 {
        self.position[1] - u64::from(self.margin[1][0])
    }

    /// What the window needs to present this surface (`scrolled` is the worker's).
    pub fn view(&self, scale: f32) -> ScrollView {
        let logical = |value: u64| value as f64 / f64::from(scale);
        ScrollView {
            scrolled: [0.0; 2],
            position: self.position.map(logical),
            extent: self.extent.map(logical),
            scale,
            panes: self.panes,
            height: self.height,
            margin: self.margin,
        }
    }
}

/// A row position moved by `pixels` logical pixels (negative moves forward, as a wheel)
/// over rows of `row_height` logical pixels: the first row shown and the fraction of it
/// above the view, within rows 0 to `last`. The last row stands at the top whole.
pub(crate) fn scroll_rows(
    row: u64,
    fraction: f64,
    pixels: f32,
    row_height: f32,
    last: u64,
) -> (u64, f64) {
    if !pixels.is_finite() || row_height.is_nan() || row_height <= 0.0 {
        return (row.min(last), fraction);
    }
    let rows = fraction - f64::from(pixels) / f64::from(row_height);
    let whole = rows.floor();
    let target = row as f64 + whole;
    if target < 0.0 {
        (0, 0.0)
    } else if target >= last as f64 {
        (last, 0.0)
    } else {
        (target as u64, (rows - whole).clamp(0.0, 1.0 - f64::EPSILON))
    }
}

/// The row position at `amount` (0 to 1) of rows 0 to `last`.
pub(crate) fn seek_rows(amount: f64, last: u64) -> (u64, f64) {
    let rows = amount.clamp(0.0, 1.0) * last as f64;
    let row = (rows.floor() as u64).min(last);
    if row >= last {
        (last, 0.0)
    } else {
        (row, rows - row as f64)
    }
}

/// The fraction of a row of `row` physical pixels, as whole pixels below one row.
pub(crate) fn fraction_pixels(fraction: f64, row: u32) -> u64 {
    ((fraction * f64::from(row)).round() as u64).min(u64::from(row.saturating_sub(1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_surface_holds_one_view_around_it_cut_at_the_edges_and_the_budget() {
        let surface = Surface::plan(1000, 600, 1.0, 23, [0, 5000], [262_144, 100_000]);
        assert_eq!(surface.margin, [[0, 250], [600, 600]]);
        assert_eq!(surface.panes, [500, 500]);
        assert_eq!(surface.width(), 1000 + 500);
        assert_eq!(surface.surface_height(), 1800);
        assert_eq!(surface.region(1), (750, 750));
        assert_eq!(surface.top(), 4400);
        // At the start and the end the content's edge cuts the margin.
        let start = Surface::plan(1000, 600, 1.0, 23, [100, 40], [262_144, 100_000]);
        assert_eq!(start.margin, [[100, 250], [40, 600]]);
        let end = Surface::plan(1000, 600, 1.0, 23, [0, 99_900], [262_144, 100_000]);
        assert_eq!(end.margin[1], [600, 100]);
        // A large view keeps within the pixel budget.
        let large = Surface::plan(2560, 1440, 2.0, 46, [0, 50_000], [262_144, 100_000]);
        let pixels = u64::from(large.width()) * u64::from(large.surface_height());
        assert!(pixels <= SURFACE_PIXELS, "{pixels}");
        assert!(large.margin[1][0] > 0 && large.margin[1][0] < 1440);
        // Small rows keep within the row bound.
        let tiny = Surface::plan(400, 4000, 1.0, 12, [0, 50_000], [262_144, 100_000]);
        assert!(u64::from(tiny.surface_height()) / 12 + 2 <= MAX_ROWS);
        assert!(tiny.margin[1][0] > 0);
        let view = surface.view(2.0);
        assert_eq!(view.position, [0.0, 2500.0]);
        assert_eq!(view.extent, [131_072.0, 50_000.0]);
    }

    #[test]
    fn rows_move_by_pixels_and_stop_at_the_first_and_last_row() {
        assert_eq!(scroll_rows(0, 0.0, -11.5, 23.0, 100), (0, 0.5));
        assert_eq!(scroll_rows(0, 0.5, -23.0, 23.0, 100), (1, 0.5));
        assert_eq!(scroll_rows(3, 0.25, 23.0 * 0.5, 23.0, 100), (2, 0.75));
        assert_eq!(scroll_rows(1, 0.0, 100.0, 23.0, 100), (0, 0.0));
        assert_eq!(scroll_rows(99, 0.5, -23.0, 23.0, 100), (100, 0.0));
        assert_eq!(scroll_rows(5, 0.5, f32::NAN, 23.0, 100), (5, 0.5));
        assert_eq!(seek_rows(0.5, 101), (50, 0.5));
        assert_eq!(seek_rows(1.0, 101), (101, 0.0));
        assert_eq!(seek_rows(-1.0, 101), (0, 0.0));
        assert_eq!(fraction_pixels(0.5, 23), 12);
        assert_eq!(fraction_pixels(0.9999, 23), 22);
    }
}

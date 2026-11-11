//! Diagnostic pixel matching; capture is bounded and comparison runs after the gesture.
use midnight_viewer_ui::ViewerWindow;

pub(super) struct Stripe {
    rows: usize,
    pixels: Vec<u8>,
}
const COLUMNS: usize = 64;

impl Stripe {
    pub(super) fn capture(window: &ViewerWindow) -> Option<Self> {
        // Raster surfaces already use this format: Slint shares its buffer here.
        let image = window.get_surface().to_rgba8_premultiplied()?;
        let width = image.width() as usize;
        let height = image.height() as usize;
        if width < 520 || height < 520 {
            return None;
        }
        let rows = height.min(640);
        let source = image.as_slice();
        let pixels = (0..rows)
            .flat_map(|row| (0..COLUMNS).map(move |column| source[row * width + 8 + column * 8].r))
            .collect();
        Some(Self { rows, pixels })
    }

    pub(super) fn displacement(&self, next: &Self, maximum: usize) -> Option<(f64, f64)> {
        if self.rows != next.rows || maximum * 2 >= self.rows {
            return None;
        }
        let maximum = maximum as isize;
        let error = |shift: isize| -> f64 {
            let mut total = 0_u64;
            for row in maximum..self.rows as isize - maximum {
                let old = (row + shift) as usize * COLUMNS;
                let new = row as usize * COLUMNS;
                for column in 0..COLUMNS {
                    let difference =
                        i32::from(self.pixels[old + column]) - i32::from(next.pixels[new + column]);
                    total += (difference * difference) as u64;
                }
            }
            total as f64
        };
        let candidates: Vec<f64> = (-maximum..=maximum).map(error).collect();
        let (index, best) = candidates
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.total_cmp(b.1))?;
        let mut pixels = index as f64 - maximum as f64;
        if index > 0 && index + 1 < candidates.len() {
            let left = candidates[index - 1];
            let right = candidates[index + 1];
            let curvature = left - 2.0 * best + right;
            if curvature > 0.0 {
                pixels += ((left - right) / (2.0 * curvature)).clamp(-0.5, 0.5);
            }
        }
        let unmoved = candidates[maximum as usize];
        Some((pixels, if unmoved == 0.0 { 1.0 } else { best / unmoved }))
    }
}

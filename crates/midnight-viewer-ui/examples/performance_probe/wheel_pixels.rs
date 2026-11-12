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

    // Diagnostic-only visible crop of retained pixels. Sampling follows the displayed
    // texture translation; normalized byte-position estimates are deliberately unused.
    pub(super) fn capture_retained(window: &ViewerWindow) -> Option<Self> {
        let image = window.get_surface().to_rgba8_premultiplied()?;
        let width = image.width() as usize;
        let height = image.height() as usize;
        let scale_x = width as f64
            / f64::from(window.get_raster_width() * window.get_raster_viewport_width());
        let scale_y = height as f64
            / f64::from(window.get_raster_height() * window.get_raster_viewport_height());
        let origin = [
            -f64::from(window.get_raster_offset_x() * window.get_raster_viewport_width()) * scale_x,
            -f64::from(window.get_raster_offset_y() * window.get_raster_viewport_height())
                * scale_y,
        ];
        if !scale_x.is_finite() || !scale_y.is_finite() || scale_x <= 0.0 || scale_y <= 0.0 {
            return None;
        }
        let source = image.as_slice();
        Self::translated(
            [width, height],
            origin,
            (f64::from(window.get_content_height()) * scale_y).floor() as usize,
            |x, y| source[y * width + x].r,
        )
    }

    fn translated(
        size: [usize; 2],
        origin: [f64; 2],
        viewport_rows: usize,
        sample: impl Fn(usize, usize) -> u8,
    ) -> Option<Self> {
        let rows = viewport_rows.min(640);
        if rows < 520
            || origin
                .iter()
                .any(|value| !value.is_finite() || *value < 0.0)
            || origin[0] + 8.0 + (COLUMNS - 1) as f64 * 8.0 + 1.0 >= size[0] as f64
            || origin[1] + rows as f64 >= size[1] as f64
        {
            return None;
        }
        let mut pixels = Vec::with_capacity(rows * COLUMNS);
        for row in 0..rows {
            let y = origin[1] + row as f64;
            let y0 = y.floor() as usize;
            let dy = y.fract();
            for column in 0..COLUMNS {
                let x = origin[0] + 8.0 + column as f64 * 8.0;
                let x0 = x.floor() as usize;
                let dx = x.fract();
                let top =
                    f64::from(sample(x0, y0)) * (1.0 - dx) + f64::from(sample(x0 + 1, y0)) * dx;
                let bottom = f64::from(sample(x0, y0 + 1)) * (1.0 - dx)
                    + f64::from(sample(x0 + 1, y0 + 1)) * dx;
                pixels.push((top * (1.0 - dy) + bottom * dy).round() as u8);
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn pattern(x: usize, y: usize) -> u8 {
        ((x.wrapping_mul(37) ^ y.wrapping_mul(61) ^ (y / 7).wrapping_mul(73)) % 256) as u8
    }

    #[test]
    fn retained_crop_tracks_pixels_across_fractional_translation_and_source_rebase() {
        let before = Stripe::translated([1000, 1000], [0.0, 100.0], 640, pattern).unwrap();
        let after =
            Stripe::translated([1000, 1000], [0.0, 58.25], 640, |x, y| pattern(x, y + 50)).unwrap();
        let (shift, ratio) = before.displacement(&after, 140).unwrap();
        assert!((shift - 8.25).abs() < 0.35, "shift={shift}");
        assert!(ratio < 0.1, "ratio={ratio}");
        let unchanged =
            Stripe::translated([1000, 1000], [0.0, 50.0], 640, |x, y| pattern(x, y + 50)).unwrap();
        assert_eq!(before.pixels, unchanged.pixels);
    }

    #[test]
    fn retained_crop_rejects_uncovered_and_nonfinite_coordinates() {
        for origin in [
            [-1.0, 0.0],
            [0.0, -1.0],
            [f64::NAN, 0.0],
            [600.0, 0.0],
            [0.0, 400.0],
        ] {
            assert!(Stripe::translated([1000, 1000], origin, 640, pattern).is_none());
        }
        assert!(Stripe::translated([1000, 1000], [0.0; 2], 519, pattern).is_none());
    }
}

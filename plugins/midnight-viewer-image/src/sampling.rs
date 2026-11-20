//! Bounded viewport sampling from retained pixels. Never allocates a full scaled image.
use image::RgbaImage;
use midnight_image_viewer_sdk::api::{self, Frame, Viewport};
use std::io;

pub(super) fn render(image: &RgbaImage, viewport: Viewport) -> io::Result<Frame> {
    viewport.validate()?;
    let mut rgba = vec![0; api::rgba_length(viewport.width, viewport.height)?];
    if viewport.scale >= 1.0 {
        for y in 0..viewport.height {
            for x in 0..viewport.width {
                // Original pixels stay exact at 1:1; magnification exposes the source pixel grid.
                let pixel = nearest(
                    image,
                    viewport.origin_x + (f64::from(x) + 0.5) / viewport.scale,
                    viewport.origin_y + (f64::from(y) + 0.5) / viewport.scale,
                );
                let offset = (y as usize * viewport.width as usize + x as usize) * 4;
                rgba[offset..offset + 4].copy_from_slice(&pixel);
            }
        }
    } else {
        reduce(image, viewport, &mut rgba);
    }
    Ok(Frame {
        width: viewport.width,
        height: viewport.height,
        rgba,
    })
}

/// Source coverage of each output column or row along one axis: the first
/// source index and the covered fraction of each source pixel from there.
struct Spans {
    starts: Vec<usize>,
    /// `bounds[i]..bounds[i + 1]` indexes `weights` for output `i`.
    bounds: Vec<usize>,
    weights: Vec<f64>,
}

impl Spans {
    fn new(count: u32, origin: f64, scale: f64, extent: u32) -> Self {
        let span = 1.0 / scale;
        let limit = f64::from(extent);
        let mut spans = Self {
            starts: Vec::with_capacity(count as usize),
            bounds: Vec::with_capacity(count as usize + 1),
            weights: Vec::new(),
        };
        spans.bounds.push(0);
        for index in 0..count {
            let start = origin + f64::from(index) / scale;
            let end = (start + span).min(limit);
            let start = start.max(0.0);
            let first = start.floor() as u32;
            spans.starts.push(first as usize);
            if end > start {
                for source in first..end.ceil() as u32 {
                    let covered = end.min(f64::from(source) + 1.0) - start.max(f64::from(source));
                    spans.weights.push(covered.max(0.0));
                }
            }
            spans.bounds.push(spans.weights.len());
        }
        spans
    }

    fn get(&self, index: usize) -> (usize, &[f64]) {
        (
            self.starts[index],
            &self.weights[self.bounds[index]..self.bounds[index + 1]],
        )
    }
}

/// Area-average each output pixel over the source pixels it covers, weighting
/// colors by alpha so transparent neighbours add no color halo. Separable: each
/// source row is reduced horizontally once per output row it contributes to.
fn reduce(image: &RgbaImage, viewport: Viewport, rgba: &mut [u8]) {
    let columns = Spans::new(
        viewport.width,
        viewport.origin_x,
        viewport.scale,
        image.width(),
    );
    let rows = Spans::new(
        viewport.height,
        viewport.origin_y,
        viewport.scale,
        image.height(),
    );
    let span = 1.0 / viewport.scale;
    let stride = image.width() as usize * 4;
    let source = image.as_raw();
    let width = viewport.width as usize;
    let mut sums = vec![[0.0f64; 4]; width];
    for (y, output) in rgba.chunks_exact_mut(width * 4).enumerate() {
        sums.fill([0.0; 4]);
        let (first_row, row_weights) = rows.get(y);
        for (offset, &height) in row_weights.iter().enumerate() {
            let row = &source[(first_row + offset) * stride..][..stride];
            for (x, sum) in sums.iter_mut().enumerate() {
                let (first_column, column_weights) = columns.get(x);
                if column_weights.is_empty() {
                    continue;
                }
                let pixels = &row[first_column * 4..][..column_weights.len() * 4];
                let mut line = [0.0f64; 4];
                for (pixel, &width) in pixels.as_chunks::<4>().0.iter().zip(column_weights) {
                    let alpha_weight = width * f64::from(pixel[3]);
                    line[0] += f64::from(pixel[0]) * alpha_weight;
                    line[1] += f64::from(pixel[1]) * alpha_weight;
                    line[2] += f64::from(pixel[2]) * alpha_weight;
                    line[3] += alpha_weight;
                }
                for (total, value) in sum.iter_mut().zip(line) {
                    *total += value * height;
                }
            }
        }
        for (pixel, sum) in output.as_chunks_mut::<4>().0.iter_mut().zip(&sums) {
            *pixel = if sum[3] == 0.0 {
                [0; 4]
            } else {
                [
                    (sum[0] / sum[3]).round() as u8,
                    (sum[1] / sum[3]).round() as u8,
                    (sum[2] / sum[3]).round() as u8,
                    (sum[3] / (span * span)).round() as u8,
                ]
            };
        }
    }
}

fn nearest(image: &RgbaImage, x: f64, y: f64) -> [u8; 4] {
    if x < 0.0 || y < 0.0 || x >= f64::from(image.width()) || y >= f64::from(image.height()) {
        return [0; 4];
    }
    image.get_pixel(x.floor() as u32, y.floor() as u32).0
}

#[cfg(test)]
fn area(image: &RgbaImage, x: f64, y: f64, span: f64) -> [u8; 4] {
    let right = (x + span).min(f64::from(image.width()));
    let bottom = (y + span).min(f64::from(image.height()));
    let left = x.max(0.0);
    let top = y.max(0.0);
    if right <= left || bottom <= top {
        return [0; 4];
    }
    let mut sums = [0.0; 4];
    for iy in top.floor() as u32..bottom.ceil() as u32 {
        let height = (bottom.min(f64::from(iy) + 1.0) - top.max(f64::from(iy))).max(0.0);
        for ix in left.floor() as u32..right.ceil() as u32 {
            let width = (right.min(f64::from(ix) + 1.0) - left.max(f64::from(ix))).max(0.0);
            let pixel = image.get_pixel(ix, iy).0;
            let alpha_weight = width * height * f64::from(pixel[3]);
            for channel in 0..3 {
                sums[channel] += f64::from(pixel[channel]) * alpha_weight;
            }
            sums[3] += alpha_weight;
        }
    }
    if sums[3] == 0.0 {
        return [0; 4];
    }
    [
        (sums[0] / sums[3]).round() as u8,
        (sums[1] / sums[3]).round() as u8,
        (sums[2] / sums[3]).round() as u8,
        (sums[3] / (span * span)).round() as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The separable filter reproduces the direct per-pixel area average, including
    /// partial coverage at fractional origins, panned edges and transparency.
    #[test]
    fn separable_reduction_matches_the_direct_area_average() {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut compared = 0;
        for (width, height) in [(1, 1), (7, 3), (64, 41), (257, 130)] {
            let image = RgbaImage::from_fn(width, height, |_, _| {
                let value = next().to_le_bytes();
                // Every eighth pixel is fully transparent; the rest vary in alpha.
                let alpha = if value[4] % 8 == 0 { 0 } else { value[3] };
                image::Rgba([value[0], value[1], value[2], alpha])
            });
            for (scale, origin_x, origin_y) in [
                (0.5, 0.0, 0.0),
                (0.26, 0.0, 0.0),
                (0.3333, 0.25, 0.75),
                (0.9, -3.5, 2.25),
                (0.07, -100.0, -50.0),
                (0.61, 40.0, 1000.0),
            ] {
                let viewport = Viewport {
                    width: 37,
                    height: 23,
                    scale,
                    origin_x,
                    origin_y,
                };
                let frame = render(&image, viewport).unwrap();
                for y in 0..viewport.height {
                    for x in 0..viewport.width {
                        let expected = area(
                            &image,
                            origin_x + f64::from(x) / scale,
                            origin_y + f64::from(y) / scale,
                            1.0 / scale,
                        );
                        let offset = (y * viewport.width + x) as usize * 4;
                        assert_eq!(
                            frame.rgba[offset..offset + 4],
                            expected,
                            "{width}x{height} at {scale} from ({origin_x}, {origin_y}), pixel ({x}, {y})"
                        );
                        compared += 1;
                    }
                }
            }
        }
        assert_eq!(compared, 4 * 6 * 37 * 23);
    }
}

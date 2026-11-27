//! The application icon in the About sky, prepared to be drawn one to one.
//!
//! The sky shows the icon in a 72 px box and floats it 2 px up and down. Both renderers
//! put an image on whole device pixels, so one image floated in whole-pixel steps, and
//! neither filters a reduction (FemtoVG on WGPU samples bilinearly without mipmaps, the
//! software renderer takes the nearest pixel). [`frames`] scales the chosen icon's
//! full-size 256 px image once to the box's device pixels with a bicubic filter widened
//! by the reduction, in sub-pixel steps of a downward shift, so every step is as sharp
//! as the others; the prefiltered `assets/branding/ui` images come only in fixed sizes
//! (the 162 px box at 225 % would get the 144 px one enlarged). The sky draws one step
//! one to one per frame: the float's whole device pixels move the image and its
//! fraction picks the step. Measurements: `docs/verification/2026-11-27-about-icon.md`.
use crate::{AboutView, MainWindow};
use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};
use std::cell::RefCell;

/// The tallest image built: WGPU's default texture limit.
const MAX_HEIGHT: u32 = 8192;

pub(crate) fn install(window: &MainWindow) {
    window
        .global::<AboutView>()
        .on_logo_frames(|icon, pixels, phases| frames(&icon, pixels, phases));
}

thread_local! {
    /// The last icon, size, step count and the frames built for them: reopening About
    /// at the same size reuses them.
    static LAST: RefCell<Option<(Image, i32, i32, Image)>> = const { RefCell::new(None) };
}

/// `icon` fitted into a square of `pixels` device pixels, `phases` times: the k-th copy
/// moved down by k / `phases` of a pixel. The copies are stacked top to bottom, each in
/// a cell one pixel taller than the square, so the result is `pixels` wide and
/// `phases × (pixels + 1)` high. An empty image when the icon's pixels cannot be read
/// or the result would be empty or taller than a texture may be.
pub(crate) fn frames(icon: &Image, pixels: i32, phases: i32) -> Image {
    LAST.with(|last| {
        if let Some((source, size, steps, built)) = &*last.borrow()
            && *size == pixels
            && *steps == phases
            && source == icon
        {
            return built.clone();
        }
        let built = build(icon, pixels, phases)
            .map(Image::from_rgba8_premultiplied)
            .unwrap_or_default();
        *last.borrow_mut() = Some((icon.clone(), pixels, phases, built.clone()));
        built
    })
}

fn build(icon: &Image, pixels: i32, phases: i32) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let size = u32::try_from(pixels).ok().filter(|&size| size > 0)?;
    let phases = u32::try_from(phases).ok().filter(|&phases| phases > 0)?;
    let cell = size + 1;
    let height = cell
        .checked_mul(phases)
        .filter(|&height| height <= MAX_HEIGHT)?;
    let source = icon.to_rgba8_premultiplied()?;
    let (source_width, source_height) = (source.width(), source.height());
    if source_width == 0 || source_height == 0 {
        return None;
    }
    let side = size as f32;
    let scale = (side / source_width as f32).min(side / source_height as f32);
    let left = (side - source_width as f32 * scale) / 2.0;
    let top = (side - source_height as f32 * scale) / 2.0;
    // Every source row at the square's columns.
    let columns = taps(source_width, size, scale, left);
    let width = size as usize;
    let mut rows = vec![[0.0_f32; 4]; source_height as usize * width];
    for (row, scaled) in source
        .as_slice()
        .chunks_exact(source_width as usize)
        .zip(rows.chunks_exact_mut(width))
    {
        for (out, taps) in scaled.iter_mut().zip(&columns) {
            for &(at, weight) in taps {
                let pixel = row[at];
                for (channel, value) in out.iter_mut().zip([pixel.r, pixel.g, pixel.b, pixel.a]) {
                    *channel += weight * f32::from(value);
                }
            }
        }
    }
    let mut atlas = SharedPixelBuffer::<Rgba8Pixel>::new(size, height);
    let mut line = vec![[0.0_f32; 4]; width];
    for (phase, cell_pixels) in atlas
        .make_mut_slice()
        .chunks_exact_mut(width * cell as usize)
        .enumerate()
    {
        let shift = phase as f32 / phases as f32;
        for (taps, out) in taps(source_height, cell, scale, top + shift)
            .iter()
            .zip(cell_pixels.chunks_exact_mut(width))
        {
            line.fill([0.0; 4]);
            for &(at, weight) in taps {
                for (sum, scaled) in line.iter_mut().zip(&rows[at * width..(at + 1) * width]) {
                    for (channel, value) in sum.iter_mut().zip(scaled) {
                        *channel += weight * value;
                    }
                }
            }
            for (pixel, [r, g, b, a]) in out.iter_mut().zip(&line) {
                // The filter overshoots a little at hard edges; keep the colour
                // premultiplied and in range.
                let alpha = a.round().clamp(0.0, 255.0);
                let channel = |value: f32| value.round().clamp(0.0, alpha) as u8;
                *pixel = Rgba8Pixel::new(channel(*r), channel(*g), channel(*b), alpha as u8);
            }
        }
    }
    Some(atlas)
}

/// For each of `count` output pixels, the source pixels and their weights: the
/// source of `length` pixels is scaled by `scale` and moved by `offset` output pixels.
/// Outside the source the image is transparent.
fn taps(length: u32, count: u32, scale: f32, offset: f32) -> Vec<Vec<(usize, f32)>> {
    // Shrinking widens the filter to cover every source pixel under an output pixel.
    let stretch = scale.min(1.0);
    let reach = 2.0 / stretch;
    (0..count)
        .map(|index| {
            // The output pixel's centre in source pixels, whose centres are whole.
            let centre = (index as f32 + 0.5 - offset) / scale - 0.5;
            let first = (centre - reach).ceil() as i64;
            let last = (centre + reach).floor() as i64;
            let weights: Vec<(i64, f32)> = (first..=last)
                .map(|at| (at, catmull_rom((at as f32 - centre) * stretch)))
                .collect();
            let total: f32 = weights.iter().map(|(_, weight)| weight).sum();
            weights
                .into_iter()
                .filter(|&(at, weight)| at >= 0 && at < i64::from(length) && weight != 0.0)
                .map(|(at, weight)| (at as usize, weight / total))
                .collect()
        })
        .collect()
}

/// The Catmull-Rom cubic: sharp, with little ringing.
fn catmull_rom(x: f32) -> f32 {
    let x = x.abs();
    if x < 1.0 {
        (1.5 * x - 2.5) * x * x + 1.0
    } else if x < 2.0 {
        ((-0.5 * x + 2.5) * x - 4.0) * x + 2.0
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `side` px square image, transparent but for an opaque white square from
    /// `from` to `to` in both directions.
    fn square(side: u32, from: u32, to: u32) -> Image {
        let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(side, side);
        for (index, pixel) in pixels.make_mut_slice().iter_mut().enumerate() {
            let (x, y) = (index as u32 % side, index as u32 / side);
            if (from..to).contains(&x) && (from..to).contains(&y) {
                *pixel = Rgba8Pixel::new(255, 255, 255, 255);
            }
        }
        Image::from_rgba8(pixels)
    }

    /// The opacity of every pixel of one step's cell, row by row.
    fn cell(frames: &SharedPixelBuffer<Rgba8Pixel>, size: u32, phase: u32) -> Vec<Vec<f32>> {
        let width = size as usize;
        frames
            .as_slice()
            .chunks_exact(width)
            .skip(phase as usize * (width + 1))
            .take(width + 1)
            .map(|row| row.iter().map(|pixel| f32::from(pixel.a) / 255.0).collect())
            .collect()
    }

    /// Where the opacity's middle is, in pixels from the cell's top and left.
    fn middle(cell: &[Vec<f32>]) -> (f32, f32) {
        let (mut total, mut x, mut y) = (0.0, 0.0, 0.0);
        for (row, values) in cell.iter().enumerate() {
            for (column, value) in values.iter().enumerate() {
                total += value;
                x += value * (column as f32 + 0.5);
                y += value * (row as f32 + 0.5);
            }
        }
        (x / total, y / total)
    }

    #[test]
    fn every_step_is_the_icon_moved_down_by_its_share_of_a_pixel() {
        // The 256 px square from 40 to 216 is 49.5 px from 12.375 to 61.875 at 72 px.
        let icon = square(256, 40, 216);
        for (size, phases) in [(72, 16), (90, 16), (162, 16), (300, 4)] {
            let frames = build(&icon, size, phases).expect("frames");
            assert_eq!(
                (frames.width(), frames.height()),
                (size as u32, phases as u32 * (size as u32 + 1))
            );
            let scale = size as f32 / 256.0;
            let (rest_x, rest_y) = middle(&cell(&frames, size as u32, 0));
            assert!(
                (rest_x - 128.0 * scale).abs() < 0.02 && (rest_y - 128.0 * scale).abs() < 0.02,
                "{size} px: the square's middle is at {rest_x}, {rest_y}"
            );
            for phase in 1..phases as u32 {
                let (x, y) = middle(&cell(&frames, size as u32, phase));
                let shift = phase as f32 / phases as f32;
                assert!(
                    (x - rest_x).abs() < 0.02 && (y - rest_y - shift).abs() < 0.02,
                    "{size} px, step {phase}: moved by {}, {} instead of 0, {shift}",
                    x - rest_x,
                    y - rest_y
                );
            }
        }
    }

    #[test]
    fn edges_are_shaded_by_how_much_of_a_pixel_they_cover() {
        // At 72 px the square's edges are at 11.25 and 60.75 px: its first and last
        // columns and rows are three quarters covered, the ones inside fully.
        let frames = build(&square(256, 40, 216), 72, 1).expect("frames");
        let rows = cell(&frames, 72, 0);
        for (at, covered) in [(10, 0.0), (11, 0.75), (12, 1.0), (35, 1.0), (59, 1.0)] {
            for (name, value) in [("row", rows[at][35]), ("column", rows[35][at])] {
                assert!(
                    (value - covered).abs() < 0.06,
                    "{name} {at}: {value} instead of {covered}"
                );
            }
        }
        for (at, covered) in [(60, 0.75), (61, 0.0)] {
            assert!(
                (rows[at][35] - covered).abs() < 0.06,
                "row {at}: {}",
                rows[at][35]
            );
        }
        // A hard edge shrunk by 3.6 times has no jagged stairs: along each edge every
        // pixel is shaded alike, away from the corners.
        let edge: Vec<f32> = (14..58).map(|x| rows[11][x]).collect();
        assert!(
            edge.iter().all(|value| (value - edge[0]).abs() < 0.01),
            "{edge:?}"
        );
    }

    #[test]
    fn colour_never_exceeds_opacity() {
        let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(256, 256);
        // Alternating opaque white and transparent stripes overshoot the most.
        for (index, pixel) in pixels.make_mut_slice().iter_mut().enumerate() {
            if (index % 256) / 3 % 2 == 0 {
                *pixel = Rgba8Pixel::new(255, 255, 255, 255);
            }
        }
        let frames = build(&Image::from_rgba8(pixels), 100, 4).expect("frames");
        assert!(
            frames
                .as_slice()
                .iter()
                .all(|pixel| pixel.r <= pixel.a && pixel.g <= pixel.a && pixel.b <= pixel.a)
        );
    }

    #[test]
    fn nothing_is_built_without_pixels_or_room() {
        let icon = square(256, 40, 216);
        assert!(build(&Image::default(), 72, 16).is_none());
        assert!(build(&icon, 0, 16).is_none());
        assert!(build(&icon, 72, 0).is_none());
        assert!(build(&icon, 1000, 16).is_none(), "16 001-px high");
        assert_eq!(frames(&icon, -1, 16).size().width, 0);
    }

    #[test]
    fn the_same_request_reuses_the_frames() {
        let icon = square(256, 40, 216);
        let pixels = |image: &Image| {
            image
                .to_rgba8_premultiplied()
                .expect("pixels")
                .as_slice()
                .as_ptr()
        };
        let first = pixels(&frames(&icon, 72, 16));
        assert_eq!(pixels(&frames(&icon, 72, 16)), first, "built again");
        assert_eq!(frames(&icon, 90, 16).size().width, 90);
    }
}

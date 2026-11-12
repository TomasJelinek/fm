//! Bounded viewport sampling from retained pixels. Never allocates a full scaled image.
use image::RgbaImage;
use midnight_image_viewer_sdk::api::{self, Frame, Viewport};
use std::io;

pub(super) fn render(image: &RgbaImage, viewport: Viewport) -> io::Result<Frame> {
    viewport.validate()?;
    let mut rgba = vec![0; api::rgba_length(viewport.width, viewport.height)?];
    for y in 0..viewport.height {
        for x in 0..viewport.width {
            let pixel = if viewport.scale >= 1.0 {
                // Original pixels stay exact at 1:1; magnification exposes the source pixel grid.
                nearest(
                    image,
                    viewport.origin_x + (f64::from(x) + 0.5) / viewport.scale,
                    viewport.origin_y + (f64::from(y) + 0.5) / viewport.scale,
                )
            } else {
                area(
                    image,
                    viewport.origin_x + f64::from(x) / viewport.scale,
                    viewport.origin_y + f64::from(y) / viewport.scale,
                    1.0 / viewport.scale,
                )
            };
            let offset = (y as usize * viewport.width as usize + x as usize) * 4;
            rgba[offset..offset + 4].copy_from_slice(&pixel);
        }
    }
    Ok(Frame {
        width: viewport.width,
        height: viewport.height,
        rgba,
    })
}

fn nearest(image: &RgbaImage, x: f64, y: f64) -> [u8; 4] {
    if x < 0.0 || y < 0.0 || x >= f64::from(image.width()) || y >= f64::from(image.height()) {
        return [0; 4];
    }
    image.get_pixel(x.floor() as u32, y.floor() as u32).0
}

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

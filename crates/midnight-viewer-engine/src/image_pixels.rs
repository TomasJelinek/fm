//! Convert bounded worker straight-alpha RGBA into the host's premultiplied surface.
pub(crate) fn premultiply(rgba: &mut [u8]) {
    // The alpha bytes of four pixels, read as one little-endian word.
    const OPAQUE: u128 = 0xff00_0000_ff00_0000_ff00_0000_ff00_0000;
    let (quads, rest) = rgba.as_chunks_mut::<16>();
    for quad in quads {
        // An opaque pixel keeps its colour ((c * 255 + 127) / 255 == c), and rendered
        // document pages are opaque: four of them are checked at once.
        if u128::from_le_bytes(*quad) & OPAQUE != OPAQUE {
            premultiply_pixels(quad);
        }
    }
    premultiply_pixels(rest);
}

fn premultiply_pixels(rgba: &mut [u8]) {
    for pixel in rgba.as_chunks_mut::<4>().0 {
        let alpha = u16::from(pixel[3]);
        for channel in &mut pixel[..3] {
            *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transparent_pixels_have_no_color_and_partial_alpha_is_premultiplied() {
        let mut pixels = [128, 64, 32, 128, 255, 90, 30, 0, 12, 34, 56, 255];
        premultiply(&mut pixels);
        assert_eq!(pixels, [64, 32, 16, 128, 0, 0, 0, 0, 12, 34, 56, 255]);
    }

    /// Every pixel of a run of whole and partial quads, opaque or not, as one at a time.
    #[test]
    fn quads_premultiply_as_single_pixels_do() {
        let pixels: Vec<u8> = (0..4 * 37)
            .map(|index: usize| match index % 4 {
                3 if index % 24 < 12 => 255,
                3 => (index * 37 % 256) as u8,
                _ => (index * 91 % 256) as u8,
            })
            .collect();
        let mut expected = pixels.clone();
        premultiply_pixels(&mut expected);
        let mut quads = pixels;
        premultiply(&mut quads);
        assert_eq!(quads, expected);
    }
}

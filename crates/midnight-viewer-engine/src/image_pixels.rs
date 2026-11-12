//! Convert bounded worker straight-alpha RGBA into the host's premultiplied surface.
pub(crate) fn premultiply(rgba: &mut [u8]) {
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
}

//! Resampling of native icon artwork to the edge, in device pixels, it is drawn at.
//!
//! Every provider draws its artwork from the smallest source at least as large as the
//! requested edge and reduces it here once. Only artwork that has no larger source is
//! enlarged. Pixels are filtered in premultiplied alpha, so the hidden color of
//! transparent pixels neither darkens nor tints the edges of the artwork.

/// Straight-alpha RGBA8 `rgba` of `width` by `height` pixels as an `edge` by `edge`
/// bitmap: scaled so that its longer side is `edge` and centered on the shorter one.
/// Artwork that already has that size is returned unchanged. `None` for an empty or
/// inconsistent source.
pub fn fit(rgba: &[u8], width: u32, height: u32, edge: u32) -> Option<Vec<u8>> {
    let pixels = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)?;
    if width == 0 || height == 0 || edge == 0 || rgba.len() != pixels {
        return None;
    }
    if (width, height) == (edge, edge) {
        return Some(rgba.to_vec());
    }
    let longest = width.max(height);
    // Rounded, so a square stays square and a slim source keeps at least one pixel.
    let scaled = |side: u32| {
        u32::try_from(
            (u64::from(side) * u64::from(edge) + u64::from(longest) / 2) / u64::from(longest),
        )
        .unwrap_or(edge)
        .clamp(1, edge)
    };
    let (fitted_width, fitted_height) = (scaled(width), scaled(height));
    let source = image::RgbaImage::from_raw(width, height, rgba.to_vec())?;
    let fitted = resize(&source, fitted_width, fitted_height);
    if (fitted_width, fitted_height) == (edge, edge) {
        return Some(fitted.into_raw());
    }
    let mut canvas = image::RgbaImage::new(edge, edge);
    image::imageops::replace(
        &mut canvas,
        &fitted,
        i64::from((edge - fitted_width) / 2),
        i64::from((edge - fitted_height) / 2),
    );
    Some(canvas.into_raw())
}

/// Scales in premultiplied alpha: Lanczos keeps reduced artwork sharp, and its
/// support widens with the reduction, so every source pixel contributes; it would
/// ring around enlarged pixels, which Catmull-Rom does not.
fn resize(source: &image::RgbaImage, width: u32, height: u32) -> image::RgbaImage {
    let mut premultiplied = image::Rgba32FImage::new(source.width(), source.height());
    for (from, to) in source.pixels().zip(premultiplied.pixels_mut()) {
        let alpha = f32::from(from[3]) / 255.0;
        let channel = |index: usize| f32::from(from[index]) / 255.0 * alpha;
        *to = image::Rgba([channel(0), channel(1), channel(2), alpha]);
    }
    let filter = if width < source.width() || height < source.height() {
        image::imageops::FilterType::Lanczos3
    } else {
        image::imageops::FilterType::CatmullRom
    };
    let scaled = image::imageops::resize(&premultiplied, width, height, filter);
    let mut result = image::RgbaImage::new(width, height);
    for (from, to) in scaled.pixels().zip(result.pixels_mut()) {
        let alpha = from[3].clamp(0.0, 1.0);
        let opacity = (alpha * 255.0).round() as u8;
        if opacity == 0 {
            continue;
        }
        let channel = |index: usize| ((from[index] / alpha).clamp(0.0, 1.0) * 255.0).round() as u8;
        *to = image::Rgba([channel(0), channel(1), channel(2), opacity]);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: u32, height: u32, pixel: [u8; 4]) -> Vec<u8> {
        pixel.repeat(width as usize * height as usize)
    }

    #[test]
    fn artwork_of_the_requested_edge_is_returned_pixel_for_pixel() {
        let source: Vec<u8> = (0..24 * 24 * 4)
            .map(|index| (index * 7 % 251) as u8)
            .collect();
        assert_eq!(fit(&source, 24, 24, 24).as_deref(), Some(&source[..]));
    }

    #[test]
    fn inconsistent_or_empty_sources_are_refused() {
        assert_eq!(fit(&[0; 15], 2, 2, 8), None);
        assert_eq!(fit(&[], 0, 0, 8), None);
        assert_eq!(fit(&[0; 16], 2, 2, 0), None);
    }

    #[test]
    fn reduction_keeps_flat_colors_and_full_coverage() {
        let reduced = fit(&solid(256, 256, [200, 100, 50, 255]), 256, 256, 45).unwrap();
        assert_eq!(reduced.len(), 45 * 45 * 4);
        assert!(
            reduced
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| *pixel == [200, 100, 50, 255])
        );
    }

    #[test]
    fn transparent_pixels_carry_no_color_and_do_not_tint_the_seam() {
        // An opaque red left half beside a transparent right half whose hidden color
        // is green. Straight-alpha filtering would tint the seam green.
        let source: Vec<u8> = (0..32 * 32)
            .flat_map(|index| {
                if index % 32 < 16 {
                    [255, 0, 0, 255]
                } else {
                    [0, 255, 0, 0]
                }
            })
            .collect();
        for edge in [20, 48] {
            let scaled = fit(&source, 32, 32, edge).unwrap();
            let mut partial = 0;
            for pixel in scaled.as_chunks::<4>().0 {
                if pixel[3] == 0 {
                    assert_eq!(pixel[..3], [0, 0, 0], "transparent pixels carry no color");
                    continue;
                }
                assert!(pixel[0] >= 250 && pixel[1] <= 5, "edge {edge}: {pixel:?}");
                partial += usize::from(pixel[3] < 255);
            }
            assert!(partial > 0, "the seam is antialiased at edge {edge}");
        }
    }

    #[test]
    fn a_slim_source_is_centered_on_its_shorter_side() {
        let fitted = fit(&solid(4, 2, [10, 20, 30, 255]), 4, 2, 8).unwrap();
        let pixel = |x: usize, y: usize| fitted.as_chunks::<4>().0[y * 8 + x];
        // Eight by four, rows two to five.
        for x in 0..8 {
            assert_eq!(pixel(x, 0)[3], 0, "above");
            assert_eq!(pixel(x, 1)[3], 0, "above");
            assert_eq!(pixel(x, 3), [10, 20, 30, 255], "inside");
            assert_eq!(pixel(x, 6)[3], 0, "below");
        }
    }
}

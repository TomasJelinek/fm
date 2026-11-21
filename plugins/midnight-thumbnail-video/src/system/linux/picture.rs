//! Decoder output to oriented sRGB RGBA within the output bounds, with the bundled profile's
//! rules: never enlarged, quarter-turn orientation, BT.709/BT.601/SD gamuts shown without
//! gamut mapping, the BT.709 camera transfer decoded to sRGB, wide gamut and HDR refused.
use crate::media::{OutputBounds, output_length};
use std::{io, sync::OnceLock};

pub(super) struct Pixels {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) rgba: Vec<u8>,
}

/// A frame's colour description in FFmpeg's (ISO/IEC 23091-2) code points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Colour {
    pub(super) primaries: i32,
    pub(super) transfer: i32,
    pub(super) matrix: i32,
    pub(super) full_range: bool,
}

/// How a frame converts: the YCbCr matrix (601 or 709), its range, and whether the BT.709
/// camera transfer is decoded to sRGB afterwards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Conversion {
    pub(super) matrix: i32,
    pub(super) full_range: bool,
    pub(super) transfer_709: bool,
}

/// Primaries shown without gamut mapping: BT.709 (1), unspecified (2) and the SD gamuts
/// BT.470BG (5) and SMPTE 170M (6). Transfers: BT.709/unspecified/BT.601 and sRGB, plus the
/// BT.470M/BG tags (4, 5) that SD streams carry for the BT.601 camera curve.
pub(super) fn displayable(primaries: i32, transfer: i32) -> bool {
    matches!(primaries, 1 | 2 | 5 | 6) && matches!(transfer, 1 | 2 | 4 | 5 | 6 | 13)
}

/// The conversion for `colour`; `rgb` for packed RGB sources, whose matrix is the identity.
pub(super) fn conversion(colour: Colour, rgb: bool, height: u32) -> io::Result<Conversion> {
    if !displayable(colour.primaries, colour.transfer) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Video color space requires unsupported gamut or HDR conversion",
        ));
    }
    let matrix = match colour.matrix {
        0 if rgb => 709,
        1 => 709,
        2 if height <= 576 => 601,
        2 => 709,
        5 | 6 => 601,
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Unsupported video color matrix",
            ));
        }
    };
    Ok(Conversion {
        matrix,
        full_range: rgb || colour.full_range,
        transfer_709: colour.transfer != 13 && !(colour.transfer == 2 && rgb),
    })
}

/// The scaled size for a `width` x `height` source shown within `bounds` after `rotation`.
pub(super) fn fit(
    width: u32,
    height: u32,
    bounds: OutputBounds,
    rotation: i32,
) -> io::Result<(u32, u32)> {
    output_length(bounds.width, bounds.height)?;
    if width == 0 || height == 0 {
        return Err(io::Error::other("Decoded video dimensions are invalid"));
    }
    let (bound_width, bound_height) = if matches!(rotation, 90 | 270) {
        (bounds.height, bounds.width)
    } else {
        (bounds.width, bounds.height)
    };
    let scale = (f64::from(bound_width) / f64::from(width))
        .min(f64::from(bound_height) / f64::from(height))
        .min(1.0);
    let scaled_width = (f64::from(width) * scale).round().max(1.0) as u32;
    let scaled_height = (f64::from(height) * scale).round().max(1.0) as u32;
    Ok((scaled_width, scaled_height))
}

fn transfer_table() -> &'static [u8; 256] {
    static TABLE: OnceLock<[u8; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        std::array::from_fn(|value| {
            let encoded = value as f64 / 255.0;
            let linear = if encoded < 0.081 {
                encoded / 4.5
            } else {
                ((encoded + 0.099) / 1.099).powf(1.0 / 0.45)
            };
            let srgb = if linear <= 0.0031308 {
                linear * 12.92
            } else {
                1.055 * linear.powf(1.0 / 2.4) - 0.055
            };
            (srgb * 255.0).round().clamp(0.0, 255.0) as u8
        })
    })
}

/// Rotate `scaled` (rows of `stride` bytes, `width` x `height` RGBA) clockwise by `rotation`
/// into a tight buffer, decoding the BT.709 transfer to sRGB when asked. Alpha is opaque.
pub(super) fn finish(
    scaled: &[u8],
    stride: usize,
    width: u32,
    height: u32,
    rotation: i32,
    transfer_709: bool,
) -> io::Result<Pixels> {
    let (output_width, output_height) = if matches!(rotation, 90 | 270) {
        (height, width)
    } else {
        (width, height)
    };
    let mut rgba = vec![0; output_length(output_width, output_height)?];
    let (width, height) = (width as usize, height as usize);
    if stride < width * 4 || scaled.len() < stride * (height - 1) + width * 4 {
        return Err(io::Error::other("Invalid scaled video stride"));
    }
    let table = transfer_table();
    for y in 0..height {
        let row = &scaled[y * stride..y * stride + width * 4];
        for (x, source) in row.as_chunks::<4>().0.iter().enumerate() {
            let (out_x, out_y) = match rotation {
                90 => (height - y - 1, x),
                180 => (width - x - 1, height - y - 1),
                270 => (y, width - x - 1),
                _ => (x, y),
            };
            let target = (out_y * output_width as usize + out_x) * 4;
            let pixel = &mut rgba[target..target + 4];
            if transfer_709 {
                pixel[0] = table[usize::from(source[0])];
                pixel[1] = table[usize::from(source[1])];
                pixel[2] = table[usize::from(source[2])];
            } else {
                pixel[..3].copy_from_slice(&source[..3]);
            }
            pixel[3] = 255;
        }
    }
    Ok(Pixels {
        width: output_width,
        height: output_height,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_shrink_into_rotated_bounds_and_are_never_enlarged() {
        let bounds = OutputBounds {
            width: 32,
            height: 16,
        };
        assert_eq!(fit(640, 480, bounds, 0).unwrap(), (21, 16));
        // A quarter turn swaps the bound the picture must fit before rotation.
        assert_eq!(fit(640, 480, bounds, 90).unwrap(), (16, 12));
        assert_eq!(fit(4, 2, bounds, 0).unwrap(), (4, 2));
        assert!(
            fit(
                4,
                2,
                OutputBounds {
                    width: 0,
                    height: 1
                },
                0
            )
            .is_err()
        );
    }

    #[test]
    fn rotation_moves_pixels_clockwise_and_the_transfer_maps_to_srgb() {
        // 2x1: red, green.
        let scaled = [255, 0, 0, 0, 0, 255, 0, 0];
        let turned = finish(&scaled, 8, 2, 1, 90, false).unwrap();
        assert_eq!((turned.width, turned.height), (1, 2));
        assert_eq!(turned.rgba, [255, 0, 0, 255, 0, 255, 0, 255]);
        let flipped = finish(&scaled, 8, 2, 1, 180, false).unwrap();
        assert_eq!(flipped.rgba, [0, 255, 0, 255, 255, 0, 0, 255]);
        let mid = finish(&[128, 128, 128, 0], 4, 1, 1, 0, true).unwrap();
        assert!(mid.rgba[0] > 128 && mid.rgba[0] == mid.rgba[1]);
        assert_eq!(
            finish(&[0, 0, 0, 0, 255, 255, 255, 0], 8, 2, 1, 0, true)
                .unwrap()
                .rgba,
            [0, 0, 0, 255, 255, 255, 255, 255]
        );
    }

    #[test]
    fn sd_and_hd_colour_is_shown_and_wide_gamut_or_hdr_is_refused() {
        let hd = Colour {
            primaries: 1,
            transfer: 1,
            matrix: 1,
            full_range: false,
        };
        assert_eq!(
            conversion(hd, false, 1080).unwrap(),
            Conversion {
                matrix: 709,
                full_range: false,
                transfer_709: true
            }
        );
        let unspecified_sd = Colour {
            primaries: 2,
            transfer: 2,
            matrix: 2,
            full_range: true,
        };
        assert_eq!(conversion(unspecified_sd, false, 480).unwrap().matrix, 601);
        let srgb = Colour {
            primaries: 1,
            transfer: 13,
            matrix: 0,
            full_range: true,
        };
        assert!(!conversion(srgb, true, 100).unwrap().transfer_709);
        for colour in [
            Colour {
                primaries: 9,
                transfer: 1,
                matrix: 9,
                full_range: false,
            },
            Colour {
                primaries: 1,
                transfer: 16,
                matrix: 1,
                full_range: false,
            },
            Colour {
                primaries: 9,
                transfer: 18,
                matrix: 9,
                full_range: false,
            },
        ] {
            assert_eq!(
                conversion(colour, false, 2160).unwrap_err().kind(),
                io::ErrorKind::Unsupported
            );
        }
    }
}

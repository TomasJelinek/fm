//! Decoder output to oriented sRGB RGBA8 within the output bounds. Each output pixel averages
//! its whole box of source samples (a box filter), so a large frame shrinks without aliasing;
//! the frame is never enlarged. The YCbCr matrix, range and the BT.709-to-sRGB transfer follow
//! the bundled profile, so both decoders show a file alike.
use crate::media::output_length;
use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Layout {
    /// 8-bit luma plane, then interleaved 8-bit Cb/Cr at half resolution.
    Nv12,
    /// 16-bit little-endian samples (10 significant high bits), laid out like NV12.
    P010,
    /// 8-bit luma plane, then Cb and Cr planes at half resolution and half stride.
    I420,
    /// Like I420 with the Cr plane first.
    Yv12,
    /// Packed 4:2:2: Y0 Cb Y1 Cr.
    Yuy2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Matrix {
    Bt601,
    Bt709,
}

/// The geometry and color of one decoded frame buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Format {
    pub layout: Layout,
    /// Luma rows and row bytes allocated in the buffer.
    pub rows: u32,
    pub stride: u32,
    /// The visible picture inside the buffer.
    pub left: u32,
    pub top: u32,
    pub width: u32,
    pub height: u32,
    pub matrix: Matrix,
    pub full_range: bool,
    /// Decode BT.709/BT.601 camera transfer to sRGB, as the bundled profile does.
    pub transfer_709: bool,
    /// Clockwise quarter turns that show the picture upright: 0, 90, 180 or 270.
    pub rotation: u32,
}

pub(super) struct Pixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Format {
    /// Bytes the buffer must hold for this layout.
    fn required(&self) -> Option<usize> {
        let stride = self.stride as usize;
        let rows = self.rows as usize;
        let chroma_rows = rows.div_ceil(2);
        let luma = stride.checked_mul(rows)?;
        match self.layout {
            Layout::Nv12 | Layout::P010 => luma.checked_add(stride.checked_mul(chroma_rows)?),
            Layout::I420 | Layout::Yv12 => {
                luma.checked_add((stride / 2).checked_mul(chroma_rows)?.checked_mul(2)?)
            }
            Layout::Yuy2 => Some(luma),
        }
    }

    fn row_bytes(&self) -> Option<u32> {
        match self.layout {
            Layout::Nv12 | Layout::I420 | Layout::Yv12 => Some(self.left.checked_add(self.width)?),
            Layout::P010 | Layout::Yuy2 => self.left.checked_add(self.width)?.checked_mul(2),
        }
    }

    /// Reject geometry that does not fit the buffer before any sample is read.
    fn check(&self, length: usize) -> io::Result<()> {
        let fits = self.width > 0
            && self.height > 0
            && u64::from(self.width) * u64::from(self.height) <= 32 * 1024 * 1024
            && matches!(self.rotation, 0 | 90 | 180 | 270)
            && self
                .top
                .checked_add(self.height)
                .is_some_and(|bottom| bottom <= self.rows)
            && self.row_bytes().is_some_and(|row| row <= self.stride)
            && self.required().is_some_and(|required| required <= length);
        if !fits {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Decoded video frame does not match its format",
            ));
        }
        Ok(())
    }
}

/// The output size for a picture of `width`x`height` turned by `rotation`, within `bound`.
pub(super) fn fitted(
    width: u32,
    height: u32,
    rotation: u32,
    bound: (u32, u32),
) -> io::Result<(u32, u32)> {
    output_length(bound.0, bound.1)?;
    let (bound_w, bound_h) = if rotation == 90 || rotation == 270 {
        (bound.1, bound.0)
    } else {
        bound
    };
    let scale = (f64::from(bound_w) / f64::from(width))
        .min(f64::from(bound_h) / f64::from(height))
        .min(1.0);
    let scaled_w = (f64::from(width) * scale).round().max(1.0) as u32;
    let scaled_h = (f64::from(height) * scale).round().max(1.0) as u32;
    Ok((scaled_w, scaled_h))
}

/// Convert, shrink and orient one frame. `buffer` is the locked decoder output.
pub(super) fn convert(buffer: &[u8], format: &Format, bound: (u32, u32)) -> io::Result<Pixels> {
    format.check(buffer.len())?;
    let (scaled_w, scaled_h) = fitted(format.width, format.height, format.rotation, bound)?;
    let (output_w, output_h) = if format.rotation == 90 || format.rotation == 270 {
        (scaled_h, scaled_w)
    } else {
        (scaled_w, scaled_h)
    };
    let mut rgba = vec![0; output_length(output_w, output_h)?];
    let planes = Planes::new(buffer, format);
    let coefficients = Coefficients::new(format.matrix, format.full_range);
    let table = format.transfer_709.then(transfer_table);
    let (width, height) = (format.width as usize, format.height as usize);
    let (scaled_w, scaled_h) = (scaled_w as usize, scaled_h as usize);
    for out_y in 0..scaled_h {
        let y0 = out_y * height / scaled_h;
        let y1 = ((out_y + 1) * height / scaled_h).max(y0 + 1);
        for out_x in 0..scaled_w {
            let x0 = out_x * width / scaled_w;
            let x1 = ((out_x + 1) * width / scaled_w).max(x0 + 1);
            let (mut luma, mut blue, mut red) = (0u64, 0u64, 0u64);
            for y in y0..y1 {
                for x in x0..x1 {
                    let (sample_y, sample_cb, sample_cr) =
                        planes.sample(format.left as usize + x, format.top as usize + y);
                    luma += u64::from(sample_y);
                    blue += u64::from(sample_cb);
                    red += u64::from(sample_cr);
                }
            }
            let count = ((y1 - y0) * (x1 - x0)) as f64;
            let mut pixel =
                coefficients.rgb(luma as f64 / count, blue as f64 / count, red as f64 / count);
            if let Some(table) = &table {
                for component in &mut pixel {
                    *component = table[usize::from(*component)];
                }
            }
            let (target_x, target_y) = match format.rotation {
                90 => (scaled_h - out_y - 1, out_x),
                180 => (scaled_w - out_x - 1, scaled_h - out_y - 1),
                270 => (out_y, scaled_w - out_x - 1),
                _ => (out_x, out_y),
            };
            let target = (target_y * output_w as usize + target_x) * 4;
            rgba[target..target + 4].copy_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
        }
    }
    Ok(Pixels {
        width: output_w,
        height: output_h,
        rgba,
    })
}

/// 8-bit samples of a checked buffer.
struct Planes<'a> {
    buffer: &'a [u8],
    layout: Layout,
    stride: usize,
    chroma: usize,
    second: usize,
}

impl<'a> Planes<'a> {
    fn new(buffer: &'a [u8], format: &Format) -> Self {
        let stride = format.stride as usize;
        let luma = stride * format.rows as usize;
        let quarter = (stride / 2) * (format.rows as usize).div_ceil(2);
        Self {
            buffer,
            layout: format.layout,
            stride,
            chroma: luma,
            second: luma + quarter,
        }
    }

    /// (Y, Cb, Cr) at a luma position; chroma is the sample covering it.
    #[inline]
    fn sample(&self, x: usize, y: usize) -> (u8, u8, u8) {
        let b = self.buffer;
        match self.layout {
            Layout::Nv12 => {
                let chroma = self.chroma + (y / 2) * self.stride + (x / 2) * 2;
                (b[y * self.stride + x], b[chroma], b[chroma + 1])
            }
            Layout::P010 => {
                // The high byte of each little-endian 16-bit sample keeps its top eight bits.
                let chroma = self.chroma + (y / 2) * self.stride + (x / 2) * 4;
                (b[y * self.stride + x * 2 + 1], b[chroma + 1], b[chroma + 3])
            }
            Layout::I420 | Layout::Yv12 => {
                let offset = (y / 2) * (self.stride / 2) + x / 2;
                let (cb, cr) = if self.layout == Layout::I420 {
                    (self.chroma + offset, self.second + offset)
                } else {
                    (self.second + offset, self.chroma + offset)
                };
                (b[y * self.stride + x], b[cb], b[cr])
            }
            Layout::Yuy2 => {
                let pair = y * self.stride + (x / 2) * 4;
                (b[pair + (x % 2) * 2], b[pair + 1], b[pair + 3])
            }
        }
    }
}

struct Coefficients {
    luma_offset: f64,
    luma_scale: f64,
    chroma_scale: f64,
    red_cr: f64,
    green_cb: f64,
    green_cr: f64,
    blue_cb: f64,
}

impl Coefficients {
    fn new(matrix: Matrix, full_range: bool) -> Self {
        let (kr, kb) = match matrix {
            Matrix::Bt601 => (0.299, 0.114),
            Matrix::Bt709 => (0.2126, 0.0722),
        };
        let kg = 1.0 - kr - kb;
        let (luma_offset, luma_scale, chroma_scale) = if full_range {
            (0.0, 1.0, 1.0)
        } else {
            (16.0, 255.0 / 219.0, 255.0 / 224.0)
        };
        Self {
            luma_offset,
            luma_scale,
            chroma_scale,
            red_cr: 2.0 * (1.0 - kr),
            green_cb: 2.0 * (1.0 - kb) * kb / kg,
            green_cr: 2.0 * (1.0 - kr) * kr / kg,
            blue_cb: 2.0 * (1.0 - kb),
        }
    }

    fn rgb(&self, luma: f64, cb: f64, cr: f64) -> [u8; 3] {
        let y = (luma - self.luma_offset) * self.luma_scale;
        let cb = (cb - 128.0) * self.chroma_scale;
        let cr = (cr - 128.0) * self.chroma_scale;
        let clamp = |value: f64| value.round().clamp(0.0, 255.0) as u8;
        [
            clamp(y + self.red_cr * cr),
            clamp(y - self.green_cb * cb - self.green_cr * cr),
            clamp(y + self.blue_cb * cb),
        ]
    }
}

/// BT.709/BT.601 camera transfer decoded and re-encoded as sRGB, as the bundled profile does.
fn transfer_table() -> [u8; 256] {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format(layout: Layout, width: u32, height: u32, stride: u32) -> Format {
        Format {
            layout,
            rows: height,
            stride,
            left: 0,
            top: 0,
            width,
            height,
            matrix: Matrix::Bt601,
            full_range: true,
            transfer_709: false,
            rotation: 0,
        }
    }

    /// A full-range BT.601 NV12 frame whose left half is pure red and right half pure blue.
    fn halves(width: u32, height: u32) -> Vec<u8> {
        let (w, h) = (width as usize, height as usize);
        // Full-range BT.601: red is (76, 85, 255), blue is (29, 255, 107).
        let mut bytes = vec![0; w * h + w * h.div_ceil(2)];
        for y in 0..h {
            for x in 0..w {
                bytes[y * w + x] = if x < w / 2 { 76 } else { 29 };
            }
        }
        for y in 0..h.div_ceil(2) {
            for x in 0..w / 2 {
                let (cb, cr) = if x * 2 < w / 2 { (85, 255) } else { (255, 107) };
                bytes[w * h + y * w + x * 2] = cb;
                bytes[w * h + y * w + x * 2 + 1] = cr;
            }
        }
        bytes
    }

    fn near(pixel: &[u8], expected: [u8; 3]) -> bool {
        pixel[..3]
            .iter()
            .zip(expected)
            .all(|(actual, expected)| actual.abs_diff(expected) <= 3)
            && pixel[3] == 255
    }

    #[test]
    fn nv12_converts_with_its_matrix_and_shrinks_by_whole_boxes() {
        let bytes = halves(8, 4);
        let pixels = convert(&bytes, &format(Layout::Nv12, 8, 4, 8), (4, 4)).unwrap();
        assert_eq!((pixels.width, pixels.height), (4, 2));
        let rows: Vec<&[u8]> = pixels.rgba.chunks(4).collect();
        assert!(near(rows[0], [255, 0, 0]), "{:?}", rows[0]);
        assert!(near(rows[1], [255, 0, 0]), "{:?}", rows[1]);
        assert!(near(rows[2], [0, 0, 255]), "{:?}", rows[2]);
        assert!(near(rows[3], [0, 0, 255]), "{:?}", rows[3]);
    }

    #[test]
    fn quarter_turns_put_the_left_half_where_it_belongs_upright() {
        let bytes = halves(8, 4);
        for (rotation, size, red_first) in [
            (90, (4, 8), true),
            (180, (8, 4), false),
            (270, (4, 8), false),
        ] {
            let mut format = format(Layout::Nv12, 8, 4, 8);
            format.rotation = rotation;
            let pixels = convert(&bytes, &format, (8, 8)).unwrap();
            assert_eq!((pixels.width, pixels.height), size, "{rotation}");
            let first = &pixels.rgba[..4];
            let expected = if red_first { [255, 0, 0] } else { [0, 0, 255] };
            assert!(near(first, expected), "{rotation}: {first:?}");
        }
    }

    #[test]
    fn limited_range_bt709_white_and_black_are_full_scale() {
        // Limited range: Y 16..235, chroma 128 is neutral.
        let mut bytes = vec![235, 235, 16, 16];
        bytes.extend_from_slice(&[128, 128]);
        let mut format = format(Layout::Nv12, 2, 2, 2);
        format.matrix = Matrix::Bt709;
        format.full_range = false;
        let pixels = convert(&bytes, &format, (2, 2)).unwrap();
        assert!(near(&pixels.rgba[..4], [255, 255, 255]));
        assert!(near(&pixels.rgba[8..12], [0, 0, 0]));
    }

    #[test]
    fn every_layout_reads_its_own_chroma_position() {
        // A 2x2 neutral-gray frame in each layout: every output pixel is the same gray.
        let gray = |pixels: &Pixels| {
            pixels
                .rgba
                .chunks(4)
                .all(|pixel| near(pixel, [128, 128, 128]))
        };
        let cases: [(Layout, Vec<u8>, u32); 5] = [
            (Layout::Nv12, vec![128, 128, 128, 128, 128, 128], 2),
            (Layout::I420, vec![128, 128, 128, 128, 128, 128], 2),
            (Layout::Yv12, vec![128, 128, 128, 128, 128, 128], 2),
            (Layout::Yuy2, vec![128; 8], 4),
            (Layout::P010, [0u8, 128].repeat(6), 4),
        ];
        for (layout, bytes, stride) in cases {
            let pixels = convert(&bytes, &format(layout, 2, 2, stride), (2, 2)).unwrap();
            assert!(gray(&pixels), "{layout:?}: {:?}", pixels.rgba);
        }
    }

    #[test]
    fn geometry_outside_the_buffer_is_rejected_before_reading() {
        let bytes = halves(8, 4);
        let mut cropped = format(Layout::Nv12, 8, 4, 8);
        cropped.top = 1;
        assert!(convert(&bytes, &cropped, (8, 8)).is_err());
        let mut wide = format(Layout::Nv12, 8, 4, 8);
        wide.left = 2;
        assert!(convert(&bytes, &wide, (8, 8)).is_err());
        assert!(convert(&bytes[..40], &format(Layout::Nv12, 8, 4, 8), (8, 8)).is_err());
        let mut turned = format(Layout::Nv12, 8, 4, 8);
        turned.rotation = 45;
        assert!(convert(&bytes, &turned, (8, 8)).is_err());
        assert!(convert(&bytes, &format(Layout::Nv12, 8, 4, 8), (0, 8)).is_err());
    }
}

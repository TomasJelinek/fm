use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use midnight_image_viewer_sdk::{Image, api::Viewport};
use std::io::{self, Cursor, Read, Seek, SeekFrom};

/// Counts reads like the host broker: each request is clamped to the brokered
/// maximum and counts against the session's cumulative read budget.
struct Measured {
    data: Cursor<Vec<u8>>,
    calls: usize,
    largest: usize,
    requested: u64,
}

impl Measured {
    fn new(data: Vec<u8>) -> Self {
        Self {
            data: Cursor::new(data),
            calls: 0,
            largest: 0,
            requested: 0,
        }
    }
}

impl Read for Measured {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let remaining = self.data.get_ref().len() as u64 - self.data.position();
        let length = (output.len() as u64)
            .min(midnight_image_viewer_sdk::api::MAX_READ_BYTES as u64)
            .min(remaining) as usize;
        self.calls += 1;
        self.largest = self.largest.max(output.len());
        self.requested += length as u64;
        self.data.read(&mut output[..length])
    }
}

impl Seek for Measured {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.data.seek(position)
    }
}

#[test]
fn uncompressed_image_uses_bounded_broker_batches_and_render_never_rereads() {
    let original = RgbImage::from_pixel(1024, 1024, Rgb([91, 142, 203]));
    let mut data = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(original)
        .write_to(&mut data, ImageFormat::Bmp)
        .unwrap();
    let mut source = Measured::new(data.into_inner());
    let image = midnight_viewer_image::decode(&mut source).unwrap();
    assert!(source.calls <= 24, "broker reads: {}", source.calls);
    assert!(source.largest <= midnight_image_viewer_sdk::api::MAX_READ_BYTES);
    let calls = source.calls;
    let frame = image
        .render(Viewport {
            width: 1,
            height: 1,
            scale: 1.0,
            origin_x: 0.0,
            origin_y: 0.0,
        })
        .unwrap();
    assert_eq!(frame.rgba, [91, 142, 203, 255]);
    assert_eq!(source.calls, calls);
}

/// Little-endian baseline RGB TIFF with one row per strip, as libtiff writes small
/// rows by default, and its directory after the image data.
fn one_row_strip_tiff(width: u32, height: u32) -> Vec<u8> {
    let row = width as usize * 3;
    let mut tiff = b"II*\0\0\0\0\0".to_vec();
    let mut offsets = Vec::new();
    for y in 0..height {
        offsets.push(tiff.len() as u32);
        tiff.extend((0..row).map(|i| (i as u32 * 7 + y * 3) as u8));
    }
    let bits = tiff.len() as u32;
    tiff.extend([8, 0, 8, 0, 8, 0]);
    let offset_array = tiff.len() as u32;
    tiff.extend(offsets.iter().flat_map(|offset| offset.to_le_bytes()));
    let count_array = tiff.len() as u32;
    tiff.extend((0..height).flat_map(|_| (row as u32).to_le_bytes()));
    let directory = tiff.len() as u32;
    tiff[4..8].copy_from_slice(&directory.to_le_bytes());
    // Tag, type (3 SHORT, 4 LONG), count, value or offset.
    let entries: [(u16, u16, u32, u32); 9] = [
        (256, 4, 1, width),
        (257, 4, 1, height),
        (258, 3, 3, bits),
        (259, 3, 1, 1),
        (262, 3, 1, 2),
        (273, 4, height, offset_array),
        (277, 3, 1, 3),
        (278, 4, 1, 1),
        (279, 4, height, count_array),
    ];
    tiff.extend((entries.len() as u16).to_le_bytes());
    for (tag, kind, count, value) in entries {
        tiff.extend(tag.to_le_bytes());
        tiff.extend(kind.to_le_bytes());
        tiff.extend(count.to_le_bytes());
        tiff.extend(value.to_le_bytes());
    }
    tiff.extend(0u32.to_le_bytes());
    tiff
}

#[test]
fn seeking_tiff_strips_reuse_fetched_bytes_within_the_read_budget() {
    let tiff = one_row_strip_tiff(500, 400);
    let length = tiff.len() as u64;
    let mut source = Measured::new(tiff);
    let image = midnight_viewer_image::decode(&mut source).unwrap();
    assert_eq!(
        (image.metadata().width, image.metadata().height),
        (500, 400)
    );
    let pixel = image
        .render(Viewport {
            width: 1,
            height: 1,
            scale: 1.0,
            origin_x: 2.0,
            origin_y: 3.0,
        })
        .unwrap();
    assert_eq!(pixel.rgba, [51, 58, 65, 255]);
    eprintln!(
        "one-row-strip TIFF: requested {} of {length} bytes in {} reads",
        source.requested, source.calls
    );
    // A seek-discarding 256 KiB buffer requested about 400 x 256 KiB here, and
    // ordinary multi-strip TIFFs exceeded the host's 256 MiB budget.
    assert!(
        source.requested < 2 * length,
        "requested {} bytes for a {length}-byte TIFF in {} reads",
        source.requested,
        source.calls
    );
}

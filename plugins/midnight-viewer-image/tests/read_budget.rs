use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use midnight_image_viewer_sdk::{Image, api::Viewport};
use std::io::{self, Cursor, Read, Seek, SeekFrom};

struct Measured {
    data: Cursor<Vec<u8>>,
    calls: usize,
    largest: usize,
}

impl Read for Measured {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.calls += 1;
        self.largest = self.largest.max(output.len());
        self.data.read(output)
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
    data.set_position(0);
    let mut source = Measured {
        data,
        calls: 0,
        largest: 0,
    };
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

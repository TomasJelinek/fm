use image::{DynamicImage, ImageFormat, RgbaImage};
use midnight_thumbnail_sdk::api::Request;
use std::io::Cursor;

fn request() -> Request {
    Request {
        id: 7,
        source_length: 0,
        width: 10,
        height: 10,
        frame: 0,
    }
}

#[test]
fn real_png_preserves_aspect_ratio_and_transparency() {
    let mut encoded = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(RgbaImage::from_pixel(40, 20, image::Rgba([255, 0, 0, 127])))
        .write_to(&mut encoded, ImageFormat::Png)
        .unwrap();
    encoded.set_position(0);
    let result = midnight_thumbnail_image::decode(&request(), &mut encoded).unwrap();
    assert_eq!((result.width, result.height), (10, 5));
    assert_eq!(result.rgba.len(), 200);
    assert!(
        result
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| *pixel == [255, 0, 0, 127])
    );
}

#[test]
fn malformed_and_excessive_dimensions_fail_without_a_frame() {
    for bytes in [
        Vec::new(),
        b"not an image".to_vec(),
        b"GIF89a\xff\xff\xff\xff\x00\x00\x00".to_vec(),
    ] {
        assert!(midnight_thumbnail_image::decode(&request(), &mut Cursor::new(bytes)).is_err());
    }
}

#[test]
fn exif_orientation_is_applied_before_thumbnail_dimensions() {
    let mut encoded = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        40,
        20,
        image::Rgb([100, 40, 10]),
    ))
    .write_to(&mut encoded, ImageFormat::Jpeg)
    .unwrap();
    let jpeg = encoded.into_inner();
    // Minimal big-endian TIFF/EXIF IFD0 orientation=6 (rotate 90 degrees clockwise).
    let exif = b"Exif\0\0MM\0\x2a\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01\0\x06\0\0\0\0\0\0";
    let mut rotated = vec![0xff, 0xd8, 0xff, 0xe1];
    rotated.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
    rotated.extend_from_slice(exif);
    rotated.extend_from_slice(&jpeg[2..]);
    let result = midnight_thumbnail_image::decode(&request(), &mut Cursor::new(rotated)).unwrap();
    assert_eq!((result.width, result.height), (5, 10));
}

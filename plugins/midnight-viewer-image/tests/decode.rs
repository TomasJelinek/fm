use image::{DynamicImage, ImageEncoder, ImageFormat, Rgba, RgbaImage};
use midnight_image_viewer_sdk::{Image, api::Viewport};
use std::io::Cursor;

fn encoded(pixels: RgbaImage) -> Cursor<Vec<u8>> {
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(pixels)
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
    bytes.set_position(0);
    bytes
}

fn colors() -> RgbaImage {
    RgbaImage::from_raw(
        2,
        2,
        vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 0, 255, 255, 255, 255,
        ],
    )
    .unwrap()
}

fn viewport(width: u32, height: u32, scale: f64, x: f64, y: f64) -> Viewport {
    Viewport {
        width,
        height,
        scale,
        origin_x: x,
        origin_y: y,
    }
}

#[test]
fn actual_pixels_preserve_alpha_and_original_resolution() {
    let original = colors();
    let image = midnight_viewer_image::decode(&mut encoded(original.clone())).unwrap();
    assert_eq!((image.metadata().width, image.metadata().height), (2, 2));
    assert_eq!(
        image.render(viewport(2, 2, 1.0, 0.0, 0.0)).unwrap().rgba,
        original.into_raw()
    );
}

#[test]
fn zoom_pan_and_transparent_surrounding_use_retained_original_pixels() {
    let image = midnight_viewer_image::decode(&mut encoded(colors())).unwrap();
    let zoomed = image.render(viewport(4, 2, 2.0, 0.0, 0.0)).unwrap();
    assert_eq!(
        zoomed.rgba,
        [
            255, 0, 0, 255, 255, 0, 0, 255, 0, 255, 0, 255, 0, 255, 0, 255
        ]
        .repeat(2)
    );
    assert_eq!(
        image.render(viewport(1, 2, 1.0, 1.0, 0.0)).unwrap().rgba,
        [0, 255, 0, 255, 255, 255, 255, 255]
    );
    assert_eq!(
        image.render(viewport(3, 1, 1.0, -1.0, 0.0)).unwrap().rgba,
        [0, 0, 0, 0, 255, 0, 0, 255, 0, 255, 0, 255]
    );
}

#[test]
fn fit_filters_alpha_without_a_transparent_pixel_color_halo() {
    let image = midnight_viewer_image::decode(&mut encoded(colors())).unwrap();
    assert_eq!(
        image.render(viewport(1, 1, 0.5, 0.0, 0.0)).unwrap().rgba,
        [170, 170, 85, 191]
    );
}

#[test]
fn malformed_and_excessive_dimensions_fail_without_pixels() {
    for bytes in [
        vec![],
        b"not an image".to_vec(),
        b"GIF89a\xff\xff\xff\xff\x00\x00\x00".to_vec(),
    ] {
        assert!(midnight_viewer_image::decode(&mut Cursor::new(bytes)).is_err());
    }
    let image = midnight_viewer_image::decode(&mut encoded(colors())).unwrap();
    assert!(image.render(viewport(4096, 4096, 1.0, 0.0, 0.0)).is_err());
    assert!(image.render(viewport(1, 1, f64::NAN, 0.0, 0.0)).is_err());
}

#[test]
fn exif_orientation_changes_dimensions_before_viewport_sampling() {
    let original = image::RgbImage::from_pixel(40, 20, image::Rgb([100, 40, 10]));
    let mut encoded = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(original)
        .write_to(&mut encoded, ImageFormat::Jpeg)
        .unwrap();
    let jpeg = encoded.into_inner();
    let exif = b"Exif\0\0MM\0\x2a\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01\0\x06\0\0\0\0\0\0";
    let mut rotated = vec![0xff, 0xd8, 0xff, 0xe1];
    rotated.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
    rotated.extend_from_slice(exif);
    rotated.extend_from_slice(&jpeg[2..]);
    let image = midnight_viewer_image::decode(&mut Cursor::new(rotated)).unwrap();
    assert_eq!((image.metadata().width, image.metadata().height), (20, 40));
}

#[test]
fn invalid_embedded_icc_is_rejected_instead_of_claiming_srgb() {
    let mut bytes = Vec::new();
    let mut encoder = image::codecs::png::PngEncoder::new(&mut bytes);
    encoder
        .set_icc_profile(b"invalid profile".to_vec())
        .unwrap();
    encoder
        .write_image(&[255, 0, 0, 127], 1, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    assert!(midnight_viewer_image::decode(&mut Cursor::new(bytes)).is_err());
}

#[test]
fn embedded_rgb_profile_converts_to_srgb_and_preserves_alpha() {
    let mut bytes = Vec::new();
    let mut encoder = image::codecs::png::PngEncoder::new(&mut bytes);
    encoder
        .set_icc_profile(moxcms::ColorProfile::new_display_p3().encode().unwrap())
        .unwrap();
    encoder
        .write_image(&[160, 100, 40, 127], 1, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    let image = midnight_viewer_image::decode(&mut Cursor::new(bytes)).unwrap();
    let rgba = image.render(viewport(1, 1, 1.0, 0.0, 0.0)).unwrap().rgba;
    assert_eq!(rgba[3], 127);
    assert!(rgba[0] > 160 && rgba[1] < 100 && rgba[2] < 40, "{rgba:?}");
}

#[test]
fn gif_exposes_only_its_first_still_frame() {
    let mut encoded = Vec::new();
    {
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut encoded);
        for color in [Rgba([255, 0, 0, 255]), Rgba([0, 255, 0, 255])] {
            encoder
                .encode_frame(image::Frame::new(RgbaImage::from_pixel(2, 1, color)))
                .unwrap();
        }
    }
    let image = midnight_viewer_image::decode(&mut Cursor::new(encoded)).unwrap();
    assert_eq!(
        image.render(viewport(2, 1, 1.0, 0.0, 0.0)).unwrap().rgba,
        [255, 0, 0, 255, 255, 0, 0, 255]
    );
}

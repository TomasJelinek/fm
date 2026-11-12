//! Independent scalar framebuffer oracle for mask coverage, clipping and fallback content.
use super::*;

fn reference(raster: &mut Raster, image: &Image, x: f64, y: f64, brush: [u8; 4]) {
    let left = (x + f64::from(image.placement.left)).floor() as i64;
    let top = (y - f64::from(image.placement.top)).floor() as i64;
    for row in 0..i64::from(raster.height) {
        for column in 0..i64::from(raster.width) {
            let source_x = column - left;
            let source_y = row - top;
            if source_x < 0
                || source_y < 0
                || source_x >= i64::from(image.placement.width)
                || source_y >= i64::from(image.placement.height)
            {
                continue;
            }
            let index = (source_y as usize * image.placement.width as usize) + source_x as usize;
            let color = match image.content {
                Content::Mask => {
                    let alpha =
                        ((u16::from(image.data[index]) * u16::from(brush[3]) + 127) / 255) as u8;
                    [
                        ((u16::from(brush[0]) * u16::from(alpha) + 127) / 255) as u8,
                        ((u16::from(brush[1]) * u16::from(alpha) + 127) / 255) as u8,
                        ((u16::from(brush[2]) * u16::from(alpha) + 127) / 255) as u8,
                        alpha,
                    ]
                }
                Content::Color => {
                    let mut pixel: [u8; 4] =
                        image.data[index * 4..index * 4 + 4].try_into().unwrap();
                    if !matches!(image.source, Source::ColorOutline(_)) {
                        for channel in &mut pixel[..3] {
                            *channel =
                                ((u16::from(*channel) * u16::from(image.data[index * 4 + 3]) + 127)
                                    / 255) as u8;
                        }
                    }
                    pixel
                }
                Content::SubpixelMask => continue,
            };
            let at = (row as usize * raster.width as usize + column as usize) * 4;
            for channel in 0..4 {
                raster.rgba[at + channel] = (u16::from(color[channel])
                    + (u16::from(raster.rgba[at + channel]) * (255 - u16::from(color[3])) + 127)
                        / 255)
                    .min(255) as u8;
            }
        }
    }
}

fn compare(image: &Image, brush: [u8; 4]) {
    for background in [[0; 4], [11, 23, 47, 101], [25, 29, 37, 255]] {
        for (x, y) in [
            (0.0, 0.0),
            (3.0, 4.0),
            (-7.0, -5.0),
            (18.0, 17.0),
            (-0.25, 0.75),
            (5.999, 7.001),
            (1000.0, 0.0),
            (-1000.0, 0.0),
            (0.0, 1000.0),
            (0.0, -1000.0),
        ] {
            let mut actual = Raster {
                width: 23,
                height: 21,
                rgba: background.repeat(23 * 21),
            };
            let mut expected = Raster {
                width: actual.width,
                height: actual.height,
                rgba: actual.rgba.clone(),
            };
            paint_image(&mut actual, image, x, y, brush);
            reference(&mut expected, image, x, y, brush);
            assert_eq!(
                actual.rgba, expected.rgba,
                "content={:?} brush={brush:?} background={background:?} x={x} y={y}",
                image.content
            );
        }
    }
}

#[test]
fn all_mask_coverages_match_scalar_rounding_and_clipping() {
    let mut image = Image {
        source: Source::Outline,
        content: Content::Mask,
        placement: swash::zeno::Placement {
            left: -2,
            top: 3,
            width: 16,
            height: 16,
        },
        data: (0..=255).collect(),
    };
    for alpha in [0, 1, 77, 127, 128, 254, 255] {
        for rgb in [[0, 0, 0], [255, 255, 255], [32, 89, 170], [255, 1, 127]] {
            compare(&image, [rgb[0], rgb[1], rgb[2], alpha]);
        }
    }
    // Empty masks and bearings can never write outside the destination.
    image.placement.width = 0;
    image.data.clear();
    compare(&image, [255; 4]);
    image.placement.width = 16;
    image.placement.height = 0;
    compare(&image, [255; 4]);
}

#[test]
fn color_bitmap_and_subpixel_fallback_keep_the_existing_pixel_contract() {
    for (content, source) in [
        (Content::Color, Source::ColorOutline(0)),
        (Content::Color, Source::ColorBitmap(StrikeWith::BestFit)),
        (Content::SubpixelMask, Source::Outline),
    ] {
        let mut data = Vec::new();
        for alpha in 0..=255 {
            let straight = [32, 89, 170, alpha];
            let pixel = if matches!(source, Source::ColorOutline(_)) {
                premultiply(straight)
            } else {
                straight
            };
            data.extend(pixel);
        }
        compare(
            &Image {
                source,
                content,
                placement: swash::zeno::Placement {
                    left: 2,
                    top: -1,
                    width: 16,
                    height: 16,
                },
                data,
            },
            [17, 197, 253, 77],
        );
    }
}

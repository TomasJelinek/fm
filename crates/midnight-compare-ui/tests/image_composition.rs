use slint::{
    ComponentHandle, Rgba8Pixel, SharedPixelBuffer,
    platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
mod support;
use support::TestPlatform;
slint::slint! {
    import { ImageComparison } from "../ui/image-comparison.slint";
    export component CompositionTest inherits Window {
        in property <float> pixel-width: 24;
        width: root.pixel-width * 1px; height: 24px;
        in property <image> left;
        in property <image> right;
        in-out property <int> mode;
        in-out property <float> amount: 0.5;
        in property <float> split-position: 0.5;
        ImageComparison { left-source: root.left; right-source: root.right; mode: root.mode; amount: root.amount; split-position: root.split-position; }
    }
}
#[test]
fn retained_source_images_have_exact_split_swipe_and_onion_endpoints() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = CompositionTest::new().unwrap();
    let image =
        |pixel: [u8; 4]| {
            slint::Image::from_rgba8_premultiplied(
                SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&pixel.repeat(24 * 24), 24, 24),
            )
        };
    window.set_left(image([255, 0, 0, 255]));
    window.set_right(image([0, 0, 255, 255]));
    window.show().unwrap();
    native.set_size(slint::PhysicalSize::new(24, 24));
    let mut pixels = vec![slint::Rgb8Pixel::default(); 24 * 24];
    for (mode, amount, left, right) in [
        (0, 0.5, [255u8, 0, 0], [0u8, 0, 255]),
        (1, 0.0, [0, 0, 255], [0, 0, 255]),
        (1, 0.25, [0, 0, 255], [0, 0, 255]),
        (1, 0.5, [255, 0, 0], [0, 0, 255]),
        (1, 1.0, [255, 0, 0], [255, 0, 0]),
        (2, 0.0, [255, 0, 0], [255, 0, 0]),
        (2, 0.5, [128, 0, 128], [128, 0, 128]),
        (2, 1.0, [0, 0, 255], [0, 0, 255]),
    ] {
        window.set_mode(mode);
        window.set_amount(amount);
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 24);
        });
        for (x, expected) in [(6, left), (18, right)] {
            let actual = pixels[12 * 24 + x];
            for (got, want) in [actual.r, actual.g, actual.b].into_iter().zip(expected) {
                assert!(
                    i16::from(got).abs_diff(i16::from(want)) <= 1,
                    "Mode {mode}/{amount}: pixel {x}: {actual:?}"
                );
            }
        }
        if mode == 1 && amount == 0.25 {
            let edge = pixels[12 * 24 + 2];
            assert_eq!(
                [edge.r, edge.g, edge.b],
                [255, 0, 0],
                "Swipe must reveal from the left edge"
            );
        }
    }
    // An odd physical viewport must not stretch either half at actual-pixel zoom.
    let gradient = |width: u32, side: usize| {
        let mut bytes = Vec::new();
        for _ in 0..24 {
            for x in 0..width {
                let mut pixel = [0u8, 0, 0, 255];
                pixel[side] = (x * 10) as u8;
                bytes.extend(pixel);
            }
        }
        slint::Image::from_rgba8_premultiplied(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
            &bytes, width, 24,
        ))
    };
    window.set_left(gradient(12, 0));
    window.set_right(gradient(13, 1));
    window.set_mode(0);
    window.set_split_position(12.0 / 25.0);
    window.set_pixel_width(25.0);
    native.set_size(slint::PhysicalSize::new(25, 24));
    window
        .window()
        .dispatch_event(slint::platform::WindowEvent::Resized {
            size: slint::LogicalSize::new(25.0, 24.0),
        });
    let mut odd = vec![slint::Rgb8Pixel::default(); 25 * 24];
    native.draw_if_needed(|renderer| {
        renderer.render(&mut odd, 25);
    });
    for x in 0..25 {
        let expected = if x < 12 {
            [(x * 10) as u8, 0, 0]
        } else {
            [0, ((x - 12) * 10) as u8, 0]
        };
        let actual = odd[12 * 25 + x];
        assert_eq!(
            [actual.r, actual.g, actual.b],
            expected,
            "Odd-width pixel {x} changed scale"
        );
    }
}

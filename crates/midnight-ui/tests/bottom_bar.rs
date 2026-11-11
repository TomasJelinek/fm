//! Rendered footer contents must share the vertical center at every supported density.
use i_slint_core::{
    item_tree::ItemRc,
    items::{ComplexText, SimpleText},
    window::WindowInner,
};
use midnight_ui::{ActionIcons, MainWindow, ShortcutHints, Theme};
use slint::{
    ComponentHandle, Image, Rgb8Pixel, SharedPixelBuffer,
    platform::{
        Platform, PlatformError, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::rc::Rc;

// Large-font icons are intentionally omitted in 98px cells at the old 1340px width.
// Give the icon-paint contract enough actual window space at every tested font.
const WIDTH: usize = 3400;
const HEIGHT: usize = 820;
const NATIVE_RGB: [u8; 3] = [239, 31, 73];

struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

#[derive(Debug)]
struct Bounds {
    left: usize,
    right: usize,
    top: usize,
    bottom: usize,
    pixels: usize,
}
impl Bounds {
    fn width(&self) -> f32 {
        (self.right - self.left + 1) as f32
    }
    fn height(&self) -> f32 {
        (self.bottom - self.top + 1) as f32
    }
    fn center_y(&self) -> f32 {
        (self.top + self.bottom + 1) as f32 / 2.0
    }
}

fn bounds(
    pixels: &[Rgb8Pixel],
    stride: usize,
    xs: std::ops::Range<usize>,
    ys: std::ops::Range<usize>,
    matches: impl Fn(Rgb8Pixel) -> bool,
) -> Option<Bounds> {
    let mut result: Option<Bounds> = None;
    for y in ys {
        for x in xs.clone() {
            if !matches(pixels[y * stride + x]) {
                continue;
            }
            match &mut result {
                Some(found) => {
                    found.left = found.left.min(x);
                    found.right = found.right.max(x);
                    found.top = found.top.min(y);
                    found.bottom = found.bottom.max(y);
                    found.pixels += 1;
                }
                empty => {
                    *empty = Some(Bounds {
                        left: x,
                        right: x,
                        top: y,
                        bottom: y,
                        pixels: 1,
                    });
                }
            }
        }
    }
    result
}

fn visible_caption(item: ItemRc) -> Option<ItemRc> {
    let matches = item
        .downcast::<SimpleText>()
        .is_some_and(|text| text.as_pin_ref().text() == "F1 Help")
        || item
            .downcast::<ComplexText>()
            .is_some_and(|text| text.as_pin_ref().text() == "F1 Help");
    if matches && item.is_visible() && item.geometry().height() > 0.0 {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(caption) = visible_caption(item) {
            return Some(caption);
        }
    }
    None
}

fn assert_caption_line_box(window: &MainWindow, center: f32, height: f32) {
    let root = ItemRc::new_root(WindowInner::from_pub(window.window()).component());
    let caption = visible_caption(root).expect("visible Help caption");
    let geometry = caption.geometry();
    let origin = caption.map_to_native_window(geometry.origin);
    assert!(
        (origin.y + geometry.height() / 2.0 - center).abs() <= 0.01,
        "caption line box must be exactly centered independently of glyph ink"
    );
    assert!(
        (geometry.height() - height).abs() <= 0.01,
        "caption line box must retain the full footer height"
    );
}

fn ink_center_tolerance(font: i32) -> f32 {
    // Bounds measure inclusive pixels, so their centers advance in 0.5px steps.
    // Round the existing 15% / 3px metric allowance to that measurement grid.
    // For font32 this is 5px, rather than an unrepresentable 4.8px threshold.
    ((font as f32 * 0.15).max(3.0) * 2.0).ceil() / 2.0
}

fn rgb(color: slint::Color) -> Rgb8Pixel {
    Rgb8Pixel {
        r: color.red(),
        g: color.green(),
        b: color.blue(),
    }
}

fn solid_native(width: u32, height: u32) -> Image {
    let bytes: Vec<_> = (0..width * height).flat_map(|_| NATIVE_RGB).collect();
    Image::from_rgb8(SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(
        &bytes, width, height,
    ))
}

#[test]
fn footer_icons_and_labels_render_centered_without_resizing_or_tinting_native_images() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = MainWindow::new().unwrap();
    midnight_ui::install_file_keys(&window);
    assert_eq!(
        window
            .global::<ShortcutHints>()
            .invoke_function_key(0, 0, false)
            .action,
        "app.help",
        "render the real enabled Help slot"
    );
    window.show().unwrap();
    native.set_size(slint::PhysicalSize::new(WIDTH as u32, HEIGHT as u32));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(WIDTH as f32, HEIGHT as f32),
    });
    let theme = window.global::<Theme>();
    let mut pixels = vec![Rgb8Pixel::default(); WIDTH * HEIGHT];
    for (density, font) in [(0, 8), (1, 8), (0, 11), (1, 11), (1, 15), (0, 32), (1, 32)] {
        window.invoke_apply_fonts("".into(), font, font);
        window.invoke_apply_preferences(0, density);
        let top = HEIGHT as f32 - theme.get_window_padding() - theme.get_bottom_height();
        let bottom = top + theme.get_bottom_height();
        let center = (top + bottom) / 2.0;
        // Include the surrounding gap so containment assertions detect escaped
        // paint, but stop before the neighboring command row. A font-sized margin
        // includes its shell icon at large fonts and misidentifies that as label ink.
        let band = (top - theme.get_gap() / 2.0).floor().max(0.0) as usize
            ..(bottom + theme.get_window_padding() / 2.0)
                .ceil()
                .min(HEIGHT as f32) as usize;
        for (source_width, source_height) in [(1, 1), (2, 1), (1, 2)] {
            window.set_action_icons(ActionIcons {
                help: solid_native(source_width, source_height),
                ..Default::default()
            });
            native.draw_if_needed(|renderer| {
                renderer.render(&mut pixels, WIDTH);
            });
            let image = bounds(&pixels, WIDTH, 0..WIDTH, band.clone(), |pixel| {
                [pixel.r, pixel.g, pixel.b] == NATIVE_RGB
            })
            .unwrap_or_else(|| panic!(
                "enabled Help must preserve native RGB: density={density}, font={font}, source={source_width}x{source_height}, window={WIDTH}x{HEIGHT}"
            ));
            assert!(
                (image.center_y() - center).abs() <= 0.75,
                "icon must be vertically centered: density={density}, font={font}, source={source_width}x{source_height}, icon={image:?}, button_center={center}"
            );
            let scale = theme.get_icon_size() / source_width.max(source_height) as f32;
            let expected_width = source_width as f32 * scale;
            let expected_height = source_height as f32 * scale;
            assert!(
                (image.width() - expected_width).abs() <= 1.0
                    && (image.height() - expected_height).abs() <= 1.0,
                "native aspect ratio and themed icon dimensions must remain intact: {image:?}"
            );
            assert!(
                image.pixels as f32 >= expected_width * expected_height * 0.6,
                "the native image's full interior must retain its RGB"
            );
            assert!(
                image.top as f32 > top && (image.bottom as f32) < bottom,
                "native icon paint must stay within the existing button height"
            );

            // Locate the first actual inter-button gap rather than assuming
            // uniform button widths; captions have different preferred widths.
            let background = rgb(theme.get_window_background());
            let label_left = (theme.get_window_padding() + 8.0 + theme.get_icon_size()) as usize;
            let right = (label_left..WIDTH)
                .find(|x| pixels[center as usize * WIDTH + x] == background)
                .expect("the Help button must end before the next function button");
            let foreground = rgb(theme.get_text());
            // At 8px, glyphs can consist mostly of partially covered pixels.
            // Include those without admitting the much darker button background;
            // retain the original mask for the default and large-font cases.
            let ink_tolerance = if font < 11 { 80 } else { 40 };
            let label = bounds(&pixels, WIDTH, label_left..right, band.clone(), |pixel| {
                pixel.r.abs_diff(foreground.r) <= ink_tolerance
                    && pixel.g.abs_diff(foreground.g) <= ink_tolerance
                    && pixel.b.abs_diff(foreground.b) <= ink_tolerance
            })
            .expect("the Help label must remain visible");
            assert!(
                label.height() >= font as f32 * 0.4,
                "a function key must remain identifiable instead of an ellipsis alone: density={density}, font={font}, source={source_width}x{source_height}, label={label:?}"
            );
            // Glyph ink is asymmetric (ascenders/descenders and elision). Its
            // center has a small font-relative tolerance, unlike the image box.
            // Small-font hinting can place Segoe UI's ink 2.5px off the centered
            // line box. Keep a 3px raster/metric floor and scale for larger fonts.
            assert_caption_line_box(&window, center, theme.get_bottom_height());
            let text_tolerance = ink_center_tolerance(font);
            assert!(
                (label.center_y() - center).abs() <= text_tolerance,
                "label must be visually centered: density={density}, font={font}, label={label:?}, button_center={center}"
            );
            assert!(
                label.top as f32 > top && (label.bottom as f32) < bottom,
                "label ink must stay within the existing button height"
            );
        }
    }
    // At the original width, the large-font Help caption deliberately gets the
    // icon's space. Verify this responsive mode separately from native icon paint.
    const COMPACT_WIDTH: usize = 1340;
    native.set_size(slint::PhysicalSize::new(
        COMPACT_WIDTH as u32,
        HEIGHT as u32,
    ));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(COMPACT_WIDTH as f32, HEIGHT as f32),
    });
    let mut pixels = vec![Rgb8Pixel::default(); COMPACT_WIDTH * HEIGHT];
    for density in [0, 1] {
        window.invoke_apply_fonts("".into(), 32, 32);
        window.invoke_apply_preferences(0, density);
        window.set_action_icons(ActionIcons {
            help: solid_native(1, 1),
            ..Default::default()
        });
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, COMPACT_WIDTH);
        });
        let top = HEIGHT as f32 - theme.get_window_padding() - theme.get_bottom_height();
        let bottom = top + theme.get_bottom_height();
        let center = (top + bottom) / 2.0;
        let band = top.floor() as usize..bottom.ceil() as usize;
        assert!(
            bounds(
                &pixels,
                COMPACT_WIDTH,
                0..COMPACT_WIDTH,
                band.clone(),
                |pixel| { [pixel.r, pixel.g, pixel.b] == NATIVE_RGB }
            )
            .is_none(),
            "large-font fixed cells must reserve their limited width for the action caption: density={density}"
        );
        let background = rgb(theme.get_window_background());
        let label_left = (theme.get_window_padding() + 8.0) as usize;
        let right = (label_left..COMPACT_WIDTH)
            .find(|x| pixels[center as usize * COMPACT_WIDTH + x] == background)
            .expect("the compact Help button must end before its neighbor");
        let foreground = rgb(theme.get_text());
        let label = bounds(&pixels, COMPACT_WIDTH, label_left..right, band, |pixel| {
            pixel.r.abs_diff(foreground.r) <= 40
                && pixel.g.abs_diff(foreground.g) <= 40
                && pixel.b.abs_diff(foreground.b) <= 40
        })
        .expect("the compact Help caption must remain visible");
        assert!(
            label.height() >= 32.0 * 0.4,
            "the action must remain readable: {label:?}"
        );
        assert_caption_line_box(&window, center, theme.get_bottom_height());
        assert!(
            (label.center_y() - center).abs() <= ink_center_tolerance(32),
            "compact caption must remain vertically centered: density={density}, label={label:?}"
        );
        assert!(label.top as f32 > top && (label.bottom as f32) < bottom);
    }
    window.hide().unwrap();
}

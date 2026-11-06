//! Rendered footer contents must share the vertical center at every supported density.
use midnight_ui::{ActionIcons, MainWindow, Theme};
use slint::{
    ComponentHandle, Image, Rgb8Pixel, SharedPixelBuffer,
    platform::{
        Platform, PlatformError, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::rc::Rc;

const WIDTH: usize = 1340;
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
    xs: std::ops::Range<usize>,
    ys: std::ops::Range<usize>,
    matches: impl Fn(Rgb8Pixel) -> bool,
) -> Option<Bounds> {
    let mut result: Option<Bounds> = None;
    for y in ys {
        for x in xs.clone() {
            if !matches(pixels[y * WIDTH + x]) {
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
            let image = bounds(&pixels, 0..WIDTH, band.clone(), |pixel| {
                [pixel.r, pixel.g, pixel.b] == NATIVE_RGB
            })
            .expect("the enabled Help icon must preserve the native RGB pixels");
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
            let label = bounds(&pixels, label_left..right, band.clone(), |pixel| {
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
            let text_tolerance = (font as f32 * 0.15).max(3.0);
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
    window.hide().unwrap();
}

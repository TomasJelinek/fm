//! Verify compact panel details, fixed function-button height and size warnings.
#[path = "support/ui.rs"]
mod ui;

use fm_ui::{FileRow, MainWindow, Theme};
use i_slint_core::{
    item_tree::ItemRc,
    items::{AccessibleRole, ComplexText, SimpleText},
};
use slint::{
    ComponentHandle, ModelRc, VecModel,
    platform::{
        Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{cell::Cell, rc::Rc};

struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}
fn click(window: &MainWindow, x: f32, y: f32) {
    let position = slint::LogicalPosition::new(x, y);
    let button = PointerEventButton::Left;
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    window
        .window()
        .dispatch_event(WindowEvent::PointerPressed { position, button });
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased { position, button });
}
fn pixel(color: slint::Color) -> slint::Rgb8Pixel {
    slint::Rgb8Pixel {
        r: color.red(),
        g: color.green(),
        b: color.blue(),
    }
}

fn size_text(item: ItemRc, expected: &str) -> Option<ItemRc> {
    if item
        .downcast::<SimpleText>()
        .is_some_and(|text| text.as_pin_ref().text() == expected)
        || item
            .downcast::<ComplexText>()
            .is_some_and(|text| text.as_pin_ref().text() == expected)
    {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = size_text(item, expected) {
            return Some(found);
        }
    }
    None
}
fn luminance(color: slint::Color) -> f32 {
    let channel = |value: u8| {
        let value = value as f32 / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(color.red()) + 0.7152 * channel(color.green()) + 0.0722 * channel(color.blue())
}

#[test]
fn compact_geometry_and_unfinished_sizes_render_in_every_theme() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = MainWindow::new().unwrap();
    fm_ui::install_file_keys(&window);
    window.set_left_status("100 items · status is available in the path tooltip".into());
    window.set_desktop_status("Finished successfully".into());
    window.set_left_rows(ModelRc::new(VecModel::from(
        (0..100)
            .map(|index| FileRow {
                identity: format!("row-{index}").into(),
                name: format!("Row {index}").into(),
                size: match index {
                    1 => "…",
                    2 => "≈ 7 KiB",
                    _ => "7 KiB",
                }
                .into(),
                size_state: match index {
                    1 => 1,
                    2 => 2,
                    _ => 0,
                },
                size_hint: "Unfinished 文档: scan cancelled".into(),
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    )));
    let selected = Rc::new(Cell::new((-1, -1)));
    let captured = selected.clone();
    window.on_row_click(move |pane, row, _, _| captured.set((pane, row)));
    window.show().unwrap();
    native.set_size(slint::PhysicalSize::new(1340, 820));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(1340.0, 820.0),
    });
    let mut pixels = vec![slint::Rgb8Pixel::default(); 1340 * 820];
    for density in 0..2 {
        for scheme in 0..3 {
            window.invoke_apply_preferences(scheme, density);
            native.draw_if_needed(|renderer| {
                renderer.render(&mut pixels, 1340);
            });
            let theme = window.global::<Theme>();
            assert_eq!(
                theme.get_bottom_height(),
                if density == 0 { 22.0 } else { 40.0 }
            );
            if density == 1 {
                assert!(theme.get_control_height() >= 40.0);
            }
            let top = window.get_left_list_top();
            let warning = pixel(theme.get_pending_size());
            for (row, expected_text) in ["7 KiB", "…", "≈ 7 KiB"].into_iter().enumerate() {
                let row_item =
                    ui::root_item(&window, &format!("Row {row}"), AccessibleRole::ListItem);
                let size_item = size_text(row_item, expected_text).expect("the row's size caption");
                let rect = size_item.geometry();
                let origin = size_item.map_to_native_window(rect.origin);
                assert!(origin.x >= 0.0 && origin.y >= top);
                assert!(
                    origin.x + rect.size.width <= 1340.0 && origin.y + rect.size.height <= 820.0
                );
                let x1 = origin.x.floor() as usize;
                let x2 = (origin.x + rect.size.width).ceil() as usize;
                let y1 = origin.y.floor() as usize;
                let y2 = (origin.y + rect.size.height).ceil() as usize;
                let warning_pixels = (y1..y2)
                    .flat_map(|y| &pixels[y * 1340 + x1..y * 1340 + x2])
                    .filter(|value| {
                        value.r.abs_diff(warning.r) <= 10
                            && value.g.abs_diff(warning.g) <= 10
                            && value.b.abs_diff(warning.b) <= 10
                    })
                    .count();
                if row == 1 && warning_pixels == 0 {
                    let mut dump = b"P6\n1340 820\n255\n".to_vec();
                    dump.extend(pixels.iter().flat_map(|value| [value.r, value.g, value.b]));
                    let artifacts = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../../_build/qa/interaction");
                    std::fs::create_dir_all(&artifacts).unwrap();
                    std::fs::write(artifacts.join("compact-feedback-debug.ppm"), dump).unwrap();
                }
                assert_eq!(
                    warning_pixels > 0,
                    row == 1,
                    "only active calculations use warning color; completed partials stay calm; theme={scheme} density={density} row={row}"
                );
            }
            for background in [
                theme.get_pane_background(),
                theme.get_alternate_row(),
                theme.get_cursor_background(),
                theme.get_inactive_cursor(),
                theme.get_selected_background(),
            ] {
                let fg = luminance(theme.get_pending_size());
                let bg = luminance(background);
                let contrast = (fg.max(bg) + 0.05) / (fg.min(bg) + 0.05);
                assert!(
                    contrast >= 4.5,
                    "unfinished-size text contrast is {contrast} in theme {scheme}"
                );
            }
            let pane_bottom = window.get_left_details_top();
            selected.set((-1, -1));
            click(&window, 100.0, pane_bottom - 3.0);
            assert_eq!(
                selected.get(),
                (
                    0,
                    ((pane_bottom - 3.0 - top) / theme.get_row_height()) as i32
                ),
                "files fill the space above panel details, and success feedback must not consume height"
            );
            // The function keys must paint within the advertised 22/40px band.
            let x = 30usize;
            let y = (820.0 - theme.get_window_padding() - theme.get_bottom_height()) as usize;
            assert_eq!(pixels[(y + 2) * 1340 + x], pixel(theme.get_surface()));
            assert_eq!(
                pixels[(y - 2) * 1340 + x],
                pixel(theme.get_window_background())
            );
        }
    }
    // The longer keyboard reference must remain dismissible in the smallest
    // supported window, including the 40px touch Close button.
    native.set_size(slint::PhysicalSize::new(1040, 480));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(1040.0, 480.0),
    });
    pixels.resize(1040 * 480, slint::Rgb8Pixel::default());
    for density in 0..2 {
        window.invoke_apply_preferences(0, density);
        let height = window.get_effective_minimum_height().ceil().max(480.0) as usize;
        native.set_size(slint::PhysicalSize::new(1040, height as u32));
        window.window().dispatch_event(WindowEvent::Resized {
            size: slint::LogicalSize::new(1040.0, height as f32),
        });
        pixels.resize(1040 * height, slint::Rgb8Pixel::default());
        window.invoke_focus_files();
        for pressed in [true, false] {
            window.window().dispatch_event(if pressed {
                WindowEvent::KeyPressed {
                    text: slint::platform::Key::F1.into(),
                }
            } else {
                WindowEvent::KeyReleased {
                    text: slint::platform::Key::F1.into(),
                }
            });
        }
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 1040);
        });
        assert_eq!(
            pixels[300 * 1040 + 220],
            pixel(window.global::<Theme>().get_surface())
        );
        ui::click_popup(&window, "Close", AccessibleRole::Button);
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 1040);
        });
        assert_ne!(
            pixels[300 * 1040 + 220],
            pixel(window.global::<Theme>().get_surface()),
            "Help Close must stay visible at minimum window size; density={density}"
        );
    }
    window.hide().unwrap();
}

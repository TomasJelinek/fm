//! Toolbar regressions use actual widget bounds and pointer input.
use i_slint_core::{
    accessibility::AccessibleStringProperty, item_tree::ItemRc, window::WindowInner,
};
use midnight_viewer_ui::ViewerWindow;
use slint::{
    ComponentHandle,
    platform::{
        Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

fn find(item: ItemRc, label: &str) -> Option<ItemRc> {
    if item
        .accessible_string_property(AccessibleStringProperty::Label)
        .is_some_and(|value| value == label)
    {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = find(item, label) {
            return Some(found);
        }
    }
    None
}

fn draw(
    native: &MinimalSoftwareWindow,
    window: &ViewerWindow,
    width: u32,
    height: u32,
    name: &str,
) {
    native.set_size(slint::PhysicalSize::new(width, height));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(width as f32, height as f32),
    });
    let mut pixels = vec![slint::Rgb8Pixel::default(); (width * height) as usize];
    let start = Instant::now();
    loop {
        slint::platform::update_timers_and_animations();
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, width as usize);
        });
        if !window.window().has_active_animations() {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "Widget animations did not settle"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    if let Some(directory) = std::env::var_os("ME_VIEWER_POLISH_SCREENSHOTS") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        let mut data = format!("P6\n{width} {height}\n255\n").into_bytes();
        for pixel in pixels {
            data.extend([pixel.r, pixel.g, pixel.b]);
        }
        std::fs::write(directory.join(format!("{name}-{width}x{height}.ppm")), data).unwrap();
    }
}

fn control(window: &ViewerWindow, label: &str, width: f32, height: f32) -> ItemRc {
    let item = find(
        ItemRc::new_root(WindowInner::from_pub(window.window()).component()),
        label,
    )
    .unwrap_or_else(|| panic!("Missing {label}"));
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    assert!(
        origin.x >= 0.0
            && origin.y >= 0.0
            && origin.x + rect.size.width <= width + 0.5
            && origin.y + rect.size.height <= height + 0.5,
        "{label} outside {width}x{height}: {origin:?}, {rect:?}"
    );
    item
}

#[test]
fn compact_pdf_keeps_large_page_counter_and_zoom_actions_clickable() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = ViewerWindow::new().unwrap();
    window.set_image_mode(true);
    window.set_document_page(9999);
    window.set_document_pages(10000);
    window.set_zoom_percent(1600.0);
    window.set_status("Page 9999 / 10000 · 1600%".into());
    let turns = Rc::new(RefCell::new(Vec::new()));
    let capture = turns.clone();
    window.on_document_turn(move |delta| capture.borrow_mut().push(delta));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let capture = commands.clone();
    window.on_image_command(move |command| capture.borrow_mut().push(command));
    window.show().unwrap();
    for light in [false, true] {
        window.invoke_apply_palette(light);
        window.set_background_color(if light {
            slint::Color::from_rgb_u8(248, 248, 248)
        } else {
            slint::Color::from_rgb_u8(24, 28, 36)
        });
        window.set_foreground(if light {
            slint::Color::from_rgb_u8(24, 28, 36)
        } else {
            slint::Color::from_rgb_u8(240, 240, 240)
        });
        for (width, height) in [(320, 200), (1000, 720)] {
            window.set_compact(width < 500);
            draw(
                &native,
                &window,
                width,
                height,
                if light { "pdf-light" } else { "pdf-dark" },
            );
            for label in [
                "Previous document page (Ctrl+PageUp)",
                "Next document page (Ctrl+PageDown)",
                "Fit image to window",
                "Actual image pixels",
                "Zoom image out",
                "Zoom image in",
            ] {
                let item = control(&window, label, width as f32, height as f32);
                let rect = item.geometry();
                let origin = item.map_to_native_window(rect.origin);
                let position = slint::LogicalPosition::new(
                    origin.x + rect.size.width / 2.0,
                    origin.y + rect.size.height / 2.0,
                );
                for event in [
                    WindowEvent::PointerMoved { position },
                    WindowEvent::PointerPressed {
                        position,
                        button: PointerEventButton::Left,
                    },
                    WindowEvent::PointerReleased {
                        position,
                        button: PointerEventButton::Left,
                    },
                ] {
                    window.window().dispatch_event(event);
                }
            }
            assert!(
                window.get_content_height() > 40.0,
                "PDF must retain a useful viewport"
            );
        }
    }
    assert_eq!(*turns.borrow(), [-1, 1, -1, 1, -1, 1, -1, 1]);
    assert_eq!(*commands.borrow(), [0, 1, 3, 2].repeat(4));
    window.hide().unwrap();
}

#[test]
fn larger_ui_font_grows_typography_widgets_without_changing_document_font() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = ViewerWindow::new().unwrap();
    window.invoke_apply_palette(false);
    window.set_source_mode(true);
    window.set_viewer_font_family("Consolas".into());
    window.set_viewer_font_size(14);
    window.set_ui_font_size(32.0);
    window.show().unwrap();
    draw(&native, &window, 1000, 720, "source-large-font");
    let font = control(&window, "Viewer font family", 1000.0, 720.0);
    assert!(
        font.geometry().size.height >= 40.0,
        "Large UI fonts need taller native font controls: {:?}",
        font.geometry()
    );
    assert_eq!(window.get_viewer_font_family(), "Consolas");
    assert_eq!(window.get_viewer_font_size(), 14);
    window.hide().unwrap();
}

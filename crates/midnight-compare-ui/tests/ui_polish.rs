//! Actual comparison actions must remain reachable at minimum size.
mod support;
use midnight_compare_ui::{ComparatorWindow, ComparatorWindows};
use midnight_core::{FileCompareLaunch, FileCompareSource, ViewerAppearance};
use slint::{
    ComponentHandle,
    platform::{
        PointerEventButton, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
    private_unstable_api::re_exports::{AccessibleStringProperty, ItemRc, WindowInner},
};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

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
    window: &ComparatorWindow,
    width: u32,
    height: u32,
    name: &str,
) {
    native.set_size(slint::PhysicalSize::new(width, height));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(width as f32, height as f32),
    });
    window.set_compact(width < 500);
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
    if let Some(directory) = std::env::var_os("ME_COMPARE_POLISH_SCREENSHOTS") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        let mut data = format!("P6\n{width} {height}\n255\n").into_bytes();
        for pixel in pixels {
            data.extend([pixel.r, pixel.g, pixel.b]);
        }
        std::fs::write(directory.join(format!("{name}-{width}x{height}.ppm")), data).unwrap();
    }
}

#[test]
fn comparison_actions_fit_small_window_and_do_not_expand_across_normal_window() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(support::TestPlatform(native.clone()))).unwrap();
    let window = ComparatorWindow::new().unwrap();
    let changes = Rc::new(RefCell::new(Vec::new()));
    let capture = changes.clone();
    window.on_difference(move |backwards| capture.borrow_mut().push(backwards));
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
        for (width, height) in [(320, 200), (1100, 740)] {
            draw(
                &native,
                &window,
                width,
                height,
                if light { "text-light" } else { "text-dark" },
            );
            for label in [
                if width < 500 {
                    "‹ Difference"
                } else {
                    "Previous difference"
                },
                if width < 500 {
                    "Difference ›"
                } else {
                    "Next difference"
                },
                "Close",
            ] {
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
                        && origin.x + rect.size.width <= width as f32 + 0.5
                        && origin.y + rect.size.height <= height as f32 + 0.5,
                    "{label} outside {width}x{height}: {origin:?} {rect:?}"
                );
                if width > 500 {
                    assert!(
                        rect.size.width < 230.0,
                        "{label} should fit its caption, not expand across the toolbar: {rect:?}"
                    );
                }
                if label == "Close" {
                    continue;
                }
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
        }
    }
    assert_eq!(*changes.borrow(), [true, false].repeat(4));
    window.hide().unwrap();
}

#[test]
fn comparator_appearance_grows_ui_controls_for_large_selected_font() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(support::TestPlatform(native.clone()))).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let manager = ComparatorWindows::new(midnight_compare_engine::Service::new(
        runtime.handle().clone(),
    ));
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../_build/qa/native-dialogs")
        .join(format!("compare-font-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let paths = [directory.join("left.txt"), directory.join("right.txt")];
    std::fs::write(&paths[0], "Shared text\nEarlier value\nČeský 世界\n").unwrap();
    std::fs::write(&paths[1], "Shared text\nUpdated value\nČeský 世界\n").unwrap();
    let window = manager
        .open(
            FileCompareLaunch {
                sources: paths.into_iter().map(FileCompareSource::Focused).collect(),
                disabled_plugins: Arc::from([]),
            },
            ViewerAppearance {
                family: "sans-serif".into(),
                font_size: 32.0,
                foreground: [240, 240, 240, 255],
                background: [24, 28, 36, 255],
                selection: [60, 80, 120, 255],
                accent: [100, 150, 220, 255],
            },
            slint::LogicalSize::new(1100.0, 740.0),
            Rc::new(|_, _| None),
        )
        .unwrap()
        .upgrade()
        .unwrap();
    draw(&native, &window, 1100, 740, "text-large-font");
    let item = find(
        ItemRc::new_root(WindowInner::from_pub(window.window()).component()),
        "Next difference",
    )
    .unwrap();
    assert!(
        item.geometry().size.height >= 40.0,
        "Selected UI font needs a taller button: {:?}",
        item.geometry()
    );
    manager.close_all();
}

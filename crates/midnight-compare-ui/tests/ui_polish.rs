//! Actual comparison actions must remain reachable at minimum size.
#[path = "support/resident.rs"]
mod resident;
#[path = "support/mod.rs"]
mod support;
use midnight_compare_ui::{ComparatorWindow, ComparatorWindows};
use midnight_core::{FileCompareLaunch, FileCompareSource, ViewerAppearance};
use slint::{
    ComponentHandle,
    platform::{
        PointerEventButton, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
    private_unstable_api::re_exports::{
        AccessibilityAction, AccessibleRole, AccessibleStringProperty, ItemRc, WindowInner,
    },
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

/// Whether every visible caption below `item` gets the width it needs.
fn captions_fit(item: &ItemRc, adapter: &Rc<dyn slint::platform::WindowAdapter>) -> bool {
    use slint::private_unstable_api::re_exports::{ComplexText, Orientation, SimpleText};
    let mut pending = vec![item.clone()];
    while let Some(item) = pending.pop() {
        if item.is_visible()
            && (item.downcast::<SimpleText>().is_some() || item.downcast::<ComplexText>().is_some())
        {
            let needed = item
                .borrow()
                .as_ref()
                .layout_info(Orientation::Horizontal, -1.0, adapter, &item)
                .preferred;
            if needed > item.geometry().width() + 0.5 {
                return false;
            }
        }
        let mut child = item.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            pending.push(next);
        }
    }
    true
}

fn draw(
    native: &MinimalSoftwareWindow,
    window: &ComparatorWindow,
    width: u32,
    height: u32,
    name: &str,
) -> Vec<slint::Rgb8Pixel> {
    native.set_size(slint::PhysicalSize::new(width, height));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(width as f32, height as f32),
    });
    window.set_compact(width < 500);
    let mut pixels = vec![slint::Rgb8Pixel::default(); (width * height) as usize];
    window.window().request_redraw();
    let start = Instant::now();
    loop {
        slint::platform::update_timers_and_animations();
        native.draw_if_needed(|renderer| {
            renderer.set_repaint_buffer_type(RepaintBufferType::NewBuffer);
            renderer.render(&mut pixels, width as usize);
            renderer.set_repaint_buffer_type(RepaintBufferType::ReusedBuffer);
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
        for pixel in &pixels {
            data.extend([pixel.r, pixel.g, pixel.b]);
        }
        std::fs::write(directory.join(format!("{name}-{width}x{height}.ppm")), data).unwrap();
    }
    pixels
}

fn fixture_directory(name: &str) -> std::path::PathBuf {
    // Native capture accepts only literal absolute paths without `.` or `..` steps,
    // so the default root is the checkout's own _build, named without dot steps.
    let root = std::env::var_os("ME_COMPARE_POLISH_SCREENSHOTS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(2)
                .unwrap()
                .join("_build")
                .join("qa")
                .join("native-dialogs")
        });
    let directory = root.join(format!("{name}-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    // The optional screenshot root may itself contain relative/dot components.
    // Normalize only the fixture this test just created, never production source paths.
    directory.canonicalize().unwrap()
}

fn wait_until(condition: impl Fn() -> bool) {
    let start = Instant::now();
    while !condition() {
        slint::platform::update_timers_and_animations();
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "Comparator UI timed out"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn identity_strip_colors_only_positive_verdicts_and_keeps_byte_scope_accessible() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(support::TestPlatform(native.clone()))).unwrap();
    let window = ComparatorWindow::new().unwrap();
    window.set_ui_font_family("monospace".into());
    window.set_status("The word identical in status is not proof".into());
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
        for (width, height, font_size) in [
            (320, 200, 12.0),
            (320, 200, 32.0),
            (1100, 740, 12.0),
            (1100, 740, 32.0),
        ] {
            window.set_ui_font_size(font_size);
            for (state, positive) in [(-1, false), (0, false), (1, true), (2, true), (3, false)] {
                window.set_identity_state(state);
                let name = format!("identity-{state}-light{light}-font{font_size}");
                let pixels = draw(&native, &window, width, height, &name);
                let result = find(
                    ItemRc::new_root(WindowInner::from_pub(window.window()).component()),
                    "Comparison result",
                )
                .expect("The current result must be accessible");
                assert!(result.is_visible());
                let rect = result.geometry();
                let origin = result.map_to_native_window(rect.origin);
                assert!(
                    rect.size.width >= width as f32 * 0.9
                        && rect.size.height >= 24.0
                        && origin.y + rect.size.height <= height as f32 + 0.5,
                    "Result outside {width}x{height}: {origin:?} {rect:?}"
                );
                let pixel = pixels[(origin.y + rect.size.height / 2.0) as usize * width as usize
                    + (origin.x + 2.0) as usize];
                let green = i16::from(pixel.g) > i16::from(pixel.r) + 20
                    && i16::from(pixel.g) > i16::from(pixel.b) + 10;
                assert_eq!(
                    green, positive,
                    "Identity state {state}, light={light}: {pixel:?}"
                );
                let value = result
                    .accessible_string_property(AccessibleStringProperty::Value)
                    .unwrap();
                if state == 1 {
                    assert!(
                        value.contains("file bytes") && value.contains("verified"),
                        "{value}"
                    );
                } else if state == 2 {
                    assert!(
                        value.contains("encoding")
                            && value.contains("BOM")
                            && value.contains("file bytes"),
                        "{value}"
                    );
                }
            }
        }
    }
    window.hide().unwrap();
}

#[test]
fn identical_text_result_is_green_and_cannot_survive_changed_inputs_or_retry() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(support::TestPlatform(native.clone()))).unwrap();
    let runtime = resident::TestRuntime::new(tokio::runtime::Runtime::new().unwrap());
    let manager = ComparatorWindows::new(midnight_compare_engine::Service::new(
        runtime.handle().clone(),
        runtime.spawner(),
    ));
    let directory = fixture_directory("compare-identical");
    let paths = [directory.join("left.txt"), directory.join("right.txt")];
    let text = "Český 世界 🐈\nSame text\n";
    std::fs::write(&paths[0], text).unwrap();
    let mut utf16 = vec![0xff, 0xfe];
    utf16.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
    std::fs::write(&paths[1], utf16).unwrap();
    let comparator_snapshot = midnight_core::FileComparatorSnapshot::default();
    let window = manager
        .open(
            FileCompareLaunch {
                sources: paths
                    .iter()
                    .cloned()
                    .map(FileCompareSource::Focused)
                    .collect(),
                disabled_plugins: Arc::from([]),
                comparators: comparator_snapshot.preferences.clone(),
                comparator_policy: comparator_snapshot.id,
            },
            ViewerAppearance {
                family: "monospace".into(),
                font_size: 14.0,
                foreground: [240, 240, 240, 255],
                background: [24, 28, 36, 255],
                selection: [60, 80, 120, 255],
                selection_text: [0; 4],
                accent: [100, 150, 220, 255],
            },
            slint::LogicalSize::new(1100.0, 740.0),
            Rc::new(|_, _| None),
        )
        .unwrap()
        .upgrade()
        .unwrap();
    wait_until(|| window.get_identity_state() == 2);
    for light in [false, true] {
        window.invoke_apply_palette(light);
        for (width, height) in [(320, 200), (1100, 740)] {
            let name = format!("identical-text-light{light}");
            let pixels = draw(&native, &window, width, height, &name);
            let result = find(
                ItemRc::new_root(WindowInner::from_pub(window.window()).component()),
                "Comparison result",
            )
            .expect("A successful comparison needs an accessible result");
            assert!(result.is_visible());
            let value = result
                .accessible_string_property(AccessibleStringProperty::Value)
                .unwrap();
            assert!(value.to_lowercase().contains("identical"), "{value}");
            assert!(
                value.contains("encoding") && value.contains("BOM"),
                "{value}"
            );
            let rect = result.geometry();
            assert!(rect.size.width >= width as f32 * 0.9 && rect.size.height >= 24.0);
            let origin = result.map_to_native_window(rect.origin);
            let x = (origin.x + 2.0) as usize;
            let y = (origin.y + rect.size.height / 2.0) as usize;
            let pixel = pixels[y * width as usize + x];
            assert!(
                i16::from(pixel.g) > i16::from(pixel.r) + 20
                    && i16::from(pixel.g) > i16::from(pixel.b) + 10,
                "Identical result must have a prominent green background: {pixel:?}"
            );
        }
    }

    std::fs::write(
        &paths[1],
        "Český 世界 🐈\nChanged input with different length\n",
    )
    .unwrap();
    window.invoke_seek(0.0);
    wait_until(|| window.get_choosing_files());
    assert!(
        window.get_choice_error(),
        "Changed source must invalidate the successful result"
    );
    window.invoke_compare_chosen_files();
    wait_until(|| window.get_identity_state() == 3);
    let pixels = draw(&native, &window, 1100, 740, "different-text-after-retry");
    let result = find(
        ItemRc::new_root(WindowInner::from_pub(window.window()).component()),
        "Comparison result",
    )
    .unwrap();
    let value = result
        .accessible_string_property(AccessibleStringProperty::Value)
        .unwrap();
    assert!(
        !value.to_lowercase().contains("identical"),
        "Old result survived retry: {value}"
    );
    let rect = result.geometry();
    let origin = result.map_to_native_window(rect.origin);
    let pixel =
        pixels[(origin.y + rect.size.height / 2.0) as usize * 1100 + (origin.x + 2.0) as usize];
    assert!(
        i16::from(pixel.g) <= i16::from(pixel.r) + 20
            || i16::from(pixel.g) <= i16::from(pixel.b) + 10,
        "Different inputs retained the old green result: {pixel:?}"
    );
    manager.close_all();
}

#[test]
fn image_mode_buttons_select_and_preserve_acknowledged_composition_until_ready() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(support::TestPlatform(native.clone()))).unwrap();
    let window = ComparatorWindow::new().unwrap();
    let changes = Rc::new(RefCell::new(Vec::new()));
    let capture = changes.clone();
    window.on_change_mode(move |mode| capture.borrow_mut().push(mode));
    let image = |pixel: [u8; 4]| {
        slint::Image::from_rgba8_premultiplied(
            slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&pixel, 1, 1),
        )
    };
    window.set_image_mode(true);
    window.set_left_surface(image([255, 0, 0, 255]));
    window.set_right_surface(image([0, 0, 255, 255]));
    window.show().unwrap();

    for (width, height, font_size) in [(320, 200, 12.0), (1100, 740, 12.0), (1100, 740, 32.0)] {
        window.set_ui_font_size(font_size);
        window.set_mode(0);
        window.set_rendered_mode(0);
        window.set_amount(0.0);
        let name = format!("image-mode-buttons-font{font_size}-requested0-rendered0-initial");
        let _ = draw(&native, &window, width, height, &name);
        for (mode, label, pending, ready) in [
            (1, "Swipe", [[255, 0, 0], [0, 0, 255]], [[0, 0, 255]; 2]),
            (2, "Onion skin", [[0, 0, 255]; 2], [[255, 0, 0]; 2]),
            (
                0,
                "Side by side",
                [[255, 0, 0]; 2],
                [[255, 0, 0], [0, 0, 255]],
            ),
        ] {
            for name in [
                "Side by side",
                "Swipe",
                "Onion skin",
                "Fit",
                "Zoom out",
                "Zoom in",
                "Close",
            ] {
                let item = find(
                    ItemRc::new_root(WindowInner::from_pub(window.window()).component()),
                    name,
                )
                .unwrap_or_else(|| panic!("Missing image comparison button {name}"));
                assert_eq!(item.accessible_role(), AccessibleRole::Button, "{name}");
                assert!(
                    item.is_visible(),
                    "{name} must be visible without opening a menu"
                );
                let adapter: Rc<dyn slint::platform::WindowAdapter> = native.clone();
                assert!(
                    captions_fit(&item, &adapter),
                    "{name} cuts its caption at {width}x{height}, font {font_size}"
                );
                let rect = item.geometry();
                let origin = item.map_to_native_window(rect.origin);
                assert!(
                    origin.x >= 0.0
                        && origin.y >= 0.0
                        && origin.x + rect.size.width <= width as f32 + 0.5
                        && origin.y + rect.size.height <= height as f32 + 0.5,
                    "{name} outside {width}x{height}: {origin:?} {rect:?}"
                );
                if name == label {
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
            assert_eq!(
                window.get_mode(),
                mode,
                "{label} must select its image mode"
            );
            assert!(
                window.get_content_focused(),
                "{label} must return focus to image gestures"
            );
            for (index, name) in ["Side by side", "Swipe", "Onion skin"]
                .into_iter()
                .enumerate()
            {
                let item = find(
                    ItemRc::new_root(WindowInner::from_pub(window.window()).component()),
                    name,
                )
                .unwrap();
                assert_eq!(
                    item.accessible_string_property(AccessibleStringProperty::Checked)
                        .as_deref(),
                    Some(if index == mode as usize {
                        "true"
                    } else {
                        "false"
                    }),
                    "Only {label} should be selected"
                );
            }
            for (acknowledged, expected) in [(false, pending), (true, ready)] {
                if acknowledged {
                    window.set_rendered_mode(mode);
                }
                let name = format!(
                    "image-mode-buttons-font{font_size}-requested{mode}-rendered{}-{}",
                    window.get_rendered_mode(),
                    if acknowledged { "ready" } else { "pending" }
                );
                let pixels = draw(&native, &window, width, height, &name);
                let canvas = find(
                    ItemRc::new_root(WindowInner::from_pub(window.window()).component()),
                    "Compared file contents",
                )
                .unwrap();
                let rect = canvas.geometry();
                let origin = canvas.map_to_native_window(rect.origin);
                for (fraction, expected) in [0.25, 0.75].into_iter().zip(expected) {
                    let x = (origin.x + rect.size.width * fraction) as usize;
                    let y = (origin.y + rect.size.height * 0.5) as usize;
                    let actual = pixels[y * width as usize + x];
                    assert_eq!(
                        [actual.r, actual.g, actual.b],
                        expected,
                        "{label} acknowledged={acknowledged} at {width}x{height}"
                    );
                }
            }
        }
    }
    assert_eq!(*changes.borrow(), [1, 2, 0].repeat(3));
    window.hide().unwrap();
}

#[test]
fn swipe_slider_and_font_chooser_keep_following_shared_state_after_user_changes() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(support::TestPlatform(native.clone()))).unwrap();
    let window = ComparatorWindow::new().unwrap();
    let amounts = Rc::new(RefCell::new(Vec::new()));
    let record = amounts.clone();
    window.on_change_amount(move |value| record.borrow_mut().push(value));
    window.show().unwrap();
    let value = |label: &str| {
        find(
            ItemRc::new_root(WindowInner::from_pub(window.window()).component()),
            label,
        )
        .unwrap_or_else(|| panic!("Missing {label}"))
        .accessible_string_property(AccessibleStringProperty::Value)
        .unwrap()
    };
    window.set_image_mode(true);
    window.set_mode(1);
    draw(&native, &window, 1100, 740, "swipe-slider");
    find(
        ItemRc::new_root(WindowInner::from_pub(window.window()).component()),
        "Swipe divider",
    )
    .unwrap()
    .accessible_action(&AccessibilityAction::SetValue("0.3".into()));
    assert!((window.get_amount() - 0.3).abs() < 1e-6);
    assert_eq!(amounts.borrow().len(), 1);
    assert!((amounts.borrow()[0] - 0.3).abs() < 1e-6);
    // A canvas drag or the backend later moves the divider; the thumb must follow it.
    window.set_amount(0.8);
    draw(&native, &window, 1100, 740, "swipe-slider-moved");
    let thumb: f32 = value("Swipe divider").parse().unwrap();
    assert!((thumb - 0.8).abs() < 1e-6, "slider thumb stayed at {thumb}");

    window.set_image_mode(false);
    window.set_font_families(slint::ModelRc::new(slint::VecModel::from(vec![
        slint::SharedString::from("System monospace"),
        "Alpha Mono".into(),
        "Beta Mono".into(),
    ])));
    window.set_fonts_ready(true);
    window.set_font_index(1);
    draw(&native, &window, 1100, 740, "font-chooser");
    assert_eq!(value("Text comparison font"), "Alpha Mono");
    // The replaced model must not detach the chooser from later backend selections.
    window.set_font_index(2);
    draw(&native, &window, 1100, 740, "font-chooser-reselected");
    assert_eq!(value("Text comparison font"), "Beta Mono");
    window.hide().unwrap();
}

#[test]
fn hidden_find_bar_adds_no_layout_gap_to_image_comparison() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(support::TestPlatform(native.clone()))).unwrap();
    let window = ComparatorWindow::new().unwrap();
    window.set_ui_font_size(14.0);
    window.set_left_name("left.png".into());
    window.set_right_name("right.png".into());
    window.set_image_mode(true);
    window.show().unwrap();
    for search_visible in [false, true] {
        window.set_search_visible(search_visible);
        draw(&native, &window, 1000, 700, "image-no-hidden-find-spacing");
        let root = ItemRc::new_root(WindowInner::from_pub(window.window()).component());
        let header = find(root.clone(), "left.png")
            .unwrap()
            .geometry()
            .size
            .height;
        let modes = find(root.clone(), "Side by side")
            .unwrap()
            .geometry()
            .size
            .height;
        let controls = find(root.clone(), "Fit").unwrap().geometry().size.height;
        let result = find(root, "Comparison result")
            .unwrap()
            .geometry()
            .size
            .height;
        // Five visible rows (paths, modes, controls, canvas, verdict) have four 5 px
        // gaps and 6 px outer padding. A hidden Find row must add no sixth item.
        let expected = 700.0 - 12.0 - 4.0 * 5.0 - header - modes - controls - result;
        assert!(
            (window.get_content_height() - expected).abs() < 1.0,
            "canvas {} vs visible-row budget {expected}; header {header}, modes {modes}, controls {controls}, result {result}",
            window.get_content_height()
        );
    }
    window.hide().unwrap();
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
            let _ = draw(
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
                "Find in compared files (Ctrl+F)",
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
                if label == "Find in compared files (Ctrl+F)" {
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
    let runtime = resident::TestRuntime::new(tokio::runtime::Runtime::new().unwrap());
    let manager = ComparatorWindows::new(midnight_compare_engine::Service::new(
        runtime.handle().clone(),
        runtime.spawner(),
    ));
    let directory = fixture_directory("compare-font");
    let paths = [directory.join("left.txt"), directory.join("right.txt")];
    std::fs::write(&paths[0], "Shared text\nEarlier value\nČeský 世界\n").unwrap();
    std::fs::write(&paths[1], "Shared text\nUpdated value\nČeský 世界\n").unwrap();
    let comparator_snapshot = midnight_core::FileComparatorSnapshot::default();
    let window = manager
        .open(
            FileCompareLaunch {
                sources: paths.into_iter().map(FileCompareSource::Focused).collect(),
                disabled_plugins: Arc::from([]),
                comparators: comparator_snapshot.preferences.clone(),
                comparator_policy: comparator_snapshot.id,
            },
            ViewerAppearance {
                family: "sans-serif".into(),
                font_size: 32.0,
                foreground: [240, 240, 240, 255],
                background: [24, 28, 36, 255],
                selection: [60, 80, 120, 255],
                selection_text: [0; 4],
                accent: [100, 150, 220, 255],
            },
            slint::LogicalSize::new(1100.0, 740.0),
            Rc::new(|_, _| None),
        )
        .unwrap()
        .upgrade()
        .unwrap();
    let _ = draw(&native, &window, 1100, 740, "text-large-font");
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

#[test]
fn app_themed_slider_paints_selected_accent_and_preserves_native_pointer_and_keys() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(support::TestPlatform(native.clone()))).unwrap();
    let window = ComparatorWindow::new().unwrap();
    window.set_image_mode(true);
    window.set_mode(1);
    window.set_rendered_mode(1);
    let changes = Rc::new(RefCell::new(Vec::new()));
    let captured = changes.clone();
    window.on_change_amount(move |amount| captured.borrow_mut().push(amount));
    window.show().unwrap();
    for (theme, expected) in [
        (2, [69, 69, 69]),
        (8, [165, 230, 90]),
        (21, [191, 255, 255]),
    ] {
        window.invoke_apply_tool_theme(theme, 0, "Segoe UI".into(), 18, 32);
        let pixels = draw(
            &native,
            &window,
            1100,
            740,
            &format!("themed-slider-accent-theme{theme}"),
        );
        let slider = find(
            ItemRc::new_root(WindowInner::from_pub(window.window()).component()),
            "Swipe divider",
        )
        .unwrap();
        let rect = slider.geometry();
        let origin = slider.map_to_native_window(rect.origin);
        assert!(rect.size.width > 40.0 && rect.size.height >= 20.0);
        let accent = window.get_accent().to_argb_u8();
        assert_eq!([accent.red, accent.green, accent.blue], expected);
        let count = (origin.y.ceil() as usize..(origin.y + rect.size.height).floor() as usize)
            .flat_map(|y| {
                (origin.x.ceil() as usize..(origin.x + rect.size.width).floor() as usize)
                    .map(move |x| y * 1100 + x)
            })
            .filter(|&index| {
                pixels.get(index).is_some_and(|pixel| {
                    pixel.r == accent.red && pixel.g == accent.green && pixel.b == accent.blue
                })
            })
            .count();
        assert!(count >= 8, "the actual slider face must use the app accent");
    }
    let slider = find(
        ItemRc::new_root(WindowInner::from_pub(window.window()).component()),
        "Swipe divider",
    )
    .unwrap();
    let rect = slider.geometry();
    let origin = slider.map_to_native_window(rect.origin);
    let position = slint::LogicalPosition::new(
        origin.x + rect.size.width - 1.0,
        origin.y + rect.size.height / 2.0,
    );
    // Pinned Fluent uses a 20px thumb. Drag it to the edge: an interior
    // track click maps x / full width and does not select the exact maximum.
    let thumb = slint::LogicalPosition::new(
        origin.x + 10.0 + (rect.size.width - 20.0) * window.get_amount(),
        position.y,
    );
    let before_pointer = changes.borrow().len();
    for event in [
        WindowEvent::PointerMoved { position: thumb },
        WindowEvent::PointerPressed {
            position: thumb,
            button: PointerEventButton::Left,
        },
        WindowEvent::PointerMoved { position },
        WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        },
    ] {
        window.window().dispatch_event(event);
    }
    assert_eq!(changes.borrow().len(), before_pointer + 1);
    assert_eq!(changes.borrow().last().copied(), Some(1.0));
    let before = changes.borrow().len();
    let home: slint::SharedString = slint::platform::Key::Home.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: home.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text: home });
    assert_eq!(changes.borrow().len(), before + 1);
    assert_eq!(changes.borrow().last().copied(), Some(0.0));
    window.hide().unwrap();
}

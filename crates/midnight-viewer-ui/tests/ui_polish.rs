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

fn click_control(window: &ViewerWindow, item: ItemRc) {
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

fn type_query(window: &ViewerWindow, text: &str) {
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.into() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text: text.into() });
}

#[test]
fn visible_find_focuses_query_and_fits_supported_viewer_footers() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = ViewerWindow::new().unwrap();
    let finds = Rc::new(std::cell::Cell::new(0));
    let capture = finds.clone();
    let weak = window.as_weak();
    window.on_find(move || {
        capture.set(capture.get() + 1);
        weak.upgrade().unwrap().invoke_focus_search();
    });
    window.set_modes(slint::ModelRc::new(slint::VecModel::from(vec![
        "Text".into(),
        "Binary".into(),
        "Source".into(),
        "Markdown".into(),
        "HTML".into(),
    ])));
    window.set_status("Ready to read".into());
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    for (index, mode) in [1, 3, 2, 4, midnight_core::VIEWER_MODE_HTML]
        .into_iter()
        .enumerate()
    {
        let web = mode == midnight_core::VIEWER_MODE_HTML;
        window.set_image_mode(false);
        window.set_web_mode(web);
        window.set_source_mode(mode == 2);
        window.set_markdown_mode(mode == 4);
        window.set_font_available(mode != 3 && !web);
        window.set_wrap_available(mode != 3 && !web);
        window.set_mode_index(index as i32);
        for (width, height, font) in [(320, 200, 12.0), (1100, 740, 32.0)] {
            window.set_compact(width < 500);
            window.set_ui_font_size(font);
            window.set_query("".into());
            window.invoke_focus_search();
            draw(
                &native,
                &window,
                width,
                height,
                &format!("find-baseline-{mode}"),
            );
            let baseline = (window.get_content_width(), window.get_content_height());
            assert!(baseline.0 > 0.0 && baseline.1 > 0.0);
            window.set_search_visible(false);
            window.set_query("".into());
            window.invoke_focus_content();
            draw(
                &native,
                &window,
                width,
                height,
                &format!("find-closed-{mode}"),
            );
            let mut labels = vec!["Find"];
            if !web {
                labels.push("Word wrap");
            }
            if mode == 1 {
                labels.push("Font and presentation");
            }
            labels.push("Viewer mode");
            let mut previous_right = 0.0;
            for label in labels {
                let item = control(&window, label, width as f32, height as f32);
                assert!(item.is_visible(), "{label} must be visible in mode {mode}");
                let rect = item.geometry();
                let origin = item.map_to_native_window(rect.origin);
                assert!(rect.size.width > 0.0 && rect.size.height > 0.0);
                assert!(origin.x >= previous_right - 0.5, "footer controls overlap");
                assert!(
                    origin.y >= window.get_content_top() + window.get_content_height(),
                    "{label} cannot cover document pixels"
                );
                previous_right = origin.x + rect.size.width;
            }
            let before = finds.get();
            click_control(
                &window,
                control(&window, "Find", width as f32, height as f32),
            );
            draw(
                &native,
                &window,
                width,
                height,
                &format!("find-open-{mode}"),
            );
            assert_eq!(finds.get(), before + 1);
            assert!(window.get_search_visible());
            assert!(!window.get_content_focused());
            let actual = (window.get_content_width(), window.get_content_height());
            assert!(
                (actual.0 - baseline.0).abs() <= 0.5 && (actual.1 - baseline.1).abs() <= 0.5,
                "Find must retain existing search geometry in mode {mode} at {width}x{height}/font{font}: {actual:?} versus {baseline:?}"
            );
            type_query(&window, "needle with spaces");
            assert_eq!(window.get_query(), "needle with spaces");
            window.invoke_focus_content();
            assert!(window.get_content_focused());
            click_control(
                &window,
                control(&window, "Find", width as f32, height as f32),
            );
            assert_eq!(finds.get(), before + 2);
            assert_eq!(window.get_query(), "needle with spaces");
            type_query(&window, "!");
            assert_eq!(window.get_query(), "needle with spaces!");
        }
    }
    assert_eq!(finds.get(), 20);
    window.set_image_mode(true);
    window.set_web_mode(false);
    window.set_source_mode(false);
    window.set_markdown_mode(false);
    window.set_search_visible(false);
    window.set_document_pages(3);
    for layout in [-1, 0, 1] {
        window.set_document_layout(layout);
        window.set_document_page(if layout < 0 { 0 } else { 2 });
        for (width, height, font) in [(320, 200, 12.0), (1100, 740, 32.0)] {
            window.set_compact(width < 500);
            window.set_ui_font_size(font);
            draw(
                &native,
                &window,
                width,
                height,
                &format!("find-unsupported-{layout}"),
            );
            let button = find(
                ItemRc::new_root(WindowInner::from_pub(window.window()).component()),
                "Find",
            );
            assert!(button.is_none_or(|item| !item.is_visible()));
        }
    }
    window.hide().unwrap();
}

#[test]
fn compact_pdf_keeps_large_page_counter_and_zoom_actions_clickable() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = ViewerWindow::new().unwrap();
    window.set_image_mode(true);
    let turns = Rc::new(RefCell::new(Vec::new()));
    let capture = turns.clone();
    window.on_document_turn(move |delta| capture.borrow_mut().push(delta));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let capture = commands.clone();
    window.on_image_command(move |command| capture.borrow_mut().push(command));
    window.show().unwrap();
    // Native font metrics differ across platforms. Exercise the entire supported
    // page-counter range without sacrificing control bounds or actual clicks.
    for (page, pages, zoom) in [(9999, 10000, 1600.0), (i32::MAX - 1, i32::MAX, 1599.9)] {
        window.set_document_page(page);
        window.set_document_pages(pages);
        window.set_zoom_percent(zoom);
        window.set_status(format!("Page {page} / {pages} · {zoom}%").into());
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
                let counter = control(
                    &window,
                    "Document page counter",
                    width as f32,
                    height as f32,
                );
                assert_eq!(
                    counter.accessible_string_property(AccessibleStringProperty::Value),
                    Some(format!("{page} / {pages}").into())
                );
                control(
                    &window,
                    "Image zoom percentage",
                    width as f32,
                    height as f32,
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
    }
    assert_eq!(*turns.borrow(), [-1, 1].repeat(8));
    assert_eq!(*commands.borrow(), [0, 1, 3, 2].repeat(8));
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

#[test]
fn pdf_layout_buttons_keep_published_selection_focus_and_bounds_at_compact_size_and_font32() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = ViewerWindow::new().unwrap();
    window.set_image_mode(true);
    window.set_document_page(i32::MAX - 1);
    window.set_document_pages(i32::MAX);
    window.set_zoom_percent(1599.9);
    let layouts = Rc::new(RefCell::new(Vec::new()));
    let capture = layouts.clone();
    window.on_change_document_layout(move |layout| capture.borrow_mut().push(layout));
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    for (width, height, font) in [
        (320, 200, 12.0),
        (320, 200, 32.0),
        (1000, 720, 12.0),
        (1000, 720, 32.0),
    ] {
        window.set_compact(width < 500);
        window.set_ui_font_size(font);
        for acknowledged in [0, 1] {
            window.set_document_layout(acknowledged);
            draw(
                &native,
                &window,
                width,
                height,
                &format!("pdf-layout-{acknowledged}-font{font}"),
            );
            for label in [
                "Previous document page (Ctrl+PageUp)",
                "Next document page (Ctrl+PageDown)",
                "Document page counter",
                "Image zoom percentage",
                "Fit image to window",
                "Actual image pixels",
                "Zoom image out",
                "Zoom image in",
            ] {
                control(&window, label, width as f32, height as f32);
            }
            for (choice, label) in [
                (0, "Paged document layout"),
                (1, "Continuous document layout"),
            ] {
                let button = control(&window, label, width as f32, height as f32);
                let expected = if choice == acknowledged {
                    "true"
                } else {
                    "false"
                };
                assert_eq!(
                    button.accessible_string_property(AccessibleStringProperty::Checked),
                    Some(expected.into()),
                    "selected layout must describe the published frame"
                );
                let rect = button.geometry();
                let origin = button.map_to_native_window(rect.origin);
                assert!(
                    origin.y >= window.get_content_top() + window.get_content_height(),
                    "layout buttons cannot cover document pixels"
                );
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
                assert_eq!(window.get_document_layout(), acknowledged);
                assert_eq!(
                    button.accessible_string_property(AccessibleStringProperty::Checked),
                    Some(expected.into()),
                    "clicking must not optimistically toggle the native button"
                );
                assert!(
                    window.get_content_focused(),
                    "layout clicks return keyboard focus"
                );
            }
            assert!(
                window.get_content_height() > 40.0,
                "PDF retains a useful viewport"
            );
        }
    }
    assert_eq!(*layouts.borrow(), [0, 1].repeat(8));
    window.hide().unwrap();
}

#[test]
fn image_and_document_footer_resizes_across_stacking_threshold_without_overlap() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = ViewerWindow::new().unwrap();
    window.set_image_mode(true);
    window.set_zoom_percent(1600.0);
    let commands = Rc::new(RefCell::new(Vec::new()));
    let capture = commands.clone();
    window.on_image_command(move |command| capture.borrow_mut().push(command));
    let turns = Rc::new(RefCell::new(Vec::new()));
    let capture = turns.clone();
    window.on_document_turn(move |delta| capture.borrow_mut().push(delta));
    let layouts = Rc::new(RefCell::new(Vec::new()));
    let capture = layouts.clone();
    window.on_change_document_layout(move |layout| capture.borrow_mut().push(layout));
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    for layout in [-1, 0, 1] {
        window.set_document_layout(layout);
        window.set_document_page(if layout < 0 { 0 } else { 9999 });
        window.set_document_pages(if layout < 0 { 1 } else { 10000 });
        // Hand-checked widths below/above the 644px and 1284px breakpoints.
        // Reuse the same window through compact -> narrow -> wide -> narrow.
        for (font, widths) in [
            (12.0, [320, 640, 660, 640]),
            (32.0, [320, 1280, 1300, 1280]),
        ] {
            for (visit, width) in widths.into_iter().enumerate() {
                window.set_compact(width < 500);
                window.set_ui_font_size(font);
                draw(
                    &native,
                    &window,
                    width,
                    740,
                    &format!("footer-resize-layout{layout}-font{font}-visit{visit}"),
                );
                assert_eq!(window.window().size(), slint::PhysicalSize::new(width, 740));
                assert!(window.get_content_width() > 0.0 && window.get_content_height() > 0.0);
                let mut labels = vec![
                    "Fit image to window",
                    "Actual image pixels",
                    "Zoom image out",
                    "Image zoom percentage",
                    "Zoom image in",
                ];
                if layout >= 0 {
                    labels.extend([
                        "Previous document page (Ctrl+PageUp)",
                        "Document page counter",
                        "Next document page (Ctrl+PageDown)",
                        "Paged document layout",
                        "Continuous document layout",
                    ]);
                }
                let mut bounds = Vec::new();
                for label in &labels {
                    let item = control(&window, label, width as f32, 740.0);
                    assert!(item.is_visible(), "{label}");
                    let rect = item.geometry();
                    let origin = item.map_to_native_window(rect.origin);
                    assert!(rect.size.width > 0.0 && rect.size.height > 0.0);
                    assert!(origin.y >= window.get_content_top() + window.get_content_height());
                    bounds.push((
                        origin.x,
                        origin.y,
                        origin.x + rect.size.width,
                        origin.y + rect.size.height,
                    ));
                }
                for (index, first) in bounds.iter().enumerate() {
                    for (other, second) in bounds.iter().enumerate().skip(index + 1) {
                        assert!(
                            first.2 <= second.0 + 0.5
                                || second.2 <= first.0 + 0.5
                                || first.3 <= second.1 + 0.5
                                || second.3 <= first.1 + 0.5,
                            "{} {first:?} overlaps {} {second:?} at {width}px/font{font}/layout{layout}",
                            labels[index],
                            labels[other]
                        );
                    }
                }
                if layout >= 0 {
                    let center_y = |label: &str| {
                        let rect = bounds[labels.iter().position(|value| *value == label).unwrap()];
                        (rect.1 + rect.3) / 2.0
                    };
                    let page = center_y("Document page counter");
                    let choice = center_y("Paged document layout");
                    let zoom = center_y("Fit image to window");
                    if visit == 2 {
                        assert!((page - choice).abs() < 3.0 && (page - zoom).abs() < 3.0);
                    } else {
                        assert!(page < choice && choice < zoom);
                    }
                    for label in [
                        "Previous document page (Ctrl+PageUp)",
                        "Next document page (Ctrl+PageDown)",
                    ] {
                        click_control(&window, control(&window, label, width as f32, 740.0));
                    }
                    for (requested, label) in [
                        (0, "Paged document layout"),
                        (1, "Continuous document layout"),
                    ] {
                        let button = control(&window, label, width as f32, 740.0);
                        click_control(&window, button.clone());
                        assert_eq!(window.get_document_layout(), layout);
                        assert_eq!(
                            button.accessible_string_property(AccessibleStringProperty::Checked),
                            Some(if requested == layout { "true" } else { "false" }.into())
                        );
                    }
                }
                for label in [
                    "Fit image to window",
                    "Actual image pixels",
                    "Zoom image out",
                    "Zoom image in",
                ] {
                    click_control(&window, control(&window, label, width as f32, 740.0));
                }
                assert!(window.get_content_focused());
            }
        }
    }
    assert_eq!(*commands.borrow(), [0, 1, 3, 2].repeat(24));
    assert_eq!(*turns.borrow(), [-1, 1].repeat(16));
    assert_eq!(*layouts.borrow(), [0, 1].repeat(16));
    window.hide().unwrap();
}

#[test]
fn web_policy_widgets_require_backend_permission_for_pointer_actions() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = ViewerWindow::new().unwrap();
    window.set_web_mode(true);
    window.set_font_available(false);
    assert!(!window.get_web_policy_change_allowed());
    let policies = Rc::new(RefCell::new(Vec::new()));
    let capture = policies.clone();
    window.on_web_policy(move |network, scripts| capture.borrow_mut().push((network, scripts)));
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    for (width, height, font) in [(320, 200, 12.0), (1100, 740, 32.0)] {
        window.set_compact(width < 500);
        window.set_ui_font_size(font);
        for allowed in [false, true] {
            window.set_web_policy_change_allowed(allowed);
            for network in [false, true] {
                window.set_web_network(network);
                window.set_web_scripts(false);
                draw(
                    &native,
                    &window,
                    width,
                    height,
                    &format!("web-policy-allowed{allowed}-network{network}-font{font}"),
                );
                for (label, enabled) in [
                    ("Allow network for this document only", allowed),
                    (
                        "Allow scripts to read and transmit this document",
                        allowed && network,
                    ),
                ] {
                    let item = control(&window, label, width as f32, height as f32);
                    assert!(item.is_visible());
                    assert_eq!(
                        item.accessible_string_property(AccessibleStringProperty::Enabled),
                        Some(if enabled { "true" } else { "false" }.into())
                    );
                    policies.borrow_mut().clear();
                    click_control(&window, item);
                    assert_eq!(policies.borrow().len(), usize::from(enabled));
                    if !enabled {
                        assert_eq!(window.get_web_network(), network);
                        assert!(!window.get_web_scripts());
                    }
                }
            }
        }
    }
    window.hide().unwrap();
}

#[test]
fn app_themed_find_checkbox_and_numeric_input_keep_native_editing_and_single_actions() {
    use i_slint_core::accessibility::AccessibilityAction;
    use std::cell::Cell;
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = ViewerWindow::new().unwrap();
    window.set_font_available(true);
    window.set_typography_visible(true);
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    window.set_search_visible(true);
    let searches = Rc::new(Cell::new(0));
    let captured = searches.clone();
    window.on_search(move |_| captured.set(captured.get() + 1));
    let toggles = Rc::new(Cell::new(0));
    let captured = toggles.clone();
    window.on_change_case_sensitive(move |_| captured.set(captured.get() + 1));
    let sizes = Rc::new(RefCell::new(Vec::new()));
    let captured = sizes.clone();
    window.on_change_font_size(move |size| captured.borrow_mut().push(size));
    window.invoke_apply_tool_theme(8, 0, "Segoe UI".into(), 18, 32);
    draw(&native, &window, 1100, 740, "themed-find-edit");
    window.invoke_focus_search();
    let query = "文档 e\u{301} — literal input without path-paste routing";
    // Slint NFC-normalizes each key event, so send the decomposed literal as
    // separate character events rather than one synthetic whole-string key.
    for character in query.chars() {
        type_query(&window, &character.to_string());
    }
    assert_eq!(window.get_query().as_str(), query);
    let enter: slint::SharedString = slint::platform::Key::Return.into();
    type_query(&window, enter.as_str());
    assert_eq!(searches.get(), 1);
    // Secondary-surface values are from the selected ThemePalettes records.
    for (theme, density, face) in [
        (2, 0, [241, 241, 241]),
        (8, 1, [28, 33, 26]),
        (21, 1, [0, 0, 0]),
    ] {
        window.invoke_apply_tool_theme(theme, density, "Segoe UI".into(), 18, 32);
        draw(&native, &window, 1100, 740, "themed-find-retained");
        assert_eq!(window.get_query().as_str(), query);
        let find = control(&window, "Find", 1100.0, 740.0);
        let origin = find.map_to_native_window(find.geometry().origin);
        let mut pixels = vec![slint::Rgb8Pixel::default(); 1100 * 740];
        window.window().request_redraw();
        assert!(native.draw_if_needed(|renderer| {
            renderer.set_repaint_buffer_type(RepaintBufferType::NewBuffer);
            renderer.render(&mut pixels, 1100);
            renderer.set_repaint_buffer_type(RepaintBufferType::ReusedBuffer);
        }));
        let pixel = pixels[(origin.y as usize + 5) * 1100 + origin.x as usize + 5];
        assert_eq!(
            [pixel.r, pixel.g, pixel.b],
            face,
            "theme {theme} button face"
        );
    }
    let checkbox = control(&window, "Case sensitive", 1100.0, 740.0);
    click_control(&window, checkbox);
    assert_eq!(toggles.get(), 1);
    type_query(&window, " ");
    assert_eq!(toggles.get(), 2, "one toggle for the Space gesture");
    assert!(!window.get_case_sensitive());
    let size = control(&window, "Viewer font size", 1100.0, 740.0);
    size.accessible_action(&AccessibilityAction::SetValue("48".into()));
    assert_eq!(sizes.borrow().as_slice(), &[48]);
    size.accessible_action(&AccessibilityAction::Increment);
    assert_eq!(sizes.borrow().as_slice(), &[48], "maximum is bounded");
    size.accessible_action(&AccessibilityAction::Decrement);
    assert_eq!(sizes.borrow().as_slice(), &[48, 47]);
    window.set_viewer_font_size(14);
    assert_eq!(
        sizes.borrow().as_slice(),
        &[48, 47],
        "restoration is not an edit"
    );
    let rect = size.geometry();
    let origin = size.map_to_native_window(rect.origin);
    let editor_point =
        slint::LogicalPosition::new(origin.x + 8.0, origin.y + rect.size.height / 2.0);
    let pointer = |position| {
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
    };
    let focused_in_size = || {
        use i_slint_core::item_tree::ParentItemTraversalMode;
        let mut focused = WindowInner::from_pub(window.window())
            .focus_item
            .borrow()
            .upgrade();
        while let Some(item) = focused {
            if item == size {
                return true;
            }
            focused = item.parent_item(ParentItemTraversalMode::StopAtPopups);
        }
        false
    };
    pointer(editor_point);
    assert!(focused_in_size());
    let tab: slint::SharedString = slint::platform::Key::Tab.into();
    type_query(&window, tab.as_str());
    assert!(!focused_in_size(), "step buttons must not add Tab stops");
    let shift: slint::SharedString = slint::platform::Key::Shift.into();
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: shift.clone(),
    });
    type_query(&window, tab.as_str());
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text: shift });
    assert!(
        focused_in_size(),
        "Shift+Tab must return to the visible numeric editor"
    );
    let down: slint::SharedString = slint::platform::Key::DownArrow.into();
    type_query(&window, down.as_str());
    assert_eq!(sizes.borrow().as_slice(), &[48, 47, 13]);
    let plus = slint::LogicalPosition::new(origin.x + rect.size.width - 12.0, editor_point.y);
    pointer(plus);
    assert_eq!(sizes.borrow().as_slice(), &[48, 47, 13, 14]);
    assert!(
        focused_in_size(),
        "pointer stepping restores the visible editor"
    );
    window.set_viewer_font_size(48);
    pointer(plus);
    assert_eq!(
        sizes.borrow().as_slice(),
        &[48, 47, 13, 14],
        "disabled boundary step is inert"
    );
    window.hide().unwrap();
}

#[test]
fn themed_find_retains_composition_selection_and_unicode_clipboard_undo_across_app_changes() {
    use i_slint_core::{
        input::{InternalKeyEvent, KeyEvent, KeyEventType},
        items::TextInput,
    };
    use slint::platform::{Clipboard, Key};
    use std::cell::Cell;

    struct ClipboardPlatform(Rc<MinimalSoftwareWindow>, Rc<RefCell<String>>);
    impl Platform for ClipboardPlatform {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
            Ok(self.0.clone())
        }
        fn set_clipboard_text(&self, text: &str, clipboard: Clipboard) {
            if clipboard == Clipboard::DefaultClipboard {
                *self.1.borrow_mut() = text.into();
            }
        }
        fn clipboard_text(&self, clipboard: Clipboard) -> Option<String> {
            (clipboard == Clipboard::DefaultClipboard).then(|| self.1.borrow().clone())
        }
    }
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    let clipboard = Rc::new(RefCell::new(String::new()));
    slint::platform::set_platform(Box::new(ClipboardPlatform(
        native.clone(),
        clipboard.clone(),
    )))
    .unwrap();
    let window = ViewerWindow::new().unwrap();
    window.set_search_visible(true);
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let searches = Rc::new(Cell::new(0));
    let captured = searches.clone();
    window.on_search(move |_| captured.set(captured.get() + 1));
    window.invoke_apply_tool_theme(8, 0, "monospace".into(), 18, 32);
    draw(&native, &window, 1100, 740, "themed-ime-before");
    window.invoke_focus_search();
    let item = WindowInner::from_pub(window.window())
        .focus_item
        .borrow()
        .upgrade()
        .expect("Find owns focus");
    let input = item.downcast::<TextInput>().expect("visible native input");
    let input = input.as_pin_ref();
    assert!(item.is_visible() && input.has_focus());
    let old_font_size = input.font_size();
    let composition = |kind, text: &str, preedit: &str| {
        let mut key_event = KeyEvent::default();
        key_event.text = text.into();
        window
            .window()
            .dispatch_event(WindowEvent::internal(InternalKeyEvent {
                key_event,
                event_type: kind,
                preedit_text: preedit.into(),
                preedit_selection: (!preedit.is_empty()).then_some(3..6),
                ..Default::default()
            }));
    };
    let chord = |text| {
        window.window().dispatch_event(WindowEvent::KeyPressed {
            text: Key::Control.into(),
        });
        type_query(&window, text);
        window.window().dispatch_event(WindowEvent::KeyReleased {
            text: Key::Control.into(),
        });
    };
    let offsets = || {
        (
            input.anchor_position_byte_offset(),
            input.cursor_position_byte_offset(),
        )
    };
    type_query(&window, "seed ");
    composition(KeyEventType::UpdateComposition, "", "文档");
    assert_eq!(window.get_query(), "seed ");
    assert_eq!(input.preedit_text(), "文档");
    assert_eq!(offsets(), (5, 5));
    window.invoke_apply_tool_theme(21, 1, "serif".into(), 18, 32);
    draw(&native, &window, 1100, 740, "themed-ime-pending");
    assert!(input.has_focus() && item.is_visible());
    assert_eq!(input.font_family(), "serif");
    assert_ne!(input.font_size(), old_font_size);
    assert_eq!(input.preedit_text(), "文档", "theme must retain preedit");
    assert_eq!(window.get_query(), "seed ");
    assert_eq!(offsets(), (5, 5), "composition insertion point is retained");
    composition(KeyEventType::CommitComposition, "文档", "");
    let committed = "seed 文档";
    assert_eq!(window.get_query(), committed, "one composition insertion");
    assert_eq!(input.preedit_text(), "");
    assert_eq!(offsets(), (committed.len() as i32, committed.len() as i32));
    assert_eq!(searches.get(), 0, "IME commit is not Find activation");
    type_query(&window, slint::SharedString::from(Key::Return).as_str());
    assert_eq!(searches.get(), 1);

    chord("a");
    let selection = offsets();
    assert_eq!(selection, (0, committed.len() as i32));
    window.invoke_apply_tool_theme(2, 0, "monospace".into(), 18, 32);
    draw(&native, &window, 1100, 740, "themed-selected-copy");
    assert_eq!(offsets(), selection, "app font changes preserve selection");
    chord("c");
    assert_eq!(clipboard.borrow().as_str(), committed);
    // Insert at Home rather than adjacent to the last IME edit: this is one
    // independent native undo item, not Slint's merged contiguous typing.
    type_query(&window, slint::SharedString::from(Key::Home).as_str());
    let pasted = "🙂 e\u{301} — pasted ";
    *clipboard.borrow_mut() = pasted.into();
    chord("v");
    let combined = format!("{pasted}{committed}");
    assert_eq!(window.get_query().as_str(), combined);
    assert_eq!(offsets(), (pasted.len() as i32, pasted.len() as i32));
    window.invoke_apply_tool_theme(8, 1, "serif".into(), 18, 32);
    draw(&native, &window, 1100, 740, "themed-paste-before-undo");
    chord("z");
    assert_eq!(
        window.get_query(),
        committed,
        "native paste undo survives theme"
    );
    chord("a");
    chord("x");
    assert_eq!(window.get_query(), "");
    assert_eq!(clipboard.borrow().as_str(), committed);
    window.invoke_apply_tool_theme(21, 0, "monospace".into(), 18, 32);
    draw(&native, &window, 1100, 740, "themed-cut-before-undo");
    chord("z");
    assert_eq!(
        window.get_query(),
        committed,
        "native cut undo survives theme"
    );
    assert_eq!(offsets(), selection);
    assert_eq!(searches.get(), 1, "editing never repeats Find activation");
    window.hide().unwrap();
}

#[test]
fn app_touch_theme_keeps_compact_image_and_document_footer_inside_and_disjoint() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = ViewerWindow::new().unwrap();
    window.set_compact(true);
    window.set_image_mode(true);
    window.set_zoom_percent(1600.0);
    window.invoke_apply_tool_theme(2, 1, "Segoe UI".into(), 12, 32);
    let fits = Rc::new(std::cell::Cell::new(0));
    let counted = fits.clone();
    window.on_image_command(move |command| {
        assert_eq!(command, 0);
        counted.set(counted.get() + 1);
    });
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    for layout in [-1, 0, 1] {
        window.set_document_layout(layout);
        window.set_document_page(if layout < 0 { 0 } else { 9999 });
        window.set_document_pages(if layout < 0 { 1 } else { 10000 });
        draw(
            &native,
            &window,
            320,
            200,
            &format!("app-touch32-footer-{layout}"),
        );
        assert_eq!(window.get_ui_font_size(), 32.0);
        assert_eq!(window.window().size(), slint::PhysicalSize::new(320, 200));
        assert!(window.get_content_width() > 0.0 && window.get_content_height() > 0.0);
        let mut labels = vec![
            "Fit image to window",
            "Actual image pixels",
            "Zoom image out",
            "Image zoom percentage",
            "Zoom image in",
        ];
        if layout >= 0 {
            labels.extend([
                "Previous document page (Ctrl+PageUp)",
                "Document page counter",
                "Next document page (Ctrl+PageDown)",
                "Paged document layout",
                "Continuous document layout",
            ]);
        }
        let bounds: Vec<_> = labels
            .iter()
            .map(|label| {
                let item = control(&window, label, 320.0, 200.0);
                let rect = item.geometry();
                let origin = item.map_to_native_window(rect.origin);
                assert!(item.is_visible() && rect.size.width > 0.0 && rect.size.height > 0.0);
                assert!(origin.y >= window.get_content_top() + window.get_content_height());
                (
                    origin.x,
                    origin.y,
                    origin.x + rect.size.width,
                    origin.y + rect.size.height,
                )
            })
            .collect();
        for (index, first) in bounds.iter().enumerate() {
            for (other, second) in bounds.iter().enumerate().skip(index + 1) {
                assert!(
                    first.2 <= second.0 + 0.5
                        || second.2 <= first.0 + 0.5
                        || first.3 <= second.1 + 0.5
                        || second.3 <= first.1 + 0.5,
                    "{} {first:?} overlaps {} {second:?} in compact touch32/layout{layout}",
                    labels[index],
                    labels[other],
                );
            }
        }
        click_control(
            &window,
            control(&window, "Fit image to window", 320.0, 200.0),
        );
        assert!(window.get_content_focused());
        assert_eq!(window.get_document_layout(), layout);
    }
    assert_eq!(fits.get(), 3);
    window.hide().unwrap();
}

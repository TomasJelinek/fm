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
use std::{cell::RefCell, rc::Rc};

#[test]
fn fitting_and_original_pixel_scrollbars_keep_the_same_image_viewport() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = ViewerWindow::new().unwrap();
    window.set_image_mode(true);
    window.show().unwrap();
    native.set_size(slint::PhysicalSize::new(900, 600));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(900.0, 600.0),
    });
    let mut pixels = vec![slint::Rgb8Pixel::default(); 900 * 600];
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 900);
    });
    let fit = (window.get_content_width(), window.get_content_height());
    window.set_vertical_extent(0.4);
    window.set_horizontal_extent(0.4);
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 900);
    });
    assert_eq!(
        (window.get_content_width(), window.get_content_height()),
        fit,
        "scrollbar changes must not invalidate image Fit or trigger a second raster"
    );
    window.set_vertical_extent(1.0);
    window.set_horizontal_extent(1.0);
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 900);
    });
    assert_eq!(
        (window.get_content_width(), window.get_content_height()),
        fit
    );
    window.hide().unwrap();
}

struct TestPlatform(Rc<MinimalSoftwareWindow>);

#[test]
fn markdown_typography_controls_share_one_row_in_regular_and_compact_windows() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = ViewerWindow::new().unwrap();
    window.set_markdown_mode(true);
    window.set_viewer_font_family("sans-serif".into());
    window.set_font_family_index(0);
    window.set_viewer_font_size(16);
    window.invoke_apply_palette(false);
    window.set_status("Markdown".into());
    window.set_modes(slint::ModelRc::new(slint::VecModel::from(vec![
        "Text".into(),
        "Markdown".into(),
    ])));
    window.set_mode_index(1);
    window.show().unwrap();
    for (width, height, compact) in [(900u32, 600u32, false), (320, 200, true)] {
        window.set_compact(compact);
        native.set_size(slint::PhysicalSize::new(width, height));
        window.window().dispatch_event(WindowEvent::Resized {
            size: slint::LogicalSize::new(width as f32, height as f32),
        });
        let mut pixels = vec![slint::Rgb8Pixel::default(); (width * height) as usize];
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, width as usize);
        });
        let root = ItemRc::new_root(WindowInner::from_pub(window.window()).component());
        let mut centers = Vec::new();
        for label in [
            "Markdown presentation",
            "Viewer font family",
            "Viewer font size",
        ] {
            let item =
                find_control(root.clone(), label).unwrap_or_else(|| panic!("Missing {label}"));
            if label == "Viewer font family" {
                assert_eq!(
                    item.accessible_role(),
                    i_slint_core::items::AccessibleRole::Combobox,
                    "the font control must offer installed font choices"
                );
            }
            let rect = item.geometry();
            let origin = item.map_to_native_window(rect.origin);
            assert!(
                origin.x >= 0.0 && origin.x + rect.size.width <= width as f32 + 0.5,
                "{label} overflows {width}px toolbar"
            );
            centers.push(origin.y + rect.size.height / 2.0);
        }
        assert!(
            centers.iter().all(|y| (*y - centers[0]).abs() < 3.0),
            "Markdown toolbar must occupy one row at {width}px: {centers:?}"
        );
        assert!(
            window.get_content_top() < 50.0,
            "one-row toolbar must leave the reading area available"
        );
        if let Some(directory) = std::env::var_os("MIDNIGHT_VIEWER_TOOLBAR_SCREENSHOTS") {
            let path =
                std::path::PathBuf::from(directory).join(format!("markdown-toolbar-{width}.png"));
            std::thread::spawn(move || {
                let file = std::fs::File::create(path).unwrap();
                let mut encoder = png::Encoder::new(file, width, height);
                encoder.set_color(png::ColorType::Rgb);
                encoder.set_depth(png::BitDepth::Eight);
                let mut writer = encoder.write_header().unwrap();
                let rgb: Vec<_> = pixels
                    .into_iter()
                    .flat_map(|pixel| [pixel.r, pixel.g, pixel.b])
                    .collect();
                writer.write_image_data(&rgb).unwrap();
            })
            .join()
            .unwrap();
        }
    }
    window.hide().unwrap();
}

#[test]
fn changing_text_page_width_keeps_viewport_and_active_vertical_drag_stable() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = ViewerWindow::new().unwrap();
    window.set_vertical_extent(0.1);
    window.set_position(0.2);
    window.show().unwrap();
    native.set_size(slint::PhysicalSize::new(900, 600));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(900.0, 600.0),
    });
    let mut pixels = vec![slint::Rgb8Pixel::default(); 900 * 600];
    let draw = |pixels: &mut Vec<_>| {
        native.draw_if_needed(|renderer| {
            renderer.render(pixels, 900);
        });
    };
    draw(&mut pixels);
    let before = (
        window.get_content_width(),
        window.get_content_height(),
        window.get_vertical_bar_height(),
    );
    let thumb = (before.2 * 0.1).max(28.0);
    let at = |position| {
        slint::LogicalPosition::new(
            window.get_vertical_bar_left() + window.get_vertical_bar_width() / 2.0,
            window.get_vertical_bar_top() + position * (before.2 - thumb) + thumb / 2.0,
        )
    };
    let requested = Rc::new(RefCell::new(Vec::new()));
    let received = requested.clone();
    window.on_seek(move |position| received.borrow_mut().push(position));
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position: at(0.2) });
    window.window().dispatch_event(WindowEvent::PointerPressed {
        position: at(0.2),
        button: PointerEventButton::Left,
    });
    window.set_horizontal_extent(0.2);
    draw(&mut pixels);
    assert_eq!(
        (
            window.get_content_width(),
            window.get_content_height(),
            window.get_vertical_bar_height()
        ),
        before,
        "a newly long line must not resize the viewport or cancel the held scrollbar"
    );
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position: at(0.8) });
    draw(&mut pixels);
    assert!((window.get_vertical_thumb_position() - 0.8).abs() < 0.0001);
    assert!((requested.borrow().last().unwrap() - 0.8).abs() < 0.0001);
    window.set_horizontal_extent(1.0);
    draw(&mut pixels);
    assert_eq!(
        (
            window.get_content_width(),
            window.get_content_height(),
            window.get_vertical_bar_height()
        ),
        before
    );
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position: at(0.6) });
    assert!((requested.borrow().last().unwrap() - 0.6).abs() < 0.0001);
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position: at(0.6),
            button: PointerEventButton::Left,
        });
    window.hide().unwrap();
}

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

fn find_control(item: ItemRc, label: &str) -> Option<ItemRc> {
    if item
        .accessible_string_property(AccessibleStringProperty::Label)
        .is_some_and(|value| value == label)
    {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = find_control(item, label) {
            return Some(found);
        }
    }
    None
}

#[test]
fn text_viewer_exposes_a_real_font_and_presentation_control() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = ViewerWindow::new().unwrap();
    window.show().unwrap();
    native.set_size(slint::PhysicalSize::new(900, 600));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(900.0, 600.0),
    });
    let mut pixels = vec![slint::Rgb8Pixel::default(); 900 * 600];
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 900);
    });
    let root = ItemRc::new_root(WindowInner::from_pub(window.window()).component());
    assert!(
        find_control(root, "Font and presentation").is_some(),
        "persistent font choices must be editable through a visible widget"
    );
    window.hide().unwrap();
}

#[test]
fn edited_font_widget_restores_the_other_modes_size_before_the_next_input() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = ViewerWindow::new().unwrap();
    window.set_source_mode(true);
    let changes = Rc::new(RefCell::new(Vec::new()));
    let received = changes.clone();
    let accepted = window.as_weak();
    window.on_change_font_size(move |size| {
        received.borrow_mut().push(size);
        accepted.upgrade().unwrap().set_viewer_font_size(size);
    });
    window.show().unwrap();
    native.set_size(slint::PhysicalSize::new(900, 600));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(900.0, 600.0),
    });
    let mut pixels = vec![slint::Rgb8Pixel::default(); 900 * 600];
    let mut size_widget = || {
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 900);
        });
        let root = ItemRc::new_root(WindowInner::from_pub(window.window()).component());
        find_control(root, "Viewer font size").unwrap()
    };
    size_widget().accessible_action(&i_slint_core::accessibility::AccessibilityAction::Increment);
    assert_eq!(*changes.borrow(), [15]);
    window.set_source_mode(false);
    window.set_markdown_mode(true);
    window.set_viewer_font_size(16);
    assert_eq!(
        size_widget()
            .accessible_string_property(AccessibleStringProperty::Value)
            .unwrap(),
        "16",
        "the reused widget must display Markdown's saved size after source editing"
    );
    size_widget().accessible_action(&i_slint_core::accessibility::AccessibilityAction::Increment);
    assert_eq!(*changes.borrow(), [15, 17]);
    window.hide().unwrap();
}

#[test]
fn source_and_markdown_widgets_fit_narrow_window_and_emit_real_input() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = ViewerWindow::new().unwrap();
    window.set_compact(true);
    window.set_source_mode(true);
    let numbers = Rc::new(RefCell::new(Vec::new()));
    let received = numbers.clone();
    window.on_change_line_numbers(move |value| received.borrow_mut().push(value));
    let fonts = Rc::new(RefCell::new(Vec::new()));
    let received = fonts.clone();
    window.on_change_font_family(move |value| received.borrow_mut().push(value.to_string()));
    let presets = Rc::new(RefCell::new(Vec::new()));
    let received = presets.clone();
    window.on_change_markdown_preset(move |value| received.borrow_mut().push(value));
    window.show().unwrap();
    native.set_size(slint::PhysicalSize::new(320, 240));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(320.0, 240.0),
    });
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let mut pixels = vec![slint::Rgb8Pixel::default(); 320 * 240];
    let mut click = |label: &str| {
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 320);
        });
        let root = ItemRc::new_root(WindowInner::from_pub(window.window()).component());
        let item = find_control(root, label).unwrap_or_else(|| panic!("Missing {label}"));
        let rect = item.geometry();
        let origin = item.map_to_native_window(rect.origin);
        assert!(
            origin.x >= 0.0 && origin.x + rect.size.width <= 320.5,
            "{label} must fit the narrow viewport: {origin:?}, {rect:?}"
        );
        let position = slint::LogicalPosition::new(
            origin.x + rect.size.width / 2.0,
            origin.y + rect.size.height / 2.0,
        );
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
    };
    click("Source line numbers");
    assert_eq!(*numbers.borrow(), [false]);
    assert!(window.get_content_focused());
    window.set_viewer_font_family("sans-serif".into());
    window.set_font_families(slint::ModelRc::new(slint::VecModel::from(vec![
        "sans-serif".into(),
        "Noto Sans CJK JP".into(),
        "Yu Gothic UI".into(),
    ])));
    window.set_font_family_index(0);
    for keys in [
        vec![slint::platform::Key::DownArrow],
        vec![slint::platform::Key::DownArrow],
        vec![slint::platform::Key::UpArrow, slint::platform::Key::UpArrow],
    ] {
        click("Viewer font family");
        for key in keys.into_iter().chain([slint::platform::Key::Return]) {
            window
                .window()
                .dispatch_event(WindowEvent::KeyPressed { text: key.into() });
            window
                .window()
                .dispatch_event(WindowEvent::KeyReleased { text: key.into() });
        }
    }
    assert_eq!(
        *fonts.borrow(),
        [
            "Noto Sans CJK JP",
            "Yu Gothic UI",
            "Noto Sans CJK JP",
            "sans-serif"
        ],
        "every keyboard selection step immediately reaches the font callback"
    );
    click("Viewer font size");
    window.set_source_mode(false);
    window.set_markdown_mode(true);
    click("Markdown presentation");
    for key in [
        slint::platform::Key::DownArrow,
        slint::platform::Key::Return,
    ] {
        window
            .window()
            .dispatch_event(WindowEvent::KeyPressed { text: key.into() });
        window
            .window()
            .dispatch_event(WindowEvent::KeyReleased { text: key.into() });
    }
    assert_eq!(
        *presets.borrow(),
        [1],
        "native preset keyboard must reach the preference callback"
    );
    click("Markdown presentation");
    for key in [
        slint::platform::Key::DownArrow,
        slint::platform::Key::Return,
    ] {
        window
            .window()
            .dispatch_event(WindowEvent::KeyPressed { text: key.into() });
        window
            .window()
            .dispatch_event(WindowEvent::KeyReleased { text: key.into() });
    }
    assert_eq!(
        *presets.borrow(),
        [1, 2],
        "preset selection must leave the popup keyboard usable"
    );
    assert!(window.get_content_height() > 40.0);
    window.hide().unwrap();
}

#[test]
fn document_pager_disables_endpoints_and_sends_relative_page_requests() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = ViewerWindow::new().unwrap();
    window.set_image_mode(true);
    window.set_document_page(1);
    window.set_document_pages(2);
    let turns = Rc::new(RefCell::new(Vec::new()));
    let received = turns.clone();
    window.on_document_turn(move |delta| received.borrow_mut().push(delta));
    window.show().unwrap();
    native.set_size(slint::PhysicalSize::new(900, 600));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(900.0, 600.0),
    });
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let mut pixels = vec![slint::Rgb8Pixel::default(); 900 * 600];
    let mut click = |label: &str| {
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 900);
        });
        let root = ItemRc::new_root(WindowInner::from_pub(window.window()).component());
        let item = find_control(root, label).unwrap();
        let rect = item.geometry();
        let origin = item.map_to_native_window(rect.origin);
        let position = slint::LogicalPosition::new(
            origin.x + rect.size.width / 2.0,
            origin.y + rect.size.height / 2.0,
        );
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
    };
    click("Previous document page (Ctrl+PageUp)");
    assert!(turns.borrow().is_empty());
    click("Next document page (Ctrl+PageDown)");
    assert_eq!(*turns.borrow(), [1]);
    window.set_document_page(2);
    click("Next document page (Ctrl+PageDown)");
    assert_eq!(*turns.borrow(), [1]);
    click("Previous document page (Ctrl+PageUp)");
    assert_eq!(*turns.borrow(), [1, -1]);
    assert!(window.get_content_focused());
    window.hide().unwrap();
}

#[test]
fn wrap_checkbox_emits_changes_only_when_available_and_returns_content_focus() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = ViewerWindow::new().unwrap();
    let changes = Rc::new(RefCell::new(Vec::new()));
    let received = changes.clone();
    window.on_change_wrap(move |wrap| received.borrow_mut().push(wrap));
    window.show().unwrap();
    native.set_size(slint::PhysicalSize::new(900, 600));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(900.0, 600.0),
    });
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let mut pixels = vec![slint::Rgb8Pixel::default(); 900 * 600];
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 900);
    });
    let root = WindowInner::from_pub(window.window()).component();
    let item = find_control(ItemRc::new_root(root), "Word wrap").expect("accessible Wrap checkbox");
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    let position = slint::LogicalPosition::new(
        origin.x + rect.size.width / 2.0,
        origin.y + rect.size.height / 2.0,
    );
    let click = || {
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
    };
    click();
    assert!(
        changes.borrow().is_empty(),
        "Binary/Markdown cannot toggle wrapping"
    );
    window.set_wrap_available(true);
    click();
    assert_eq!(*changes.borrow(), [true]);
    assert!(window.get_content_focused());
    window.set_wrap_enabled(true);
    click();
    assert_eq!(*changes.borrow(), [true, false]);
    window.hide().unwrap();
}

#[test]
fn image_controls_hide_text_tools_and_route_buttons_drag_and_control_wheel() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = ViewerWindow::new().unwrap();
    window.set_image_mode(true);
    window.set_search_visible(true);
    let commands = Rc::new(RefCell::new(Vec::new()));
    let received = commands.clone();
    window.on_image_command(move |command| received.borrow_mut().push(command));
    let zoom = Rc::new(RefCell::new(Vec::new()));
    let received = zoom.clone();
    window.on_image_zoom(move |delta, x, y| received.borrow_mut().push((delta, x, y)));
    let drag = Rc::new(RefCell::new(Vec::new()));
    let received = drag.clone();
    window.on_image_drag(move |x, y| received.borrow_mut().push((x, y)));
    window.show().unwrap();
    native.set_size(slint::PhysicalSize::new(900, 600));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(900.0, 600.0),
    });
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let mut pixels = vec![slint::Rgb8Pixel::default(); 900 * 600];
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 900);
    });
    let root = ItemRc::new_root(WindowInner::from_pub(window.window()).component());
    assert!(find_control(root.clone(), "Word wrap").is_none());
    assert!(find_control(root.clone(), "Viewer mode").is_none());
    assert!(
        window.get_content_height() > 520.0,
        "hidden search has no layout height beyond the stable image scrollbar gutter"
    );
    let button = PointerEventButton::Left;
    for label in [
        "Fit image to window",
        "Actual image pixels",
        "Zoom image in",
        "Zoom image out",
    ] {
        let item = find_control(root.clone(), label).unwrap();
        let rect = item.geometry();
        let origin = item.map_to_native_window(rect.origin);
        let position = slint::LogicalPosition::new(
            origin.x + rect.size.width / 2.0,
            origin.y + rect.size.height / 2.0,
        );
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
    assert_eq!(*commands.borrow(), [0, 1, 2, 3]);
    assert!(window.get_content_focused());
    let position = slint::LogicalPosition::new(250.0, 150.0);
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: slint::platform::Key::Control.into(),
    });
    window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position,
            delta_x: 0.0,
            delta_y: 120.0,
        });
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: slint::platform::Key::Control.into(),
    });
    assert_eq!(zoom.borrow().len(), 1);
    assert_eq!(zoom.borrow()[0].0, 120.0);
    window
        .window()
        .dispatch_event(WindowEvent::PointerPressed { position, button });
    let position = slint::LogicalPosition::new(270.0, 180.0);
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased { position, button });
    assert_eq!(*drag.borrow(), [(20.0, 30.0)]);
    window.hide().unwrap();
}

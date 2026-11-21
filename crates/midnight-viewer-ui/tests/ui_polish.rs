//! Toolbar regressions use actual widget bounds and pointer input.
use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::ItemRc,
    items::StyledTextItem,
    styled_text::get_raw_text,
    window::{WindowInner, WindowKind},
};
use midnight_core::{ShortcutContext, ShortcutMap};
use midnight_viewer_ui::{MediaControlsState, ViewerWindow};
use slint::{
    ComponentHandle,
    platform::{
        Key, Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

thread_local! {
    // Widget animations run on this virtual clock; drawing advances it by frames.
    static CLOCK: Cell<Duration> = const { Cell::new(Duration::ZERO) };
}

struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
    fn duration_since_start(&self) -> Duration {
        CLOCK.with(Cell::get)
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
    // At most two virtual seconds of 16 ms frames, without real sleeping.
    for frame in 0.. {
        slint::platform::update_timers_and_animations();
        native.draw_if_needed(|renderer| {
            renderer.set_repaint_buffer_type(RepaintBufferType::NewBuffer);
            renderer.render(&mut pixels, width as usize);
            renderer.set_repaint_buffer_type(RepaintBufferType::ReusedBuffer);
        });
        if !window.window().has_active_animations() {
            break;
        }
        assert!(frame < 125, "Widget animations did not settle");
        CLOCK.with(|clock| clock.set(clock.get() + Duration::from_millis(16)));
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

/// The texts of the window's open tooltip popups.
fn tooltip_texts(window: &ViewerWindow) -> Vec<String> {
    fn styled_texts(item: ItemRc, texts: &mut Vec<String>) {
        if let Some(text) = item.downcast::<StyledTextItem>() {
            texts.push(get_raw_text(&text.as_pin_ref().text()).into_owned());
        }
        let mut child = item.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            styled_texts(item, texts);
        }
    }
    let roots: Vec<ItemRc> = WindowInner::from_pub(window.window())
        .active_popups()
        .iter()
        .filter(|popup| matches!(popup.window_kind, WindowKind::ToolTip))
        .map(|popup| ItemRc::new_root(popup.component.clone()))
        .collect();
    let mut texts = Vec::new();
    for root in roots {
        styled_texts(root, &mut texts);
    }
    texts
}

/// Rests the pointer on the middle of `label`'s control until its tooltip opens (at most
/// two seconds of the virtual clock) and returns the tooltip's texts; the pointer then
/// leaves the window, which closes the tooltip.
fn hover_tooltip(window: &ViewerWindow, label: &str, width: f32, height: f32) -> Vec<String> {
    let item = control(window, label, width, height);
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    let position = slint::LogicalPosition::new(
        origin.x + rect.size.width / 2.0,
        origin.y + rect.size.height / 2.0,
    );
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    let mut texts = Vec::new();
    for _ in 0..125 {
        CLOCK.with(|clock| clock.set(clock.get() + Duration::from_millis(16)));
        slint::platform::update_timers_and_animations();
        texts = tooltip_texts(window);
        if !texts.is_empty() {
            break;
        }
    }
    window.window().dispatch_event(WindowEvent::PointerExited);
    slint::platform::update_timers_and_animations();
    assert!(
        tooltip_texts(window).is_empty(),
        "the {label} tooltip closes when the pointer leaves"
    );
    texts
}

#[test]
fn paused_video_preserves_aspect_and_refits_on_resize_without_another_frame() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = ViewerWindow::new().unwrap();
    let color = [17, 190, 90, 255];
    let source = color.repeat(80 * 160);
    window.set_surface(slint::Image::from_rgba8_premultiplied(
        slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&source, 80, 160),
    ));
    window.set_media_controls(MediaControlsState {
        session: "portrait-video".into(),
        playing: false,
        can_play_pause: true,
        can_seek: true,
        position: 0.25,
        time: "0:15 / 1:00".into(),
        status: "".into(),
        error: "".into(),
    });
    window.show().unwrap();
    draw(&native, &window, 900, 600, "portrait-video-initial");
    // Match ordinary frame presentation, which stores an explicit raster viewport.
    // Every later size below reuses exactly the same already decoded/paused image.
    window.set_raster_viewport_width(window.get_content_width());
    window.set_raster_viewport_height(window.get_content_height());
    for (width, height) in [(900, 600), (360, 320), (1000, 700)] {
        window.set_compact(width < 500);
        draw(&native, &window, width, height, "paused-portrait-video-fit");
        for label in ["Play", "Playback position", "Playback time"] {
            let item = control(&window, label, width as f32, height as f32);
            let geometry = item.geometry();
            let position = item.map_to_native_window(geometry.origin);
            assert!(
                geometry.size.width > 0.0 && geometry.size.height > 0.0,
                "visible {label}"
            );
            assert!(position.x >= 0.0 && position.y >= 0.0, "{label} origin");
            assert!(
                position.x + geometry.size.width <= width as f32,
                "{label} width"
            );
            assert!(
                position.y + geometry.size.height <= height as f32,
                "{label} height"
            );
        }
        let mut pixels = vec![slint::Rgb8Pixel::default(); (width * height) as usize];
        window.window().request_redraw();
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, width as usize);
        });
        let mut bounds = [width, height, 0, 0];
        for (index, pixel) in pixels.iter().enumerate() {
            if [pixel.r, pixel.g, pixel.b] == color[..3] {
                let x = index as u32 % width;
                let y = index as u32 / width;
                bounds = [
                    bounds[0].min(x),
                    bounds[1].min(y),
                    bounds[2].max(x + 1),
                    bounds[3].max(y + 1),
                ];
            }
        }
        assert!(
            bounds[2] > bounds[0] && bounds[3] > bounds[1],
            "paused frame must remain visible"
        );
        let actual = [
            (bounds[2] - bounds[0]) as f32,
            (bounds[3] - bounds[1]) as f32,
        ];
        let scale = (window.get_content_width() / 80.0).min(window.get_content_height() / 160.0);
        assert!(
            (actual[0] - 80.0 * scale).abs() < 2.0 && (actual[1] - 160.0 * scale).abs() < 2.0,
            "portrait video was stretched/clipped at {width}x{height}: rendered {actual:?}, canvas {}x{}",
            window.get_content_width(),
            window.get_content_height()
        );
        let canvas = control(&window, "File content", width as f32, height as f32);
        let rect = canvas.geometry();
        let origin = canvas.map_to_native_window(rect.origin);
        assert!(
            ((bounds[0] + bounds[2]) as f32 / 2.0 - (origin.x + rect.size.width / 2.0)).abs() < 2.0
        );
        assert!(
            ((bounds[1] + bounds[3]) as f32 / 2.0 - (origin.y + rect.size.height / 2.0)).abs()
                < 2.0
        );
    }
    // Returning to text keeps the existing fill contract and Find interaction.
    window.set_media_controls(MediaControlsState::default());
    window.set_image_mode(false);
    window.invoke_focus_search();
    type_query(&window, "needle");
    assert_eq!(window.get_query(), "needle");
    assert!(window.get_search_visible());
    window.hide().unwrap();
}

#[test]
fn video_controls_keep_file_navigation_shortcuts_in_content_context() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = ViewerWindow::new().unwrap();
    let bindings = ShortcutMap::new(&[]).unwrap();
    let actions = Rc::new(RefCell::new(Vec::new()));
    let received = actions.clone();
    window.on_input_key(move |event, widget_focused| {
        let context = if widget_focused {
            ShortcutContext::ViewerSearch
        } else {
            ShortcutContext::Viewer
        };
        let Some(chord) = midnight_ui_input::chord(&event) else {
            return false;
        };
        let Some(action) = bindings.resolve(&chord, context) else {
            return false;
        };
        received
            .borrow_mut()
            .push((chord.display(), action.id.clone()));
        true
    });
    window.set_image_mode(true);
    window.set_media_controls(MediaControlsState {
        session: "video-keyboard-test".into(),
        playing: true,
        can_play_pause: true,
        can_seek: true,
        position: 0.25,
        time: "0:15 / 1:00".into(),
        status: "".into(),
        error: "".into(),
    });
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    draw(&native, &window, 900, 600, "video-keyboard");
    for label in ["Pause", "Playback position"] {
        let size = control(&window, label, 900.0, 600.0).geometry().size;
        assert!(size.width > 0.0 && size.height > 0.0, "visible {label}");
    }
    window.invoke_focus_content();
    assert!(window.get_content_focused());

    for (key, alt) in [
        (Key::Space, false),
        (Key::RightArrow, true),
        (Key::Backspace, false),
        (Key::LeftArrow, true),
    ] {
        if alt {
            window.window().dispatch_event(WindowEvent::KeyPressed {
                text: Key::Alt.into(),
            });
        }
        for event in [
            WindowEvent::KeyPressed { text: key.into() },
            WindowEvent::KeyReleased { text: key.into() },
        ] {
            window.window().dispatch_event(event);
        }
        if alt {
            window.window().dispatch_event(WindowEvent::KeyReleased {
                text: Key::Alt.into(),
            });
        }
    }
    let expected = [
        ("Space", "viewer.next-file"),
        ("Alt+Right", "viewer.next-file"),
        ("Backspace", "viewer.previous-file"),
        ("Alt+Left", "viewer.previous-file"),
    ]
    .map(|(chord, action)| (chord.to_owned(), action.to_owned()));
    assert_eq!(actions.borrow().as_slice(), &expected);
    window.hide().unwrap();
}

#[test]
fn video_timeline_seeks_once_on_release_and_discards_a_replaced_session() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = ViewerWindow::new().unwrap();
    let seeks = Rc::new(RefCell::new(Vec::new()));
    let received = seeks.clone();
    window.on_media_seek(move |session, fraction| {
        received.borrow_mut().push((session, fraction));
    });
    window.set_image_mode(true);
    let media = MediaControlsState {
        session: "first-video".into(),
        playing: false,
        can_play_pause: true,
        can_seek: true,
        position: 0.15,
        time: "0:09 / 1:00".into(),
        status: "".into(),
        error: "".into(),
    };
    window.set_media_controls(media.clone());
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    draw(&native, &window, 900, 600, "video-timeline");

    let timeline = control(&window, "Playback position", 900.0, 600.0);
    let rect = timeline.geometry();
    let origin = timeline.map_to_native_window(rect.origin);
    let pointer = |fraction: f32, down: Option<bool>| {
        let position = slint::LogicalPosition::new(
            origin.x + rect.size.width * fraction,
            origin.y + rect.size.height / 2.0,
        );
        let button = PointerEventButton::Left;
        let event = match down {
            Some(true) => WindowEvent::PointerPressed { position, button },
            Some(false) => WindowEvent::PointerReleased { position, button },
            None => WindowEvent::PointerMoved { position },
        };
        window.window().dispatch_event(event);
    };
    pointer(0.35, None);
    pointer(0.35, Some(true));
    pointer(0.48, None);
    pointer(0.75, None);
    assert!(seeks.borrow().is_empty(), "drag only previews a seek");
    pointer(0.75, Some(false));
    {
        let requests = seeks.borrow();
        let [(session, fraction)] = requests.as_slice() else {
            panic!("expected one seek on release, got {requests:?}");
        };
        assert_eq!(session.as_str(), "first-video");
        assert!((0.72..=0.80).contains(fraction), "released at {fraction}");
    }

    pointer(0.35, None);
    pointer(0.35, Some(true));
    pointer(0.55, None);
    window.set_media_controls(MediaControlsState {
        session: "replacement-video".into(),
        ..media
    });
    pointer(0.65, None);
    pointer(0.65, Some(false));
    assert_eq!(
        seeks.borrow().len(),
        1,
        "releasing the old gesture after replacement sends no seek"
    );
    window.hide().unwrap();
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

/// The image and HTML zoom buttons show only − and +. Resting the pointer on one opens a
/// tooltip that names its action and its key, as the Find bar's buttons do, in the full
/// and the compact toolbar.
#[test]
fn zoom_buttons_name_their_action_and_key_in_a_tooltip() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = ViewerWindow::new().unwrap();
    window.show().unwrap();
    let cases = [
        (
            "image",
            [
                ("Zoom image out", "Zoom image out (Ctrl+-)"),
                ("Zoom image in", "Zoom image in (Ctrl++)"),
            ],
        ),
        (
            "html",
            [
                ("Zoom HTML out", "Zoom HTML out (Ctrl+-)"),
                ("Zoom HTML in", "Zoom HTML in (Ctrl++)"),
            ],
        ),
    ];
    for (mode, buttons) in cases {
        window.set_image_mode(mode == "image");
        window.set_web_mode(mode == "html");
        for (width, height) in [(1000, 720), (320, 200)] {
            window.set_compact(width < 500);
            draw(
                &native,
                &window,
                width,
                height,
                &format!("{mode}-zoom-tooltips"),
            );
            for (label, tip) in buttons {
                assert_eq!(
                    hover_tooltip(&window, label, width as f32, height as f32),
                    [tip],
                    "{label} at {width}x{height}"
                );
            }
        }
    }
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
                for (label, enabled) in [
                    ("Allow network for this document only", allowed),
                    (
                        "Allow scripts to read and transmit this document",
                        allowed && network,
                    ),
                ] {
                    // Each control starts from the same backend policy. Clicking Online
                    // updates its two-way network binding before the JavaScript case.
                    window.set_web_network(network);
                    window.set_web_scripts(false);
                    draw(
                        &native,
                        &window,
                        width,
                        height,
                        &format!("web-policy-allowed{allowed}-network{network}-font{font}"),
                    );
                    let item = control(&window, label, width as f32, height as f32);
                    assert!(item.is_visible());
                    assert_eq!(
                        item.accessible_string_property(AccessibleStringProperty::Enabled),
                        Some(if enabled { "true" } else { "false" }.into())
                    );
                    policies.borrow_mut().clear();
                    click_control(&window, item);
                    assert_eq!(policies.borrow().len(), usize::from(enabled));
                    if enabled {
                        let requested = if label == "Allow network for this document only" {
                            (!network, false)
                        } else {
                            (network, true)
                        };
                        assert_eq!(policies.borrow().as_slice(), &[requested]);
                    } else {
                        assert_eq!(window.get_web_network(), network);
                        assert!(!window.get_web_scripts());
                    }
                    // The boxes are bound two-way and change themselves on a click;
                    // the viewer then presents the policy in force
                    // (web_input::present_policy). Present this unchanged policy
                    // before the next box is checked.
                    window.set_web_network(network);
                    window.set_web_scripts(false);
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

/// The width a text item needs to show all of its text on one line.
fn text_width(item: &ItemRc) -> f32 {
    use i_slint_core::{
        items::{ComplexText, Item, SimpleText},
        layout::Orientation,
    };
    let adapter = item.window_adapter().expect("a shown window");
    if let Some(text) = item.downcast::<SimpleText>() {
        return text
            .as_pin_ref()
            .layout_info(Orientation::Horizontal, -1.0, &adapter, item)
            .preferred;
    }
    let text = item.downcast::<ComplexText>().expect("a text item");
    text.as_pin_ref()
        .layout_info(Orientation::Horizontal, -1.0, &adapter, item)
        .preferred
}

/// The Source viewer's controls in a 480 x 320 window, at the 320 x 200 minimum, on both
/// sides of the compact (500 px) and narrow (680 px) widths and in a short 560 x 200
/// window, with the layouts the viewer selects for those widths, mouse and touch
/// density, and small and large interface fonts. Every control lies inside the window,
/// in its row and beside its neighbours; the Lines caption shows its whole text; a click
/// on the box and one on the caption each reach the line-number preference. With Find
/// open as well, a window too short for the Find bar, the typography row and some text
/// hides that row and offers Close in the Find bar, which brings the row back.
///
/// The box's native check box is an opacity-0 layer over the drawn box (WrappingCheckBox),
/// so a probe that ignores transparent layers misses it at every window size.
#[test]
fn source_toolbar_controls_stay_inside_small_windows_and_take_clicks() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = ViewerWindow::new().unwrap();
    window.set_source_mode(true);
    window.set_modes(slint::ModelRc::new(slint::VecModel::from(vec![
        "Text".into(),
        "Binary".into(),
        "Source".into(),
    ])));
    window.set_mode_index(2);
    window.set_wrap_available(true);
    window.set_viewer_font_family("Consolas".into());
    window.set_status("Ln 12, Col 1  Bytes 0–2048 / 4096".into());
    let numbers = Rc::new(RefCell::new(Vec::new()));
    let received = numbers.clone();
    window.on_change_line_numbers(move |value| received.borrow_mut().push(value));
    let weak = window.as_weak();
    window.on_find(move || weak.upgrade().unwrap().invoke_focus_search());
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let mut expected = Vec::new();
    for (width, height) in [
        (480u32, 320u32),
        (320, 200),
        (499, 320),
        (500, 320),
        (679, 400),
        (680, 400),
        (560, 200),
    ] {
        for (density, font) in [(0, 12), (0, 32), (1, 15), (1, 32)] {
            window.invoke_apply_tool_theme(0, density, "Segoe UI".into(), font, font);
            lay_out(&window, width, height);
            let case = format!("{width}x{height} density {density} font {font}");
            draw(
                &native,
                &window,
                width,
                height,
                &format!("source-toolbar-density{density}-font{font}"),
            );
            let (window_width, window_height) = (width as f32, height as f32);
            for row in [
                &[
                    "Source line numbers",
                    "Lines",
                    "Viewer font family",
                    "Viewer font size",
                ][..],
                &["Find", "Word wrap", "Viewer mode"],
            ] {
                let mut right = 0.0_f32;
                let mut middle = None;
                for label in row {
                    let item = control(&window, label, window_width, window_height);
                    let rect = item.geometry();
                    let origin = item.map_to_native_window(rect.origin);
                    assert!(
                        item.is_visible() && rect.size.width > 0.0 && rect.size.height > 0.0,
                        "{label} has no visible area at {case}"
                    );
                    assert!(
                        origin.x >= right - 0.5,
                        "{label} overlaps the control before it at {case}"
                    );
                    right = origin.x + rect.size.width;
                    let center = origin.y + rect.size.height / 2.0;
                    let row_center = *middle.get_or_insert(center);
                    assert!(
                        (center - row_center).abs() < 3.0,
                        "{label} leaves its row at {case}"
                    );
                }
            }
            let caption = control(&window, "Lines", window_width, window_height);
            assert!(
                caption.geometry().size.width >= text_width(&caption) - 0.5,
                "the Lines caption is cut at {case}: {:?}",
                caption.geometry()
            );
            for label in ["Source line numbers", "Lines"] {
                click_control(
                    &window,
                    control(&window, label, window_width, window_height),
                );
                expected.push(!expected.last().copied().unwrap_or(true));
                assert!(
                    window.get_content_focused(),
                    "{label} returns the focus to the text at {case}"
                );
            }
            // With Find open too, the typography row gives way where the window cannot also
            // keep a control height of text, and the Find bar's Close brings it back.
            window.set_query("needle".into());
            window.invoke_focus_search();
            draw(
                &native,
                &window,
                width,
                height,
                &format!("source-toolbar-find-density{density}-font{font}"),
            );
            let case = format!("{case} with Find open");
            let covered = !shown(&window, "Source line numbers");
            assert_eq!(
                shown(&window, "Close find"),
                covered,
                "Close is offered exactly while the row is hidden at {case}"
            );
            if width == 320 {
                assert!(
                    covered,
                    "the minimum window keeps text before the row at {case}"
                );
            } else if (width, height) == (480, 320) {
                assert!(!covered, "480 x 320 has room for both at {case}");
            }
            let mut labels: Vec<_> = FIND_BAR.into_iter().chain(SOURCE_FOOTER).collect();
            labels.extend(if covered {
                &["Close find"][..]
            } else {
                &SOURCE_TYPOGRAPHY[..]
            });
            assert_inside_and_apart(&window, &labels, window_width, window_height, &case);
            if covered {
                click_control(
                    &window,
                    control(&window, "Close find", window_width, window_height),
                );
                draw(
                    &native,
                    &window,
                    width,
                    height,
                    "source-toolbar-find-closed",
                );
                assert!(!window.get_search_visible() && window.get_content_focused());
                let labels: Vec<_> = SOURCE_TYPOGRAPHY.into_iter().chain(SOURCE_FOOTER).collect();
                assert_inside_and_apart(&window, &labels, window_width, window_height, &case);
                // Find opens again in the row's place.
                click_control(
                    &window,
                    control(&window, "Find", window_width, window_height),
                );
                draw(&native, &window, width, height, "source-toolbar-find-again");
                assert!(window.get_search_visible() && !shown(&window, "Source line numbers"));
            }
            window.set_search_visible(false);
            window.set_query("".into());
            window.invoke_focus_content();
        }
    }
    assert_eq!(*numbers.borrow(), expected);
    window.hide().unwrap();
}

const FIND_BAR: [&str; 6] = [
    "Find in file",
    "Clear find text",
    "Case sensitive",
    "Find whole words",
    "Previous match",
    "Next match",
];
const SOURCE_TYPOGRAPHY: [&str; 4] = [
    "Source line numbers",
    "Lines",
    "Viewer font family",
    "Viewer font size",
];
const SOURCE_FOOTER: [&str; 3] = ["Find", "Word wrap", "Viewer mode"];

/// What the viewer applies for a window of this logical size (lib.rs, apply_window_size).
fn lay_out(window: &ViewerWindow, width: u32, height: u32) {
    window.set_compact(width < 500);
    window.set_narrow_typography(width < 680);
    window.set_available_width(width as f32);
    window.set_available_height(height as f32);
}

/// Whether a control with this label is shown.
fn shown(window: &ViewerWindow, label: &str) -> bool {
    find(
        ItemRc::new_root(WindowInner::from_pub(window.window()).component()),
        label,
    )
    .is_some_and(|item| item.is_visible())
}

/// Each of `labels` lies inside the window with an area of its own.
fn assert_inside_and_apart(
    window: &ViewerWindow,
    labels: &[&str],
    width: f32,
    height: f32,
    case: &str,
) {
    let bounds: Vec<_> = labels
        .iter()
        .map(|label| {
            let item = control(window, label, width, height);
            let rect = item.geometry();
            let origin = item.map_to_native_window(rect.origin);
            assert!(
                item.is_visible() && rect.size.width > 0.0 && rect.size.height > 0.0,
                "{label} has no visible area at {case}"
            );
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
                "{} {first:?} overlaps {} {second:?} at {case}",
                labels[index],
                labels[other],
            );
        }
    }
}

/// Markdown's, Text's and the HTML viewer's rows above the text give way to an open Find
/// bar in windows too short for both, in compact and wider layouts with large fonts, so
/// the footer stays inside the window. Close (and Aa in Text mode) brings each row back.
#[test]
fn rows_above_the_text_give_way_to_find_in_short_windows_and_come_back() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = ViewerWindow::new().unwrap();
    let weak = window.as_weak();
    window.on_find(move || weak.upgrade().unwrap().invoke_focus_search());
    window.set_modes(slint::ModelRc::new(slint::VecModel::from(vec![
        "Text".into(),
        "Markdown".into(),
        "HTML".into(),
    ])));
    window.set_status("Ln 12, Col 1".into());
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let modes: [(&str, &[&str], &[&str], &str); 3] = [
        (
            "markdown",
            &[
                "Markdown presentation",
                "Viewer font family",
                "Viewer font size",
            ],
            &["Find", "Word wrap", "Viewer mode"],
            "Close find",
        ),
        (
            "text",
            &["Viewer font family", "Viewer font size"],
            &["Find", "Word wrap", "Font and presentation", "Viewer mode"],
            "Font and presentation",
        ),
        (
            "html",
            &[
                "Allow network for this document only",
                "Allow scripts to read and transmit this document",
                "Zoom HTML out",
                "Zoom HTML in",
            ],
            &["Find", "Viewer mode"],
            "Close find",
        ),
    ];
    for (index, (mode, row, footer, back)) in modes.into_iter().enumerate() {
        window.set_mode_index(index as i32);
        window.set_markdown_mode(mode == "markdown");
        window.set_typography_visible(mode == "text");
        window.set_web_mode(mode == "html");
        window.set_font_available(mode != "html");
        window.set_wrap_available(mode != "html");
        window.set_web_policy_change_allowed(mode == "html");
        for (width, height, density, font) in [
            (320u32, 200u32, 1, 32),
            (560, 200, 0, 32),
            (560, 200, 1, 32),
        ] {
            window.invoke_apply_tool_theme(0, density, "Segoe UI".into(), font, font);
            lay_out(&window, width, height);
            let (window_width, window_height) = (width as f32, height as f32);
            let case = format!("{mode} at {width}x{height} density {density} font {font}");
            draw(&native, &window, width, height, &format!("{mode}-short"));
            let labels: Vec<_> = row.iter().chain(footer).copied().collect();
            assert_inside_and_apart(&window, &labels, window_width, window_height, &case);
            window.set_query("needle".into());
            window.invoke_focus_search();
            draw(
                &native,
                &window,
                width,
                height,
                &format!("{mode}-short-find"),
            );
            assert!(
                !shown(&window, row[0]) && shown(&window, "Close find"),
                "the row gives way to Find at {case}"
            );
            let labels: Vec<_> = FIND_BAR
                .into_iter()
                .chain(footer.iter().copied())
                .chain(["Close find"])
                .collect();
            assert_inside_and_apart(&window, &labels, window_width, window_height, &case);
            click_control(&window, control(&window, back, window_width, window_height));
            draw(
                &native,
                &window,
                width,
                height,
                &format!("{mode}-short-back"),
            );
            assert!(!window.get_search_visible(), "{back} closes Find at {case}");
            let labels: Vec<_> = row.iter().chain(footer).copied().collect();
            assert_inside_and_apart(&window, &labels, window_width, window_height, &case);
            window.set_query("".into());
        }
    }
    window.hide().unwrap();
}

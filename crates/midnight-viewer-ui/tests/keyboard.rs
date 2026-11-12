use midnight_viewer_ui::ViewerWindow;
use slint::{
    ComponentHandle,
    platform::{
        Key, Platform, PlatformError, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{cell::RefCell, rc::Rc};

struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

fn key(window: &ViewerWindow, text: impl Into<slint::SharedString>) {
    let text = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

#[test]
fn actual_search_widget_keeps_spaces_and_exposes_escape_to_host() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = ViewerWindow::new().unwrap();
    let events = Rc::new(RefCell::new(Vec::new()));
    let received = events.clone();
    window.on_input_key(move |event, search| {
        received.borrow_mut().push((event.text.to_string(), search));
        event.text == slint::SharedString::from(Key::Escape) || (!search && event.text == " ")
    });
    window.show().unwrap();
    window.set_compact(false);
    native.set_size(slint::PhysicalSize::new(900, 600));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(900.0, 600.0),
    });
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let mut pixels = vec![slint::Rgb8Pixel::default(); 900 * 600];
    window.set_compact(true);
    native.set_size(slint::PhysicalSize::new(320, 200));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(320.0, 200.0),
    });
    window.set_document_pages(4096);
    let mut compact_pixels = vec![slint::Rgb8Pixel::default(); 320 * 200];
    // A compact PDF reserves two footer rows for page and zoom controls. Keep
    // the larger viewport contract for text/images; ui_polish also clicks every
    // PDF footer action at this minimum size and checks its actual bounds.
    for (mode, image, page, minimum_height) in [
        ("text", false, 0, 100.0),
        ("image", true, 0, 100.0),
        ("PDF", true, 2, 40.0),
    ] {
        window.set_image_mode(image);
        window.set_document_page(page);
        native.draw_if_needed(|renderer| {
            renderer.render(&mut compact_pixels, 320);
        });
        assert_eq!(window.window().size(), slint::PhysicalSize::new(320, 200));
        assert!((270.0..=308.0).contains(&window.get_content_width()));
        assert!(
            window.get_content_height() > minimum_height,
            "compact {mode} content: {}x{} at {},{}",
            window.get_content_width(),
            window.get_content_height(),
            window.get_content_left(),
            window.get_content_top(),
        );
    }
    window.set_image_mode(false);
    window.set_document_page(0);
    window.set_compact(false);
    native.set_size(slint::PhysicalSize::new(900, 600));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(900.0, 600.0),
    });
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 900);
    });
    window.invoke_focus_content();
    key(&window, " ");
    assert_eq!(events.borrow().last(), Some(&(" ".into(), false)));
    window.invoke_focus_search();
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 900);
    });
    for text in ["hello", " ", "世界"] {
        key(&window, text);
    }
    assert_eq!(window.get_query(), "hello 世界");
    key(&window, Key::Escape);
    assert_eq!(
        events.borrow().last(),
        Some(&(slint::SharedString::from(Key::Escape).to_string(), true))
    );
    assert!(window.get_content_width() > 800.0 && window.get_content_height() > 450.0);
    window.hide().unwrap();
}

#[test]
fn viewer_manager_routes_canonical_chords_and_preserves_search_context() {
    use midnight_core::*;
    use midnight_viewer_ui::ViewerWindows;
    use std::sync::Arc;

    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let manager = ViewerWindows::new(midnight_viewer_engine::Service::new(
        runtime.handle().clone(),
    ));
    let path = std::env::current_dir()
        .unwrap()
        .join("viewer-input-test.txt");
    let entries: Arc<[Entry]> = vec![Entry::new(EntryId(1), path.clone(), EntryKind::File)].into();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let received = seen.clone();
    let bindings = Rc::new(RefCell::new(
        ShortcutMap::new(&[
            ShortcutBinding {
                action: "viewer.find".into(),
                keys: vec!["Ctrl+Digit9".into()],
            },
            ShortcutBinding {
                action: "viewer.close".into(),
                keys: vec!["Ctrl+K".into()],
            },
        ])
        .unwrap(),
    ));
    let live_bindings = bindings.clone();
    // Input routing does not depend on the asynchronous content-open result.
    let window = manager
        .open(
            ViewerLaunch {
                associations: Default::default(),
                tree: None,
                generated: None,
                preferences: Arc::new(Default::default()),
                focused: path,
                view: Arc::new(DirectoryIndex::from_entries(&entries)),
                entries,
                archive: None,
                basic: false,
                disabled_plugins: Arc::from([]),
            },
            ViewerAppearance {
                family: "monospace".into(),
                font_size: 14.0,
                foreground: [255; 4],
                background: [0, 0, 0, 255],
                selection: [80, 80, 80, 255],
                accent: [100, 100, 255, 255],
            },
            slint::LogicalSize::new(900.0, 600.0),
            Rc::new(move |chord, search| {
                received.borrow_mut().push((chord.display(), search));
                live_bindings
                    .borrow()
                    .resolve(
                        chord,
                        if search {
                            ShortcutContext::ViewerSearch
                        } else {
                            ShortcutContext::Viewer
                        },
                    )
                    .map(|action| action.id.clone())
            }),
            Rc::new(|_| {}),
        )
        .unwrap()
        .upgrade()
        .unwrap();
    let mut pixels = vec![slint::Rgb8Pixel::default(); 900 * 600];
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 900);
    });
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    key(&window, "9");
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
    assert!(
        window.get_search_visible(),
        "canonical digit override opens Find"
    );
    assert!(
        seen.borrow()
            .iter()
            .any(|entry| entry == &("Ctrl+Digit9".into(), false))
    );
    key(&window, " ");
    assert_eq!(window.get_query(), " ", "search keeps ordinary space input");
    assert_eq!(seen.borrow().last(), Some(&("Space".into(), true)));
    *bindings.borrow_mut() = ShortcutMap::new(&[]).unwrap();
    let searches = Rc::new(RefCell::new(Vec::new()));
    let searched = searches.clone();
    let weak = window.as_weak();
    window.on_search(move |backwards| {
        let window = weak.upgrade().unwrap();
        searched
            .borrow_mut()
            .push((window.get_query().to_string(), backwards));
    });
    for mode in ["Text", "Binary", "Source", "Markdown"] {
        window.set_modes(slint::ModelRc::new(slint::VecModel::from(vec![
            mode.into(),
        ])));
        window.set_search_visible(false);
        window.set_query("".into());
        window.invoke_focus_content();
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 900);
        });
        window.window().dispatch_event(WindowEvent::KeyPressed {
            text: Key::Control.into(),
        });
        key(&window, "f");
        window.window().dispatch_event(WindowEvent::KeyReleased {
            text: Key::Control.into(),
        });
        assert!(
            window.get_search_visible(),
            "default Ctrl+F opens Find in {mode}"
        );
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 900);
        });
        key(&window, "needle café 世界");
        assert_eq!(
            window.get_query(),
            "needle café 世界",
            "Find owns typing in {mode}"
        );
        key(&window, Key::Return);
        assert_eq!(
            searches.borrow().last(),
            Some(&("needle café 世界".into(), false))
        );
    }
    key(&window, Key::Escape);
    assert!(
        !window.window().is_visible(),
        "Escape closes from search even when viewer.close is remapped"
    );
    manager.close_all();
}

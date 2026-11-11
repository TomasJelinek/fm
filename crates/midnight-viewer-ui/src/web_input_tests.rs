use super::web_input::browser_key;
use midnight_core::viewer_web::{HostShortcut, Modifiers};

#[test]
fn textarea_enter_and_shift_enter_forward_native_line_break_text() {
    use super::*;
    let (_runtime, _manager, open, _service) = image_presentation_tests::open();
    open.timer.stop();
    open.window.set_image_mode(false);
    open.window.set_web_mode(true);
    *open.key_resolver.borrow_mut() = Rc::new(|_, _| None);
    let inputs = Rc::new(RefCell::new(Vec::new()));
    let capture = inputs.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        capture.borrow_mut().push(input);
        true
    });
    for (shift, text) in [(false, "\r"), (true, "\r"), (false, "\n")] {
        inputs.borrow_mut().clear();
        let key = viewer_web::Key {
            key: "Enter".into(),
            code: "Enter".into(),
            pressed: true,
            repeat: false,
            modifiers: Modifiers {
                shift,
                ..Default::default()
            },
        };
        web_input::route_key(&open, key, Some(text), true);
        assert!(
            matches!(inputs.borrow().last(), Some(ViewerInput::Web { input: viewer_web::CommandKind::Text(value), .. }) if value == text)
        );
    }
    for key in ["Enter", "a"] {
        inputs.borrow_mut().clear();
        web_input::route_key(
            &open,
            viewer_web::Key {
                key: key.into(),
                code: "Enter".into(),
                pressed: true,
                repeat: false,
                modifiers: Modifiers {
                    control: true,
                    ..Default::default()
                },
            },
            Some("\r"),
            true,
        );
        assert!(inputs.borrow().iter().all(|input| !matches!(
            input,
            ViewerInput::Web {
                input: viewer_web::CommandKind::Text(_),
                ..
            }
        )));
    }
    close(&open);
}

#[test]
fn alt_gr_forwards_printable_text_without_shortcuts_or_control_characters() {
    use super::*;
    let (_runtime, _manager, open, _service) = image_presentation_tests::open();
    open.timer.stop();
    open.window.set_image_mode(false);
    open.window.set_web_mode(true);
    *open.key_resolver.borrow_mut() =
        Rc::new(|_, _| panic!("AltGr must not resolve host shortcuts"));
    let inputs = Rc::new(RefCell::new(Vec::new()));
    let capture = inputs.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        capture.borrow_mut().push(input);
        true
    });
    let key = viewer_web::Key {
        key: "@".into(),
        code: "KeyV".into(),
        pressed: true,
        repeat: false,
        modifiers: Modifiers {
            control: true,
            alt: true,
            ..Default::default()
        },
    };
    web_input::route_key(&open, key.clone(), Some("@"), false);
    assert!(
        matches!(inputs.borrow().last(), Some(ViewerInput::Web { input: viewer_web::CommandKind::Text(text), .. }) if text == "@")
    );
    inputs.borrow_mut().clear();
    web_input::route_key(&open, key, Some("\u{1}"), false);
    assert!(inputs.borrow().iter().all(|input| !matches!(
        input,
        ViewerInput::Web {
            input: viewer_web::CommandKind::Text(_),
            ..
        }
    )));
    close(&open);
}

#[test]
fn remapped_html_shortcuts_use_the_live_resolver_and_space_waits_for_browser() {
    use super::*;
    let (_runtime, _manager, open, _service) = image_presentation_tests::open();
    open.timer.stop();
    open.window.set_web_mode(true);
    open.window.set_image_mode(false);
    let bindings = ShortcutMap::new(&[
        ShortcutBinding {
            action: "viewer.find".into(),
            keys: vec!["Ctrl+G".into()],
        },
        ShortcutBinding {
            action: "viewer.copy".into(),
            keys: vec!["Ctrl+Y".into()],
        },
        ShortcutBinding {
            action: "viewer.next-file".into(),
            keys: vec!["Ctrl+N".into()],
        },
    ])
    .unwrap();
    *open.key_resolver.borrow_mut() = Rc::new(move |chord, search| {
        bindings
            .resolve(
                chord,
                if search {
                    ShortcutContext::ViewerSearch
                } else {
                    ShortcutContext::Viewer
                },
            )
            .map(|action| action.id.clone())
    });
    let inputs = Rc::new(RefCell::new(Vec::new()));
    let capture = inputs.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        capture.borrow_mut().push(input);
        true
    });
    let control = |key: &str| viewer_web::Key {
        key: key.into(),
        code: format!("Key{}", key.to_uppercase()),
        pressed: true,
        repeat: false,
        modifiers: Modifiers {
            control: true,
            ..Default::default()
        },
    };
    assert!(web_input::route_key(&open, control("g"), None, true));
    assert!(
        open.window.get_search_visible(),
        "remapped Find must open the host field"
    );
    let mut released = control("g");
    released.pressed = false;
    assert!(web_input::route_key(&open, released, None, true));
    open.window.set_search_visible(false);
    open.window.invoke_focus_content();
    assert!(web_input::route_key(&open, control("g"), None, true));
    assert!(
        open.window.get_search_visible(),
        "Find must work after key release in the search field"
    );
    assert!(matches!(
        inputs.borrow().last(),
        Some(ViewerInput::Web {
            input: viewer_web::CommandKind::Focus(false),
            ..
        })
    ));
    open.window.invoke_focus_content();
    assert!(web_input::route_key(&open, control("y"), None, true));
    assert!(matches!(
        inputs.borrow().last(),
        Some(ViewerInput::Copy { .. })
    ));
    assert!(web_input::route_key(&open, control("n"), None, true));
    assert!(matches!(
        inputs.borrow().last(),
        Some(ViewerInput::NextFile)
    ));
    inputs.borrow_mut().clear();
    assert!(web_input::route_key(&open, control("f"), None, true));
    assert!(
        matches!(inputs.borrow().last(), Some(ViewerInput::Web { input: viewer_web::CommandKind::Key(key), .. }) if key.key == "f")
    );
    inputs.borrow_mut().clear();
    let space = viewer_web::Key {
        key: " ".into(),
        code: "Space".into(),
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    };
    assert!(web_input::route_key(&open, space, Some(" "), true));
    assert!(
        inputs
            .borrow()
            .iter()
            .all(|input| !matches!(input, ViewerInput::NextFile))
    );
    web_input::received(&open, HostShortcut::CycleNext);
    assert!(
        matches!(inputs.borrow().last(), Some(ViewerInput::Web { input: viewer_web::CommandKind::Wheel(wheel), .. }) if wheel.delta_y < 0.0)
    );
    close(&open);
}

#[test]
fn browser_arbitrated_space_dispatches_its_configured_action() {
    use super::*;
    let (_runtime, _manager, open, _service) = image_presentation_tests::open();
    open.timer.stop();
    open.window.set_web_mode(true);
    open.window.set_image_mode(false);
    *open.key_resolver.borrow_mut() = Rc::new(|chord, search| {
        (!search && chord.key == "Space").then(|| "viewer.next-file".into())
    });
    let inputs = Rc::new(RefCell::new(Vec::new()));
    let capture = inputs.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        capture.borrow_mut().push(input);
        true
    });
    web_input::received(&open, HostShortcut::CycleNext);
    assert!(matches!(
        inputs.borrow().last(),
        Some(ViewerInput::NextFile)
    ));
    close(&open);
}

#[test]
fn space_goes_to_browser_for_live_editable_focus_arbitration() {
    assert_eq!(browser_key(" "), " ");
    assert_eq!(browser_key("Space"), " ");
}

#[test]
fn escape_remains_mandatory_with_an_empty_html_shortcut_map() {
    use super::*;
    let (_runtime, _manager, open, _service) = image_presentation_tests::open();
    open.timer.stop();
    open.window.set_web_mode(true);
    *open.key_resolver.borrow_mut() = Rc::new(|_, _| None);
    web_input::route_key(
        &open,
        viewer_web::Key {
            key: "Escape".into(),
            code: "Escape".into(),
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        },
        None,
        true,
    );
    assert!(open.closed.get());
}

#[test]
fn html_surface_keeps_search_zoom_and_policy_in_normal_viewer() {
    use super::*;
    use std::sync::Arc;
    let (_runtime, _manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    let inputs = Rc::new(RefCell::new(Vec::new()));
    let captured = inputs.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        captured.borrow_mut().push(input);
        true
    });
    let frame = |revision| {
        ViewerOutput::Frame(Box::new(ViewerFrame {
            generation: 2,
            raster: None,
            raster_rebase: None,
            position_sequence: [0; 2],
            revision,
            mode: VIEWER_MODE_HTML,
            modes: vec![VIEWER_MODE_HTML],
            wrap_enabled: false,
            wrap_available: false,
            image: None,
            width: 2,
            height: 2,
            pixels: Arc::from([255; 16]),
            tiles: None,
            accessible_text: "HTML".into(),
            position: 0.0,
            vertical_extent: 1.0,
            horizontal_position: 0.0,
            horizontal_extent: 1.0,
            caption: "HTML document".into(),
            status: "Offline".into(),
        }))
    };
    present_outputs(&open, &service, &|_| {}, vec![frame(2)]);
    assert!(open.window.get_web_mode());
    assert!(!open.window.get_image_mode());
    assert!(!open.window.get_font_available());
    assert!(!open.window.get_web_network());
    assert!(!open.window.get_web_scripts());
    assert!(dispatch_key(&open, "viewer.zoom-in"));
    assert!(
        matches!(inputs.borrow().last(), Some(ViewerInput::Web { input: viewer_web::CommandKind::Zoom(value), .. }) if *value == 1.25)
    );
    assert!(dispatch_key(&open, "viewer.find"));
    assert!(open.window.get_search_visible());
    open.window.set_query("needle".into());
    assert!(dispatch_key(&open, "viewer.find-next"));
    assert!(
        matches!(inputs.borrow().last(), Some(ViewerInput::Search { query, .. }) if query == "needle")
    );
    open.window.invoke_web_policy(true, true);
    assert!(open.window.get_web_network());
    assert!(open.window.get_web_scripts());
    present_outputs(&open, &service, &|_| {}, vec![frame(3)]);
    assert_eq!(open.frame.get(), 2, "pre-policy pixels must remain blocked");
    open.window.invoke_web_policy(false, true);
    assert!(!open.window.get_web_network());
    assert!(
        !open.window.get_web_scripts(),
        "offline always disables scripts"
    );
    close(&open);
}

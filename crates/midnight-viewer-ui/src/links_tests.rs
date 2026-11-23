//! A viewer window's links (`links.rs`): hover questions and answers, the release that
//! follows a link, Tab/Shift+Tab/Enter only while there is a link to move to, history
//! from keys and mouse buttons, and requests for the main window.
use super::image_presentation_tests::{self, present as present_outputs};
use super::*;
use midnight_core::{ExternalUrl, ViewerLinkInput, ViewerLinkRequest};
use slint::platform::{Key, WindowEvent};
use slint::winit_030::winit::{
    dpi::PhysicalPosition,
    event::{DeviceId, ElementState, MouseButton, WindowEvent as NativeEvent},
};

/// A retained Markdown frame whose displayed camera is at (100, 200).
fn frame(revision: u64) -> ViewerOutput {
    let ViewerOutput::Frame(mut frame) = text_presentation_tests_frame(revision) else {
        unreachable!()
    };
    // The plugin API's document (Markdown) and text modes.
    frame.mode = 4;
    frame.modes = vec![4, 1];
    frame.width = 600;
    frame.height = 600;
    frame.pixels = vec![255; 600 * 600 * 4].into();
    frame.raster = Some(ViewerRasterInfo {
        surface: ViewerSurfaceId(90),
        pixel_id: 1,
        origin: [100.0, 200.0],
        raster_origin: [0.0, 0.0],
        viewport: [400, 240],
        coverage_min: [0.0; 2],
        coverage_max: [200.0, 360.0],
        limit_min: [Some(0.0); 2],
        limit_max: [None; 2],
        pan_sequence: 0,
        scroll_per_pixel: [0.001, 0.0001],
        scroll_map: None,
        settled: true,
        gutter: None,
        bands: None,
    });
    ViewerOutput::Frame(frame)
}

fn text_presentation_tests_frame(revision: u64) -> ViewerOutput {
    super::text_presentation_tests::frame(revision, 0, 0.2)
}

/// The inputs the window sends its worker, instead of a worker.
fn captured(open: &OpenWindow) -> Rc<RefCell<Vec<ViewerInput>>> {
    open.timer.stop();
    let inputs = Rc::new(RefCell::new(Vec::new()));
    let capture = inputs.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        capture.borrow_mut().push(input);
        true
    });
    inputs
}

fn hovers(inputs: &RefCell<Vec<ViewerInput>>) -> Vec<(f32, f32)> {
    inputs
        .borrow()
        .iter()
        .filter_map(|input| match input {
            ViewerInput::Raster {
                input: ViewerRasterInput::Hover { x, y, .. },
                ..
            } => Some((*x, *y)),
            _ => None,
        })
        .collect()
}

fn links(inputs: &RefCell<Vec<ViewerInput>>) -> Vec<ViewerLinkInput> {
    inputs
        .borrow()
        .iter()
        .filter_map(|input| match input {
            ViewerInput::Link(input) => Some(*input),
            _ => None,
        })
        .collect()
}

#[test]
fn one_hover_question_waits_at_a_time_and_its_answer_sets_the_cursor_and_status() {
    let (_runtime, _manager, open, service) = image_presentation_tests::open();
    let inputs = captured(&open);
    present_outputs(&open, &service, &|_| {}, vec![frame(10)]);
    let scale = [
        400.0 / open.window.get_raster_viewport_width().max(1.0),
        240.0 / open.window.get_raster_viewport_height().max(1.0),
    ];
    for x in [10.0, 11.0, 12.0] {
        open.window.invoke_hover_point(x, 20.0);
    }
    assert_eq!(hovers(&inputs), [(10.0 * scale[0], 20.0 * scale[1])]);
    let answer = |status: Option<&str>| ViewerOutput::LinkHover(status.map(str::to_owned));
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![answer(Some("Link to https://example.com/"))],
    );
    assert!(open.window.get_link_hover(), "the pointer cursor");
    assert_eq!(
        open.window.get_link_status(),
        "Link to https://example.com/"
    );
    assert_eq!(
        hovers(&inputs)[1..],
        [(12.0 * scale[0], 20.0 * scale[1])],
        "the newest position is asked next, the one between is skipped"
    );
    present_outputs(&open, &service, &|_| {}, vec![answer(None)]);
    assert!(!open.window.get_link_hover());
    assert_eq!(open.window.get_link_status(), "");
    // An answer of no link (also the worker's answer to a hover no shown text reads)
    // lets the next hover through.
    open.window.invoke_hover_point(13.0, 20.0);
    assert_eq!(hovers(&inputs)[2..], [(13.0 * scale[0], 20.0 * scale[1])]);
    // Leaving the text clears it, and a late answer cannot bring it back.
    open.window.invoke_hover_exit();
    present_outputs(&open, &service, &|_| {}, vec![answer(Some("Link to late"))]);
    assert!(!open.window.get_link_hover());
    assert_eq!(open.window.get_link_status(), "");
    close(&open);
}

#[test]
fn a_release_goes_to_the_worker_with_the_displayed_camera() {
    let (_runtime, _manager, open, service) = image_presentation_tests::open();
    let inputs = captured(&open);
    present_outputs(&open, &service, &|_| {}, vec![frame(10)]);
    open.window.invoke_release_point(30.0, 40.0);
    let released: Vec<_> = inputs
        .borrow()
        .iter()
        .filter_map(|input| match input {
            ViewerInput::Raster {
                surface,
                input:
                    ViewerRasterInput::Release {
                        pixel_id, origin, ..
                    },
            } => Some((*surface, *pixel_id, *origin)),
            _ => None,
        })
        .collect();
    let camera = open.retained_raster.borrow().camera.origin;
    assert_eq!(released, [(ViewerSurfaceId(90), 1, camera)]);
    close(&open);
}

fn key(open: &OpenWindow, key: Key, shift: bool) {
    let window = open.window.window();
    if shift {
        window.dispatch_event(WindowEvent::KeyPressed {
            text: Key::Shift.into(),
        });
    }
    window.dispatch_event(WindowEvent::KeyPressed { text: key.into() });
    window.dispatch_event(WindowEvent::KeyReleased { text: key.into() });
    if shift {
        window.dispatch_event(WindowEvent::KeyReleased {
            text: Key::Shift.into(),
        });
    }
}

/// Tab, Shift+Tab and Enter resolve to the link actions, as the default catalog binds them.
fn link_keys(open: &OpenWindow) {
    *open.key_resolver.borrow_mut() = Rc::new(|chord: &ShortcutChord, search| {
        if search {
            return None;
        }
        let action = match (chord.key.as_str(), chord.shift) {
            ("Tab", false) => "viewer.next-link",
            ("Tab", true) => "viewer.previous-link",
            ("Enter", false) => "viewer.follow-link",
            ("Left", true) if chord.alt => "viewer.link-back",
            ("Right", true) if chord.alt => "viewer.link-forward",
            _ => return None,
        };
        Some(action.to_owned())
    });
}

#[test]
fn tab_shift_tab_and_enter_stay_with_the_links_only_while_there_is_one_to_move_to() {
    let (_runtime, _manager, open, service) = image_presentation_tests::open();
    let inputs = captured(&open);
    link_keys(&open);
    present_outputs(&open, &service, &|_| {}, vec![frame(10)]);
    let focus = |count, focused| ViewerOutput::LinkFocus { count, focused };
    open.window.invoke_focus_content();
    // Without links, Tab moves the keyboard focus on to the window's controls.
    key(&open, Key::Tab, false);
    assert!(links(&inputs).is_empty());
    assert!(!open.window.get_content_focused());
    open.window.invoke_focus_content();
    present_outputs(&open, &service, &|_| {}, vec![focus(2, None)]);
    key(&open, Key::Tab, false);
    key(&open, Key::Return, false);
    assert_eq!(
        links(&inputs),
        [ViewerLinkInput::Next],
        "Enter waits for a focused link"
    );
    assert!(open.window.get_content_focused());
    present_outputs(&open, &service, &|_| {}, vec![focus(2, Some(1))]);
    key(&open, Key::Return, false);
    key(&open, Key::Tab, true);
    assert_eq!(
        links(&inputs)[1..],
        [ViewerLinkInput::Follow, ViewerLinkInput::Previous]
    );
    // From the last link, Tab leaves the links.
    key(&open, Key::Tab, false);
    assert_eq!(links(&inputs).len(), 3);
    assert!(!open.window.get_content_focused());
    close(&open);
}

#[test]
fn html_pages_keep_their_keys_but_history_keys_and_mouse_buttons_go_back_and_forward() {
    let (_runtime, _manager, open, service) = image_presentation_tests::open();
    let inputs = captured(&open);
    present_outputs(&open, &service, &|_| {}, vec![frame(10)]);
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::LinkFocus {
            count: 3,
            focused: Some(0),
        }],
    );
    for action in ["viewer.link-back", "viewer.link-forward"] {
        assert!(dispatch_key(&open, action));
    }
    open.window.invoke_link_history(true);
    open.window.invoke_link_history(false);
    assert_eq!(
        links(&inputs),
        [
            ViewerLinkInput::Back,
            ViewerLinkInput::Forward,
            ViewerLinkInput::Back,
            ViewerLinkInput::Forward
        ]
    );
    open.window.set_web_mode(true);
    for action in [
        "viewer.next-link",
        "viewer.previous-link",
        "viewer.follow-link",
    ] {
        assert!(!dispatch_key(&open, action), "{action} stays with the page");
    }
    assert_eq!(links(&inputs).len(), 4);
    close(&open);
}

fn cursor(open: &OpenWindow, x: f32, y: f32) -> NativeEvent {
    let scale = f64::from(open.window.window().scale_factor());
    NativeEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(f64::from(x) * scale, f64::from(y) * scale),
    }
}

fn button(button: MouseButton) -> NativeEvent {
    NativeEvent::MouseInput {
        device_id: DeviceId::dummy(),
        state: ElementState::Pressed,
        button,
    }
}

#[test]
fn an_html_page_does_not_get_the_mouse_back_and_forward_buttons() {
    let (_runtime, _manager, open, service) = image_presentation_tests::open();
    let inputs = captured(&open);
    present_outputs(&open, &service, &|_| {}, vec![frame(10)]);
    open.window.set_image_mode(false);
    open.window.set_web_mode(true);
    let x = open.window.get_content_left() + 20.0;
    let y = open.window.get_content_top() + 20.0;
    assert!(web_input::observe(&open, &cursor(&open, x, y), true));
    for pressed in [MouseButton::Back, MouseButton::Forward] {
        assert!(web_input::observe(&open, &button(pressed), true));
    }
    assert_eq!(
        links(&inputs),
        [ViewerLinkInput::Back, ViewerLinkInput::Forward]
    );
    assert!(
        !inputs.borrow().iter().any(|input| matches!(
            input,
            ViewerInput::Web {
                input: viewer_web::CommandKind::Pointer(viewer_web::Pointer {
                    button: viewer_web::PointerButton::Back | viewer_web::PointerButton::Forward,
                    ..
                }),
                ..
            }
        )),
        "the browser never sees them"
    );
    close(&open);
}

#[test]
fn a_request_goes_to_the_main_window_and_its_answer_back_to_the_worker() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    let inputs = captured(&open);
    let asked = Rc::new(RefCell::new(Vec::new()));
    let record = asked.clone();
    manager.set_link_listener(Rc::new(move |request, reply| {
        record.borrow_mut().push(request);
        reply("Opened in your browser: https://example.com/".into());
    }));
    let url = ExternalUrl::parse("https://example.com/").unwrap();
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::LinkRequest(ViewerLinkRequest::OpenExternal(
            url.clone(),
        ))],
    );
    assert_eq!(*asked.borrow(), [ViewerLinkRequest::OpenExternal(url)]);
    assert!(inputs.borrow().iter().any(|input| matches!(
        input,
        ViewerInput::LinkOutcome(text) if text == "Opened in your browser: https://example.com/"
    )));
    close(&open);
}

/// The HTML row's Back and Forward buttons follow the worker's history state; they, the
/// Reload button and F5/Ctrl+R (`viewer.reload`) reach the worker; Alt+Left and Alt+Right,
/// once Chromium handed them back, go through the history while there is a page in that
/// direction and otherwise keep their file action.
#[test]
fn html_browser_controls_follow_the_history_and_alt_arrows_prefer_it() {
    let (_runtime, _manager, open, service) = image_presentation_tests::open();
    let inputs = captured(&open);
    present_outputs(&open, &service, &|_| {}, vec![frame(10)]);
    open.window.set_image_mode(false);
    open.window.set_web_mode(true);
    open.window.invoke_focus_content();
    assert!(!open.window.get_link_back_available() && !open.window.get_link_forward_available());
    let history = |back, forward| ViewerOutput::LinkHistory { back, forward };
    present_outputs(&open, &service, &|_| {}, vec![history(true, false)]);
    assert!(open.window.get_link_back_available() && !open.window.get_link_forward_available());
    // The default bindings: Alt+Left and Alt+Right cycle files.
    *open.key_resolver.borrow_mut() = Rc::new(|chord: &ShortcutChord, _| {
        let action = match (chord.key.as_str(), chord.alt) {
            ("Left", true) => "viewer.previous-file",
            ("Right", true) => "viewer.next-file",
            ("F5", false) => "viewer.reload",
            _ => return None,
        };
        Some(action.to_owned())
    });
    let shortcut = |value| {
        present_outputs(
            &open,
            &service,
            &|_| {},
            vec![ViewerOutput::WebShortcut(value)],
        )
    };
    shortcut(viewer_web::HostShortcut::AltLeft);
    shortcut(viewer_web::HostShortcut::AltRight);
    assert_eq!(links(&inputs), [ViewerLinkInput::Back]);
    assert!(
        matches!(inputs.borrow().last(), Some(ViewerInput::NextFile)),
        "no page forward: Alt+Right keeps the next file"
    );
    present_outputs(&open, &service, &|_| {}, vec![history(false, true)]);
    shortcut(viewer_web::HostShortcut::AltRight);
    shortcut(viewer_web::HostShortcut::AltLeft);
    assert_eq!(
        links(&inputs),
        [ViewerLinkInput::Back, ViewerLinkInput::Forward]
    );
    assert!(matches!(
        inputs.borrow().last(),
        Some(ViewerInput::PreviousFile)
    ));
    open.window.invoke_link_history(true);
    open.window.invoke_reload();
    assert!(dispatch_key(&open, "viewer.reload"));
    assert_eq!(
        links(&inputs)[2..],
        [
            ViewerLinkInput::Back,
            ViewerLinkInput::Reload,
            ViewerLinkInput::Reload
        ]
    );
    close(&open);
}

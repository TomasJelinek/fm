//! Widgets that assign their own value must keep showing the state in force.
//! Drives the real controls through accessibility actions and keys.
use super::image_presentation_tests::present as present_outputs;
use super::*;
use i_slint_core::{
    accessibility::{AccessibilityAction, AccessibleStringProperty},
    item_tree::ItemRc,
    window::WindowInner,
};
use slint::platform::{Key, WindowEvent};

const ONLINE: &str = "Allow network for this document only";
const SCRIPTS: &str = "Allow scripts to read and transmit this document";

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

fn control(open: &OpenWindow, label: &str) -> ItemRc {
    let inner = WindowInner::from_pub(open.window.window());
    // Instantiate conditional rows and run pending change handlers.
    inner.ensure_tree_instantiated();
    find(ItemRc::new_root(inner.component()), label).unwrap_or_else(|| panic!("missing {label}"))
}

fn shown(open: &OpenWindow, label: &str, property: AccessibleStringProperty) -> String {
    control(open, label)
        .accessible_string_property(property)
        .unwrap_or_default()
        .into()
}

fn text_frame(revision: u64, generation: u64, mode: u16, modes: &[u16]) -> ViewerOutput {
    let ViewerOutput::Frame(mut frame) = text_presentation_tests::frame(revision, 0, 0.2) else {
        unreachable!()
    };
    frame.generation = generation;
    frame.mode = mode;
    frame.media_control = (mode == VIEWER_MODE_VIDEO).then_some(MediaControlId(0));
    frame.modes = modes.to_vec();
    ViewerOutput::Frame(frame)
}

fn press(open: &OpenWindow, key: Key) {
    let text: slint::SharedString = key.into();
    open.window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    open.window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

/// Open the chooser's popup and pick the entry below the current one.
fn choose_next_mode(open: &OpenWindow) {
    control(open, "Viewer mode").accessible_action(&AccessibilityAction::Expand);
    press(open, Key::DownArrow);
    press(open, Key::Return);
    slint::platform::update_timers_and_animations();
}

type Inputs = Rc<RefCell<Vec<ViewerInput>>>;

/// Record inputs; the returned flag decides whether the sender accepts them.
fn capture(open: &OpenWindow) -> (Inputs, Rc<Cell<bool>>) {
    let inputs = Inputs::default();
    let accept = Rc::new(Cell::new(true));
    let (captured, accepting) = (inputs.clone(), accept.clone());
    *open.input.borrow_mut() = Rc::new(move |input| {
        if accepting.get() {
            captured.borrow_mut().push(input);
        }
        accepting.get()
    });
    (inputs, accept)
}

/// Admit the measured canvas before a synthetic producer can deliver its frame.
fn select_and_acknowledge_layout(
    open: &OpenWindow,
    service: &Service,
    inputs: &Inputs,
    mode: u16,
    generation: u64,
    following_controls: Vec<ViewerOutput>,
) {
    assert!(
        inputs.borrow().is_empty(),
        "previous inputs must be accounted for"
    );
    let token = open.layout.borrow().token.checked_add(1).unwrap();
    let mut outputs = vec![ViewerOutput::LayoutSelected {
        token,
        mode,
        content_generation: generation,
    }];
    outputs.extend(following_controls);
    present_outputs(open, service, &|_| {}, outputs);
    assert_eq!(open.layout.borrow().token, token);
    assert!(!open.layout.borrow().pending_ack);
    assert!(open.layout.borrow().awaiting_frame);
    let inputs = std::mem::take(&mut *inputs.borrow_mut());
    assert!(
        matches!(
            inputs.as_slice(),
            [ViewerInput::LayoutReady { token: accepted, geometry }]
                if *accepted == token && geometry.width > 1 && geometry.height > 1
                    && geometry.scale.is_finite() && geometry.scale > 0.0
        ),
        "one exact measured layout acknowledgement, got {inputs:?}"
    );
}

#[test]
fn newer_seek_rejects_pixels_already_prepared_for_the_previous_seek() {
    let (_runtime, _manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    let (inputs, _) = capture(&open);
    select_and_acknowledge_layout(&open, &service, &inputs, VIEWER_MODE_VIDEO, 7, vec![]);
    let mut state = MediaSnapshot {
        session: MediaSessionId(7),
        control: MediaControlId(0),
        revision: 1,
        position: Duration::ZERO,
        duration: Some(Duration::from_secs(10)),
        seekable: true,
        playing: false,
        phase: MediaPhase::Paused,
    };
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![
            text_frame(10, 7, VIEWER_MODE_VIDEO, &[VIEWER_MODE_VIDEO]),
            ViewerOutput::MediaSnapshot(state.clone()),
        ],
    );
    assert_eq!(open.frame.get(), 10);
    let picture = open.window.get_surface();
    let status = open.window.get_status();

    open.window.invoke_media_seek("7".into(), 0.9);
    // The first seek has completed on the worker and its genuine conversion is
    // already queued for native delivery, outside the backend mailbox.
    let ViewerOutput::Frame(mut stale) = text_frame(11, 7, VIEWER_MODE_VIDEO, &[VIEWER_MODE_VIDEO])
    else {
        unreachable!()
    };
    stale.pixels = std::sync::Arc::from([255, 0, 0, 255].repeat(4));
    stale.caption = "Prepared first seek at 90 percent".into();
    stale.media_control = Some(MediaControlId(1));
    let pixels = pixel_preparation::prepare(&stale).unwrap().unwrap();
    state.control = MediaControlId(1);
    state.revision = 2;
    state.position = Duration::from_secs(9);

    // A second accepted seek supersedes that prepared batch before the event
    // loop presents it. This never relies on an arbitrary frame revision floor.
    open.window.invoke_media_seek("7".into(), 0.1);
    let sent = std::mem::take(&mut *inputs.borrow_mut());
    assert!(
        matches!(sent.as_slice(), [
        ViewerInput::MediaControl(MediaControl { session: MediaSessionId(7), id: MediaControlId(1), action: MediaAction::Seek(first) }),
        ViewerInput::MediaControl(MediaControl { session: MediaSessionId(7), id: MediaControlId(2), action: MediaAction::Seek(second) }),
    ] if (first.as_secs_f64() - 9.0).abs() < 0.001
        && (second.as_secs_f64() - 1.0).abs() < 0.001),
        "{sent:?}"
    );
    for control in [Some(MediaControlId(1)), None, Some(MediaControlId(3))] {
        let mut rejected = stale.clone();
        rejected.media_control = control;
        for prepared in [
            Ok(Some(pixels.clone())),
            Err("stale conversion failure".into()),
        ] {
            pixel_delivery::deliver_batch(
                &open,
                &service,
                &|_| {},
                pixel_preparation::PreparedBatch {
                    outputs: vec![
                        ViewerOutput::Frame(rejected.clone()),
                        ViewerOutput::MediaSnapshot(state.clone()),
                    ],
                    pixels: prepared,
                },
            );
            assert_eq!(
                open.frame.get(),
                10,
                "a superseded seek must not publish its prepared frame"
            );
            assert_eq!(open.window.get_surface(), picture);
            assert_eq!(
                open.window.get_status(),
                status,
                "a superseded copy failure must stay retired"
            );
            assert!(
                open.prepared_pixels.borrow().is_none(),
                "stale pixels cannot replace the accepted lease"
            );
            assert!((open.window.get_media_controls().position - 0.1).abs() < 0.0001);
            assert_eq!(open.window.get_media_controls().status, "Seeking…");
        }
    }

    // Play then Pause can overtake the sole decoded preview for seek 2. They
    // change playback intent, not its content, so that preview must still land.
    open.window.invoke_media_toggle("7".into());
    open.window.invoke_media_toggle("7".into());
    let sent = std::mem::take(&mut *inputs.borrow_mut());
    assert!(
        matches!(
            sent.as_slice(),
            [
                ViewerInput::MediaControl(MediaControl {
                    id: MediaControlId(3),
                    action: MediaAction::SetPlaying(true),
                    ..
                }),
                ViewerInput::MediaControl(MediaControl {
                    id: MediaControlId(4),
                    action: MediaAction::SetPlaying(false),
                    ..
                }),
            ]
        ),
        "{sent:?}"
    );
    let ViewerOutput::Frame(mut fresh) = text_frame(12, 7, VIEWER_MODE_VIDEO, &[VIEWER_MODE_VIDEO])
    else {
        unreachable!()
    };
    fresh.media_control = Some(MediaControlId(2));
    fresh.pixels = std::sync::Arc::from([0, 0, 255, 255].repeat(4));
    let prepared = pixel_preparation::prepare(&fresh).unwrap();
    pixel_delivery::deliver_batch(
        &open,
        &service,
        &|_| {},
        pixel_preparation::PreparedBatch {
            outputs: vec![ViewerOutput::Frame(fresh)],
            pixels: Ok(prepared),
        },
    );
    assert_eq!(
        open.frame.get(),
        12,
        "later Play/Pause cannot starve the only sought preview"
    );
    assert_ne!(open.window.get_surface(), picture);
    assert_eq!(
        open.prepared_pixels
            .borrow()
            .as_ref()
            .unwrap()
            .frame_revision,
        12
    );
}

#[test]
fn retired_media_cannot_repopulate_controls_or_pixels_after_a_failed_replacement() {
    let (_runtime, _manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    let (inputs, _) = capture(&open);
    // The presentation fixture deliberately has no shortcuts. Install the real
    // default resolver so physical keyboard events exercise the media gate.
    let bindings = ShortcutMap::new(&[]).unwrap();
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
    let physical_find = || {
        open.window.invoke_focus_content();
        for (text, pressed) in [
            (Key::Control.into(), true),
            ("f".into(), true),
            ("f".into(), false),
            (Key::Control.into(), false),
        ] {
            open.window.window().dispatch_event(if pressed {
                WindowEvent::KeyPressed { text }
            } else {
                WindowEvent::KeyReleased { text }
            });
        }
    };
    select_and_acknowledge_layout(&open, &service, &inputs, VIEWER_MODE_VIDEO, 7, vec![]);
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![text_frame(10, 7, VIEWER_MODE_VIDEO, &[VIEWER_MODE_VIDEO])],
    );
    assert_eq!(open.frame.get(), 10);
    assert!(!open.window.get_media_controls().session.is_empty());
    assert!(!open.window.get_accessible_text().is_empty());
    physical_find();
    assert!(
        !open.window.get_search_visible(),
        "active video owns its input context"
    );

    // Control retirement must survive pixel preparation failure before another
    // route is announced. It also invalidates a previously prepared old frame.
    pixel_delivery::deliver_batch(
        &open,
        &service,
        &|_| {},
        pixel_preparation::PreparedBatch {
            outputs: vec![ViewerOutput::MediaRetired {
                session: MediaSessionId(7),
            }],
            pixels: Err("synthetic preparation failure".into()),
        },
    );
    assert!(open.window.get_media_controls().session.is_empty());
    assert!(open.window.get_accessible_text().is_empty());
    assert_eq!(
        (
            open.window.get_surface().size().width,
            open.window.get_surface().size().height
        ),
        (0, 0)
    );
    let accepted = inputs.borrow().len();
    open.window.invoke_media_toggle("7".into());
    open.window.invoke_media_seek("7".into(), 0.5);
    assert_eq!(inputs.borrow().len(), accepted);
    physical_find();
    assert!(
        open.window.get_search_visible(),
        "retired video must release Ctrl+F"
    );
    let mut snapshot = MediaSnapshot {
        session: MediaSessionId(7),
        control: MediaControlId(0),
        revision: 50,
        position: Duration::ZERO,
        duration: Some(Duration::from_secs(8)),
        seekable: true,
        playing: true,
        phase: MediaPhase::Playing,
    };
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![
            ViewerOutput::MediaSnapshot(snapshot.clone()),
            text_frame(11, 7, VIEWER_MODE_VIDEO, &[VIEWER_MODE_VIDEO]),
        ],
    );
    assert_eq!(open.frame.get(), 10);
    assert!(open.window.get_media_controls().session.is_empty());
    assert_eq!(
        (
            open.window.get_surface().size().width,
            open.window.get_surface().size().height
        ),
        (0, 0)
    );

    // The mailbox sends the latest layout first. An old retirement arriving in
    // the same drain or later cannot retire that newer video or reject its frame.
    snapshot.session = MediaSessionId(9);
    assert!(inputs.borrow().is_empty());
    select_and_acknowledge_layout(
        &open,
        &service,
        &inputs,
        VIEWER_MODE_VIDEO,
        9,
        vec![ViewerOutput::MediaRetired {
            session: MediaSessionId(7),
        }],
    );
    assert_eq!(open.window.get_media_controls().session, "9");
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![
            ViewerOutput::MediaRetired {
                session: MediaSessionId(7),
            },
            ViewerOutput::MediaSnapshot(snapshot),
            text_frame(12, 9, VIEWER_MODE_VIDEO, &[VIEWER_MODE_VIDEO]),
        ],
    );
    assert_eq!(open.window.get_media_controls().session, "9");
    assert_eq!(open.frame.get(), 12);
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::MediaRetired {
            session: MediaSessionId(7),
        }],
    );
    assert_eq!(open.window.get_media_controls().session, "9");
    assert_eq!(
        (
            open.window.get_surface().size().width,
            open.window.get_surface().size().height
        ),
        (2, 2)
    );
}

#[test]
fn video_status_changes_keep_the_canvas_and_same_session_advancing_pixels() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    let (inputs, _) = capture(&open);
    let snapshot = |revision, seconds, playing| {
        ViewerOutput::MediaSnapshot(MediaSnapshot {
            session: MediaSessionId(7),
            control: MediaControlId(u64::from(playing)),
            revision,
            position: Duration::from_secs(seconds),
            duration: Some(Duration::from_secs(8)),
            seekable: true,
            playing,
            phase: if playing {
                MediaPhase::Playing
            } else {
                MediaPhase::Paused
            },
        })
    };
    select_and_acknowledge_layout(
        &open,
        &service,
        &inputs,
        VIEWER_MODE_VIDEO,
        7,
        vec![snapshot(1, 0, false)],
    );
    let frame = |revision, color: [u8; 4]| {
        let ViewerOutput::Frame(mut frame) =
            text_frame(revision, 7, VIEWER_MODE_VIDEO, &[VIEWER_MODE_VIDEO])
        else {
            unreachable!()
        };
        frame.media_control = Some(MediaControlId(u64::from(revision > 10)));
        frame.pixels = std::sync::Arc::from(color.repeat(4));
        frame.caption = format!("Playback frame {revision}");
        frame
    };
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::Frame(frame(10, [17, 83, 149, 255]))],
    );
    assert_eq!(open.frame.get(), 10);
    let first_picture = open.window.get_surface();
    let ready = window_geometry(&open.window);
    assert_eq!(
        open.last_geometry.get(),
        (ready.width, ready.height, ready.scale.to_bits())
    );
    control(&open, "Play").accessible_action(&AccessibilityAction::Default);
    assert_eq!(
        open.window.get_media_controls().status,
        "Starting playback…"
    );
    // The status row is always reserved: a pending command neither moves nor resizes
    // the video, so the backend gets no Resize.
    let pending = window_geometry(&open.window);
    assert_eq!(
        (pending.width, pending.height),
        (ready.width, ready.height),
        "a pending command must not resize the canvas"
    );
    pump_geometry(&open);
    let events = std::mem::take(&mut *inputs.borrow_mut());
    assert!(matches!(
        events.as_slice(),
        [ViewerInput::MediaControl(MediaControl {
            session: MediaSessionId(7),
            id: MediaControlId(1),
            action: MediaAction::SetPlaying(true)
        }),]
    ));
    let advancing = frame(11, [211, 37, 61, 255]);
    assert!(open.layout.borrow().accepts_frame(&advancing));
    assert!(
        open.media_controls
            .accepts_frame(MediaSessionId(7), advancing.media_control)
    );
    assert!(open.palette.allows(&advancing));
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::Frame(advancing), snapshot(2, 1, true)],
    );
    assert_eq!(open.window.get_media_controls().position, 0.125);
    assert_eq!(
        open.frame.get(),
        11,
        "a video canvas resize cannot require a new media session generation"
    );
    assert_ne!(open.window.get_surface(), first_picture);
    assert_eq!(open.window.get_caption(), "Playback frame 11");
    assert!(open.window.get_media_controls().status.is_empty());
    let settled = window_geometry(&open.window);
    assert_eq!((settled.width, settled.height), (ready.width, ready.height));
    pump_geometry(&open);
    assert!(std::mem::take(&mut *inputs.borrow_mut()).is_empty());
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::Frame(frame(12, [31, 191, 67, 255]))],
    );
    assert_eq!(
        open.frame.get(),
        12,
        "a cleared status also preserves the same media generation"
    );
    open.window
        .window()
        .set_size(slint::LogicalSize::new(960.0, 640.0));
    let resized = window_geometry(&open.window);
    assert!(resized.width != settled.width || resized.height != settled.height);
    pump_geometry(&open);
    assert!(
        matches!(std::mem::take(&mut *inputs.borrow_mut()).as_slice(), [ViewerInput::Resize(geometry)]
        if geometry.width == resized.width && geometry.height == resized.height)
    );
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::Frame(frame(13, [71, 53, 229, 255]))],
    );
    assert_eq!(
        open.frame.get(),
        13,
        "native-size changes do not retire a live video generation"
    );
    manager.close_all();
}

#[test]
fn text_source_resize_still_requires_new_position_generation() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    let (inputs, _) = capture(&open);
    for (mode, generation, revision, width, height) in
        [(1, 7, 20, 960.0, 640.0), (2, 9, 30, 900.0, 600.0)]
    {
        select_and_acknowledge_layout(&open, &service, &inputs, mode, generation, vec![]);
        present_outputs(
            &open,
            &service,
            &|_| {},
            vec![text_frame(revision, generation, mode, &[mode])],
        );
        assert_eq!(open.frame.get(), revision);
        assert!(!open.window.get_image_mode());
        let picture = open.window.get_surface();
        let before = window_geometry(&open.window);
        open.window
            .window()
            .set_size(slint::LogicalSize::new(width, height));
        let resized = window_geometry(&open.window);
        assert!(resized.width != before.width || resized.height != before.height);
        pump_geometry(&open);
        assert!(
            matches!(std::mem::take(&mut *inputs.borrow_mut()).as_slice(), [ViewerInput::Resize(geometry)]
            if geometry.width == resized.width && geometry.height == resized.height)
        );
        present_outputs(
            &open,
            &service,
            &|_| {},
            vec![text_frame(revision + 1, generation, mode, &[mode])],
        );
        assert_eq!(
            open.frame.get(),
            revision,
            "text/Source resize must reject the previous position generation"
        );
        assert_eq!(open.window.get_surface(), picture);
        present_outputs(
            &open,
            &service,
            &|_| {},
            vec![text_frame(revision + 2, generation + 1, mode, &[mode])],
        );
        assert_eq!(open.frame.get(), revision + 2);
    }
    manager.close_all();
}

#[test]
fn mounted_video_controls_follow_applied_commands_and_content_replacement() {
    let (_runtime, _manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    let (inputs, _) = capture(&open);
    let present = |output| present_outputs(&open, &service, &|_| {}, vec![output]);
    let state = |session, control, revision, seconds, phase| {
        ViewerOutput::MediaSnapshot(MediaSnapshot {
            session: MediaSessionId(session),
            control: MediaControlId(control),
            revision,
            position: Duration::from_secs(seconds),
            duration: Some(Duration::from_secs(8)),
            seekable: true,
            playing: matches!(&phase, MediaPhase::Playing),
            phase,
        })
    };
    let assert_control = |id, action| {
        let events = std::mem::take(&mut *inputs.borrow_mut());
        let sent = match events.as_slice() {
            [ViewerInput::MediaControl(control)] => *control,
            other => panic!("expected exactly one playback control, got {other:?}"),
        };
        assert_eq!(
            sent,
            MediaControl {
                session: MediaSessionId(7),
                id: MediaControlId(id),
                action,
            }
        );
    };

    select_and_acknowledge_layout(&open, &service, &inputs, VIEWER_MODE_VIDEO, 7, vec![]);
    assert_eq!(open.window.get_media_controls().status, "Opening video…");
    present(text_frame(10, 7, VIEWER_MODE_VIDEO, &[VIEWER_MODE_VIDEO]));
    assert_eq!(open.frame.get(), 10);
    assert!(!open.layout.borrow().awaiting_frame);
    present(state(7, 0, 1, 1, MediaPhase::Paused));
    let play = control(&open, "Play");
    assert_eq!(
        play.accessible_role(),
        i_slint_core::items::AccessibleRole::Button
    );
    assert_eq!(
        shown(&open, "Play", AccessibleStringProperty::Enabled),
        "true"
    );
    assert!(play.geometry().size.width > 0.0 && play.geometry().size.height > 0.0);
    play.accessible_action(&AccessibilityAction::Default);
    assert_control(1, MediaAction::SetPlaying(true));
    present(state(7, 1, 2, 2, MediaPhase::Playing));
    control(&open, "Pause").accessible_action(&AccessibilityAction::Default);
    assert_control(2, MediaAction::SetPlaying(false));
    present(state(7, 2, 3, 2, MediaPhase::Paused));

    open.window.invoke_media_seek("7".into(), 0.5);
    assert_control(3, MediaAction::Seek(Duration::from_secs(4)));
    present(state(7, 2, 4, 6, MediaPhase::Paused));
    assert_eq!(open.window.get_media_controls().status, "Seeking…");
    assert_eq!(open.window.get_media_controls().position, 0.5);
    present(state(7, 3, 5, 3, MediaPhase::Paused));
    assert!(open.window.get_media_controls().status.is_empty());
    assert_eq!(open.window.get_media_controls().position, 0.375);

    select_and_acknowledge_layout(&open, &service, &inputs, 1, 8, vec![]);
    present(state(7, 3, 6, 4, MediaPhase::Playing));
    assert!(open.window.get_media_controls().session.is_empty());
    let sent = inputs.borrow().len();
    open.window.invoke_media_toggle("7".into());
    assert_eq!(inputs.borrow().len(), sent);

    select_and_acknowledge_layout(&open, &service, &inputs, VIEWER_MODE_VIDEO, 9, vec![]);
    present(state(
        9,
        0,
        1,
        0,
        MediaPhase::Failed("Invalid video".into()),
    ));
    assert_eq!(open.window.get_media_controls().error, "Invalid video");
}

#[test]
fn mode_chooser_follows_presented_mode_after_model_changes_and_user_choice() {
    let (_runtime, _manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    let (inputs, accept) = capture(&open);
    let present = |output| present_outputs(&open, &service, &|_| {}, vec![output]);
    let value = || shown(&open, "Viewer mode", AccessibleStringProperty::Value);

    // A one-mode file instantiates the chooser; the next file replaces its model.
    present(text_frame(10, 7, 1, &[1]));
    assert_eq!(value(), "Text");
    present(text_frame(11, 7, 1, &[1, 3]));
    assert_eq!(value(), "Text");
    // Same plugin, binary content: only the presented mode index changes.
    present(text_frame(12, 7, 3, &[1, 3]));
    assert_eq!(value(), "Binary", "a model change detached the chooser");

    // The user's own choice must not detach it either.
    present(text_frame(13, 7, 1, &[1, 3]));
    assert_eq!(value(), "Text");
    choose_next_mode(&open);
    assert!(matches!(inputs.borrow().last(), Some(ViewerInput::Mode(3))));
    assert_eq!(value(), "Binary");
    // The backend applies the choice under a new generation, then the next
    // file of the same plugin opens as text.
    present(text_frame(14, 8, 3, &[1, 3]));
    assert_eq!(value(), "Binary");
    present(text_frame(15, 9, 1, &[1, 3]));
    assert_eq!(value(), "Text", "a user choice detached the chooser");

    // A refused request restores the mode in force.
    accept.set(false);
    let sent = inputs.borrow().len();
    choose_next_mode(&open);
    assert_eq!(inputs.borrow().len(), sent);
    assert_eq!(open.window.get_mode_index(), 0);
    assert_eq!(value(), "Text");
}

#[test]
fn web_policy_boxes_show_policy_in_force_after_document_change_and_refusal() {
    let (_runtime, _manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    let (inputs, accept) = capture(&open);
    let present = |output| present_outputs(&open, &service, &|_| {}, vec![output]);
    let checked = |label| shown(&open, label, AccessibleStringProperty::Checked);
    let html = [VIEWER_MODE_HTML];
    let state = |generation| ViewerOutput::WebState {
        generation,
        policy: viewer_web::Policy::default(),
        policy_change_allowed: true,
        zoom: 1.0,
    };

    present(text_frame(10, 7, VIEWER_MODE_HTML, &html));
    present(state(7));
    control(&open, ONLINE).accessible_action(&AccessibilityAction::Default);
    control(&open, SCRIPTS).accessible_action(&AccessibilityAction::Default);
    assert_eq!([checked(ONLINE), checked(SCRIPTS)], ["true", "true"]);
    assert!(matches!(
        inputs.borrow().last(),
        Some(ViewerInput::Web {
            input: viewer_web::CommandKind::Policy(viewer_web::Policy {
                network: true,
                scripts: true
            }),
            ..
        })
    ));

    // The next HTML document starts offline with scripts disabled.
    present(ViewerOutput::LayoutSelected {
        token: 1,
        mode: VIEWER_MODE_HTML,
        content_generation: 8,
    });
    assert!(!open.window.get_web_network() && !open.window.get_web_scripts());
    assert_eq!([checked(ONLINE), checked(SCRIPTS)], ["false", "false"]);
    // Before its first frame no policy can be sent; the box must not claim one.
    control(&open, ONLINE).accessible_action(&AccessibilityAction::Default);
    assert!(!open.window.get_web_network());
    assert_eq!(checked(ONLINE), "false");

    present(text_frame(11, 8, VIEWER_MODE_HTML, &html));
    present(state(8));
    control(&open, ONLINE).accessible_action(&AccessibilityAction::Default);
    assert_eq!(checked(ONLINE), "true");
    // A full command queue refuses the change: network stays enabled and shown.
    accept.set(false);
    control(&open, ONLINE).accessible_action(&AccessibilityAction::Default);
    assert!(open.window.get_web_network());
    assert_eq!(checked(ONLINE), "true");
    // The accepted change recreates the browser context. Its next frame and the
    // backend's state reply remain authoritative for both boxes.
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![text_frame(12, 9, VIEWER_MODE_HTML, &html), state(9)],
    );
    assert_eq!([checked(ONLINE), checked(SCRIPTS)], ["false", "false"]);
}

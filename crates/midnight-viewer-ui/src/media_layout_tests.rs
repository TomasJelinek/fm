//! Pressing a video control must not move the video, and the media bar is one row:
//! the status shown while a command is in flight, and errors, take the front of the
//! viewer's status line instead of a row of their own.
use crate::{ViewerWindow, media_controls::MediaControls};
use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::{ItemRc, ParentItemTraversalMode},
    window::WindowInner,
};
use midnight_core::{MediaControl, MediaControlId, MediaPhase, MediaSessionId, MediaSnapshot};
use slint::{
    ComponentHandle,
    platform::{
        Key, Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{cell::RefCell, rc::Rc, time::Duration};

const WIDTH: u32 = 900;
const HEIGHT: u32 = 600;

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

/// Whether some text on the window starts with `prefix` (a Text's label is its text).
fn shows(item: ItemRc, prefix: &str) -> bool {
    if item
        .accessible_string_property(AccessibleStringProperty::Label)
        .is_some_and(|value| value.starts_with(prefix))
    {
        return true;
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if shows(item, prefix) {
            return true;
        }
    }
    false
}

fn shows_status(window: &ViewerWindow, prefix: &str) -> bool {
    shows(
        ItemRc::new_root(WindowInner::from_pub(window.window()).component()),
        prefix,
    )
}

/// Window-space rectangle of the media bar: the nearest ancestor of the Play/Pause button
/// that is taller than the button.
fn bar(window: &ViewerWindow, button: &str) -> [f32; 4] {
    let item = control(window, button);
    let height = item.geometry().size.height;
    let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(ancestor) = parent {
        let rect = ancestor.geometry();
        if rect.size.height > height + 0.5 {
            let origin = ancestor.map_to_native_window(rect.origin);
            return [origin.x, origin.y, rect.size.width, rect.size.height];
        }
        parent = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    panic!("no media bar around {button}");
}

fn control(window: &ViewerWindow, label: &str) -> ItemRc {
    find(
        ItemRc::new_root(WindowInner::from_pub(window.window()).component()),
        label,
    )
    .unwrap_or_else(|| panic!("Missing {label}"))
}

/// Window-space rectangle of a labelled widget.
fn bounds(window: &ViewerWindow, label: &str) -> [f32; 4] {
    let item = control(window, label);
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    [origin.x, origin.y, rect.size.width, rect.size.height]
}

fn draw(native: &MinimalSoftwareWindow) {
    let mut pixels = vec![slint::Rgb8Pixel::default(); (WIDTH * HEIGHT) as usize];
    slint::platform::update_timers_and_animations();
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, WIDTH as usize);
    });
}

/// The video area and every row of the controls, as laid out and drawn now.
#[derive(Debug, PartialEq)]
struct Layout {
    video: [f32; 4],
    content: [f32; 4],
    button: [f32; 4],
    timeline: [f32; 4],
    bar: [f32; 4],
}

fn layout(native: &MinimalSoftwareWindow, window: &ViewerWindow) -> Layout {
    window.window().request_redraw();
    draw(native);
    let playing = window.get_media_controls().playing;
    Layout {
        video: bounds(window, "File content"),
        content: [
            window.get_content_left(),
            window.get_content_top(),
            window.get_content_width(),
            window.get_content_height(),
        ],
        button: bounds(window, if playing { "Pause" } else { "Play" }),
        timeline: bounds(window, "Playback position"),
        bar: bar(window, if playing { "Pause" } else { "Play" }),
    }
}

fn snapshot(control: u64, revision: u64, seconds: u64, phase: MediaPhase) -> MediaSnapshot {
    MediaSnapshot {
        session: MediaSessionId(7),
        control: MediaControlId(control),
        revision,
        position: Duration::from_secs(seconds),
        duration: Some(Duration::from_secs(120)),
        seekable: true,
        playing: phase == MediaPhase::Playing,
        phase,
    }
}

fn pointer(window: &ViewerWindow, position: slint::LogicalPosition, down: Option<bool>) {
    let button = PointerEventButton::Left;
    window.window().dispatch_event(match down {
        Some(true) => WindowEvent::PointerPressed { position, button },
        Some(false) => WindowEvent::PointerReleased { position, button },
        None => WindowEvent::PointerMoved { position },
    });
}

fn at(bounds: [f32; 4], fraction: f32) -> slint::LogicalPosition {
    slint::LogicalPosition::new(
        bounds[0] + bounds[2] * fraction,
        bounds[1] + bounds[3] / 2.0,
    )
}

#[test]
fn pressing_video_controls_keeps_the_video_area_and_controls_in_place() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = ViewerWindow::new().unwrap();
    let sent = Rc::new(RefCell::new(Vec::<MediaControl>::new()));
    let accepts = Rc::new(std::cell::Cell::new(true));
    let media = MediaControls::install(
        &window,
        Rc::new({
            let sent = sent.clone();
            let accepts = accepts.clone();
            move |control| {
                sent.borrow_mut().push(control);
                accepts.get()
            }
        }),
    );
    let pixels = [17, 190, 90, 255].repeat(160 * 90);
    window.set_surface(slint::Image::from_rgba8_premultiplied(
        slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&pixels, 160, 90),
    ));
    window.show().unwrap();
    native.set_size(slint::PhysicalSize::new(WIDTH, HEIGHT));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(WIDTH as f32, HEIGHT as f32),
    });
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));

    assert!(media.begin(MediaSessionId(7)));
    draw(&native);
    let opening = [
        bounds(&window, "File content"),
        [
            window.get_content_left(),
            window.get_content_top(),
            window.get_content_width(),
            window.get_content_height(),
        ],
    ];
    assert!(media.apply(snapshot(0, 1, 30, MediaPhase::Paused)));
    let base = layout(&native, &window);
    assert!(base.video[3] > 0.0 && base.content[3] > 0.0, "{base:?}");
    // One row: the bar is the button and its 6 px inset above and below, no empty status row.
    assert!(
        base.bar[3] <= base.button[3] + 12.5,
        "media bar {:.1} px tall around a {:.1} px button",
        base.bar[3],
        base.button[3]
    );

    let check = |step: &str| {
        let now = layout(&native, &window);
        assert_eq!(now, base, "video or controls moved: {step}");
    };

    // Play, then Pause: during the press, while the command is in flight, and
    // once the worker confirms it.
    let mut revision = 1;
    let mut control_id = 0;
    for (name, phase) in [("Play", MediaPhase::Playing), ("Pause", MediaPhase::Paused)] {
        let centre = at(bounds(&window, name), 0.5);
        pointer(&window, centre, None);
        pointer(&window, centre, Some(true));
        check(&format!("{name} pressed"));
        pointer(&window, centre, Some(false));
        control_id += 1;
        assert_eq!(sent.borrow().len(), control_id as usize, "{name} sent");
        assert_ne!(window.get_media_controls().status.as_str(), "");
        check(&format!("{name} in flight"));
        revision += 1;
        assert!(media.apply(snapshot(control_id, revision, 30, phase)));
        check(&format!("{name} confirmed"));
    }

    // Dragging the position bar: press, moves, release (Seeking…), confirmation.
    let timeline = bounds(&window, "Playback position");
    pointer(&window, at(timeline, 0.3), None);
    pointer(&window, at(timeline, 0.3), Some(true));
    check("position bar pressed");
    for fraction in [0.4, 0.55, 0.7] {
        pointer(&window, at(timeline, fraction), None);
        check(&format!("position bar dragged to {fraction}"));
    }
    pointer(&window, at(timeline, 0.7), Some(false));
    control_id += 1;
    assert_eq!(sent.borrow().len(), control_id as usize, "seek sent");
    assert_eq!(window.get_media_controls().status.as_str(), "Seeking…");
    assert!(
        shows_status(&window, "Seeking… · "),
        "the status line leads with the playback status"
    );
    check("seek in flight");
    revision += 1;
    assert!(media.apply(snapshot(control_id, revision, 84, MediaPhase::Paused)));
    check("seek confirmed");

    // Stepping with the keyboard on the focused position bar.
    for key in [Key::RightArrow, Key::LeftArrow, Key::End, Key::Home] {
        window
            .window()
            .dispatch_event(WindowEvent::KeyPressed { text: key.into() });
        window
            .window()
            .dispatch_event(WindowEvent::KeyReleased { text: key.into() });
        control_id += 1;
        assert_eq!(
            sent.borrow().len(),
            control_id as usize,
            "{key:?} seek sent"
        );
        check(&format!("{key:?} seek in flight"));
        revision += 1;
        assert!(media.apply(snapshot(control_id, revision, 30, MediaPhase::Paused)));
        check(&format!("{key:?} seek confirmed"));
    }

    // A refused command shows its error at the front of the status line.
    accepts.set(false);
    let centre = at(bounds(&window, "Play"), 0.5);
    pointer(&window, centre, Some(true));
    pointer(&window, centre, Some(false));
    assert_ne!(window.get_media_controls().error.as_str(), "");
    let error = window.get_media_controls().error.to_string();
    assert!(
        shows_status(&window, &format!("{error} · ")),
        "the status line leads with the error"
    );
    check("refused command");
    accepts.set(true);
    revision += 1;
    assert!(media.apply(snapshot(control_id, revision, 120, MediaPhase::Ended)));
    assert_eq!(
        window.get_media_controls().status.as_str(),
        "Playback ended"
    );
    check("playback ended");
    assert_eq!(
        [base.video, base.content],
        opening,
        "Opening video… and a ready video differ"
    );
    window.hide().unwrap();
}

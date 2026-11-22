//! Winit supplies physical, layout-independent input and IME once per browser event.
use super::*;
use i_slint_core::window::{WindowInner, WindowKind};
use midnight_core::viewer_web as api;
use midnight_ui_input::WheelMotion;
use slint::winit_030::winit::{
    event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent},
    keyboard::{Key, PhysicalKey},
    platform::modifier_supplement::KeyEventExtModifierSupplement,
};

/// Logical pixels per touchpad line (X11 finger motion), as the shared scroller
/// converts them. A notch glides by the text viewer's distance instead
/// (`image_motion::notch_pixels`).
const LINE: f32 = midnight_ui_input::LINE_PIXELS;
/// A wheel event with Ctrl, Alt or Super acts once instead of scrolling: Chromium
/// zooms with Ctrl, counting 120 units as one step. It keeps the browser's previous
/// 40 px line, so Ctrl+wheel zooms as fast as before.
const ACTING_LINE: f64 = 40.0;
/// Chromium reports no scroll position: a newly presented page frame is the only sign
/// that the page moved. Once its position is known (see [`Began`]), a page that paints
/// nothing new for this long while deltas arrive is at an edge. The shared kinetic
/// scroller stops momentum after the same time.
const STALL: Duration = Duration::from_millis(120);
/// Chromium took up to 650 ms to paint the first scroll of a freshly loaded page (4 of
/// 96 measured gestures), so an unpainted scroll is not taken to be at an edge sooner.
const FIRST_PAINT: Duration = Duration::from_secs(1);

/// When a scroll began and the page frame shown then.
#[derive(Clone, Copy, Default)]
struct Began {
    frame: u64,
    at: Duration,
}

impl Began {
    fn now(open: &OpenWindow) -> Self {
        Self {
            frame: open.frame.get(),
            at: image_motion::now(open),
        }
    }

    /// The page's position is known once it painted since the scroll began, or once a
    /// first scroll frame would have been painted.
    fn known(&self, frame: u64, now: Duration) -> bool {
        frame != self.frame || now.saturating_sub(self.at) >= FIRST_PAINT
    }
}

#[derive(Default)]
pub(super) struct Native {
    cursor: Option<[f64; 2]>,
    modifiers: api::Modifiers,
    dragging: bool,
    last_click: Option<(std::time::Instant, api::PointerButton, [f64; 2], u8)>,
    shortcut_keys: std::collections::HashMap<String, String>,
    /// Document policy in force (offline for every new document).
    policy: api::Policy,
    glide: Glide,
    /// Requests a redraw (or steps the glide without native events) while it runs.
    redraw: Timer,
    /// Touchpad reports and momentum sent at once: when the stream began, and when its
    /// newest delta went out.
    stream: Option<(Began, Duration)>,
}

/// Wheel notches over the page glide exactly as in the text viewer: the shared
/// [`WheelMotion`] runs here, and each presented frame sends its share to the page as
/// one pixel delta. The provider turns Chromium's own wheel animation off.
#[derive(Default)]
struct Glide {
    motion: WheelMotion,
    /// Surface point and modifiers of the newest notch; every frame's delta goes there.
    target: Option<([f64; 2], api::Modifiers)>,
    /// A frame's physical delta that the backend queue refused. It goes out first, and
    /// the glide's clock waits for it instead of collecting a catch-up jump.
    unsent: Option<[f64; 2]>,
    /// Content generation the glide scrolls.
    generation: u64,
    began: Began,
    /// Newest presented page frame the glide has seen, and when the first delta after
    /// it went out.
    painted: u64,
    quiet: Option<Duration>,
}

impl Glide {
    fn active(&self) -> bool {
        self.motion.active() || self.unsent.is_some()
    }

    fn cancel(&mut self) {
        self.motion.cancel();
        self.target = None;
        self.unsent = None;
        self.quiet = None;
    }
}

/// Physical scale and the surface pixels per drawn pixel on each axis. The surface is
/// drawn over its logical raster viewport (1:1 physical pixels at native resolution),
/// which can differ from the canvas by a clipped half pixel.
fn surface_scale(open: &OpenWindow) -> (f64, [f64; 2]) {
    let scale = f64::from(open.window.window().scale_factor());
    let (raster_width, raster_height) = open.raster_size.get();
    let drawn = [
        f64::from(open.window.get_raster_viewport_width()) * scale,
        f64::from(open.window.get_raster_viewport_height()) * scale,
    ];
    (
        scale,
        [
            f64::from(raster_width) / drawn[0].max(1.0),
            f64::from(raster_height) / drawn[1].max(1.0),
        ],
    )
}

/// The page's position for touchpad momentum: its newest presented frame (see
/// [`STALL`]), or unknown while the touchpad stream's first scroll may still be painting.
pub(super) fn content_position(open: &OpenWindow) -> [Option<f64>; 2] {
    let frame = open.frame.get();
    let now = image_motion::now(open);
    let known = open
        .web_native
        .borrow()
        .stream
        .is_none_or(|(began, _)| began.known(frame, now));
    [known.then_some(frame as f64); 2]
}

/// A touchpad report or momentum frame goes to the page now.
fn streamed(open: &OpenWindow, native: &mut Native) {
    let now = image_motion::now(open);
    native.stream = Some(match native.stream {
        Some((began, last)) if now.saturating_sub(last) < STALL => (began, now),
        _ => (Began::now(open), now),
    });
}

fn cancel_glide(open: &OpenWindow) {
    let mut native = open.web_native.borrow_mut();
    native.glide.cancel();
    native.redraw.stop();
}

/// The glide's frame clock runs (tests, and the acceptance seams' frames).
#[cfg(any(test, feature = "acceptance-seams"))]
pub(super) fn gliding(open: &OpenWindow) -> bool {
    open.web_native.borrow().redraw.running()
}

/// One presented frame of the notch glide: send this frame's delta to the page.
fn step(open: &OpenWindow) {
    let now = image_motion::now(open);
    let next = {
        let mut native = open.web_native.borrow_mut();
        let native = &mut *native;
        let glide = &mut native.glide;
        if !glide.active() {
            native.redraw.stop();
            return;
        }
        if open.closed.get()
            || !open.window.get_web_mode()
            || open.window.get_sandbox_notice_visible()
            || open.layout.borrow().awaiting_frame
            || glide.generation != open.frame_generation.get()
        {
            glide.cancel();
            native.redraw.stop();
            return;
        }
        let painted = open.frame.get();
        if painted != glide.painted {
            glide.painted = painted;
            glide.quiet = None;
        }
        if glide.began.known(painted, now)
            && glide
                .quiet
                .is_some_and(|since| now.saturating_sub(since) >= STALL)
        {
            glide.cancel();
            native.redraw.stop();
            return;
        }
        let delta = glide.unsent.take().or_else(|| {
            let (x, y) = glide.motion.tick(now)?;
            let (scale, ratio) = surface_scale(open);
            Some([
                f64::from(x) * scale * ratio[0],
                f64::from(y) * scale * ratio[1],
            ])
        });
        match (delta, glide.target) {
            (Some(delta), Some(target)) => (delta, target),
            _ => {
                if !glide.active() {
                    glide.cancel();
                    native.redraw.stop();
                }
                return;
            }
        }
    };
    let ([delta_x, delta_y], ([x, y], modifiers)) = next;
    let sent = send(
        open,
        api::CommandKind::Wheel(api::Wheel {
            x,
            y,
            delta_x,
            delta_y,
            modifiers,
        }),
    );
    let glide = &mut open.web_native.borrow_mut().glide;
    if sent {
        glide.quiet.get_or_insert(now);
    } else {
        glide.unsent = Some([delta_x, delta_y]);
    }
}

/// Add a notch to the glide. Like the text viewer, only undelivered distance adds up,
/// and a reversed notch drops the old direction's remainder.
fn glide(open: &OpenWindow, native: &mut Native, target: [f64; 2], lines: (f32, f32)) {
    let generation = open.frame_generation.get();
    let glide = &mut native.glide;
    if !glide.active() || glide.generation != generation {
        glide.cancel();
        glide.generation = generation;
        glide.began = Began::now(open);
        glide.painted = glide.began.frame;
    }
    let (x, y) = (
        lines.0 * image_motion::notch_pixels(open, true),
        lines.1 * image_motion::notch_pixels(open, false),
    );
    // With less motion, the notch goes to the page whole on the next frame.
    let now = image_motion::now(open);
    let pushed = if image_motion::reduce_motion(open) {
        glide.motion.push_direct(x, y, now)
    } else {
        glide.motion.push(x, y, now)
    };
    if !pushed {
        return;
    }
    if let Some(unsent) = &mut glide.unsent {
        for (old, new) in unsent.iter_mut().zip([x, y]) {
            if new != 0.0 && old.signum() != f64::from(new).signum() {
                *old = 0.0;
            }
        }
        if *unsent == [0.0; 2] {
            glide.unsent = None;
        }
    }
    glide.target = Some((target, native.modifiers));
    if !native.redraw.running() {
        native.redraw.restart();
    }
    open.window.window().request_redraw();
}

/// Show the policy in force. Online/JavaScript are two-way bound check boxes that
/// change themselves on click, so a refused change must restore this state.
pub(super) fn present_policy(open: &OpenWindow, policy: api::Policy) {
    open.web_native.borrow_mut().policy = policy;
    open.window.set_web_network(policy.network);
    open.window.set_web_scripts(policy.scripts);
}

pub(super) fn send(open: &OpenWindow, input: api::CommandKind) -> bool {
    open.enqueue(ViewerInput::Web {
        frame: open.frame.get(),
        input,
    })
}

pub(super) fn route_key(
    open: &OpenWindow,
    input: api::Key,
    text: Option<&str>,
    shortcuts: bool,
) -> bool {
    if input.key == "Escape" {
        if input.pressed {
            close(open);
        }
        return true;
    }
    {
        let mut native = open.web_native.borrow_mut();
        if !input.pressed && native.shortcut_keys.remove(&input.code).is_some() {
            return true;
        }
        if let Some(action) = native.shortcut_keys.get(&input.code).cloned() {
            drop(native);
            if input.repeat && open.window.get_content_focused() {
                let _ = navigate(open, &action);
            }
            return true;
        }
    }
    // These physical keys reach Chromium before any configured action. Its key event
    // supplies live editable focus; only a non-editable key returns for host arbitration.
    let space = input.key == " "
        && !input.modifiers.control
        && !input.modifiers.alt
        && !input.modifiers.meta;
    let physical_navigation = !input.modifiers.control
        && !input.modifiers.shift
        && !input.modifiers.meta
        && ((input.key == "Backspace" && !input.modifiers.alt)
            || (matches!(input.key.as_str(), "ArrowLeft" | "ArrowRight") && input.modifiers.alt));
    if shortcuts && !space && !physical_navigation && input.pressed && !input.repeat {
        let key = if input.code.starts_with("Digit") {
            input.code.as_str()
        } else if input.key == "+" {
            "Plus"
        } else {
            input.key.as_str()
        };
        if let Ok(mut chord) = ShortcutChord::parse(key) {
            chord.control = input.modifiers.control;
            chord.shift = input.modifiers.shift;
            chord.alt = input.modifiers.alt;
            chord.meta = input.modifiers.meta;
            let action = (open.key_resolver.borrow())(&chord, false);
            if let Some(action) = action
                && dispatch_keyboard_shortcut(open, &action)
            {
                open.web_native
                    .borrow_mut()
                    .shortcut_keys
                    .insert(input.code, action);
                return true;
            }
        }
    }
    let text_input = input.pressed
        && !input.modifiers.meta
        && ((!input.modifiers.control && !input.modifiers.alt) || !shortcuts);
    // Chromium textareas need the native character event after Enter's RawKeyDown.
    let enter_text = input.key == "Enter"
        && !input.modifiers.control
        && !input.modifiers.alt
        && !input.modifiers.meta;
    if input.pressed && open.window.get_content_focused() {
        send(open, api::CommandKind::Focus(true));
    }
    send(open, api::CommandKind::Key(input));
    if text_input
        && let Some(text) = text.filter(|text| {
            !text.is_empty()
                && (!text.chars().any(char::is_control)
                    || enter_text && matches!(*text, "\r" | "\n"))
        })
    {
        send(open, api::CommandKind::Text(text.into()));
    }
    true
}

pub(super) fn browser_key(key: &str) -> &str {
    if key == "Space" { " " } else { key }
}

pub(super) fn navigate(open: &OpenWindow, action: &str) -> Option<bool> {
    let (key, control) = match action {
        "viewer.up" => ("ArrowUp", false),
        "viewer.down" => ("ArrowDown", false),
        "viewer.left" => ("ArrowLeft", false),
        "viewer.right" => ("ArrowRight", false),
        "viewer.page-up" => ("PageUp", false),
        "viewer.page-down" => ("PageDown", false),
        "viewer.first" => ("Home", true),
        "viewer.last" => ("End", true),
        "viewer.select-page" => ("a", true),
        _ => return None,
    };
    let code = if key == "a" { "KeyA" } else { key };
    let input = api::Key {
        key: key.into(),
        code: code.into(),
        pressed: true,
        repeat: false,
        modifiers: api::Modifiers {
            control,
            ..Default::default()
        },
    };
    let accepted = send(open, api::CommandKind::Key(input.clone()));
    Some(
        send(
            open,
            api::CommandKind::Key(api::Key {
                pressed: false,
                ..input
            }),
        ) && accepted,
    )
}

fn shortcut(open: &OpenWindow, shortcut: api::HostShortcut) -> bool {
    let action = match shortcut {
        api::HostShortcut::Close => "viewer.close",
        api::HostShortcut::Find => "viewer.find",
        api::HostShortcut::FindNext => "viewer.find-next",
        api::HostShortcut::FindPrevious => "viewer.find-previous",
        api::HostShortcut::Copy => "viewer.copy",
        api::HostShortcut::ZoomIn => "viewer.zoom-in",
        api::HostShortcut::ZoomOut => "viewer.zoom-out",
        api::HostShortcut::ZoomReset => "viewer.actual-pixels",
        _ => return false,
    };
    dispatch_key(open, action)
}

pub(super) fn received(open: &OpenWindow, value: api::HostShortcut) {
    let physical = match value {
        api::HostShortcut::Backspace => Some(("Backspace", false)),
        api::HostShortcut::AltLeft => Some(("Left", true)),
        api::HostShortcut::AltRight => Some(("Right", true)),
        _ => None,
    };
    if let Some((key, alt)) = physical {
        if !open.closed.get() && open.window.get_web_mode() && open.window.get_content_focused() {
            let chord = ShortcutChord {
                key: key.into(),
                alt,
                ..Default::default()
            };
            let action = (open.key_resolver.borrow())(&chord, false);
            if let Some(action) = action {
                dispatch_key(open, &action);
            }
        }
        // An unbound key has no file action. Replaying it would re-enter arbitration.
        return;
    }
    if matches!(
        value,
        api::HostShortcut::CycleNext | api::HostShortcut::CyclePrevious
    ) {
        if open.closed.get() || !open.window.get_web_mode() || !open.window.get_content_focused() {
            return;
        }
        let backwards = value == api::HostShortcut::CyclePrevious;
        let chord = ShortcutChord {
            key: "Space".into(),
            shift: backwards,
            ..Default::default()
        };
        let action = (open.key_resolver.borrow())(&chord, false);
        if action.is_some_and(|action| dispatch_key(open, &action)) {
            return;
        }
        // Unbound Space retains ordinary page scrolling. Sending another Space key here
        // would re-enter Chromium's host arbitration, so use a bounded wheel gesture.
        let (width, height) = open.raster_size.get();
        send(
            open,
            api::CommandKind::Wheel(api::Wheel {
                x: f64::from(width) * 0.5,
                y: f64::from(height) * 0.5,
                delta_x: 0.0,
                delta_y: f64::from(height) * if backwards { 0.9 } else { -0.9 },
                modifiers: Default::default(),
            }),
        );
    } else {
        shortcut(open, value);
    }
}

/// Tests: a native event the shared tracker did not classify; a wheel line is a notch.
#[cfg(test)]
pub(super) fn observe(open: &OpenWindow, event: &WindowEvent, shortcuts: bool) -> bool {
    observe_native(open, event, shortcuts, false)
}

/// `finger`: the shared tracker classified this wheel event as touchpad motion, or it
/// is a momentum frame. It reaches the page at once, like the text viewer's touchpad
/// path; wheel notches glide instead.
pub(super) fn observe_native(
    open: &OpenWindow,
    event: &WindowEvent,
    shortcuts: bool,
    finger: bool,
) -> bool {
    if matches!(event, WindowEvent::RedrawRequested) {
        step(open);
        return false;
    }
    // As in the text viewer: a press, key, touch or focus change stops a notch glide.
    if matches!(
        event,
        WindowEvent::MouseInput {
            state: ElementState::Pressed,
            ..
        } | WindowEvent::Touch(_)
            | WindowEvent::Focused(false)
            | WindowEvent::PinchGesture { .. }
    ) || matches!(event, WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed)
    {
        cancel_glide(open);
    }
    // Modifier state and host shortcut releases remain authoritative while a file
    // change presents another viewer mode or a shortcut moves focus into Find.
    if let WindowEvent::ModifiersChanged(value) = event {
        let state = value.state();
        open.web_native.borrow_mut().modifiers = api::Modifiers {
            shift: state.shift_key(),
            control: state.control_key(),
            alt: state.alt_key(),
            meta: state.super_key(),
        };
        return false;
    }
    if let WindowEvent::KeyboardInput { event, .. } = event
        && event.state == ElementState::Released
    {
        let code = match event.physical_key {
            PhysicalKey::Code(value) => format!("{value:?}"),
            _ => "Unidentified".into(),
        };
        if open
            .web_native
            .borrow_mut()
            .shortcut_keys
            .remove(&code)
            .is_some()
        {
            return true;
        }
    }
    if matches!(event, WindowEvent::Focused(false)) {
        let mut native = open.web_native.borrow_mut();
        native.shortcut_keys.clear();
        native.modifiers = Default::default();
        native.dragging = false;
    }
    // A notice in the page's place (a refused browser) takes input like any control.
    if !open.window.get_web_mode() || open.window.get_sandbox_notice_visible() {
        return false;
    }
    // Embedded SDK popups own pointer, wheel, keyboard and IME input even when
    // their geometry overlaps the browser surface. Keep the cursor current for
    // the first wheel after dismissal; native focus loss still reaches Chromium.
    if !matches!(event, WindowEvent::Focused(_))
        && WindowInner::from_pub(open.window.window())
            .active_popups()
            .iter()
            .any(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
    {
        let mut native = open.web_native.borrow_mut();
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                native.cursor = Some([position.x, position.y]);
            }
            WindowEvent::CursorLeft { .. } => native.cursor = None,
            _ => {}
        }
        native.dragging = false;
        native.last_click = None;
        native.glide.cancel();
        return false;
    }
    let (scale, ratio) = surface_scale(open);
    let left = f64::from(open.window.get_content_left()) * scale;
    let top = f64::from(open.window.get_content_top()) * scale;
    let width = f64::from(open.window.get_content_width()) * scale;
    let height = f64::from(open.window.get_content_height()) * scale;
    let mut native = open.web_native.borrow_mut();
    let point = native.cursor.map(|[x, y]| [x - left, y - top]);
    let inside = point.is_some_and(|[x, y]| x >= 0.0 && y >= 0.0 && x < width && y < height);
    let surface_point = |[x, y]: [f64; 2]| [x * ratio[0], y * ratio[1]];
    let modifiers = native.modifiers;
    let command = match event {
        WindowEvent::CursorMoved { position, .. } => {
            native.cursor = Some([position.x, position.y]);
            let x = position.x - left;
            let y = position.y - top;
            if !native.dragging && !(x >= 0.0 && y >= 0.0 && x < width && y < height) {
                return false;
            }
            let [x, y] = surface_point([x, y]);
            api::CommandKind::Pointer(api::Pointer {
                kind: api::PointerKind::Move,
                button: api::PointerButton::None,
                x,
                y,
                modifiers,
                click_count: 0,
            })
        }
        WindowEvent::CursorLeft { .. } => {
            native.cursor = None;
            api::CommandKind::Pointer(api::Pointer {
                kind: api::PointerKind::Leave,
                button: api::PointerButton::None,
                x: 0.0,
                y: 0.0,
                modifiers,
                click_count: 0,
            })
        }
        WindowEvent::MouseInput { state, button, .. } => {
            if !inside && !native.dragging {
                if *state == ElementState::Pressed {
                    send(open, api::CommandKind::Focus(false));
                }
                return false;
            }
            // The mouse's Back and Forward buttons go through the documents opened from
            // links, as in the other viewers (`links.rs`); the page does not get them.
            if inside && matches!(button, MouseButton::Back | MouseButton::Forward) {
                let back = *button == MouseButton::Back;
                let pressed = *state == ElementState::Pressed;
                drop(native);
                if pressed {
                    crate::links::history(open, back);
                }
                return true;
            }
            let pressed = *state == ElementState::Pressed;
            native.dragging = pressed;
            if pressed {
                open.window.invoke_focus_content();
                let _ = open
                    .window
                    .window()
                    .with_winit_window(|window| window.set_ime_allowed(true));
                send(open, api::CommandKind::Focus(true));
            }
            let position = point.unwrap_or([0.0; 2]);
            let button = match button {
                MouseButton::Left => api::PointerButton::Left,
                MouseButton::Middle => api::PointerButton::Middle,
                MouseButton::Right => api::PointerButton::Right,
                MouseButton::Back => api::PointerButton::Back,
                MouseButton::Forward => api::PointerButton::Forward,
                _ => api::PointerButton::None,
            };
            let now = std::time::Instant::now();
            let previous = native
                .last_click
                .filter(|(time, old_button, old_position, _)| {
                    *old_button == button
                        && now.duration_since(*time) < Duration::from_millis(500)
                        && (0..2)
                            .all(|axis| (old_position[axis] - position[axis]).abs() <= 4.0 * scale)
                });
            let click_count = previous.map_or(
                1,
                |(_, _, _, count)| if pressed { count % 3 + 1 } else { count },
            );
            if pressed {
                native.last_click = Some((now, button, position, click_count));
            }
            let [x, y] = surface_point(position);
            api::CommandKind::Pointer(api::Pointer {
                kind: if pressed {
                    api::PointerKind::Down
                } else {
                    api::PointerKind::Up
                },
                button,
                x,
                y,
                modifiers,
                click_count,
            })
        }
        WindowEvent::MouseWheel { delta, .. } if inside => {
            let [x, y] = surface_point(point.unwrap_or([0.0; 2]));
            let acting = modifiers.control || modifiers.alt || modifiers.meta;
            let (delta_x, delta_y) = match delta {
                MouseScrollDelta::LineDelta(lines_x, lines_y) if !finger && !acting => {
                    glide(open, &mut native, [x, y], (*lines_x, *lines_y));
                    return true;
                }
                MouseScrollDelta::LineDelta(lines_x, lines_y) => {
                    let line = if acting { ACTING_LINE } else { f64::from(LINE) };
                    (
                        f64::from(*lines_x) * line * scale,
                        f64::from(*lines_y) * line * scale,
                    )
                }
                MouseScrollDelta::PixelDelta(value) => (value.x, value.y),
            };
            // Finger motion, momentum and acting wheels replace a notch glide.
            native.glide.cancel();
            if !acting {
                streamed(open, &mut native);
            }
            api::CommandKind::Wheel(api::Wheel {
                x,
                y,
                delta_x: delta_x * ratio[0],
                delta_y: delta_y * ratio[1],
                modifiers,
            })
        }
        WindowEvent::MouseWheel { .. } => {
            // A wheel over the toolbar or a scrollbar scrolls that instead.
            native.glide.cancel();
            return false;
        }
        WindowEvent::KeyboardInput {
            event,
            is_synthetic,
            ..
        } => {
            let code = match event.physical_key {
                PhysicalKey::Code(value) => format!("{value:?}"),
                _ => "Unidentified".into(),
            };
            if !open.window.get_content_focused() && !native.shortcut_keys.contains_key(&code) {
                return false;
            }
            let key = match &event.logical_key {
                Key::Character(value) => value.to_string(),
                Key::Named(value) => format!("{value:?}"),
                _ => return false,
            };
            drop(native);
            return route_key(
                open,
                api::Key {
                    key: browser_key(&key).into(),
                    code,
                    pressed: event.state == ElementState::Pressed,
                    repeat: event.repeat,
                    modifiers,
                },
                event.text_with_all_modifiers(),
                shortcuts && !is_synthetic,
            );
        }
        WindowEvent::Ime(event) if open.window.get_content_focused() => {
            api::CommandKind::Ime(match event {
                Ime::Enabled => api::Ime::Start,
                Ime::Preedit(text, cursor) => {
                    let (start, end) = cursor.unwrap_or((text.len(), text.len()));
                    api::Ime::Update {
                        text: text.clone(),
                        selection_start: start as u32,
                        selection_end: end as u32,
                    }
                }
                Ime::Commit(text) => api::Ime::Commit(text.clone()),
                Ime::Disabled => api::Ime::End,
            })
        }
        WindowEvent::Focused(focused) => {
            if !focused {
                native.dragging = false;
                native.modifiers = Default::default();
                native.shortcut_keys.clear();
            }
            api::CommandKind::Focus(*focused && open.window.get_content_focused())
        }
        _ => return false,
    };
    drop(native);
    send(open, command);
    !matches!(
        event,
        WindowEvent::Focused(_) | WindowEvent::CursorLeft { .. }
    )
}

pub(super) fn bind(open: &Rc<OpenWindow>) {
    let weak = Rc::downgrade(open);
    open.window.on_web_policy(move |network, scripts| {
        if let Some(open) = weak.upgrade() {
            let policy = api::Policy {
                network,
                scripts: network && scripts,
            };
            if send(&open, api::CommandKind::Policy(policy)) {
                present_policy(&open, policy);
                open.text_position.borrow_mut().changing_content();
                open.window.set_surface(slint::Image::default());
            } else {
                let current = open.web_native.borrow().policy;
                present_policy(&open, current);
            }
        }
    });
    // Like the text viewer's wheel clock: wake the native redraw that steps the glide,
    // or step it directly when no native events arrive.
    let weak = Rc::downgrade(open);
    let native = open.web_native.borrow();
    native.redraw.start(
        TimerMode::Repeated,
        midnight_ui_input::WHEEL_FRAME,
        move || {
            let Some(open) = weak.upgrade() else { return };
            if open.closed.get() || !open.web_native.borrow().glide.active() {
                cancel_glide(&open);
            } else if open.image_native.borrow().active {
                open.window.window().request_redraw();
            } else {
                step(&open);
            }
        },
    );
    native.redraw.stop();
}

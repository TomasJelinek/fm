//! Winit supplies physical, layout-independent input and IME once per browser event.
use super::*;
use midnight_core::viewer_web as api;
use slint::winit_030::winit::{
    event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent},
    keyboard::{Key, PhysicalKey},
    platform::modifier_supplement::KeyEventExtModifierSupplement,
};

#[derive(Default)]
pub(super) struct Native {
    cursor: Option<[f64; 2]>,
    modifiers: api::Modifiers,
    dragging: bool,
    last_click: Option<(std::time::Instant, api::PointerButton, [f64; 2], u8)>,
    shortcut_keys: std::collections::HashMap<String, String>,
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
    // Space reaches Chromium before the configured action can run. Chromium's key event
    // reports live editable focus and requests host arbitration only outside an editor.
    let space = input.key == " "
        && !input.modifiers.control
        && !input.modifiers.alt
        && !input.modifiers.meta;
    if shortcuts && !space && input.pressed && !input.repeat {
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
                && dispatch_key(open, &action)
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
    if matches!(
        value,
        api::HostShortcut::CycleNext | api::HostShortcut::CyclePrevious
    ) {
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

pub(super) fn observe(open: &OpenWindow, event: &WindowEvent, shortcuts: bool) -> bool {
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
    if !open.window.get_web_mode() {
        return false;
    }
    let scale = f64::from(open.window.window().scale_factor());
    let left = f64::from(open.window.get_content_left()) * scale;
    let top = f64::from(open.window.get_content_top()) * scale;
    let width = f64::from(open.window.get_content_width()) * scale;
    let height = f64::from(open.window.get_content_height()) * scale;
    let mut native = open.web_native.borrow_mut();
    let point = native.cursor.map(|[x, y]| [x - left, y - top]);
    let inside = point.is_some_and(|[x, y]| x >= 0.0 && y >= 0.0 && x < width && y < height);
    let (raster_width, raster_height) = open.raster_size.get();
    let ratio = [
        f64::from(raster_width) / width.max(1.0),
        f64::from(raster_height) / height.max(1.0),
    ];
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
            let (delta_x, delta_y) = match delta {
                MouseScrollDelta::PixelDelta(value) => (value.x, value.y),
                MouseScrollDelta::LineDelta(x, y) => {
                    (f64::from(*x) * 40.0 * scale, f64::from(*y) * 40.0 * scale)
                }
            };
            api::CommandKind::Wheel(api::Wheel {
                x,
                y,
                delta_x: delta_x * ratio[0],
                delta_y: delta_y * ratio[1],
                modifiers,
            })
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
                open.window.set_web_network(policy.network);
                open.window.set_web_scripts(policy.scripts);
                open.text_position.borrow_mut().changing_content();
                open.window.set_surface(slint::Image::default());
            }
        }
    });
}

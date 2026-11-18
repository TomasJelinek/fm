use super::*;

/// CEF's documented "no range" (`CefRange::InvalidRange`). The IME range parameters
/// are not optional: libcef drops the whole call when it receives NULL.
const INVALID_RANGE: Range = Range {
    from: u32::MAX,
    to: u32::MAX,
};

fn modifiers(value: api::Modifiers) -> u32 {
    (u32::from(value.shift) * 2)
        | (u32::from(value.control) * 4)
        | (u32::from(value.alt) * 8)
        | (u32::from(value.meta) * 128)
}

fn mouse(x: f64, y: f64, flags: api::Modifiers, scale: f64) -> MouseEvent {
    MouseEvent {
        x: (x / scale) as i32,
        y: (y / scale) as i32,
        modifiers: modifiers(flags),
    }
}

/// CEF button type and EVENTFLAG_*_MOUSE_BUTTON bit. Chromium has no Back/Forward
/// button type, so those buttons are never turned into a primary click.
fn click_button(button: api::PointerButton) -> Option<(MouseButtonType, u32)> {
    match button {
        api::PointerButton::Left => Some((MouseButtonType::LEFT, 16)),
        api::PointerButton::Middle => Some((MouseButtonType::MIDDLE, 32)),
        api::PointerButton::Right => Some((MouseButtonType::RIGHT, 64)),
        api::PointerButton::None | api::PointerButton::Back | api::PointerButton::Forward => None,
    }
}

/// Converts physical wheel deltas to whole DIPs, keeping each axis' fractional
/// remainder so precision-touchpad scrolling below one DIP per event accumulates.
/// A reversed direction discards the opposite remainder.
fn wheel_steps(remainder: &mut [f64; 2], delta: [f64; 2], scale: f64) -> [i32; 2] {
    let mut steps = [0; 2];
    for axis in 0..2 {
        let value = delta[axis] / scale;
        if value != 0.0 && remainder[axis] * value < 0.0 {
            remainder[axis] = 0.0;
        }
        let total = remainder[axis] + value;
        let whole = total.trunc();
        remainder[axis] = total - whole;
        steps[axis] = whole as i32;
    }
    steps
}

/// CEF zoom level for an absolute zoom factor; each level scales by 1.2.
fn zoom_level(factor: f64) -> f64 {
    factor.ln() / 1.2f64.ln()
}

/// Absolute zoom reported in `State`. The libm level round trip is inexact (10.0
/// returns as 10.000000000000004), and an out-of-range State fails protocol validation.
pub(super) fn reported_zoom(level: f64) -> f64 {
    let factor = 1.2f64.powf(level);
    if factor.is_nan() {
        1.0
    } else {
        factor.clamp(0.1, 10.0)
    }
}

pub(super) fn command(instance: &BrowserInstance, command: api::Command) {
    let doc = &instance.document;
    let Some(host) = instance.browser.host() else {
        return;
    };
    let scale = doc.viewport.lock().map(|v| v.scale).unwrap_or(1.0);
    match command.kind {
        api::CommandKind::Resize(viewport) => {
            if let Ok(mut surface) = doc.surface.lock() {
                if viewport.revision <= surface.revision { return; }
                if surface.resize(viewport.revision, viewport.width, viewport.height).is_err() { return; }
            }
            if let Ok(mut current) = doc.viewport.lock() { *current = viewport; }
            if let Ok(mut layers) = doc.layers.lock() { *layers = crate::surface::Layers::default(); }
            if let Ok(mut frame) = doc.output.frame.lock() { *frame = None; }
            if let Ok(mut remainder) = doc.wheel_remainder.lock() { *remainder = [0.0; 2]; }
            host.notify_screen_info_changed();
            host.was_resized();
        }
        api::CommandKind::Pointer(pointer) => {
            let mut event = mouse(pointer.x, pointer.y, pointer.modifiers, scale);
            event.modifiers |= doc.mouse_buttons.load(Ordering::Acquire) as u32;
            match pointer.kind {
                api::PointerKind::Move => host.send_mouse_move_event(Some(&event), 0),
                api::PointerKind::Leave => host.send_mouse_move_event(Some(&event), 1),
                api::PointerKind::Down | api::PointerKind::Up => {
                    let Some((button, flag)) = click_button(pointer.button) else { return; };
                    host.set_focus(1);
                    if pointer.kind == api::PointerKind::Down {
                        doc.mouse_buttons.fetch_or(u64::from(flag), Ordering::AcqRel);
                        event.modifiers |= flag;
                    } else {
                        doc.mouse_buttons.fetch_and(!u64::from(flag), Ordering::AcqRel);
                        event.modifiers &= !flag;
                    }
                    host.send_mouse_click_event(Some(&event), button, i32::from(pointer.kind == api::PointerKind::Up), pointer.click_count.max(1) as i32);
                }
            }
        }
        api::CommandKind::Wheel(wheel) => {
            #[cfg(feature = "diagnostic-timing")]
            crate::trace::record(crate::trace::Event::Wheel, command.header.sequence, 0);
            let mut event = mouse(wheel.x, wheel.y, wheel.modifiers, scale);
            event.modifiers |= doc.mouse_buttons.load(Ordering::Acquire) as u32;
            let Ok(mut remainder) = doc.wheel_remainder.lock() else { return; };
            let [x, y] = wheel_steps(&mut remainder, [wheel.delta_x, wheel.delta_y], scale);
            drop(remainder);
            if x != 0 || y != 0 {
                host.send_mouse_wheel_event(Some(&event), x, y);
            }
        }
        api::CommandKind::Key(key) => {
            if navigation::reload(instance, &key) { return; }
            let code = crate::platform::key_code(&key);
            host.send_key_event(Some(&KeyEvent { type_: if key.pressed { KeyEventType::RAWKEYDOWN } else { KeyEventType::KEYUP },
                modifiers: modifiers(key.modifiers) | if key.repeat { crate::host_keys::REPEAT } else { 0 }, windows_key_code: code,
                native_key_code: crate::platform::native_key_code(&key),
                is_system_key: i32::from(key.modifiers.alt), ..Default::default() }));
        }
        api::CommandKind::Text(text) => {
            for unit in text.encode_utf16() {
                host.send_key_event(Some(&KeyEvent { type_: KeyEventType::CHAR,
                    windows_key_code: i32::from(unit), character: unit, unmodified_character: unit,
                    ..Default::default() }));
            }
        }
        api::CommandKind::Ime(ime) => match ime {
            api::Ime::Start => {},
            api::Ime::Update { text, selection_start, selection_end } => {
                let start = text[..selection_start as usize].encode_utf16().count() as u32;
                let end = text[..selection_end as usize].encode_utf16().count() as u32;
                host.ime_set_composition(Some(&CefString::from(text.as_str())), None, Some(&INVALID_RANGE), Some(&Range { from: start, to: end }));
            }
            api::Ime::Commit(text) => host.ime_commit_text(Some(&CefString::from(text.as_str())), Some(&INVALID_RANGE), 0),
            api::Ime::End => host.ime_finish_composing_text(0),
        },
        api::CommandKind::Focus(focused) => {
            // A button released while the window was inactive never reaches the provider.
            // End Chromium's capture instead of letting later hovers keep the held-button
            // flags (which would turn them into drags); this never synthesizes a click.
            if !focused && doc.mouse_buttons.swap(0, Ordering::AcqRel) != 0 {
                host.send_capture_lost_event();
            }
            host.set_focus(i32::from(focused));
        }
        api::CommandKind::Find(find) => {
            if let Ok(mut finding) = doc.finding.lock() { finding.request(command.header.sequence, find); }
            dispatch_find(doc, &host);
        }
        api::CommandKind::Zoom(factor) => host.set_zoom_level(zoom_level(factor)),
        api::CommandKind::Copy => {
            let event = doc.selection.lock().ok().map(|selection| selection.copy_event());
            if let Some(event) = event { doc.emit(event); }
        }
        api::CommandKind::ResourceReply(reply) => resources::reply(doc, reply),
        api::CommandKind::Policy(_) => doc.emit(api::EventKind::Error { code: api::ErrorCode::Denied, message: "Policy changes require fresh Open with a new document generation and resource capability".into() }),
        api::CommandKind::FrameAck { .. } => {},
        api::CommandKind::Open { .. } | api::CommandKind::Close => {},
    }
}

pub(super) fn dispatch_find(doc: &Document, host: &BrowserHost) {
    let find = doc.finding.lock().ok().and_then(|mut q| q.start());
    if let Some(find) = find {
        if find.query.is_empty() {
            host.stop_finding(1);
            let sequence = doc
                .finding
                .lock()
                .ok()
                .and_then(|mut q| q.finish_without_native());
            if let Some(request_sequence) = sequence {
                doc.emit(api::EventKind::FindResult(api::FindResult {
                    request_sequence,
                    matches: 0,
                    active: 0,
                    complete: true,
                }));
            }
            return;
        }
        host.find(
            Some(&CefString::from(find.query.as_str())),
            i32::from(find.forward),
            i32::from(find.match_case),
            i32::from(find.find_next),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn back_and_forward_buttons_never_become_primary_clicks() {
        for button in [
            api::PointerButton::None,
            api::PointerButton::Back,
            api::PointerButton::Forward,
        ] {
            assert!(click_button(button).is_none(), "{button:?}");
        }
        assert_eq!(
            click_button(api::PointerButton::Left),
            Some((MouseButtonType::LEFT, 16))
        );
        assert_eq!(
            click_button(api::PointerButton::Middle),
            Some((MouseButtonType::MIDDLE, 32))
        );
        assert_eq!(
            click_button(api::PointerButton::Right),
            Some((MouseButtonType::RIGHT, 64))
        );
    }

    #[test]
    fn sub_dip_wheel_deltas_accumulate_and_reversal_restarts() {
        let mut remainder = [0.0; 2];
        // Eight quarter-DIP touchpad events used to be truncated to zero each.
        let total: i32 = (0..8)
            .map(|_| wheel_steps(&mut remainder, [0.0, -0.25], 1.0)[1])
            .sum();
        assert_eq!(total, -2);
        assert_eq!(remainder, [0.0, 0.0]);
        // Physical deltas at 150 % scale: 0.75 physical px = 0.5 DIP.
        assert_eq!(wheel_steps(&mut remainder, [0.75, 0.0], 1.5), [0, 0]);
        assert_eq!(wheel_steps(&mut remainder, [0.75, 0.0], 1.5), [1, 0]);
        // A reversed direction does not first consume the opposite remainder.
        assert_eq!(wheel_steps(&mut remainder, [0.75, 0.0], 1.5), [0, 0]);
        assert_eq!(wheel_steps(&mut remainder, [-1.5, 0.0], 1.5), [-1, 0]);
        assert_eq!(
            wheel_steps(&mut remainder, [120.0, -240.0], 1.0),
            [120, -240]
        );
    }

    #[test]
    fn reported_zoom_round_trip_stays_inside_the_protocol_range() {
        let header = api::Header {
            session: 1,
            document: 1,
            sequence: 1,
        };
        for factor in [0.1, 0.25, 0.5, 0.9, 1.0, 1.1, 2.0, 3.0, 5.0, 9.99, 10.0] {
            let zoom = reported_zoom(zoom_level(factor));
            assert!(
                (zoom - factor).abs() <= factor * 1e-12,
                "{factor} -> {zoom}"
            );
            let state = api::Message::Event(api::Event {
                header,
                kind: api::EventKind::State(api::State {
                    title: String::new(),
                    url: String::new(),
                    loading: false,
                    editable: false,
                    zoom,
                }),
            });
            assert!(state.validate().is_ok(), "{factor} -> {zoom}");
        }
        assert_eq!(reported_zoom(f64::NAN), 1.0);
        assert_eq!(reported_zoom(f64::INFINITY), 10.0);
        assert_eq!(reported_zoom(1000.0), 10.0);
        assert_eq!(reported_zoom(-1000.0), 0.1);
    }
}

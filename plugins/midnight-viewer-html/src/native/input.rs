use super::*;

/// CEF's documented "no range" (`CefRange::InvalidRange`). The IME range parameters
/// are not optional: libcef drops the whole call when it receives NULL.
const INVALID_RANGE: Range = Range {
    from: u32::MAX,
    to: u32::MAX,
};

/// Chromium switch that applies every wheel event at once. CEF delivers OSR wheel
/// events as non-precise deltas, which Chromium otherwise animates over about 170 ms;
/// the host already sends its wheel glide and touchpad momentum one frame at a time.
pub(super) const SMOOTH_SCROLLING_OFF: &str = "disable-smooth-scrolling";

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

/// The host's per-frame deltas of one wheel glide add up to whole pixels only up to
/// float rounding. A total this close below a whole DIP counts as that DIP, so the
/// glide's last DIP is not held back until the next wheel event.
const WHOLE_DIP_TOLERANCE: f64 = 1e-3;

/// A wheel gesture ends when no wheel event follows for longer than this: longer than
/// the host's whole glide of a notch (200 ms), sent one delta per frame, so neither a
/// glide nor a short stall of this loop ends one early.
const GESTURE_GAP: Duration = Duration::from_millis(250);

/// The DIPs of the current wheel gesture that Chromium has not scrolled yet, per axis,
/// and when the gesture's last event arrived.
///
/// Chromium scrolls whole DIPs of whole units ([`crate::platform::WheelScale::step`]):
/// on Windows a unit is 5/18 DIP per wheel line, so above three lines some distances
/// are no whole number of units. Each event therefore sends the units that scroll the
/// most whole DIPs not past those due, one or more as soon as one DIP fits, and carries
/// what Chromium will not scroll for them to the gesture's next event. At its end that
/// rest, less than one unit's scroll, is dropped rather than added to the next gesture.
#[derive(Debug, Default)]
pub(super) struct WheelGesture {
    remainder: [f64; 2],
    last: Option<Instant>,
}

impl WheelGesture {
    /// The CEF units of one wheel event of physical `delta` pixels at `scale` that
    /// arrived at `now`, across and down.
    fn units(&mut self, delta: [f64; 2], scale: f64, now: Instant) -> [i32; 2] {
        let scales = crate::platform::wheel_scales();
        self.steps(delta.map(|value| value / scale), now, |axis, dips| {
            scales[axis].step(dips)
        })
    }

    /// The units of one event of `delta` DIPs per axis at `now`; `step` gives the units
    /// on an axis that scroll the most whole DIPs not past a whole number of DIPs, and
    /// that scroll. A reversed direction discards the opposite remainder.
    fn steps(
        &mut self,
        delta: [f64; 2],
        now: Instant,
        step: impl Fn(usize, i64) -> crate::platform::WheelStep,
    ) -> [i32; 2] {
        if self
            .last
            .is_some_and(|last| now.saturating_duration_since(last) > GESTURE_GAP)
        {
            self.remainder = [0.0; 2];
        }
        self.last = Some(now);
        let bound = crate::platform::MAX_WHEEL_DIPS as f64;
        std::array::from_fn(|axis| {
            let value = delta[axis];
            if !value.is_finite() {
                return 0;
            }
            if value != 0.0 && self.remainder[axis] * value < 0.0 {
                self.remainder[axis] = 0.0;
            }
            let due = (self.remainder[axis] + value).clamp(-bound, bound);
            let whole = (due + WHOLE_DIP_TOLERANCE.copysign(due)).trunc() as i64;
            let sent = step(axis, whole);
            self.remainder[axis] = due - sent.dips as f64;
            sent.units
        })
    }
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
            if let Ok(mut gesture) = doc.wheel_gesture.lock() { *gesture = WheelGesture::default(); }
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
            crate::trace::wheel(command.header.document, command.header.sequence);
            let mut event = mouse(wheel.x, wheel.y, wheel.modifiers, scale);
            event.modifiers |= doc.mouse_buttons.load(Ordering::Acquire) as u32;
            let Ok(mut gesture) = doc.wheel_gesture.lock() else { return; };
            let [x, y] = gesture.units([wheel.delta_x, wheel.delta_y], scale, Instant::now());
            drop(gesture);
            // CEF on Windows scrolls one axis per event, the vertical one when both move.
            if y != 0 {
                host.send_mouse_wheel_event(Some(&event), 0, y);
            }
            if x != 0 {
                host.send_mouse_wheel_event(Some(&event), x, 0);
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
    let action = doc.finding.lock().ok().and_then(|mut q| q.start());
    if let Some(action) = action {
        let find = action.find;
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
        if action.restart {
            // An unchanged initial search is otherwise suppressed by Chromium.
            // Stop its session, then ask native Find for a new real result.
            host.stop_finding(1);
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

    use crate::platform::WheelStep;

    /// The host's frame while it glides a notch.
    const FRAME: Duration = Duration::from_millis(8);

    /// Whole DIPs one to one, as CEF on Linux reads them.
    fn dips(_: usize, dips: i64) -> WheelStep {
        WheelStep {
            units: dips as i32,
            dips,
        }
    }

    /// Five wheel lines on Windows without CEF's `f32` rounding: a unit is 25/18 DIP and
    /// Chromium scrolls the whole DIPs of an event's units, so 1, 2, 4, 5, 6, 8, … DIPs.
    fn five_lines(_: usize, dips: i64) -> WheelStep {
        let units = (18 * dips.abs() + 17) / 25;
        WheelStep {
            units: (dips.signum() * units) as i32,
            dips: dips.signum() * (units * 25 / 18),
        }
    }

    /// One event of physical `delta` pixels at `scale`, one frame after `gesture`'s last.
    fn wheel_steps(gesture: &mut WheelGesture, delta: [f64; 2], scale: f64) -> [i32; 2] {
        let now = gesture.last.map_or_else(Instant::now, |last| last + FRAME);
        gesture.steps(delta.map(|value| value / scale), now, dips)
    }

    #[test]
    fn sub_dip_wheel_deltas_accumulate_and_reversal_restarts() {
        let mut gesture = WheelGesture::default();
        // Eight quarter-DIP touchpad events used to be truncated to zero each.
        let total: i32 = (0..8)
            .map(|_| wheel_steps(&mut gesture, [0.0, -0.25], 1.0)[1])
            .sum();
        assert_eq!(total, -2);
        assert_eq!(gesture.remainder, [0.0, 0.0]);
        // Physical deltas at 150 % scale: 0.75 physical px = 0.5 DIP.
        assert_eq!(wheel_steps(&mut gesture, [0.75, 0.0], 1.5), [0, 0]);
        assert_eq!(wheel_steps(&mut gesture, [0.75, 0.0], 1.5), [1, 0]);
        // A reversed direction does not first consume the opposite remainder.
        assert_eq!(wheel_steps(&mut gesture, [0.75, 0.0], 1.5), [0, 0]);
        assert_eq!(wheel_steps(&mut gesture, [-1.5, 0.0], 1.5), [-1, 0]);
        assert_eq!(wheel_steps(&mut gesture, [120.0, -240.0], 1.0), [120, -240]);
    }

    #[test]
    fn a_glide_of_per_frame_deltas_scrolls_its_whole_distance() {
        // Ten frames of 0.1 px add up to 0.9999999999999999 in f64; at 2x, ten frames
        // of 0.6 physical px to 2.9999999999999996 DIP. Neither may stop a DIP short.
        for (frame, scale, expected) in [(-0.1, 1.0, -1), (0.6, 2.0, 3)] {
            let mut gesture = WheelGesture::default();
            let total: i32 = (0..10)
                .map(|_| wheel_steps(&mut gesture, [frame, 0.0], scale)[0])
                .sum();
            assert_eq!(total, expected, "{frame} x 10 at {scale}x");
            assert!(gesture.remainder[0].abs() < WHOLE_DIP_TOLERANCE);
        }
        // A real fraction still waits for the rest of its DIP.
        let mut gesture = WheelGesture::default();
        assert_eq!(wheel_steps(&mut gesture, [0.0, 0.998], 1.0), [0, 0]);
        assert_eq!(wheel_steps(&mut gesture, [0.0, 0.002], 1.0), [0, 1]);
    }

    #[test]
    fn a_gesture_carries_what_chromium_does_not_scroll_and_drops_it_when_it_ends() {
        let start = Instant::now();
        let mut gesture = WheelGesture::default();
        // At five lines 3 DIPs are two units, which scroll 2: the third DIP waits.
        assert_eq!(gesture.steps([0.0, 3.0], start, five_lines), [0, 2]);
        // In the same gesture it goes out with the next DIP, as two units again.
        assert_eq!(gesture.steps([0.0, 1.0], start + FRAME, five_lines), [0, 2]);
        // Less than one unit's scroll sends nothing yet.
        assert_eq!(
            gesture.steps([0.0, 0.5], start + 2 * FRAME, five_lines),
            [0, 0]
        );
        // 3.5 DIPs: two units scroll 2, and 1.5 DIPs wait.
        assert_eq!(
            gesture.steps([0.0, 3.0], start + 3 * FRAME, five_lines),
            [0, 2]
        );
        let last = start + 3 * FRAME;
        // Within the gesture the next DIP would take them along: 2.5 DIPs, two units.
        let mut carried = WheelGesture {
            remainder: gesture.remainder,
            last: gesture.last,
        };
        assert_eq!(
            carried.steps([0.0, 1.0], last + GESTURE_GAP, five_lines),
            [0, 2]
        );
        // After a longer pause the gesture has ended and they are dropped: one unit.
        assert_eq!(
            gesture.steps([0.0, 1.0], last + GESTURE_GAP + FRAME, five_lines),
            [0, 1]
        );
        assert_eq!(gesture.remainder, [0.0, 0.0]);
    }

    /// The DIPs Chromium scrolls at `lines` wheel lines for a gesture of `deltas` DIPs,
    /// one per frame, sent as the provider sends them.
    #[cfg(windows)]
    fn scrolled(lines: u32, deltas: &[f64]) -> i64 {
        let scale = crate::platform::WheelScale::with_lines(lines);
        let mut gesture = WheelGesture::default();
        let start = Instant::now();
        deltas
            .iter()
            .zip(0..)
            .map(|(&delta, frame)| {
                let now = start + FRAME * frame;
                let [_, units] = gesture.steps([0.0, delta], now, |_, dips| scale.step(dips));
                assert!(units >= 0, "{lines} lines: {units} units");
                i64::from(scale.dips(units.unsigned_abs()))
            })
            .sum()
    }

    /// The host's notch is 60 DIPs. At five wheel lines it used to scroll 50 DIPs and at
    /// seven 35, the rest waiting for the next gesture; touchpad glides jumped in steps
    /// of 25 to 35 DIPs.
    #[cfg(windows)]
    #[test]
    fn a_notch_scrolls_its_distance_within_one_chromium_step_at_any_line_count() {
        let eased = |frame: f64| 60.0 * (1.0 - (1.0 - frame / 25.0).powi(2));
        let gestures = [
            ("at once", vec![60.0]),
            ("in 25 frames of 2.4 DIPs", vec![2.4; 25]),
            (
                "eased over 25 frames",
                (1..=25)
                    .map(|frame| eased(f64::from(frame)) - eased(f64::from(frame - 1)))
                    .collect(),
            ),
            (
                "as a touchpad flick of 28 DIPs",
                vec![0.7, 1.3, 2.9, 4.4, 6.1, 5.2, 3.3, 2.0, 1.1, 0.6, 0.4],
            ),
        ];
        for lines in 1..=20 {
            let scale = crate::platform::WheelScale::with_lines(lines);
            // The most one unit more scrolls up to a notch: no gesture can come closer.
            let mut gap = 1;
            let mut units = 0;
            while scale.dips(units) <= 60 {
                gap = gap.max(scale.dips(units + 1) - scale.dips(units));
                units += 1;
            }
            for (what, deltas) in &gestures {
                let distance = deltas.iter().sum::<f64>().round() as i64;
                let moved = scrolled(lines, deltas);
                assert!(
                    moved <= distance && moved > distance - i64::from(gap),
                    "{lines} lines, {what}: {moved} of {distance} DIPs (one unit scrolls up to {gap})"
                );
            }
        }
        // At Windows' default of three lines every whole DIP is reachable.
        for (what, deltas) in &gestures[..3] {
            assert_eq!(scrolled(3, deltas), 60, "{what}");
        }
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

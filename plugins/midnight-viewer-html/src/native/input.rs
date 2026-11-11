use super::*;

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
            host.notify_screen_info_changed();
            host.was_resized();
        }
        api::CommandKind::Pointer(pointer) => {
            let mut event = mouse(pointer.x, pointer.y, pointer.modifiers, scale);
            event.modifiers |= doc.mouse_buttons.load(Ordering::Acquire) as u32;
            let button = match pointer.button {
                api::PointerButton::Left => MouseButtonType::LEFT,
                api::PointerButton::Middle => MouseButtonType::MIDDLE,
                api::PointerButton::Right => MouseButtonType::RIGHT,
                _ => MouseButtonType::LEFT,
            };
            let flag = match pointer.button { api::PointerButton::Left => 16, api::PointerButton::Middle => 32, api::PointerButton::Right => 64, _ => 0 };
            match pointer.kind {
                api::PointerKind::Move => host.send_mouse_move_event(Some(&event), 0),
                api::PointerKind::Leave => host.send_mouse_move_event(Some(&event), 1),
                api::PointerKind::Down | api::PointerKind::Up => {
                    host.set_focus(1);
                    if pointer.kind == api::PointerKind::Down {
                        doc.mouse_buttons.fetch_or(flag as u64, Ordering::AcqRel);
                        event.modifiers |= flag;
                    } else {
                        doc.mouse_buttons.fetch_and(!(flag as u64), Ordering::AcqRel);
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
            host.send_mouse_wheel_event(Some(&event), (wheel.delta_x / scale) as i32, (wheel.delta_y / scale) as i32);
        }
        api::CommandKind::Key(key) => {
            let code = crate::platform::key_code(&key);
            host.send_key_event(Some(&KeyEvent { type_: if key.pressed { KeyEventType::RAWKEYDOWN } else { KeyEventType::KEYUP },
                modifiers: modifiers(key.modifiers), windows_key_code: code,
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
                host.ime_set_composition(Some(&CefString::from(text.as_str())), None, None, Some(&Range { from: start, to: end }));
            }
            api::Ime::Commit(text) => host.ime_commit_text(Some(&CefString::from(text.as_str())), None, 0),
            api::Ime::End => host.ime_finish_composing_text(0),
        },
        api::CommandKind::Focus(focused) => host.set_focus(i32::from(focused)),
        api::CommandKind::Find(find) => {
            if let Ok(mut finding) = doc.finding.lock() { finding.request(command.header.sequence, find); }
            dispatch_find(doc, &host);
        }
        api::CommandKind::Zoom(factor) => host.set_zoom_level(factor.ln() / 1.2f64.ln()),
        api::CommandKind::Copy => {
            if let Ok(selection) = doc.selection.lock() { doc.emit(selection.copy_event()); }
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

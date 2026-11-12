use midnight_web_viewer_api::*;
use std::io::{self, Cursor, Read};

fn command(kind: CommandKind) -> Message {
    Message::Command(Command {
        header: header(),
        kind,
    })
}

fn event(kind: EventKind) -> Message {
    Message::Event(Event {
        header: header(),
        kind,
    })
}

fn header() -> Header {
    Header {
        session: 11,
        document: 3,
        sequence: 5,
    }
}

fn viewport() -> Viewport {
    Viewport {
        revision: 2,
        width: 2,
        height: 1,
        scale: 1.5,
    }
}

fn resource() -> Resource {
    Resource {
        id: 44,
        length: u64::MAX,
        mime: "text/html; charset=utf-8".into(),
    }
}

fn encode(message: &Message) -> Vec<u8> {
    let mut bytes = Vec::new();
    write_message(&mut bytes, message).unwrap();
    bytes
}

#[test]
fn roundtrip_every_command_preserves_unicode_and_u64_identities() {
    let modifiers = Modifiers {
        control: true,
        ..Modifiers::default()
    };
    let kinds = vec![
        CommandKind::Open {
            source: resource(),
            viewport: viewport(),
            policy: Policy::default(),
        },
        CommandKind::Resize(viewport()),
        CommandKind::Pointer(Pointer {
            kind: PointerKind::Down,
            button: PointerButton::Left,
            x: -0.5,
            y: 14.0,
            modifiers,
            click_count: 2,
        }),
        CommandKind::Wheel(Wheel {
            x: 2.0,
            y: 3.0,
            delta_x: -5.0,
            delta_y: 12.0,
            modifiers,
        }),
        CommandKind::Key(Key {
            key: "ě".into(),
            code: "KeyE".into(),
            pressed: true,
            repeat: false,
            modifiers,
        }),
        CommandKind::Text("日本語💾".into()),
        CommandKind::Ime(Ime::Start),
        CommandKind::Ime(Ime::Update {
            text: "žluť".into(),
            selection_start: 2,
            selection_end: 4,
        }),
        CommandKind::Ime(Ime::Commit("čeština".into())),
        CommandKind::Ime(Ime::End),
        CommandKind::Focus(true),
        CommandKind::Find(Find {
            query: "東京".into(),
            forward: false,
            match_case: true,
            find_next: true,
        }),
        CommandKind::Zoom(1.25),
        CommandKind::Copy,
        CommandKind::Policy(Policy {
            network: true,
            scripts: true,
        }),
        CommandKind::ResourceReply(ResourceReply::Resolved {
            request: u64::MAX,
            resource: resource(),
        }),
        CommandKind::ResourceReply(ResourceReply::Data {
            request: 2,
            resource: 44,
            offset: u64::MAX - 3,
            bytes: vec![0, 255, 17],
        }),
        CommandKind::ResourceReply(ResourceReply::Released { resource: 44 }),
        CommandKind::ResourceReply(ResourceReply::Denied {
            request: 8,
            reason: "odepřeno".into(),
        }),
        CommandKind::FrameAck {
            frame_sequence: u64::MAX,
        },
        CommandKind::Close,
    ];
    for kind in kinds {
        let message = command(kind);
        assert_eq!(
            read_message(&mut encode(&message).as_slice()).unwrap(),
            message
        );
    }
}

#[test]
fn roundtrip_every_event_preserves_owned_pixels_and_resource_requests() {
    let kinds = vec![
        EventKind::Ready,
        EventKind::Frame(Frame {
            viewport_revision: 2,
            acknowledged_input: 5,
            width: 2,
            height: 1,
            rgba: vec![100, 50, 0, 128, 0, 0, 0, 0],
        }),
        EventKind::State(State {
            title: "žluťoučký".into(),
            url: "https://opaque.invalid/".into(),
            loading: false,
            editable: true,
            zoom: 1.0,
        }),
        EventKind::FindResult(FindResult {
            request_sequence: 5,
            matches: 3,
            active: 2,
            complete: true,
        }),
        EventKind::Selection("💾".into()),
        EventKind::ResourceResolve {
            request: 1,
            base: 44,
            url: "../a%20b.css".into(),
        },
        EventKind::ResourceRead {
            request: 2,
            resource: 44,
            offset: u64::MAX - 12,
            length: 12,
        },
        EventKind::ResourceRelease { resource: 44 },
        EventKind::HostShortcut(HostShortcut::CycleNext),
        EventKind::Error {
            code: ErrorCode::Denied,
            message: "síť zakázána".into(),
        },
        EventKind::Closed,
    ];
    for kind in kinds {
        let message = event(kind);
        assert_eq!(
            read_message(&mut encode(&message).as_slice()).unwrap(),
            message
        );
    }
}

#[test]
fn rejects_invalid_values_before_writing_any_bytes() {
    let mut invalid_viewport = viewport();
    invalid_viewport.scale = f64::NAN;
    let cases = vec![
        command(CommandKind::Resize(invalid_viewport)),
        command(CommandKind::Policy(Policy {
            network: false,
            scripts: true,
        })),
        command(CommandKind::Zoom(f64::INFINITY)),
        command(CommandKind::Zoom(0.0)),
        command(CommandKind::Text("x".repeat(65_536))),
        command(CommandKind::Ime(Ime::Update {
            text: "ě".into(),
            selection_start: 1,
            selection_end: 2,
        })),
        event(EventKind::ResourceRead {
            request: 1,
            resource: 44,
            offset: u64::MAX,
            length: 2,
        }),
        event(EventKind::ResourceRead {
            request: 1,
            resource: 44,
            offset: 0,
            length: 262_145,
        }),
        event(EventKind::Frame(Frame {
            viewport_revision: 2,
            acknowledged_input: 5,
            width: 8193,
            height: 1,
            rgba: vec![],
        })),
        event(EventKind::Frame(Frame {
            viewport_revision: 2,
            acknowledged_input: 5,
            width: 8192,
            height: 8192,
            rgba: vec![],
        })),
        event(EventKind::FindResult(FindResult {
            request_sequence: 5,
            matches: 1,
            active: 2,
            complete: true,
        })),
    ];
    for message in cases {
        let mut output = Vec::new();
        assert!(write_message(&mut output, &message).is_err());
        assert!(output.is_empty());
    }
}

fn prefix(direction: u8, tag: u8, control: u32, raw: u32) -> Vec<u8> {
    let mut bytes = b"MWV1\x01\x00".to_vec();
    bytes.extend([direction, tag]);
    bytes.extend(control.to_le_bytes());
    bytes.extend(raw.to_le_bytes());
    bytes
}

#[test]
fn rejects_oversized_control_and_raw_payload_from_prefix_alone() {
    for bytes in [
        prefix(0, 6, 65_537, 0),
        prefix(0, 13, 24, 262_145),
        prefix(1, 2, 24, 67_108_865),
        prefix(0, 6, 24, 1),
        prefix(1, 255, 24, 0),
    ] {
        let error = read_message(&mut bytes.as_slice()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}

#[test]
fn rejects_invalid_utf8_tags_version_trailing_control_and_truncation() {
    let valid = encode(&command(CommandKind::Text("a".into())));
    let mut bad_utf8 = valid.clone();
    *bad_utf8.last_mut().unwrap() = 255;
    let mut bad_version = valid.clone();
    bad_version[4] = 2;
    let mut bad_tag = valid.clone();
    bad_tag[7] = 255;
    let mut trailing = valid.clone();
    trailing[8] += 1;
    trailing.push(0);
    for bytes in [bad_utf8, bad_version, bad_tag, trailing] {
        assert_eq!(
            read_message(&mut bytes.as_slice()).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
    for length in 0..valid.len() {
        assert!(read_message(&mut &valid[..length]).is_err());
    }
}

#[test]
fn large_raw_data_has_small_control_and_streams_without_absorbing_next_message() {
    let message = command(CommandKind::ResourceReply(ResourceReply::Data {
        request: 1,
        resource: 44,
        offset: 0,
        bytes: vec![123; 262_144],
    }));
    let mut bytes = encode(&message);
    assert!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()) < 100);
    bytes.extend(encode(&event(EventKind::Closed)));
    let mut cursor = Cursor::new(bytes);
    assert_eq!(read_message(&mut cursor).unwrap(), message);
    assert_eq!(read_message(&mut cursor).unwrap(), event(EventKind::Closed));
    let mut rest = Vec::new();
    cursor.read_to_end(&mut rest).unwrap();
    assert!(rest.is_empty());
}

#[test]
fn public_validation_rejects_aggregate_control_budget() {
    let message = event(EventKind::State(State {
        title: "x".repeat(40_000),
        url: "u".repeat(30_000),
        loading: false,
        editable: false,
        zoom: 1.0,
    }));
    assert_eq!(
        message.validate().unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn host_shortcut_extension_preserves_every_v1_tag_and_round_trips_new_keys() {
    let shortcuts = [
        HostShortcut::Close,
        HostShortcut::Find,
        HostShortcut::FindNext,
        HostShortcut::FindPrevious,
        HostShortcut::CycleNext,
        HostShortcut::CyclePrevious,
        HostShortcut::ZoomIn,
        HostShortcut::ZoomOut,
        HostShortcut::ZoomReset,
        HostShortcut::Copy,
        HostShortcut::Backspace,
        HostShortcut::AltLeft,
        HostShortcut::AltRight,
    ];
    for (tag, shortcut) in shortcuts.into_iter().enumerate() {
        let message = event(EventKind::HostShortcut(shortcut));
        let bytes = encode(&message);
        assert_eq!(&bytes[..8], &[b'M', b'W', b'V', b'1', 1, 0, 1, 9]);
        assert_eq!(bytes.len(), 41);
        assert_eq!(bytes[40], tag as u8);
        assert_eq!(read_message(&mut bytes.as_slice()).unwrap(), message);
    }
}

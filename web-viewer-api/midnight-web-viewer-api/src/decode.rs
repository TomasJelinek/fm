use crate::fields::Decoder;
use crate::validation::invalid;
use crate::*;
use std::io;

pub(crate) fn command(b: &mut Decoder<'_>, tag: u8) -> io::Result<CommandKind> {
    Ok(match tag {
        1 => CommandKind::Open {
            source: b.resource()?,
            viewport: b.viewport()?,
            policy: b.policy()?,
        },
        2 => CommandKind::Resize(b.viewport()?),
        3 => CommandKind::Pointer(Pointer {
            kind: match b.u8()? {
                0 => PointerKind::Move,
                1 => PointerKind::Down,
                2 => PointerKind::Up,
                3 => PointerKind::Leave,
                _ => return Err(invalid()),
            },
            button: match b.u8()? {
                0 => PointerButton::None,
                1 => PointerButton::Left,
                2 => PointerButton::Middle,
                3 => PointerButton::Right,
                4 => PointerButton::Back,
                5 => PointerButton::Forward,
                _ => return Err(invalid()),
            },
            x: b.f64()?,
            y: b.f64()?,
            modifiers: b.modifiers()?,
            click_count: b.u8()?,
        }),
        4 => CommandKind::Wheel(Wheel {
            x: b.f64()?,
            y: b.f64()?,
            delta_x: b.f64()?,
            delta_y: b.f64()?,
            modifiers: b.modifiers()?,
        }),
        5 => CommandKind::Key(Key {
            key: b.text()?,
            code: b.text()?,
            pressed: b.bool()?,
            repeat: b.bool()?,
            modifiers: b.modifiers()?,
        }),
        6 => CommandKind::Text(b.text()?),
        7 => CommandKind::Ime(match b.u8()? {
            0 => Ime::Start,
            1 => Ime::Update {
                text: b.text()?,
                selection_start: b.u32()?,
                selection_end: b.u32()?,
            },
            2 => Ime::Commit(b.text()?),
            3 => Ime::End,
            _ => return Err(invalid()),
        }),
        8 => CommandKind::Focus(b.bool()?),
        9 => CommandKind::Find(Find {
            query: b.text()?,
            forward: b.bool()?,
            match_case: b.bool()?,
            find_next: b.bool()?,
        }),
        10 => CommandKind::Zoom(b.f64()?),
        11 => CommandKind::Copy,
        12 => CommandKind::Policy(b.policy()?),
        13 => CommandKind::ResourceReply(match b.u8()? {
            0 => ResourceReply::Resolved {
                request: b.u64()?,
                resource: b.resource()?,
            },
            1 => ResourceReply::Data {
                request: b.u64()?,
                resource: b.u64()?,
                offset: b.u64()?,
                bytes: Vec::new(),
            },
            2 => ResourceReply::Released { resource: b.u64()? },
            3 => ResourceReply::Denied {
                request: b.u64()?,
                reason: b.text()?,
            },
            _ => return Err(invalid()),
        }),
        14 => CommandKind::FrameAck {
            frame_sequence: b.u64()?,
        },
        15 => CommandKind::Close,
        _ => return Err(invalid()),
    })
}

pub(crate) fn event(b: &mut Decoder<'_>, tag: u8) -> io::Result<EventKind> {
    Ok(match tag {
        1 => EventKind::Ready,
        2 => EventKind::Frame(Frame {
            viewport_revision: b.u64()?,
            acknowledged_input: b.u64()?,
            width: b.u32()?,
            height: b.u32()?,
            rgba: Vec::new(),
        }),
        3 => EventKind::State(State {
            title: b.text()?,
            url: b.text()?,
            loading: b.bool()?,
            editable: b.bool()?,
            zoom: b.f64()?,
        }),
        4 => EventKind::FindResult(FindResult {
            request_sequence: b.u64()?,
            matches: b.u32()?,
            active: b.u32()?,
            complete: b.bool()?,
        }),
        5 => EventKind::Selection(b.text()?),
        6 => EventKind::ResourceResolve {
            request: b.u64()?,
            base: b.u64()?,
            url: b.text()?,
        },
        7 => EventKind::ResourceRead {
            request: b.u64()?,
            resource: b.u64()?,
            offset: b.u64()?,
            length: b.u32()?,
        },
        8 => EventKind::ResourceRelease { resource: b.u64()? },
        9 => EventKind::HostShortcut(match b.u8()? {
            0 => HostShortcut::Close,
            1 => HostShortcut::Find,
            2 => HostShortcut::FindNext,
            3 => HostShortcut::FindPrevious,
            4 => HostShortcut::CycleNext,
            5 => HostShortcut::CyclePrevious,
            6 => HostShortcut::ZoomIn,
            7 => HostShortcut::ZoomOut,
            8 => HostShortcut::ZoomReset,
            9 => HostShortcut::Copy,
            _ => return Err(invalid()),
        }),
        10 => EventKind::Error {
            code: match b.u8()? {
                0 => ErrorCode::InvalidRequest,
                1 => ErrorCode::Denied,
                2 => ErrorCode::MissingResource,
                3 => ErrorCode::ResourceChanged,
                4 => ErrorCode::Unsupported,
                5 => ErrorCode::Runtime,
                6 => ErrorCode::BudgetExceeded,
                _ => return Err(invalid()),
            },
            message: b.text()?,
        },
        11 => EventKind::Closed,
        _ => return Err(invalid()),
    })
}

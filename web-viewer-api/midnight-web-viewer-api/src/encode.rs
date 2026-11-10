use crate::fields::Encoder;
use crate::*;
use std::io;

pub(crate) fn command(b: &mut Encoder, kind: &CommandKind) -> io::Result<u8> {
    Ok(match kind {
        CommandKind::Open {
            source,
            viewport,
            policy,
        } => {
            b.resource(source)?;
            b.viewport(*viewport);
            b.policy(*policy);
            1
        }
        CommandKind::Resize(viewport) => {
            b.viewport(*viewport);
            2
        }
        CommandKind::Pointer(value) => {
            b.u8(match value.kind {
                PointerKind::Move => 0,
                PointerKind::Down => 1,
                PointerKind::Up => 2,
                PointerKind::Leave => 3,
            });
            b.u8(match value.button {
                PointerButton::None => 0,
                PointerButton::Left => 1,
                PointerButton::Middle => 2,
                PointerButton::Right => 3,
                PointerButton::Back => 4,
                PointerButton::Forward => 5,
            });
            b.f64(value.x);
            b.f64(value.y);
            b.modifiers(value.modifiers);
            b.u8(value.click_count);
            3
        }
        CommandKind::Wheel(value) => {
            b.f64(value.x);
            b.f64(value.y);
            b.f64(value.delta_x);
            b.f64(value.delta_y);
            b.modifiers(value.modifiers);
            4
        }
        CommandKind::Key(value) => {
            b.text(&value.key)?;
            b.text(&value.code)?;
            b.bool(value.pressed);
            b.bool(value.repeat);
            b.modifiers(value.modifiers);
            5
        }
        CommandKind::Text(value) => {
            b.text(value)?;
            6
        }
        CommandKind::Ime(value) => {
            match value {
                Ime::Start => b.u8(0),
                Ime::Update {
                    text,
                    selection_start,
                    selection_end,
                } => {
                    b.u8(1);
                    b.text(text)?;
                    b.u32(*selection_start);
                    b.u32(*selection_end);
                }
                Ime::Commit(text) => {
                    b.u8(2);
                    b.text(text)?;
                }
                Ime::End => b.u8(3),
            }
            7
        }
        CommandKind::Focus(value) => {
            b.bool(*value);
            8
        }
        CommandKind::Find(value) => {
            b.text(&value.query)?;
            b.bool(value.forward);
            b.bool(value.match_case);
            b.bool(value.find_next);
            9
        }
        CommandKind::Zoom(value) => {
            b.f64(*value);
            10
        }
        CommandKind::Copy => 11,
        CommandKind::Policy(value) => {
            b.policy(*value);
            12
        }
        CommandKind::ResourceReply(reply) => {
            match reply {
                ResourceReply::Resolved { request, resource } => {
                    b.u8(0);
                    b.u64(*request);
                    b.resource(resource)?;
                }
                ResourceReply::Data {
                    request,
                    resource,
                    offset,
                    ..
                } => {
                    b.u8(1);
                    b.u64(*request);
                    b.u64(*resource);
                    b.u64(*offset);
                }
                ResourceReply::Released { resource } => {
                    b.u8(2);
                    b.u64(*resource);
                }
                ResourceReply::Denied { request, reason } => {
                    b.u8(3);
                    b.u64(*request);
                    b.text(reason)?;
                }
            }
            13
        }
        CommandKind::FrameAck { frame_sequence } => {
            b.u64(*frame_sequence);
            14
        }
        CommandKind::Close => 15,
    })
}

pub(crate) fn event(b: &mut Encoder, kind: &EventKind) -> io::Result<u8> {
    Ok(match kind {
        EventKind::Ready => 1,
        EventKind::Frame(frame) => {
            b.u64(frame.viewport_revision);
            b.u64(frame.acknowledged_input);
            b.u32(frame.width);
            b.u32(frame.height);
            2
        }
        EventKind::State(state) => {
            b.text(&state.title)?;
            b.text(&state.url)?;
            b.bool(state.loading);
            b.bool(state.editable);
            b.f64(state.zoom);
            3
        }
        EventKind::FindResult(result) => {
            b.u64(result.request_sequence);
            b.u32(result.matches);
            b.u32(result.active);
            b.bool(result.complete);
            4
        }
        EventKind::Selection(text) => {
            b.text(text)?;
            5
        }
        EventKind::ResourceResolve { request, base, url } => {
            b.u64(*request);
            b.u64(*base);
            b.text(url)?;
            6
        }
        EventKind::ResourceRead {
            request,
            resource,
            offset,
            length,
        } => {
            b.u64(*request);
            b.u64(*resource);
            b.u64(*offset);
            b.u32(*length);
            7
        }
        EventKind::ResourceRelease { resource } => {
            b.u64(*resource);
            8
        }
        EventKind::HostShortcut(value) => {
            b.u8(match value {
                HostShortcut::Close => 0,
                HostShortcut::Find => 1,
                HostShortcut::FindNext => 2,
                HostShortcut::FindPrevious => 3,
                HostShortcut::CycleNext => 4,
                HostShortcut::CyclePrevious => 5,
                HostShortcut::ZoomIn => 6,
                HostShortcut::ZoomOut => 7,
                HostShortcut::ZoomReset => 8,
                HostShortcut::Copy => 9,
            });
            9
        }
        EventKind::Error { code, message } => {
            b.u8(match code {
                ErrorCode::InvalidRequest => 0,
                ErrorCode::Denied => 1,
                ErrorCode::MissingResource => 2,
                ErrorCode::ResourceChanged => 3,
                ErrorCode::Unsupported => 4,
                ErrorCode::Runtime => 5,
                ErrorCode::BudgetExceeded => 6,
            });
            b.text(message)?;
            10
        }
        EventKind::Closed => 11,
    })
}

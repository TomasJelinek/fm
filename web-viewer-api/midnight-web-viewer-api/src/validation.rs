use crate::*;
use std::io;

pub(crate) fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid web-viewer protocol message",
    )
}

pub(crate) fn check(condition: bool) -> io::Result<()> {
    if condition { Ok(()) } else { Err(invalid()) }
}

/// Checked packed RGBA8 size; validate before allocating a physical canvas.
pub fn rgba_length(width: u32, height: u32) -> io::Result<usize> {
    let pixels = u64::from(width) * u64::from(height);
    check(
        width != 0
            && height != 0
            && width <= MAX_EDGE
            && height <= MAX_EDGE
            && pixels <= MAX_PIXELS,
    )?;
    usize::try_from(pixels * 4).map_err(|_| invalid())
}

impl Header {
    /// Reject reserved zero identities. Sequence ordering is a session concern.
    pub fn validate(self) -> io::Result<Self> {
        check(self.session != 0 && self.document != 0 && self.sequence != 0)?;
        Ok(self)
    }
}

impl Viewport {
    /// Validate dimensions, revision and DPI before allocation or conversion.
    pub fn validate(self) -> io::Result<Self> {
        rgba_length(self.width, self.height)?;
        check(self.revision != 0 && self.scale.is_finite() && (0.25..=8.0).contains(&self.scale))?;
        Ok(self)
    }
}

impl Policy {
    /// Protocol one cannot enable scripts without enabling online requests.
    pub fn validate(self) -> io::Result<Self> {
        check(!self.scripts || self.network)?;
        Ok(self)
    }
}

impl Resource {
    /// Validate capability identity and inert MIME metadata; length has no whole-file limit.
    pub fn validate(&self) -> io::Result<()> {
        check(
            self.id != 0
                && !self.mime.is_empty()
                && self.mime.len() <= MAX_CONTROL_BYTES
                && !self.mime.chars().any(char::is_control),
        )
    }
}

impl Message {
    /// The identity common to both transport directions.
    pub fn header(&self) -> Header {
        match self {
            Self::Command(command) => command.header,
            Self::Event(event) => event.header,
        }
    }

    /// Validate all numeric, text and control/raw budgets without copying raw data.
    pub fn validate(&self) -> io::Result<()> {
        crate::wire::encode_control(self).map(|_| ())
    }

    pub(crate) fn raw(&self) -> &[u8] {
        match self {
            Self::Command(Command {
                kind: CommandKind::ResourceReply(ResourceReply::Data { bytes, .. }),
                ..
            }) => bytes,
            Self::Event(Event {
                kind: EventKind::Frame(frame),
                ..
            }) => &frame.rgba,
            _ => &[],
        }
    }
}

fn zoom(value: f64) -> io::Result<()> {
    check(value.is_finite() && (0.1..=10.0).contains(&value))
}

fn finite(values: &[f64]) -> io::Result<()> {
    check(values.iter().all(|value| value.is_finite()))
}

/// Validate an individual source range. The resource owner also checks its captured length.
pub fn validate_read(offset: u64, length: u32) -> io::Result<()> {
    check(
        length != 0
            && length as usize <= MAX_READ_BYTES
            && offset.checked_add(u64::from(length)).is_some(),
    )
}

pub(crate) fn validate_raw(message: &Message, raw_len: usize) -> io::Result<()> {
    message.header().validate()?;
    match message {
        Message::Command(command) => match &command.kind {
            CommandKind::Open {
                source,
                viewport,
                policy,
            } => {
                source.validate()?;
                viewport.validate()?;
                policy.validate()?;
            }
            CommandKind::Resize(viewport) => {
                viewport.validate()?;
            }
            CommandKind::Pointer(pointer) => {
                finite(&[pointer.x, pointer.y])?;
                check(pointer.click_count <= 3)?;
                check(
                    !matches!(pointer.kind, PointerKind::Down | PointerKind::Up)
                        || pointer.button != PointerButton::None,
                )?;
            }
            CommandKind::Wheel(wheel) => finite(&[wheel.x, wheel.y, wheel.delta_x, wheel.delta_y])?,
            CommandKind::Key(key) => check(!key.key.is_empty() && !key.code.is_empty())?,
            CommandKind::Ime(Ime::Update {
                text,
                selection_start,
                selection_end,
            }) => {
                check(
                    selection_start <= selection_end
                        && text.is_char_boundary(*selection_start as usize)
                        && text.is_char_boundary(*selection_end as usize),
                )?;
            }
            CommandKind::Zoom(value) => zoom(*value)?,
            CommandKind::Policy(policy) => {
                policy.validate()?;
            }
            CommandKind::ResourceReply(reply) => match reply {
                ResourceReply::Resolved { request, resource } => {
                    check(*request != 0)?;
                    resource.validate()?;
                }
                ResourceReply::Data {
                    request,
                    resource,
                    offset,
                    ..
                } => {
                    check(
                        *request != 0
                            && *resource != 0
                            && raw_len <= MAX_READ_BYTES
                            && offset.checked_add(raw_len as u64).is_some(),
                    )?;
                }
                ResourceReply::Released { resource } => check(*resource != 0)?,
                ResourceReply::Denied { request, .. } => check(*request != 0)?,
            },
            CommandKind::FrameAck { frame_sequence } => check(*frame_sequence != 0)?,
            _ => {}
        },
        Message::Event(event) => match &event.kind {
            EventKind::Frame(frame) => {
                check(
                    frame.viewport_revision != 0
                        && rgba_length(frame.width, frame.height)? == raw_len,
                )?;
            }
            EventKind::State(state) => zoom(state.zoom)?,
            EventKind::FindResult(result) => {
                check(result.request_sequence != 0 && result.active <= result.matches)?
            }
            EventKind::ResourceResolve { request, base, url } => {
                check(*request != 0 && *base != 0 && !url.is_empty())?
            }
            EventKind::ResourceRead {
                request,
                resource,
                offset,
                length,
            } => {
                check(*request != 0 && *resource != 0)?;
                validate_read(*offset, *length)?;
            }
            EventKind::ResourceRelease { resource } => check(*resource != 0)?,
            _ => {}
        },
    }
    if !matches!(
        message,
        Message::Command(Command {
            kind: CommandKind::ResourceReply(ResourceReply::Data { .. }),
            ..
        }) | Message::Event(Event {
            kind: EventKind::Frame(_),
            ..
        })
    ) {
        check(raw_len == 0)?;
    }
    Ok(())
}

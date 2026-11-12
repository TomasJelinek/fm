use crate::invalid;
use midnight_web_viewer_api::*;
use std::io;

/// Stateful validation for one session/document. No files, browser objects or resource broker.
/// Feed both accepted command and event directions in transport order.
#[derive(Debug)]
pub struct Session {
    session: u64,
    document: u64,
    command_sequence: u64,
    event_sequence: u64,
    viewport: Option<Viewport>,
    ready: bool,
    closing: bool,
    closed: bool,
}

impl Session {
    /// Construct a fresh generation. New documents require a new Session and default Policy.
    pub fn new(session: u64, document: u64) -> io::Result<Self> {
        Header {
            session,
            document,
            sequence: 1,
        }
        .validate()?;
        Ok(Self {
            session,
            document,
            command_sequence: 0,
            event_sequence: 0,
            viewport: None,
            ready: false,
            closing: false,
            closed: false,
        })
    }

    /// Validate and atomically accept a transition. Errors leave all state unchanged.
    /// Also validate broker request ownership and retained frames at their owning boundary.
    pub fn accept(&mut self, message: &Message) -> io::Result<()> {
        message.validate()?;
        let header = message.header();
        if self.closed || header.session != self.session || header.document != self.document {
            return Err(invalid());
        }
        match message {
            Message::Command(command) => {
                if header.sequence <= self.command_sequence {
                    return Err(invalid());
                }
                self.command(&command.kind)?;
                self.command_sequence = header.sequence;
            }
            Message::Event(event) => {
                if header.sequence <= self.event_sequence {
                    return Err(invalid());
                }
                self.event(&event.kind)?;
                self.event_sequence = header.sequence;
            }
        }
        Ok(())
    }

    fn command(&mut self, kind: &CommandKind) -> io::Result<()> {
        if self.closing
            && !matches!(
                kind,
                CommandKind::FrameAck { .. } | CommandKind::ResourceReply(_)
            )
        {
            return Err(invalid());
        }
        match kind {
            CommandKind::Open { viewport, .. } => {
                if self.viewport.is_some() {
                    return Err(invalid());
                }
                self.viewport = Some(*viewport);
            }
            _ if self.viewport.is_none() => return Err(invalid()),
            CommandKind::Resize(viewport) => {
                if self
                    .viewport
                    .is_some_and(|old| viewport.revision <= old.revision)
                {
                    return Err(invalid());
                }
                self.viewport = Some(*viewport);
            }
            CommandKind::FrameAck { frame_sequence } if *frame_sequence > self.event_sequence => {
                return Err(invalid());
            }
            CommandKind::Close => self.closing = true,
            _ => {}
        }
        Ok(())
    }

    fn event(&mut self, kind: &EventKind) -> io::Result<()> {
        match kind {
            EventKind::Error { .. } => {}
            EventKind::Closed => self.closed = true,
            _ if self.viewport.is_none() => return Err(invalid()),
            EventKind::Ready => {
                if self.ready || self.closing {
                    return Err(invalid());
                }
                self.ready = true;
            }
            EventKind::Frame(frame) => {
                let viewport = self.viewport.ok_or_else(invalid)?;
                if !self.ready
                    || self.closing
                    || frame.viewport_revision != viewport.revision
                    || frame.width != viewport.width
                    || frame.height != viewport.height
                    || frame.acknowledged_input > self.command_sequence
                {
                    return Err(invalid());
                }
            }
            EventKind::FindResult(result) if result.request_sequence > self.command_sequence => {
                return Err(invalid());
            }
            _ => {}
        }
        Ok(())
    }

    /// True after a validated Closed event. Transport termination requires owner cleanup too.
    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

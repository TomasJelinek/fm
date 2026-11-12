use crate::invalid;
use midnight_web_viewer_api::*;
use std::collections::VecDeque;
use std::io;

/// Hard queue-item ceiling; resource replies also retain their individual raw-data budget.
pub const MAX_QUEUED_COMMANDS: usize = 64;
/// At most two complete frames may await host acknowledgement (128 MiB raw maximum).
pub const MAX_RETAINED_FRAMES: usize = 2;

/// Bounded FIFO that coalesces only adjacent Resize or pointer Move commands.
/// Buttons, keys, resource ownership, Close and every other command retain exact ordering.
pub struct CommandQueue {
    capacity: usize,
    commands: VecDeque<Command>,
}

impl CommandQueue {
    /// Capacity must be 1..=MAX_QUEUED_COMMANDS.
    pub fn new(capacity: usize) -> io::Result<Self> {
        if !(1..=MAX_QUEUED_COMMANDS).contains(&capacity) {
            return Err(invalid());
        }
        Ok(Self {
            capacity,
            commands: VecDeque::new(),
        })
    }

    /// Return displaced ownership on coalescing, or original ownership on invalid/full input.
    /// Never drops a resource reply or loses a caller's buffer on pressure.
    pub fn push(&mut self, value: Command) -> Result<Option<Command>, Command> {
        let message = Message::Command(value);
        let valid = message.validate().is_ok();
        let Message::Command(value) = message else {
            unreachable!()
        };
        if !valid {
            return Err(value);
        }
        if let Some(previous) = self.commands.back_mut()
            && previous.header.session == value.header.session
            && previous.header.document == value.header.document
            && previous.header.sequence < value.header.sequence
            && coalesces(&previous.kind, &value.kind)
        {
            return Ok(Some(std::mem::replace(previous, value)));
        }
        if self.commands.len() == self.capacity {
            return Err(value);
        }
        self.commands.push_back(value);
        Ok(None)
    }

    /// Transfer ownership of the oldest command to the transport worker.
    pub fn pop(&mut self) -> Option<Command> {
        self.commands.pop_front()
    }
}

fn coalesces(previous: &CommandKind, next: &CommandKind) -> bool {
    matches!(
        (previous, next),
        (CommandKind::Resize(_), CommandKind::Resize(_))
    ) || matches!(
        (previous, next),
        (
            CommandKind::Pointer(Pointer {
                kind: PointerKind::Move,
                ..
            }),
            CommandKind::Pointer(Pointer {
                kind: PointerKind::Move,
                ..
            })
        )
    )
}

/// Owned frame retention until exact host acknowledgements arrive. One window per generation.
pub struct FrameWindow {
    capacity: usize,
    frames: VecDeque<Event>,
    last_header: Option<Header>,
}

impl FrameWindow {
    /// Capacity must be 1..=MAX_RETAINED_FRAMES.
    pub fn new(capacity: usize) -> io::Result<Self> {
        if !(1..=MAX_RETAINED_FRAMES).contains(&capacity) {
            return Err(invalid());
        }
        Ok(Self {
            capacity,
            frames: VecDeque::new(),
            last_header: None,
        })
    }

    /// Retain a validated Frame event. Returns original owned pixels on invalid/full input.
    pub fn retain(&mut self, value: Event) -> Result<(), Event> {
        if self.frames.len() == self.capacity
            || !matches!(&value.kind, EventKind::Frame(_))
            || self.last_header.is_some_and(|last| {
                last.session != value.header.session
                    || last.document != value.header.document
                    || last.sequence >= value.header.sequence
            })
        {
            return Err(value);
        }
        let message = Message::Event(value);
        let valid = message.validate().is_ok();
        let Message::Event(value) = message else {
            unreachable!()
        };
        if !valid {
            return Err(value);
        }
        self.last_header = Some(value.header);
        self.frames.push_back(value);
        Ok(())
    }

    /// Transfer exactly the acknowledged frame. Unknown/duplicate acknowledgements change nothing.
    pub fn acknowledge(&mut self, sequence: u64) -> Option<Event> {
        let index = self
            .frames
            .iter()
            .position(|event| event.header.sequence == sequence)?;
        self.frames.remove(index)
    }

    /// Transfer all remaining pixels for backend cleanup after close, EOF or process failure.
    pub fn clear(&mut self) -> Vec<Event> {
        self.frames.drain(..).collect()
    }
}

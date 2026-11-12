use crate::buffers::{BufferPool, PoolError};
use midnight_plugin_api::{abi, viewer, wire};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, atomic::AtomicU64},
};

const MAX_REQUESTS: usize = 64;
const MAX_ORDINARY: usize = 32;
const MAX_TERMINAL_BYTES: usize = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum QueueError {
    Full,
    Invalid,
    Unknown,
    Closed,
    Exhausted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Origin {
    pub id: u64,
    pub session: u64,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ticket {
    origin: Origin,
    sequence: u64,
}

impl Ticket {
    pub(crate) fn origin(self) -> Origin {
        self.origin
    }
}

#[derive(Debug)]
pub struct Output {
    pub(crate) kind: u32,
    pub(crate) payload: Box<[u8]>,
    pub(crate) content: Option<Box<[u8]>>,
}

struct Accepted {
    ticket: Ticket,
    reserved: u64,
    finished: bool,
    emitted: bool,
}
#[derive(Clone, Copy)]
struct Pending {
    ticket: Ticket,
    kind: u32,
    payload: u64,
    content: Option<u64>,
    terminal: bool,
}

pub(crate) struct Outbox {
    buffers: BufferPool,
    accepted: HashMap<u64, Accepted>,
    pending: VecDeque<Pending>,
    sequence: u64,
    ordinary: usize,
    closed: bool,
}

impl Outbox {
    pub(crate) fn new(ids: Arc<AtomicU64>) -> Self {
        Self {
            buffers: BufferPool::new(8 * 1024 * 1024, MAX_REQUESTS * MAX_TERMINAL_BYTES, ids),
            accepted: HashMap::new(),
            pending: VecDeque::new(),
            sequence: 1,
            ordinary: 0,
            closed: false,
        }
    }
    pub(crate) fn begin(&mut self, origin: Origin) -> Result<Ticket, QueueError> {
        if self.closed {
            return Err(QueueError::Closed);
        }
        if origin.id == 0 || origin.session == 0 || self.accepted.contains_key(&origin.id) {
            return Err(QueueError::Invalid);
        }
        if self.accepted.len() >= MAX_REQUESTS {
            return Err(QueueError::Full);
        }
        let next = self.sequence.checked_add(1).ok_or(QueueError::Exhausted)?;
        let reserved = self
            .buffers
            .insert(Box::new([]), true)
            .map_err(|(error, _)| QueueError::from(error))?;
        let ticket = Ticket {
            origin,
            sequence: self.sequence,
        };
        self.sequence = next;
        self.accepted.insert(
            origin.id,
            Accepted {
                ticket,
                reserved,
                finished: false,
                emitted: false,
            },
        );
        Ok(ticket)
    }

    /// Roll back admission only before work or output becomes observable.
    pub(crate) fn withdraw(&mut self, ticket: Ticket) -> Result<(), QueueError> {
        let accepted = self.active(ticket)?;
        if accepted.emitted {
            return Err(QueueError::Invalid);
        }
        self.buffers.discard(accepted.reserved)?;
        self.accepted.remove(&ticket.origin.id);
        Ok(())
    }

    pub(crate) fn emit(
        &mut self,
        ticket: Ticket,
        output: Output,
    ) -> Result<(), (QueueError, Output)> {
        if let Err(error) = self.active(ticket) {
            return Err((error, output));
        }
        if self.ordinary >= MAX_ORDINARY {
            return Err((QueueError::Full, output));
        }
        if !(abi::EVENT_OPENED..=abi::EVENT_TEXT_POSITION).contains(&output.kind)
            || output.payload.len() > wire::MAX_MESSAGE_BYTES
            || output
                .content
                .as_ref()
                .is_some_and(|bytes| bytes.len() > viewer::MAX_PAGE_BYTES)
            || wire::validate(&output.payload).is_err()
        {
            return Err((QueueError::Invalid, output));
        }
        let Output {
            kind,
            payload,
            content,
        } = output;
        let (payload, content) = if let Some(content) = content {
            match self.buffers.insert_batch([payload, content], false) {
                Ok([payload, content]) => (payload, Some(content)),
                Err((error, [payload, content])) => {
                    return Err((
                        error.into(),
                        Output {
                            kind,
                            payload,
                            content: Some(content),
                        },
                    ));
                }
            }
        } else {
            match self.buffers.insert(payload, false) {
                Ok(payload) => (payload, None),
                Err((error, payload)) => {
                    return Err((
                        error.into(),
                        Output {
                            kind,
                            payload,
                            content: None,
                        },
                    ));
                }
            }
        };
        self.pending.push_back(Pending {
            ticket,
            kind,
            payload,
            content,
            terminal: false,
        });
        self.ordinary += 1;
        self.accepted
            .get_mut(&ticket.origin.id)
            .expect("active ticket")
            .emitted = true;
        Ok(())
    }

    pub(crate) fn finish(
        &mut self,
        ticket: Ticket,
        kind: u32,
        payload: Box<[u8]>,
    ) -> Result<(), QueueError> {
        let reserved = self.active(ticket)?.reserved;
        if !matches!(
            kind,
            abi::EVENT_COMPLETED | abi::EVENT_CANCELLED | abi::EVENT_FAILED
        ) || payload.len() > MAX_TERMINAL_BYTES
            || (!payload.is_empty() && wire::validate(&payload).is_err())
        {
            return Err(QueueError::Invalid);
        }
        self.buffers
            .replace_reserved(reserved, payload)
            .map_err(|(error, _)| QueueError::from(error))?;
        if kind == abi::EVENT_CANCELLED {
            self.discard_pending(ticket)?;
        }
        self.accepted
            .get_mut(&ticket.origin.id)
            .ok_or(QueueError::Unknown)?
            .finished = true;
        self.pending.push_back(Pending {
            ticket,
            kind,
            payload: reserved,
            content: None,
            terminal: true,
        });
        Ok(())
    }

    pub(crate) fn drain_one(&mut self) -> Result<Option<abi::Event>, QueueError> {
        let Some(pending) = self.pending.front().copied() else {
            return Ok(None);
        };
        let (payload, content) = if let Some(content) = pending.content {
            let [payload, content] = self.buffers.lease_batch([pending.payload, content])?;
            (payload, content)
        } else {
            (self.buffers.lease(pending.payload)?, empty_buffer())
        };
        self.pending.pop_front();
        if !pending.terminal {
            self.ordinary -= 1;
        }
        let origin = pending.ticket.origin;
        Ok(Some(abi::Event {
            request_id: origin.id,
            session: origin.session,
            generation: origin.generation,
            kind: pending.kind,
            flags: 0,
            payload,
            content,
        }))
    }

    pub(crate) fn release(&mut self, buffer: u64) -> Result<(), QueueError> {
        self.buffers.release(buffer)?;
        let finished = self.accepted.iter().find_map(|(&id, request)| {
            (request.finished && request.reserved == buffer).then_some(id)
        });
        if let Some(id) = finished {
            self.accepted.remove(&id);
        }
        Ok(())
    }

    fn active(&self, ticket: Ticket) -> Result<&Accepted, QueueError> {
        let request = self
            .accepted
            .get(&ticket.origin.id)
            .filter(|r| r.ticket == ticket)
            .ok_or(QueueError::Unknown)?;
        if request.finished {
            Err(QueueError::Invalid)
        } else {
            Ok(request)
        }
    }

    fn discard_pending(&mut self, ticket: Ticket) -> Result<(), QueueError> {
        let mut index = 0;
        while index < self.pending.len() {
            let pending = self.pending[index];
            if pending.ticket == ticket && !pending.terminal {
                self.buffers.discard(pending.payload)?;
                if let Some(content) = pending.content {
                    self.buffers.discard(content)?;
                }
                self.pending.remove(index);
                self.ordinary -= 1;
            } else {
                index += 1;
            }
        }
        Ok(())
    }
    pub(crate) fn begin_shutdown(&mut self) {
        self.closed = true;
    }
    pub(crate) fn is_drained(&self) -> bool {
        self.accepted.is_empty() && self.pending.is_empty() && self.buffers.is_empty()
    }
}

fn empty_buffer() -> abi::Buffer {
    abi::Buffer {
        id: 0,
        bytes: abi::ByteSlice {
            data: std::ptr::null(),
            len: 0,
            reserved: 0,
        },
    }
}

impl From<PoolError> for QueueError {
    fn from(error: PoolError) -> Self {
        match error {
            PoolError::Full => Self::Full,
            PoolError::Unknown => Self::Unknown,
            PoolError::InvalidState => Self::Invalid,
            PoolError::Exhausted => Self::Exhausted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn origin(id: u64) -> Origin {
        Origin {
            id,
            session: 7,
            generation: 1,
        }
    }
    fn outbox() -> Outbox {
        Outbox::new(Arc::new(AtomicU64::new(1)))
    }
    fn page() -> Output {
        Output {
            kind: abi::EVENT_TEXT_PAGE,
            payload: vec![0xa0].into_boxed_slice(),
            content: None,
        }
    }

    #[test]
    fn optional_text_position_uses_the_normal_bounded_output_queue() {
        let mut outbox = outbox();
        let ticket = outbox.begin(origin(8)).unwrap();
        let output = Output::text_position(&viewer::TextPosition {
            revision: 1,
            source_offset: 1 << 40,
            line: 80,
            column: 3,
        })
        .unwrap();
        assert!(outbox.emit(ticket, output).is_ok());
    }

    #[test]
    fn rejected_scheduling_withdraws_reservation_without_emitting_a_terminal() {
        let mut outbox = outbox();
        let rejected = outbox.begin(origin(1)).unwrap();
        outbox.withdraw(rejected).unwrap();
        assert!(outbox.is_drained());
        assert!(outbox.drain_one().unwrap().is_none());
        let current = outbox.begin(origin(1)).unwrap();
        assert_eq!(outbox.withdraw(rejected), Err(QueueError::Unknown));
        outbox
            .finish(current, abi::EVENT_COMPLETED, Box::new([]))
            .unwrap();
        assert_eq!(outbox.withdraw(current), Err(QueueError::Invalid));
    }

    #[test]
    fn completion_follows_data_and_buffers_stay_owned_until_release() {
        let mut outbox = outbox();
        let ticket = outbox.begin(origin(1)).unwrap();
        let mut output = page();
        output.content = Some(vec![1, 2].into_boxed_slice());
        outbox.emit(ticket, output).unwrap();
        outbox
            .finish(ticket, abi::EVENT_COMPLETED, Box::new([]))
            .unwrap();
        assert!(
            outbox
                .finish(ticket, abi::EVENT_COMPLETED, Box::new([]))
                .is_err()
        );
        let data = outbox.drain_one().unwrap().unwrap();
        assert_eq!(data.kind, abi::EVENT_TEXT_PAGE);
        assert_eq!(data.content.bytes.len, 2);
        assert_eq!(data.request_id, 1);
        assert_eq!(data.session, 7);
        outbox.release(data.payload.id).unwrap();
        let terminal = outbox.drain_one().unwrap().unwrap();
        assert_eq!(terminal.kind, abi::EVENT_COMPLETED);
        outbox.release(terminal.payload.id).unwrap();
        assert!(!outbox.is_drained(), "raw content is still leased");
        outbox.release(data.content.id).unwrap();
        assert!(outbox.is_drained());
        assert!(outbox.drain_one().unwrap().is_none());
    }

    #[test]
    fn full_data_queue_preserves_rejected_output_and_cannot_starve_terminal() {
        let mut outbox = outbox();
        let ticket = outbox.begin(origin(1)).unwrap();
        for _ in 0..MAX_ORDINARY {
            outbox.emit(ticket, page()).unwrap();
        }
        let (error, output) = outbox.emit(ticket, page()).unwrap_err();
        assert_eq!(error, QueueError::Full);
        assert_eq!(&*output.payload, &[0xa0]);
        outbox
            .finish(ticket, abi::EVENT_COMPLETED, Box::new([]))
            .unwrap();
        for _ in 0..MAX_ORDINARY {
            let data = outbox.drain_one().unwrap().unwrap();
            assert_eq!(data.kind, abi::EVENT_TEXT_PAGE);
            outbox.release(data.payload.id).unwrap();
        }
        let terminal = outbox.drain_one().unwrap().unwrap();
        assert_eq!(terminal.kind, abi::EVENT_COMPLETED);
        outbox.release(terminal.payload.id).unwrap();
        assert!(outbox.is_drained());
    }

    #[test]
    fn cancellation_discards_queued_data_and_old_tickets_cannot_modify_reused_ids() {
        let mut outbox = outbox();
        let old = outbox.begin(origin(1)).unwrap();
        outbox.emit(old, page()).unwrap();
        outbox
            .finish(old, abi::EVENT_CANCELLED, Box::new([]))
            .unwrap();
        let cancelled = outbox.drain_one().unwrap().unwrap();
        assert_eq!(cancelled.kind, abi::EVENT_CANCELLED);
        outbox.release(cancelled.payload.id).unwrap();
        let current = outbox
            .begin(Origin {
                generation: 2,
                ..origin(1)
            })
            .unwrap();
        assert_ne!(old, current);
        assert_eq!(outbox.emit(old, page()).unwrap_err().0, QueueError::Unknown);
        outbox.begin_shutdown();
        assert_eq!(outbox.begin(origin(2)), Err(QueueError::Closed));
        outbox
            .finish(current, abi::EVENT_CANCELLED, Box::new([]))
            .unwrap();
        let cancelled = outbox.drain_one().unwrap().unwrap();
        assert_eq!(cancelled.generation, 2);
        outbox.release(cancelled.payload.id).unwrap();
        assert!(outbox.is_drained());
    }

    #[test]
    fn accepted_request_slots_are_reserved_until_terminal_buffers_are_released() {
        let mut outbox = outbox();
        let tickets: Vec<_> = (1..=64)
            .map(|id| outbox.begin(origin(id)).unwrap())
            .collect();
        assert_eq!(outbox.begin(origin(65)), Err(QueueError::Full));
        for ticket in tickets {
            outbox
                .finish(ticket, abi::EVENT_COMPLETED, Box::new([]))
                .unwrap();
        }
        assert_eq!(outbox.begin(origin(65)), Err(QueueError::Full));
        let terminal = outbox.drain_one().unwrap().unwrap();
        outbox.release(terminal.payload.id).unwrap();
        assert!(outbox.begin(origin(65)).is_ok());
    }
}

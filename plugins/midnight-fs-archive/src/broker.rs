//! Restricted worker transport: source/stage identifiers are never native paths.
use crate::storage::{ReadSeek, Storage, seek_position};
use midnight_filesystem_api::{self as api, MAX_READ, Message, Protocol, StagePurpose};
use std::{
    cell::{Cell, RefCell},
    io::{self, Read, Seek, SeekFrom, Write},
    rc::Rc,
};

trait Transport {
    fn exchange(&mut self, message: &Message) -> io::Result<Message>;
    fn receive(&mut self) -> io::Result<Message>;
    fn send(&mut self, message: &Message) -> io::Result<()>;
}
struct Pipes<R, W> {
    input: R,
    output: W,
}
impl<R: Read, W: Write> Transport for Pipes<R, W> {
    fn exchange(&mut self, message: &Message) -> io::Result<Message> {
        self.send(message)?;
        self.receive()
    }
    fn receive(&mut self) -> io::Result<Message> {
        api::read_message_with_protocol(&mut self.input, Protocol::V3)
    }
    fn send(&mut self, message: &Message) -> io::Result<()> {
        api::write_message_with_protocol(&mut self.output, Protocol::V3, message)
    }
}
#[derive(Clone)]
pub struct Broker {
    transport: Rc<RefCell<Box<dyn Transport>>>,
    request: Rc<Cell<u64>>,
}
impl Broker {
    pub fn new(input: impl Read + 'static, output: impl Write + 'static) -> Self {
        Self {
            transport: Rc::new(RefCell::new(Box::new(Pipes { input, output }))),
            request: Rc::new(Cell::new(0)),
        }
    }
    pub fn receive(&self) -> io::Result<Message> {
        let message = self.transport.borrow_mut().receive()?;
        if message.request() <= self.request.get() {
            return Err(invalid());
        }
        self.request.set(message.request());
        Ok(message)
    }
    pub fn send(&self, message: &Message) -> io::Result<()> {
        if message.request() != self.request.get() {
            return Err(invalid());
        }
        self.transport.borrow_mut().send(message)
    }
    fn exchange(&self, message: Message) -> io::Result<Message> {
        let reply = self.transport.borrow_mut().exchange(&message)?;
        if reply.request() != self.request.get() {
            return Err(invalid());
        }
        if let Message::Failed {
            code,
            message: diagnostic,
            ..
        } = reply
        {
            // Only staging checkpoints use cooperative cancellation. Returning
            // Interrupted to a generic source reader can make it retry forever.
            let checkpoint = matches!(
                message,
                Message::StageBegin {
                    purpose: StagePurpose::Member { .. },
                    ..
                } | Message::StageAppend { .. }
                    | Message::StageProgress { .. }
                    | Message::StageSeal { .. }
            );
            return Err(io::Error::new(
                if code == api::ErrorCode::Cancelled && checkpoint {
                    io::ErrorKind::Interrupted
                } else {
                    io::ErrorKind::Other
                },
                diagnostic,
            ));
        }
        Ok(reply)
    }
    pub fn source(&self, length: u64) -> Box<dyn ReadSeek> {
        Box::new(Source {
            broker: self.clone(),
            length,
            position: 0,
        })
    }
}
impl Storage for Broker {
    fn progress(&self, stage: u64, completed: u64) -> io::Result<()> {
        match self.exchange(Message::StageProgress {
            request: self.request.get(),
            stage,
            completed,
        })? {
            Message::StageProgressAck {
                stage: reply,
                completed: actual,
                ..
            } if reply == stage && actual == completed => Ok(()),
            _ => Err(invalid()),
        }
    }
    fn begin(&self, purpose: StagePurpose, expected: Option<u64>, maximum: u64) -> io::Result<u64> {
        match self.exchange(Message::StageBegin {
            request: self.request.get(),
            purpose,
            expected_length: expected,
            max_length: maximum,
        })? {
            Message::StageCreated { stage, .. } => Ok(stage),
            _ => Err(invalid()),
        }
    }
    fn append(&self, stage: u64, offset: u64, bytes: &[u8]) -> io::Result<()> {
        let expected = offset.checked_add(bytes.len() as u64).ok_or_else(invalid)?;
        match self.exchange(Message::StageAppend {
            request: self.request.get(),
            stage,
            offset,
            bytes: bytes.to_vec(),
        })? {
            Message::StageAck {
                stage: reply,
                next_offset,
                ..
            } if reply == stage && next_offset == expected => Ok(()),
            _ => Err(invalid()),
        }
    }
    fn seal(&self, stage: u64, length: u64) -> io::Result<()> {
        match self.exchange(Message::StageSeal {
            request: self.request.get(),
            stage,
            length,
        })? {
            Message::StageSealed {
                stage: reply,
                length: size,
                ..
            } if reply == stage && size == length => Ok(()),
            _ => Err(invalid()),
        }
    }
    fn read(&self, stage: u64, offset: u64, length: u32) -> io::Result<Vec<u8>> {
        match self.exchange(Message::SpoolRead {
            request: self.request.get(),
            stage,
            offset,
            length,
        })? {
            Message::SpoolData {
                stage: reply,
                bytes,
                ..
            } if reply == stage && bytes.len() == length as usize => Ok(bytes),
            _ => Err(invalid()),
        }
    }
}
struct Source {
    broker: Broker,
    length: u64,
    position: u64,
}
impl Read for Source {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let count = (self.length - self.position)
            .min(output.len() as u64)
            .min(u64::from(MAX_READ)) as u32;
        if count == 0 {
            return Ok(0);
        }
        match self.broker.exchange(Message::SourceRead {
            request: self.broker.request.get(),
            offset: self.position,
            length: count,
        })? {
            Message::SourceData { bytes, .. } if bytes.len() == count as usize => {
                output[..bytes.len()].copy_from_slice(&bytes);
                self.position += bytes.len() as u64;
                Ok(bytes.len())
            }
            _ => Err(invalid()),
        }
    }
}
impl Seek for Source {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.position = seek_position(self.position, self.length, from)?;
        Ok(self.position)
    }
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Invalid archive broker response",
    )
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    struct Reply;
    impl Transport for Reply {
        fn exchange(&mut self, message: &Message) -> io::Result<Message> {
            Ok(Message::Failed {
                request: message.request(),
                code: api::ErrorCode::Cancelled,
                message: "canceled".into(),
            })
        }
        fn receive(&mut self) -> io::Result<Message> {
            unreachable!()
        }
        fn send(&mut self, _: &Message) -> io::Result<()> {
            unreachable!()
        }
    }
    fn broker() -> Broker {
        Broker {
            transport: Rc::new(RefCell::new(Box::new(Reply))),
            request: Rc::new(Cell::new(7)),
        }
    }
    #[test]
    fn cooperative_staging_failure_reaches_format_as_cancellation() {
        let broker = broker();
        for result in [
            broker
                .begin(StagePurpose::Member { entry: 2 }, Some(4), 4)
                .map(|_| ()),
            broker.append(1, 0, b"data"),
            broker.progress(1, 4),
            broker.seal(1, 4),
        ] {
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
        }
    }
    #[test]
    fn cooperative_cancel_is_not_a_retryable_source_interruption() {
        let mut source = broker().source(4);
        let error = source.read(&mut [0; 4]).unwrap_err();
        assert_ne!(error.kind(), io::ErrorKind::Interrupted);
    }
}

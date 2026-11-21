//! Media Foundation parses and decodes on its own work-queue threads, while the brokered source
//! belongs to the helper's one pipe thread. Decoding runs on a dedicated thread; every byte it
//! needs travels back to the pipe thread as a positioned read request, which that thread serves
//! while it waits for the decoder's answer. No source object crosses a thread.
use super::{Answer, Command};
use midnight_thumbnail_sdk::ReadSeek;
use std::{
    io::{self, SeekFrom},
    sync::mpsc,
    thread,
};

/// Largest single read served for Media Foundation. Container samples are far smaller; a
/// larger request is answered short instead of allocating what a damaged index claims.
const MAX_READ: usize = 32 * 1024 * 1024;

pub(super) enum Event {
    Read(ReadRequest),
    Answer(io::Result<Answer>),
}

pub(super) struct ReadRequest {
    offset: u64,
    length: usize,
    reply: mpsc::Sender<io::Result<Vec<u8>>>,
}

/// The decoder thread's handle for positioned source reads. It can be cloned and used from any
/// Media Foundation thread; each read blocks until the pipe thread serves it.
#[derive(Clone)]
pub(super) struct Reads {
    events: mpsc::Sender<Event>,
    length: u64,
}

impl Reads {
    pub(super) fn length(&self) -> u64 {
        self.length
    }

    /// Up to `length` bytes at `offset`; fewer only at the end of the capture.
    pub(super) fn read_at(&self, offset: u64, length: usize) -> io::Result<Vec<u8>> {
        let (reply, answer) = mpsc::channel();
        self.events
            .send(Event::Read(ReadRequest {
                offset,
                length: length.min(MAX_READ),
                reply,
            }))
            .map_err(|_| stopped())?;
        answer.recv().map_err(|_| stopped())?
    }
}

/// Commands for the decoder thread and the events it sends back.
pub(super) struct Remote {
    commands: Option<mpsc::Sender<Command>>,
    events: mpsc::Receiver<Event>,
}

impl Remote {
    /// Start `decode` on its own thread. It receives the commands and must answer each with
    /// exactly one `Event::Answer` through `Replies`.
    pub(super) fn spawn(
        length: u64,
        decode: impl FnOnce(Reads, mpsc::Receiver<Command>, Replies) + Send + 'static,
    ) -> io::Result<Self> {
        let (commands, received) = mpsc::channel();
        let (events, receiver) = mpsc::channel();
        let reads = Reads {
            events: events.clone(),
            length,
        };
        thread::Builder::new()
            .name("media-foundation".into())
            .spawn(move || decode(reads, received, Replies(events)))?;
        Ok(Self {
            commands: Some(commands),
            events: receiver,
        })
    }

    /// Send one command and serve the decoder's reads from `source` until its answer arrives.
    /// The thread is detached: after the helper's last answer, the process exits with it.
    pub(super) fn call(
        &mut self,
        source: &mut dyn ReadSeek,
        command: Command,
    ) -> io::Result<Answer> {
        self.commands
            .as_ref()
            .ok_or_else(stopped)?
            .send(command)
            .map_err(|_| stopped())?;
        loop {
            match self.events.recv().map_err(|_| stopped())? {
                Event::Read(request) => {
                    let result = read_at(source, request.offset, request.length);
                    // A decoder that gave up on its read no longer listens.
                    let _ = request.reply.send(result);
                }
                Event::Answer(answer) => return answer,
            }
        }
    }
}

impl Drop for Remote {
    fn drop(&mut self) {
        // Closing the command channel ends the decoder's loop. Reads that it still issues fail,
        // because the event receiver is dropped with this owner.
        self.commands = None;
    }
}

/// The decoder thread's answer channel.
pub(super) struct Replies(mpsc::Sender<Event>);

impl Replies {
    pub(super) fn send(&self, answer: io::Result<Answer>) -> bool {
        self.0.send(Event::Answer(answer)).is_ok()
    }
}

fn read_at(source: &mut dyn ReadSeek, offset: u64, length: usize) -> io::Result<Vec<u8>> {
    source.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![0; length];
    let mut filled = 0;
    while filled < length {
        // A broker refusal, Interrupted included, is final: the pipe latches it.
        match source.read(&mut bytes[filled..])? {
            0 => break,
            count => filled += count,
        }
    }
    bytes.truncate(filled);
    Ok(bytes)
}

fn stopped() -> io::Error {
    io::Error::other("The Media Foundation decoder thread stopped")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read, Seek};

    #[test]
    fn reads_from_any_thread_are_served_in_order_by_the_owner_of_the_source() {
        let bytes: Vec<u8> = (0..=255).collect();
        let mut remote = Remote::spawn(256, |reads, commands, replies| {
            while let Ok(command) = commands.recv() {
                let Command::Close = command else {
                    unreachable!("the test sends only Close");
                };
                let worker = reads.clone();
                let tail = thread::spawn(move || worker.read_at(250, 64))
                    .join()
                    .unwrap();
                let head = reads.read_at(0, 3);
                let answer = match (head, tail) {
                    (Ok(head), Ok(tail))
                        if head == [0, 1, 2] && tail == [250, 251, 252, 253, 254, 255] =>
                    {
                        Ok(Answer::Closed)
                    }
                    other => Err(io::Error::other(format!("unexpected reads {other:?}"))),
                };
                replies.send(answer);
            }
        })
        .unwrap();
        let mut source = Cursor::new(bytes);
        assert!(matches!(
            remote.call(&mut source, Command::Close).unwrap(),
            Answer::Closed
        ));
    }

    #[test]
    fn a_failed_source_read_reaches_the_decoder_and_a_stopped_decoder_is_an_error() {
        struct Failing;
        impl Read for Failing {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"))
            }
        }
        impl Seek for Failing {
            fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
                Ok(0)
            }
        }
        let mut remote = Remote::spawn(8, |reads, commands, replies| {
            if commands.recv().is_ok() {
                let failure = reads.read_at(0, 8).unwrap_err();
                replies.send(Err(failure));
            }
        })
        .unwrap();
        let error = remote.call(&mut Failing, Command::Close).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        // The decoder returned after one command; the next call finds it gone.
        assert!(remote.call(&mut Failing, Command::Close).is_err());
    }
}

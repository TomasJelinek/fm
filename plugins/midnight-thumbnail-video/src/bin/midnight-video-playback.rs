//! Locked-down video playback worker. Native source access stays in its host.
#![forbid(unsafe_code)]

use midnight_thumbnail_video::{
    playback::{self, Cursor, OutputBounds},
    playback_audio::{self, AudioTrack, PcmCursor},
    playback_session::{self as session, Event, Failure, Request, Transport},
    playback_wire::{self, Incoming, MAX_READ_BYTES, Outgoing},
};
use std::{
    cell::{RefCell, RefMut},
    io::{self, Read, Seek, SeekFrom, Write},
    num::NonZeroU64,
    rc::Rc,
};

struct Pipe<R, W> {
    input: R,
    output: W,
    broken: bool,
    source_fault: Option<io::ErrorKind>,
    next_read: NonZeroU64,
}

impl<R: Read, W: Write> Pipe<R, W> {
    fn receive(&mut self) -> io::Result<Option<Incoming>> {
        self.ensure_live()?;
        let result = playback_wire::read_message(&mut self.input);
        self.broken = result.is_err();
        result
    }

    fn send(&mut self, message: Outgoing<'_>) -> io::Result<()> {
        self.ensure_live()?;
        let result = playback_wire::write_message(&mut self.output, message);
        self.broken = result.is_err();
        result
    }

    fn ensure_live(&self) -> io::Result<()> {
        if self.broken {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        Ok(())
    }

    fn check_source(&self) -> io::Result<()> {
        self.ensure_live()?;
        self.source_fault.map_or(Ok(()), |kind| Err(kind.into()))
    }

    fn failure_code(&self, error: &io::Error) -> u8 {
        if self.source_fault.is_some() {
            return 1;
        }
        match error.kind() {
            io::ErrorKind::PermissionDenied
            | io::ErrorKind::Interrupted
            | io::ErrorKind::UnexpectedEof => 1,
            io::ErrorKind::Unsupported => 2,
            io::ErrorKind::InvalidData | io::ErrorKind::InvalidInput => 3,
            _ => 4,
        }
    }

    fn reject(&mut self, detail: &'static str) -> io::Error {
        self.broken = true;
        io::Error::new(io::ErrorKind::InvalidData, detail)
    }
}

// FFmpeg's AVIO callbacks and the command loop use this pipe on one worker
// thread. No borrow extends across a call into the decoder.
type SharedPipe<R, W> = Rc<RefCell<Pipe<R, W>>>;

fn borrow_pipe<R, W>(pipe: &SharedPipe<R, W>) -> io::Result<RefMut<'_, Pipe<R, W>>> {
    pipe.try_borrow_mut()
        .map_err(|_| io::Error::other("Playback transport call overlapped another call"))
}

struct BrokeredSource<R, W> {
    pipe: SharedPipe<R, W>,
    length: u64,
    position: u64,
}

impl<R: Read, W: Write> Read for BrokeredSource<R, W> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = (self.length - self.position)
            .min(buffer.len() as u64)
            .min(MAX_READ_BYTES as u64) as usize;
        if count == 0 {
            return Ok(0);
        }
        let mut pipe = borrow_pipe(&self.pipe)?;
        pipe.check_source()?;
        let request = pipe.next_read;
        pipe.next_read = request
            .get()
            .checked_add(1)
            .and_then(NonZeroU64::new)
            .ok_or_else(|| io::Error::other("Playback read identities exhausted"))?;
        pipe.send(Outgoing::Read {
            read_request: request,
            offset: self.position,
            length: count as u32,
        })?;
        let result = match pipe.receive()? {
            Some(Incoming::ReadReply {
                read_request,
                result,
            }) if read_request == request => result,
            _ => return Err(pipe.reject("Playback read reply did not match its request")),
        };
        let bytes = result.inspect_err(|error| pipe.source_fault = Some(error.kind()))?;
        if bytes.len() > count {
            return Err(pipe.reject("Playback read reply exceeds the requested range"));
        }
        buffer[..bytes.len()].copy_from_slice(&bytes);
        self.position += bytes.len() as u64;
        Ok(bytes.len())
    }
}

impl<R, W> Seek for BrokeredSource<R, W> {
    fn seek(&mut self, origin: SeekFrom) -> io::Result<u64> {
        let position = match origin {
            SeekFrom::Start(position) => i128::from(position),
            SeekFrom::End(delta) => i128::from(self.length) + i128::from(delta),
            SeekFrom::Current(delta) => i128::from(self.position) + i128::from(delta),
        };
        let position = u64::try_from(position)
            .ok()
            .filter(|position| *position <= self.length)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "Seek outside captured video")
            })?;
        self.position = position;
        Ok(position)
    }
}

struct SessionTransport<R, W>(SharedPipe<R, W>);

impl<R: Read, W: Write> Transport for SessionTransport<R, W> {
    fn receive(&mut self) -> io::Result<Option<Request>> {
        let mut pipe = borrow_pipe(&self.0)?;
        match pipe.receive()? {
            None => Ok(None),
            Some(Incoming::Session(command)) => Ok(Some(command)),
            _ => Err(pipe.reject("Expected playback demand, seek or close")),
        }
    }

    fn emit(&mut self, event: Event) -> io::Result<()> {
        let mut pipe = borrow_pipe(&self.0)?;
        pipe.check_source()?;
        pipe.send(Outgoing::Event(&event))
    }
}

fn serve(input: impl Read, output: impl Write) -> io::Result<()> {
    let mut pipe = Pipe {
        input,
        output,
        broken: false,
        source_fault: None,
        next_read: NonZeroU64::MIN,
    };
    let Some(Incoming::Open {
        source_length,
        generation,
        width,
        height,
    }) = pipe.receive()?
    else {
        return Err(pipe.reject("Expected one playback source to open"));
    };
    let pipe = Rc::new(RefCell::new(pipe));
    let mut source = BrokeredSource {
        pipe: pipe.clone(),
        length: source_length,
        position: 0,
    };
    let mut started_session = false;
    let result = playback::with_cursor(
        &mut source,
        source_length,
        OutputBounds { width, height },
        |cursor| {
            let track = cursor.audio_track();
            if matches!(track, AudioTrack::Qualified) {
                let mut audio_source = BrokeredSource {
                    pipe: pipe.clone(),
                    length: source_length,
                    position: 0,
                };
                playback_audio::with_pcm(
                    &mut audio_source,
                    source_length,
                    cursor.metadata().duration_micros,
                    |audio| {
                        run_session(
                            &pipe,
                            cursor,
                            Some(audio),
                            generation,
                            track,
                            &mut started_session,
                        )
                    },
                )
            } else {
                run_session(&pipe, cursor, None, generation, track, &mut started_session)
            }
        },
    );
    match result {
        Err(cause) if !started_session => {
            let mut pipe = borrow_pipe(&pipe)?;
            if pipe.broken {
                return Err(cause);
            }
            let code = pipe.failure_code(&cause);
            pipe.send(Outgoing::Failed {
                request: 0,
                generation: generation.get(),
                cause: code,
            })
        }
        other => other,
    }
}

fn run_session<R: Read, W: Write>(
    pipe: &SharedPipe<R, W>,
    cursor: &mut Cursor<'_>,
    audio: Option<&mut PcmCursor<'_>>,
    generation: NonZeroU64,
    track: AudioTrack,
    started: &mut bool,
) -> io::Result<()> {
    let metadata = cursor.metadata();
    {
        let mut pipe = borrow_pipe(pipe)?;
        pipe.check_source()?;
        pipe.send(Outgoing::Opened {
            generation,
            duration_micros: metadata.duration_micros,
            width: metadata.width,
            height: metadata.height,
            audio: track,
        })?;
    }
    *started = true;
    match session::run(
        cursor,
        audio,
        generation,
        &mut SessionTransport(pipe.clone()),
    ) {
        Ok(()) => Ok(()),
        Err(Failure::Decoder {
            request,
            generation,
            cause,
        }) => {
            let mut pipe = borrow_pipe(pipe)?;
            if pipe.broken {
                return Err(cause);
            }
            let code = pipe.failure_code(&cause);
            pipe.send(Outgoing::Failed {
                request: request.get(),
                generation: generation.get(),
                cause: code,
            })
        }
        Err(Failure::Transport(cause)) => Err(cause),
        Err(Failure::Protocol(detail)) => Err(io::Error::new(io::ErrorKind::InvalidData, detail)),
    }
}

fn main() -> io::Result<()> {
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    midnight_worker_sdk::enter(&mut input, &mut output)?;
    serve(input, output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(id: u64, bytes: &[u8]) -> Vec<u8> {
        let mut reply = b"MMV2\x05\0\0\0".to_vec();
        reply.extend_from_slice(&((9 + bytes.len()) as u32).to_le_bytes());
        reply.extend_from_slice(&id.to_le_bytes());
        reply.push(0);
        reply.extend_from_slice(bytes);
        reply
    }

    fn capture(bytes: Vec<u8>, length: u64) -> BrokeredSource<io::Cursor<Vec<u8>>, Vec<u8>> {
        BrokeredSource {
            pipe: Rc::new(RefCell::new(Pipe {
                input: io::Cursor::new(bytes),
                output: Vec::new(),
                broken: false,
                source_fault: None,
                next_read: NonZeroU64::MIN,
            })),
            length,
            position: 0,
        }
    }

    #[test]
    fn broker_reads_only_within_the_capture_and_uses_an_exact_request() {
        let mut source = capture(reply(1, &[4, 5, 6]), 5);
        assert_eq!(source.seek(SeekFrom::Start(2)).unwrap(), 2);
        let mut buffer = [0u8; 8];
        assert_eq!(source.read(&mut buffer).unwrap(), 3);
        assert_eq!(&buffer[..3], &[4, 5, 6]);
        assert_eq!(source.read(&mut buffer).unwrap(), 0);
        let pipe = source.pipe.borrow();
        let mut expected = b"MMV2\x82\0\0\0".to_vec();
        expected.extend_from_slice(&20u32.to_le_bytes());
        expected.extend_from_slice(&1u64.to_le_bytes());
        expected.extend_from_slice(&2u64.to_le_bytes());
        expected.extend_from_slice(&3u32.to_le_bytes());
        assert_eq!(pipe.output, expected);
    }

    #[test]
    fn wrong_read_reply_retires_the_pipe_and_cannot_copy_or_retry() {
        let mut source = capture(reply(2, &[4, 5, 6]), 3);
        let mut buffer = [0u8; 3];
        assert_eq!(
            source.read(&mut buffer).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(buffer, [0; 3]);
        assert_eq!(source.position, 0);
        let sent = source.pipe.borrow().output.len();
        assert_eq!(
            source.read(&mut buffer).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(source.pipe.borrow().output.len(), sent);
    }

    #[test]
    fn broker_denial_latches_without_poisoning_the_terminal_reply() {
        for (status, kind) in [
            (1, io::ErrorKind::PermissionDenied),
            (2, io::ErrorKind::Interrupted),
            (3, io::ErrorKind::Other),
        ] {
            let mut denied = reply(1, &[]);
            *denied.last_mut().unwrap() = status;
            denied.extend_from_slice(&reply(2, &[4, 5, 6]));
            let mut source = capture(denied, 3);
            let mut buffer = [0; 3];
            for _ in 0..2 {
                assert_eq!(source.read(&mut buffer).unwrap_err().kind(), kind);
            }
            assert_eq!(buffer, [0; 3]);
            assert_eq!(source.position, 0);
            let pipe = source.pipe.borrow();
            assert!(!pipe.broken);
            assert_eq!(pipe.output.len(), 12 + 20);
            assert_eq!(pipe.failure_code(&io::Error::other("native I/O error")), 1);
        }
    }

    #[test]
    fn invalid_seeks_preserve_the_captured_position() {
        let mut source = capture(Vec::new(), 12);
        assert_eq!(source.seek(SeekFrom::End(-2)).unwrap(), 10);
        for invalid in [
            SeekFrom::Current(-11),
            SeekFrom::End(1),
            SeekFrom::Start(u64::MAX),
        ] {
            assert_eq!(
                source.seek(invalid).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
            assert_eq!(source.position, 10);
        }
        assert!(source.pipe.borrow().output.is_empty());
    }
}

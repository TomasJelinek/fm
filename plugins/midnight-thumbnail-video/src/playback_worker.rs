//! The MMV2 playback helper loop shared by the bundled and the platform helpers. Native source
//! access stays in the host: every read is a brokered request on the helper's one pipe.
pub use crate::playback_session::PlaybackDecoder;
use crate::{
    media::{NoDecoder, OutputBounds},
    playback_session::{self as session, Event, Failure, Request, Transport},
    playback_wire::{self, Incoming, MAX_READ_BYTES, Outgoing},
};
use midnight_thumbnail_sdk::ReadSeek;
use std::{
    cell::{RefCell, RefMut},
    io::{self, Read, Seek, SeekFrom, Write},
    num::NonZeroU64,
    rc::Rc,
};

/// Independent read cursors over the one captured source. Every cursor shares the host broker
/// and its single ordered read stream, so use them only on the thread that serves the session.
pub trait Sources {
    /// Captured length in bytes.
    fn length(&self) -> u64;
    /// A new cursor at offset zero.
    fn cursor(&self) -> Box<dyn ReadSeek + '_>;
}

/// Opens the decoder for one session: an error returned before `session` runs is reported as
/// the open failure (`NoDecoder` as cause 5); after that, `session`'s result is returned.
pub type Session<'a> = &'a mut dyn FnMut(&mut dyn PlaybackDecoder) -> io::Result<()>;

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

    /// The private MMV2 classification of a decoder failure: cause, codec and tried bytes.
    fn failure(&self, error: &io::Error) -> (u8, u8, u8) {
        if self.source_fault.is_some() {
            return (1, 0, 0);
        }
        if let Some(missing) = NoDecoder::find(error) {
            return (5, missing.codec as u8, missing.tried.0);
        }
        let cause = match error.kind() {
            io::ErrorKind::PermissionDenied
            | io::ErrorKind::Interrupted
            | io::ErrorKind::UnexpectedEof => 1,
            io::ErrorKind::Unsupported => 2,
            io::ErrorKind::InvalidData | io::ErrorKind::InvalidInput => 3,
            _ => 4,
        };
        (cause, 0, 0)
    }

    fn fail(&mut self, request: u64, generation: u64, cause: &io::Error) -> io::Result<()> {
        let (cause, codec, tried) = self.failure(cause);
        self.send(Outgoing::Failed {
            request,
            generation,
            cause,
            codec,
            tried,
        })
    }

    fn reject(&mut self, detail: &'static str) -> io::Error {
        self.broken = true;
        io::Error::new(io::ErrorKind::InvalidData, detail)
    }
}

// The decoder's reads and the command loop use this pipe on one worker thread. No borrow
// extends across a call into the decoder.
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

struct PipeSources<R, W> {
    pipe: SharedPipe<R, W>,
    length: u64,
}

impl<R: Read, W: Write> Sources for PipeSources<R, W> {
    fn length(&self) -> u64 {
        self.length
    }

    fn cursor(&self) -> Box<dyn ReadSeek + '_> {
        Box::new(BrokeredSource {
            pipe: self.pipe.clone(),
            length: self.length,
            position: 0,
        })
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

/// Serve one MMV2 session after the worker entered its sandbox: read the Open request, let
/// `open` start a decoder over the brokered capture, then answer demand until Close or EOF.
pub fn serve(
    input: impl Read,
    output: impl Write,
    open: impl FnOnce(&dyn Sources, OutputBounds, Session<'_>) -> io::Result<()>,
) -> io::Result<()> {
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
    let sources = PipeSources {
        pipe: pipe.clone(),
        length: source_length,
    };
    let mut started = false;
    let result = {
        let mut session = |decoder: &mut dyn PlaybackDecoder| {
            run_session(&pipe, decoder, generation, &mut started)
        };
        open(&sources, OutputBounds { width, height }, &mut session)
    };
    match result {
        Err(cause) if !started => {
            let mut pipe = borrow_pipe(&pipe)?;
            if pipe.broken {
                return Err(cause);
            }
            pipe.fail(0, generation.get(), &cause)
        }
        other => other,
    }
}

fn run_session<R: Read, W: Write>(
    pipe: &SharedPipe<R, W>,
    decoder: &mut dyn PlaybackDecoder,
    generation: NonZeroU64,
    started: &mut bool,
) -> io::Result<()> {
    let metadata = decoder.metadata();
    {
        let mut pipe = borrow_pipe(pipe)?;
        pipe.check_source()?;
        pipe.send(Outgoing::Opened {
            generation,
            duration_micros: metadata.duration_micros,
            width: metadata.width,
            height: metadata.height,
            audio: decoder.audio_track(),
        })?;
    }
    *started = true;
    match session::run(decoder, generation, &mut SessionTransport(pipe.clone())) {
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
            pipe.fail(request.get(), generation.get(), &cause)
        }
        Err(Failure::Transport(cause)) => Err(cause),
        Err(Failure::Protocol(detail)) => Err(io::Error::new(io::ErrorKind::InvalidData, detail)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::{AudioTrack, Block, Codec, Frame, Metadata, Tried};

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
            assert_eq!(
                pipe.failure(&io::Error::other("native I/O error")),
                (1, 0, 0)
            );
            // A source fault outranks a missing decoder.
            let missing = io::Error::from(NoDecoder {
                codec: Codec::Hevc,
                tried: Tried::BUNDLED,
            });
            assert_eq!(pipe.failure(&missing), (1, 0, 0));
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

    fn open_request(length: u64) -> Vec<u8> {
        let mut open = b"MMV2\x01\0\0\0".to_vec();
        open.extend_from_slice(&24u32.to_le_bytes());
        open.extend_from_slice(&length.to_le_bytes());
        open.extend_from_slice(&9u64.to_le_bytes());
        open.extend_from_slice(&32u32.to_le_bytes());
        open.extend_from_slice(&32u32.to_le_bytes());
        open
    }

    fn failed(cause: u8, codec: u8, tried: u8) -> Vec<u8> {
        let mut failed = b"MMV2\x85\0\0\0".to_vec();
        failed.extend_from_slice(&19u32.to_le_bytes());
        failed.extend_from_slice(&0u64.to_le_bytes());
        failed.extend_from_slice(&9u64.to_le_bytes());
        failed.extend_from_slice(&[cause, codec, tried]);
        failed
    }

    #[test]
    fn an_open_failure_reports_its_class_and_a_missing_decoder_names_its_codec() {
        for (error, expected) in [
            (
                io::Error::from(NoDecoder {
                    codec: Codec::Hevc,
                    tried: Tried::BUNDLED,
                }),
                failed(5, 2, 1),
            ),
            (
                io::Error::new(io::ErrorKind::Unsupported, "unsupported"),
                failed(2, 0, 0),
            ),
            (
                io::Error::new(io::ErrorKind::InvalidData, "bad"),
                failed(3, 0, 0),
            ),
            (io::Error::other("other"), failed(4, 0, 0)),
        ] {
            let mut output = Vec::new();
            let mut error = Some(error);
            serve(
                io::Cursor::new(open_request(12)),
                &mut output,
                |sources, bounds, _| {
                    assert_eq!(sources.length(), 12);
                    assert_eq!((bounds.width, bounds.height), (32, 32));
                    Err(error.take().unwrap())
                },
            )
            .unwrap();
            assert_eq!(output, expected);
        }
    }

    struct Still;

    impl PlaybackDecoder for Still {
        fn metadata(&self) -> Metadata {
            Metadata {
                duration_micros: Some(40_000),
                width: 2,
                height: 2,
            }
        }

        fn audio_track(&self) -> AudioTrack {
            AudioTrack::Unqualified
        }

        fn next_frame(&mut self) -> io::Result<Option<Frame>> {
            Ok(None)
        }

        fn seek(&mut self, _: u64) -> io::Result<()> {
            Ok(())
        }

        fn pcm(&mut self, _: u16) -> io::Result<Option<Block>> {
            Ok(None)
        }
    }

    #[test]
    fn an_opened_decoder_is_announced_and_a_later_error_is_not_an_open_failure() {
        let mut output = Vec::new();
        let result = serve(
            io::Cursor::new(open_request(12)),
            &mut output,
            |_, _, session| {
                session(&mut Still)?;
                Err(io::Error::other("after the session"))
            },
        );
        assert_eq!(result.unwrap_err().to_string(), "after the session");
        let mut opened = b"MMV2\x81\0\0\0".to_vec();
        opened.extend_from_slice(&25u32.to_le_bytes());
        opened.extend_from_slice(&9u64.to_le_bytes());
        opened.extend_from_slice(&40_000u64.to_le_bytes());
        opened.extend_from_slice(&2u32.to_le_bytes());
        opened.extend_from_slice(&2u32.to_le_bytes());
        opened.push(2);
        assert_eq!(output, opened);
    }
}

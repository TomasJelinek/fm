//! Media Foundation decoding for the Windows platform helpers. Media Foundation reads only a
//! byte stream over the host-brokered capture (no URL, path or network source), decodes in
//! software (no DXVA, no hardware transforms, so no GPU driver code loads) and runs in the same
//! AppContainer and job as the bundled helpers. Before any content is read, the helper accepts
//! only Microsoft-signed images, so a codec pack's decoder cannot load into it; Microsoft's own
//! Store codec extensions (HEVC, AV1, VP9, MPEG-2) are Microsoft-signed and still load.
mod audio;
mod convert;
mod remote;
mod stream;
mod video;

use crate::{
    media::{
        AudioTrack, Block, Codec, Frame, Metadata, NoDecoder, OutputBounds, Tried, output_length,
    },
    playback_worker::{PlaybackDecoder, Session, Sources},
};
use midnight_thumbnail_sdk::{ReadSeek, api};
use remote::{Reads, Remote, Replies};
use std::{io, sync::mpsc};
use windows::{
    Win32::{
        Media::MediaFoundation::{
            IMFAttributes, IMFSourceReader, MF_SOURCE_READER_DISABLE_DXVA, MF_VERSION,
            MFCreateAttributes, MFCreateSourceReaderFromByteStream, MFSTARTUP_LITE, MFShutdown,
            MFStartup,
        },
        System::{
            Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize},
            SystemServices::PROCESS_MITIGATION_BINARY_SIGNATURE_POLICY,
            Threading::{ProcessSignaturePolicy, SetProcessMitigationPolicy},
        },
    },
    core::HRESULT,
};

/// `PROCESS_MITIGATION_BINARY_SIGNATURE_POLICY.MicrosoftSignedOnly`: from now on only images
/// signed by Microsoft load. Windows components and Microsoft's Store codec extensions qualify;
/// a third party's decoder or byte-stream handler registered system-wide does not. (A process
/// that is not a Store app cannot switch on `StoreSignedOnly` at run time, and Windows refuses
/// both flags together.)
const MICROSOFT_SIGNED_ONLY: u32 = 1;

/// The thumbnail helper: one MTH1 request per process.
pub fn thumbnail_main() -> io::Result<()> {
    restrict_images()?;
    midnight_thumbnail_sdk::serve(thumbnail)
}

/// The playback helper: one MMV2 session per process.
#[cfg(feature = "playback-worker")]
pub fn playback_main() -> io::Result<()> {
    restrict_images()?;
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    midnight_worker_sdk::enter(&mut input, &mut output)?;
    crate::playback_worker::serve(input, output, open)
}

/// From here on the process loads only Microsoft-signed images. The helper's own executable
/// and its runtime are already loaded; every decoder, source and converter that Media
/// Foundation loads later must pass the check.
pub fn restrict_images() -> io::Result<()> {
    let mut policy = PROCESS_MITIGATION_BINARY_SIGNATURE_POLICY::default();
    policy.Anonymous.Flags = MICROSOFT_SIGNED_ONLY;
    // SAFETY: a fully initialized policy structure of the size passed.
    unsafe {
        SetProcessMitigationPolicy(
            ProcessSignaturePolicy,
            (&raw const policy).cast(),
            size_of_val(&policy),
        )
    }
    .map_err(|error| {
        io::Error::other(format!(
            "Cannot restrict the video helper to signed images ({:#010x})",
            error.code().0
        ))
    })
}

/// One thumbnail at 10% to 85% of the duration, like the bundled profile.
pub fn thumbnail(request: &api::Request, source: &mut dyn ReadSeek) -> io::Result<api::Frame> {
    api::rgba_length(request.width, request.height)?;
    if request.frame >= api::MAX_ANIMATION_FRAMES || request.source_length > i64::MAX as u64 {
        return Err(io::Error::other("Invalid video request"));
    }
    let mut remote = Remote::spawn(request.source_length, decode)?;
    let answer = remote.call(
        source,
        Command::Thumbnail {
            output: OutputBounds {
                width: request.width,
                height: request.height,
            },
            frame: request.frame,
        },
    );
    // Release the readers while their reads can still be served.
    let _ = remote.call(source, Command::Close);
    let Answer::Thumbnail(frame) = answer? else {
        return Err(unexpected());
    };
    Ok(api::Frame {
        id: request.id,
        frame: request.frame,
        width: frame.width,
        height: frame.height,
        rgba: frame.rgba,
    })
}

/// Open one playback session over `sources`.
pub fn open(sources: &dyn Sources, output: OutputBounds, session: Session<'_>) -> io::Result<()> {
    output_length(output.width, output.height)?;
    if sources.length() > i64::MAX as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Video source length exceeds the seek range",
        ));
    }
    let mut cursor = sources.cursor();
    let mut remote = Remote::spawn(sources.length(), decode)?;
    let opened = remote.call(&mut *cursor, Command::Open { output });
    let (metadata, audio) = match opened {
        Ok(Answer::Opened { metadata, audio }) => (metadata, audio),
        Ok(_) => return Err(unexpected()),
        Err(error) => {
            let _ = remote.call(&mut *cursor, Command::Close);
            return Err(error);
        }
    };
    let mut playback = Playback {
        remote,
        cursor,
        metadata,
        audio,
    };
    let result = session(&mut playback);
    let _ = playback.remote.call(&mut *playback.cursor, Command::Close);
    result
}

#[derive(Debug)]
pub(super) enum Command {
    Open { output: OutputBounds },
    Next,
    Seek(u64),
    Pcm(u16),
    Thumbnail { output: OutputBounds, frame: u8 },
    Close,
}

#[derive(Debug)]
pub(super) enum Answer {
    Opened {
        metadata: Metadata,
        audio: AudioTrack,
    },
    Frame(Option<Frame>),
    Seeked,
    Pcm(Option<Block>),
    Thumbnail(Frame),
    Closed,
}

/// The pipe thread's view of a session that decodes on the Media Foundation thread.
struct Playback<'a> {
    remote: Remote,
    cursor: Box<dyn ReadSeek + 'a>,
    metadata: Metadata,
    audio: AudioTrack,
}

impl Playback<'_> {
    fn call(&mut self, command: Command) -> io::Result<Answer> {
        self.remote.call(&mut *self.cursor, command)
    }
}

impl PlaybackDecoder for Playback<'_> {
    fn metadata(&self) -> Metadata {
        self.metadata
    }

    fn audio_track(&self) -> AudioTrack {
        self.audio
    }

    fn next_frame(&mut self) -> io::Result<Option<Frame>> {
        match self.call(Command::Next)? {
            Answer::Frame(frame) => Ok(frame),
            _ => Err(unexpected()),
        }
    }

    fn seek(&mut self, position_micros: u64) -> io::Result<()> {
        match self.call(Command::Seek(position_micros))? {
            Answer::Seeked => Ok(()),
            _ => Err(unexpected()),
        }
    }

    fn pcm(&mut self, credit: u16) -> io::Result<Option<Block>> {
        if self.audio != AudioTrack::Qualified {
            return Ok(None);
        }
        match self.call(Command::Pcm(credit))? {
            Answer::Pcm(block) => Ok(block),
            _ => Err(unexpected()),
        }
    }
}

/// The Media Foundation thread: COM and Media Foundation live and end here.
fn decode(reads: Reads, commands: mpsc::Receiver<Command>, replies: Replies) {
    let platform = Platform::start();
    let mut session: Option<(video::VideoReader, Option<audio::AudioReader>)> = None;
    while let Ok(command) = commands.recv() {
        let answer = match (&platform, command) {
            (_, Command::Close) => {
                session = None;
                Ok(Answer::Closed)
            }
            (Err(error), _) => Err(io::Error::new(error.kind(), error.to_string())),
            (Ok(_), Command::Open { output }) => open_session(&reads, output).map(|opened| {
                let answer = Answer::Opened {
                    metadata: opened.0.metadata(),
                    audio: opened
                        .1
                        .as_ref()
                        .map_or(AudioTrack::Absent, audio::AudioReader::track),
                };
                session = Some((
                    opened.0,
                    opened
                        .1
                        .filter(|audio| audio.track() == AudioTrack::Qualified),
                ));
                answer
            }),
            (Ok(_), Command::Thumbnail { output, frame }) => {
                video::VideoReader::open(&reads, output)
                    .and_then(|mut reader| reader.thumbnail(frame))
                    .map(Answer::Thumbnail)
            }
            (Ok(_), command) => match (&mut session, command) {
                (Some((video, _)), Command::Next) => video.next().map(Answer::Frame),
                (Some((video, audio)), Command::Seek(position)) => video
                    .seek(position)
                    .and_then(|()| audio.as_mut().map_or(Ok(()), |audio| audio.seek(position)))
                    .map(|()| Answer::Seeked),
                (Some((_, Some(audio))), Command::Pcm(credit)) => {
                    audio.pcm(credit).map(Answer::Pcm)
                }
                (Some((_, None)), Command::Pcm(_)) => Ok(Answer::Pcm(None)),
                _ => Err(unexpected()),
            },
        };
        if !replies.send(answer) {
            break;
        }
    }
    // Readers shut their sources down before Media Foundation stops.
    drop(session);
    drop(platform);
}

fn open_session(
    reads: &Reads,
    output: OutputBounds,
) -> io::Result<(video::VideoReader, Option<audio::AudioReader>)> {
    let video = video::VideoReader::open(reads, output)?;
    // Audio decodes through its own reader, so PCM demand stays independent of the video.
    let audio = audio::AudioReader::open(reads, video.metadata().duration_micros)?;
    Ok((video, audio))
}

/// COM (multithreaded) and Media Foundation without its network components.
struct Platform;

impl Platform {
    fn start() -> io::Result<Self> {
        // SAFETY: once per decoder thread, paired with the uninitialization in Drop.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .ok()
            .map_err(failure)?;
        // SAFETY: COM is initialized on this thread.
        if let Err(error) = unsafe { MFStartup(MF_VERSION, MFSTARTUP_LITE) } {
            // SAFETY: balances the successful initialization above.
            unsafe { CoUninitialize() };
            return Err(failure(error));
        }
        Ok(Self)
    }
}

impl Drop for Platform {
    fn drop(&mut self) {
        // SAFETY: balances start on the same thread after every reader was released.
        unsafe {
            let _ = MFShutdown();
            CoUninitialize();
        }
    }
}

/// A source reader over its own byte stream. Software decoding only: no Direct3D manager and
/// no hardware transforms are given, and DXVA is disabled.
fn source_reader(reads: &Reads) -> io::Result<IMFSourceReader> {
    let stream = stream::Stream::create(reads.clone());
    let mut attributes: Option<IMFAttributes> = None;
    // SAFETY: valid output pointer; the attributes are used only on this thread.
    unsafe { MFCreateAttributes(&mut attributes, 1) }.map_err(failure)?;
    let attributes = attributes.ok_or_else(unexpected)?;
    // SAFETY: a live attribute store and a static key.
    unsafe { attributes.SetUINT32(&MF_SOURCE_READER_DISABLE_DXVA, 1) }.map_err(failure)?;
    // SAFETY: live stream and attributes; the reader keeps its own references.
    unsafe { MFCreateSourceReaderFromByteStream(&stream, &attributes) }.map_err(|error| {
        if unrecognized(error.code()) {
            NoDecoder {
                codec: Codec::Unknown,
                tried: Tried::MEDIA_FOUNDATION,
            }
            .into()
        } else {
            failure(error)
        }
    })
}

/// Media Foundation recognized no container in the bytes.
fn unrecognized(code: HRESULT) -> bool {
    use windows::Win32::Media::MediaFoundation::{
        MF_E_END_OF_STREAM, MF_E_INVALID_FORMAT, MF_E_UNSUPPORTED_BYTESTREAM_TYPE,
        MF_E_UNSUPPORTED_FORMAT,
    };
    // Too few bytes to recognize anything ends the stream before a container is found.
    [
        MF_E_UNSUPPORTED_BYTESTREAM_TYPE,
        MF_E_INVALID_FORMAT,
        MF_E_UNSUPPORTED_FORMAT,
        MF_E_END_OF_STREAM,
    ]
    .contains(&code)
}

/// A Media Foundation error, by its HRESULT. Helpers never send text; this reaches tests and
/// the helper's own diagnostics only.
fn failure(error: windows::core::Error) -> io::Error {
    io::Error::other(format!(
        "Media Foundation failed ({:#010x})",
        error.code().0
    ))
}

fn unexpected() -> io::Error {
    io::Error::other("Unexpected Media Foundation decoder answer")
}

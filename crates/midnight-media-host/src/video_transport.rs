//! Host endpoint for the private MMV2 pipe after the launcher verifies worker readiness.

use crate::{
    playback::{MAX_PCM_BLOCK_FRAMES, PCM_CHANNELS},
    system_decoders::MissingDecoder,
};
use std::{
    io,
    num::{NonZeroU16, NonZeroU64},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

const HEADER_BYTES: usize = 12;
const VIDEO_FIELDS: usize = 32;
const PCM_FIELDS: usize = 26;
/// Request and epoch tags, the cause, and for a missing decoder its codec and what was tried.
const FAILED_FIELDS: usize = 19;
const PCM_BYTES_PER_FRAME: usize = PCM_CHANNELS * 2;
const MAX_READ_BYTES: usize = 256 * 1024;
const MAX_VIDEO_EDGE: u32 = 4_096;
const MAX_VIDEO_PIXELS: u64 = 4 * 1024 * 1024;
const MAX_NATURAL_PIXELS: u64 = 32 * 1024 * 1024;
pub const MAX_VIDEO_PULL_FRAMES: u8 = 4;

const OPEN: u8 = 0x01;
const PULL: u8 = 0x02;
const SEEK: u8 = 0x03;
const CLOSE: u8 = 0x04;
const READ_REPLY: u8 = 0x05;
const OPENED: u8 = 0x81;
const READ: u8 = 0x82;
const VIDEO: u8 = 0x83;
const COMPLETED: u8 = 0x84;
const FAILED: u8 = 0x85;
const PCM: u8 = 0x86;

/// One absolute deadline across all source and pipe calls in a command.
/// Session cancellation revokes the worker. Seek coalescing occurs in the caller.
#[derive(Clone, Copy)]
pub struct VideoIo<'a> {
    pub deadline: Instant,
    pub cancelled: &'a AtomicBool,
}

impl VideoIo<'_> {
    pub fn check(self) -> io::Result<()> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if Instant::now() >= self.deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Reading or decoding the video took too long",
            ));
        }
        Ok(())
    }
}

/// A private pipe capability supplied by the trusted launcher after the worker
/// passes isolation and SDK readiness checks. Keep OS supervision in the launcher.
pub trait RestrictedVideoPipe {
    /// Consume exactly this many bytes under the command deadline and cancellation.
    fn read_exact(&mut self, bytes: &mut [u8], control: VideoIo<'_>) -> io::Result<()>;

    /// Send and flush one message from these borrowed parts under the same deadline.
    fn send(&mut self, parts: &[&[u8]], control: VideoIo<'_>) -> io::Result<()>;

    /// Revoke I/O and transfer the worker to its supervised reap. Keep its slot
    /// charged until the OS confirms retirement. This operation is idempotent.
    fn retire(&mut self);
}

/// A retained, revision-checked readable source. The implementation owns the
/// original native or archive capture and answers only against that capability.
pub trait VideoSource {
    fn length(&self) -> u64;
    fn read_at(&mut self, offset: u64, bytes: &mut [u8], control: VideoIo<'_>)
    -> io::Result<usize>;
}

/// Assigned by the session owner. The child pipe is bound to one such identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionGeneration(pub NonZeroU64);

/// The child generation inside one captured-source session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SeekEpoch(pub NonZeroU64);

impl SeekEpoch {
    pub const fn get(self) -> u64 {
        self.0.get()
    }

    fn next(self) -> io::Result<Self> {
        self.get()
            .checked_add(1)
            .and_then(NonZeroU64::new)
            .map(Self)
            .ok_or_else(|| io::Error::other("Media seek epochs exhausted"))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VideoIdentity {
    pub session: SessionGeneration,
    pub epoch: SeekEpoch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OutputBounds {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioTrack {
    Absent,
    /// Stereo signed 16-bit PCM little endian at 48 kHz.
    Qualified,
    Unqualified,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VideoMetadata {
    pub duration: Option<Duration>,
    pub width: u32,
    pub height: u32,
    pub audio: AudioTrack,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioDemand {
    /// Explicit video-only playback discards audio at the decoder for this pull.
    Discard,
    Frames(NonZeroU16),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PullCredit {
    pub video_frames: u8,
    pub audio: AudioDemand,
}

#[derive(Debug)]
pub struct VideoFrame {
    pub presentation: Duration,
    pub width: u32,
    pub height: u32,
    pub rgba: Box<[u8]>,
}

#[derive(Debug)]
pub struct PcmBlock {
    pub first_frame: u64,
    pub samples: Box<[i16]>,
}

#[derive(Debug)]
pub struct PullResult {
    pub identity: VideoIdentity,
    pub frames: Vec<VideoFrame>,
    pub pcm: Option<PcmBlock>,
    pub end_of_media: bool,
}

/// Single backend owner and one command at a time. The owner reserves output
/// capacity before Pull and fences its presentation queue when it accepts Seek.
pub struct VideoTransport<P: RestrictedVideoPipe, S: VideoSource> {
    channel: Channel<P, S>,
    metadata: VideoMetadata,
    identity: VideoIdentity,
    bounds: OutputBounds,
    last_request: u64,
    next_pcm_frame: u64,
}

impl<P: RestrictedVideoPipe, S: VideoSource> VideoTransport<P, S> {
    pub fn open(
        pipe: P,
        source: S,
        session: SessionGeneration,
        bounds: OutputBounds,
        control: VideoIo<'_>,
    ) -> io::Result<Self> {
        let length = source.length();
        let mut channel = Channel {
            pipe,
            source,
            length,
            last_read: 0,
            read_buffer: Vec::new(),
            source_failure: None,
            retired: false,
        };
        if rgba_length(bounds.width, bounds.height).is_none() {
            return Err(invalid_input("Video output bounds exceed the media budget"));
        }
        let epoch = SeekEpoch(NonZeroU64::MIN);
        let mut fields = [0; 24];
        fields[..8].copy_from_slice(&length.to_le_bytes());
        fields[8..16].copy_from_slice(&epoch.get().to_le_bytes());
        fields[16..20].copy_from_slice(&bounds.width.to_le_bytes());
        fields[20..24].copy_from_slice(&bounds.height.to_le_bytes());
        channel.send(OPEN, &fields, &[], control)?;

        let header = channel.next_response(0, epoch, control)?;
        if header.kind != OPENED {
            return Err(protocol("Expected an MMV2 Opened response"));
        }
        let fields = channel.read_fields::<25>(control)?;
        let width = u32::from_le_bytes(field(&fields, 16));
        let height = u32::from_le_bytes(field(&fields, 20));
        if u64::from_le_bytes(field(&fields, 0)) != epoch.get()
            || width == 0
            || height == 0
            || u64::from(width) * u64::from(height) > MAX_NATURAL_PIXELS
        {
            return Err(protocol("Invalid MMV2 Opened metadata"));
        }
        let duration = u64::from_le_bytes(field(&fields, 8));
        let audio = match fields[24] {
            0 => AudioTrack::Absent,
            1 => AudioTrack::Qualified,
            2 => AudioTrack::Unqualified,
            _ => return Err(protocol("Invalid MMV2 audio track status")),
        };
        Ok(Self {
            channel,
            metadata: VideoMetadata {
                duration: (duration != u64::MAX).then(|| Duration::from_micros(duration)),
                width,
                height,
                audio,
            },
            identity: VideoIdentity { session, epoch },
            bounds,
            last_request: 0,
            next_pcm_frame: 0,
        })
    }

    pub fn metadata(&self) -> VideoMetadata {
        self.metadata
    }

    pub fn identity(&self) -> VideoIdentity {
        self.identity
    }

    pub fn pull(&mut self, credit: PullCredit, control: VideoIo<'_>) -> io::Result<PullResult> {
        self.channel.ensure_live()?;
        let audio_frames = match credit.audio {
            AudioDemand::Discard => 0,
            AudioDemand::Frames(frames) if self.metadata.audio == AudioTrack::Qualified => {
                frames.get()
            }
            AudioDemand::Frames(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "This media has no qualified PCM track",
                ));
            }
        };
        if credit.video_frames > MAX_VIDEO_PULL_FRAMES
            || usize::from(audio_frames) > MAX_PCM_BLOCK_FRAMES
            || (credit.video_frames == 0 && audio_frames == 0)
        {
            return Err(invalid_input("Invalid MMV2 output credit"));
        }

        let result = self.pull_inner(credit.video_frames, audio_frames, control);
        self.channel.retire_on_error(result)
    }

    fn pull_inner(
        &mut self,
        video_credit: u8,
        audio_credit: u16,
        control: VideoIo<'_>,
    ) -> io::Result<PullResult> {
        let request = self.next_request()?;
        let identity = self.identity;
        let mut command = [0; 19];
        put_tag(&mut command, request, identity.epoch);
        command[16] = video_credit;
        command[17..19].copy_from_slice(&audio_credit.to_le_bytes());
        self.channel.send(PULL, &command, &[], control)?;

        let mut frames = Vec::with_capacity(usize::from(video_credit));
        let mut pcm = None;
        let end_of_media = loop {
            let header = self
                .channel
                .next_response(request, identity.epoch, control)?;
            match header.kind {
                VIDEO if frames.len() < usize::from(video_credit) => {
                    let frame = self.read_video(header, request, control)?;
                    frames.push(frame);
                }
                PCM if pcm.is_none() && audio_credit != 0 => {
                    pcm = Some(self.read_pcm(header, request, audio_credit, control)?);
                }
                COMPLETED => {
                    let status = self.channel.completion(request, identity.epoch, control)?;
                    if status > 1 {
                        return Err(protocol("Invalid completion for a current MMV2 Pull"));
                    }
                    break status == 1;
                }
                _ => {
                    return Err(protocol(
                        "MMV2 Pull emitted an unexpected or uncredited response",
                    ));
                }
            }
        };
        Ok(PullResult {
            identity,
            frames,
            pcm,
            end_of_media,
        })
    }

    fn read_video(
        &mut self,
        header: Header,
        request: u64,
        control: VideoIo<'_>,
    ) -> io::Result<VideoFrame> {
        let metadata = self.channel.read_fields::<VIDEO_FIELDS>(control)?;
        check_tag(&metadata, request, self.identity.epoch)?;
        let width = u32::from_le_bytes(field(&metadata, 24));
        let height = u32::from_le_bytes(field(&metadata, 28));
        let presentation = i64::from_le_bytes(field(&metadata, 16));
        let length = rgba_length(width, height)
            .filter(|length| {
                width <= self.bounds.width
                    && height <= self.bounds.height
                    && presentation >= 0
                    && header.payload == VIDEO_FIELDS + *length
            })
            .ok_or_else(|| protocol("Invalid MMV2 video dimensions, timestamp or payload"))?;
        let rgba = self.channel.read_body(length, control)?;
        Ok(VideoFrame {
            presentation: Duration::from_micros(presentation as u64),
            width,
            height,
            rgba,
        })
    }

    fn read_pcm(
        &mut self,
        header: Header,
        request: u64,
        credit: u16,
        control: VideoIo<'_>,
    ) -> io::Result<PcmBlock> {
        let metadata = self.channel.read_fields::<PCM_FIELDS>(control)?;
        check_tag(&metadata, request, self.identity.epoch)?;
        let first_frame = u64::from_le_bytes(field(&metadata, 16));
        let count = u16::from_le_bytes(field(&metadata, 24));
        let length = usize::from(count) * PCM_BYTES_PER_FRAME;
        if count == 0
            || count > credit
            || first_frame != self.next_pcm_frame
            || header.payload != PCM_FIELDS + length
        {
            return Err(protocol("Invalid MMV2 PCM sequence, credit or payload"));
        }
        let next = first_frame
            .checked_add(u64::from(count))
            .ok_or_else(|| protocol("MMV2 PCM frame counter overflow"))?;
        let bytes = self.channel.read_body(length, control)?;
        let samples = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .copied()
            .map(i16::from_le_bytes)
            .collect::<Box<_>>();
        self.next_pcm_frame = next;
        Ok(PcmBlock {
            first_frame,
            samples,
        })
    }

    pub fn seek(&mut self, position: Duration, control: VideoIo<'_>) -> io::Result<SeekEpoch> {
        self.channel.ensure_live()?;
        let micros = u64::try_from(position.as_micros())
            .map_err(|_| invalid_input("MMV2 seek position overflows its timestamp"))?;
        let result = self.seek_inner(micros, control);
        self.channel.retire_on_error(result)
    }

    fn seek_inner(&mut self, micros: u64, control: VideoIo<'_>) -> io::Result<SeekEpoch> {
        let request = self.next_request()?;
        let epoch = self.identity.epoch.next()?;
        let mut command = [0; 24];
        put_tag(&mut command, request, epoch);
        command[16..24].copy_from_slice(&micros.to_le_bytes());
        self.channel.send(SEEK, &command, &[], control)?;

        let header = self.channel.next_response(request, epoch, control)?;
        if header.kind != COMPLETED || self.channel.completion(request, epoch, control)? != 0 {
            return Err(protocol(
                "MMV2 Seek did not acknowledge the requested epoch",
            ));
        }
        self.identity.epoch = epoch;
        self.next_pcm_frame = 0;
        Ok(epoch)
    }

    pub fn close(&mut self, control: VideoIo<'_>) -> io::Result<()> {
        if self.channel.retired {
            return Ok(());
        }
        let result = self.next_request().and_then(|request| {
            self.channel
                .send(CLOSE, &request.to_le_bytes(), &[], control)
        });
        self.channel.retire();
        result
    }

    fn next_request(&mut self) -> io::Result<u64> {
        self.last_request = self
            .last_request
            .checked_add(1)
            .ok_or_else(|| io::Error::other("Media request identities exhausted"))?;
        Ok(self.last_request)
    }
}

struct Header {
    kind: u8,
    payload: usize,
}

struct Channel<P: RestrictedVideoPipe, S: VideoSource> {
    pipe: P,
    source: S,
    length: u64,
    last_read: u64,
    read_buffer: Vec<u8>,
    source_failure: Option<io::Error>,
    retired: bool,
}

impl<P: RestrictedVideoPipe, S: VideoSource> Channel<P, S> {
    fn ensure_live(&self) -> io::Result<()> {
        if self.retired {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        Ok(())
    }

    fn retire(&mut self) {
        if !self.retired {
            self.retired = true;
            self.pipe.retire();
        }
    }

    fn retire_on_error<T>(&mut self, result: io::Result<T>) -> io::Result<T> {
        if result.is_err() {
            self.retire();
        }
        result
    }

    fn send(
        &mut self,
        kind: u8,
        fields: &[u8],
        body: &[u8],
        control: VideoIo<'_>,
    ) -> io::Result<()> {
        control.check()?;
        let mut header = [0; HEADER_BYTES];
        header[..4].copy_from_slice(b"MMV2");
        header[4] = kind;
        let length = (fields.len() + body.len()) as u32;
        header[8..12].copy_from_slice(&length.to_le_bytes());
        self.pipe.send(&[&header, fields, body], control)?;
        control.check()
    }

    fn read_exact(&mut self, bytes: &mut [u8], control: VideoIo<'_>) -> io::Result<()> {
        control.check()?;
        self.pipe.read_exact(bytes, control)?;
        control.check()
    }

    fn read_fields<const N: usize>(&mut self, control: VideoIo<'_>) -> io::Result<[u8; N]> {
        let mut fields = [0; N];
        self.read_exact(&mut fields, control)?;
        Ok(fields)
    }

    fn read_body(&mut self, length: usize, control: VideoIo<'_>) -> io::Result<Box<[u8]>> {
        let mut bytes = Vec::new();
        grow(&mut bytes, length)?;
        self.read_exact(&mut bytes, control)?;
        Ok(bytes.into_boxed_slice())
    }

    fn read_header(&mut self, control: VideoIo<'_>) -> io::Result<Header> {
        let header = self.read_fields::<HEADER_BYTES>(control)?;
        if &header[..4] != b"MMV2" || header[5..8] != [0; 3] {
            return Err(protocol("Invalid MMV2 frame header"));
        }
        let kind = header[4];
        let payload = u32::from_le_bytes(field(&header, 8)) as usize;
        let valid = match kind {
            OPENED => payload == 25,
            READ => payload == 20,
            VIDEO => {
                (VIDEO_FIELDS + 4..=VIDEO_FIELDS + MAX_VIDEO_PIXELS as usize * 4).contains(&payload)
            }
            COMPLETED => payload == 17,
            FAILED => payload == FAILED_FIELDS,
            PCM => (PCM_FIELDS + PCM_BYTES_PER_FRAME
                ..=PCM_FIELDS + MAX_PCM_BLOCK_FRAMES * PCM_BYTES_PER_FRAME)
                .contains(&payload),
            _ => false,
        };
        if !valid {
            return Err(protocol("Invalid MMV2 reply kind or declared length"));
        }
        Ok(Header { kind, payload })
    }

    fn next_response(
        &mut self,
        request: u64,
        epoch: SeekEpoch,
        control: VideoIo<'_>,
    ) -> io::Result<Header> {
        loop {
            let header = self.read_header(control)?;
            if header.kind == FAILED {
                let fields = self.read_fields::<FAILED_FIELDS>(control)?;
                check_tag(&fields, request, epoch)?;
                return Err(self.failure(fields[16], fields[17], fields[18]));
            }
            if self.source_failure.is_some() {
                return Err(protocol("MMV2 worker did not retire the failed source"));
            }
            if header.kind != READ {
                return Ok(header);
            }
            self.reply_to_read(control)?;
        }
    }

    fn completion(
        &mut self,
        request: u64,
        epoch: SeekEpoch,
        control: VideoIo<'_>,
    ) -> io::Result<u8> {
        let fields = self.read_fields::<17>(control)?;
        check_tag(&fields, request, epoch)?;
        Ok(fields[16])
    }

    fn reply_to_read(&mut self, control: VideoIo<'_>) -> io::Result<()> {
        let fields = self.read_fields::<20>(control)?;
        let read_id = u64::from_le_bytes(field(&fields, 0));
        let offset = u64::from_le_bytes(field(&fields, 8));
        let length = u32::from_le_bytes(field(&fields, 16)) as usize;
        if self.last_read.checked_add(1) != Some(read_id)
            || !(1..=MAX_READ_BYTES).contains(&length)
            || offset
                .checked_add(length as u64)
                .is_none_or(|end| end > self.length)
        {
            return Err(protocol("MMV2 read is outside its capture or sequence"));
        }
        self.last_read = read_id;
        let mut buffer = std::mem::take(&mut self.read_buffer);
        grow(&mut buffer, length)?;
        control.check()?;
        let outcome = self.source.read_at(offset, &mut buffer, control);
        control.check()?;
        let mut reply = [0; 9];
        reply[..8].copy_from_slice(&read_id.to_le_bytes());
        let count = match outcome {
            Ok(count) if count <= length => count,
            Ok(_) => {
                return Err(protocol(
                    "Captured source returned too much data for an MMV2 read",
                ));
            }
            Err(error) => {
                reply[8] = match error.kind() {
                    io::ErrorKind::PermissionDenied => 1,
                    io::ErrorKind::Interrupted => 2,
                    _ => 3,
                };
                self.source_failure = Some(error);
                0
            }
        };
        self.send(READ_REPLY, &reply, &buffer[..count], control)?;
        self.read_buffer = buffer;
        Ok(())
    }

    fn failure(&mut self, cause: u8, codec: u8, tried: u8) -> io::Error {
        if let Some(error) = self.source_failure.take() {
            return if cause == 1 {
                error
            } else {
                protocol("MMV2 worker did not acknowledge the source error")
            };
        }
        if cause == 5 {
            // No decoder the helper could use supports the video; the host words why.
            return MissingDecoder { codec, tried }.into();
        }
        if codec != 0 || tried != 0 {
            return protocol("MMV2 failure names a codec without a missing decoder");
        }
        match cause {
            1 => io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Media worker could not read the capture",
            ),
            2 => io::Error::new(
                io::ErrorKind::Unsupported,
                "Media format or codec is unsupported",
            ),
            3 => protocol("Media worker rejected invalid input"),
            4 => io::Error::other("Media worker failed during decoding"),
            _ => protocol("Invalid MMV2 failure cause"),
        }
    }
}

impl<P: RestrictedVideoPipe, S: VideoSource> Drop for Channel<P, S> {
    fn drop(&mut self) {
        self.retire();
    }
}

fn grow(bytes: &mut Vec<u8>, length: usize) -> io::Result<()> {
    bytes
        .try_reserve_exact(length.saturating_sub(bytes.len()))
        .map_err(|_| {
            io::Error::new(io::ErrorKind::OutOfMemory, "Media buffer allocation failed")
        })?;
    bytes.resize(length, 0);
    Ok(())
}

fn rgba_length(width: u32, height: u32) -> Option<usize> {
    let area = u64::from(width) * u64::from(height);
    (width != 0
        && height != 0
        && width <= MAX_VIDEO_EDGE
        && height <= MAX_VIDEO_EDGE
        && area <= MAX_VIDEO_PIXELS)
        .then(|| area as usize * 4)
}

fn put_tag(fields: &mut [u8], request: u64, epoch: SeekEpoch) {
    fields[..8].copy_from_slice(&request.to_le_bytes());
    fields[8..16].copy_from_slice(&epoch.get().to_le_bytes());
}

fn check_tag(fields: &[u8], request: u64, epoch: SeekEpoch) -> io::Result<()> {
    if u64::from_le_bytes(field(fields, 0)) != request
        || u64::from_le_bytes(field(fields, 8)) != epoch.get()
    {
        return Err(protocol(
            "MMV2 reply does not match its active command and epoch",
        ));
    }
    Ok(())
}

fn field<const N: usize>(fields: &[u8], offset: usize) -> [u8; N] {
    fields[offset..offset + N]
        .try_into()
        .expect("fixed MMV2 field")
}

fn invalid_input(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn protocol(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
#[path = "video_transport_tests.rs"]
mod tests;

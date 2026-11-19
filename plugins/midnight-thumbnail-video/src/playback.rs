//! Consecutive video decoding over one captured broker stream, independent of playback transport.
//! This cursor owns no clock, audio device, source pathname or process. The eventual host must
//! supervise the worker with cancellation, wall-clock deadlines and bounded source reads.

use crate::{Video, ffmpeg, native};
use midnight_thumbnail_sdk::ReadSeek;
use std::{
    cell::RefCell,
    io::{self, Read, Seek, SeekFrom},
    rc::Rc,
};

/// Aspect-preserving output bound: edges at most 4096 and at most four mebipixels.
/// Original video is never enlarged. Bounds are validated before opening the source.
#[derive(Clone, Copy, Debug)]
pub struct OutputBounds {
    pub width: u32,
    pub height: u32,
}

/// One oriented straight-alpha sRGB RGBA8 frame and its presentation time.
#[derive(Debug)]
pub struct Frame {
    /// Microseconds from the container's start time. Pre-target preroll is never returned.
    pub presentation_micros: i64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Container duration when known and the selected video's original, oriented dimensions.
/// Duration comes from the container, so it may overstate the decodable content. Zero is a
/// known zero duration; a negative/unknown native duration is `None`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Metadata {
    pub duration_micros: Option<u64>,
    pub width: u32,
    pub height: u32,
}

/// Open exactly one video stream for the duration of `consume`. The closure keeps the audited
/// AVIO bridge and its borrowed source alive without a self-referential native owner.
///
/// Only the installed VP8/VP9/Theora/FFV1/raw-video profile is accepted. The callback must not
/// reenter this function or `decode`: this single-decoder worker holds the native decode lock.
/// Unknown timestamps, decode errors and exhausted work bounds are terminal; no retry or source
/// reopen happens here. Callers must retain the captured source until their worker has retired.
pub fn with_cursor<T>(
    source: &mut dyn ReadSeek,
    source_length: u64,
    output: OutputBounds,
    consume: impl FnOnce(&mut Cursor<'_>) -> io::Result<T>,
) -> io::Result<T> {
    crate::output_length(output.width, output.height)?;
    if source_length > i64::MAX as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Video source length exceeds the native seek range",
        ));
    }
    let read_error = Rc::new(RefCell::new(None));
    let mut source = CheckedSource {
        source,
        error: read_error.clone(),
    };
    let result = crate::with_decoder(|| {
        native::with_input(&mut source, source_length, |input| {
            let video = crate::open_video(input)?;
            if video.time_base.numerator() <= 0 || video.time_base.denominator() <= 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Video stream has no usable presentation time base",
                ));
            }
            let start = native::start_time(input);
            let mut cursor = Cursor {
                input,
                video,
                output,
                start,
                minimum_time: 0,
                decoded: ffmpeg::frame::Video::empty(),
                pending: None,
                phase: Phase::Reading,
                eof_sent: false,
                read_error: read_error.clone(),
            };
            cursor.check_source()?;
            consume(&mut cursor)
        })
    });
    // Preserve a source failure even if probing interpreted the failed read as an empty stream.
    let error = read_error.borrow_mut().take();
    error.map_or(result, Err)
}

// The existing native bridge deliberately returns errno across FFI. Retain the actual read
// error on this side as well: a demuxer may translate an I/O failure into a packet EOF.
struct CheckedSource<'a> {
    source: &'a mut dyn ReadSeek,
    error: Rc<RefCell<Option<io::Error>>>,
}

impl Read for CheckedSource<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        match self.source.read(bytes) {
            Ok(count) => Ok(count),
            Err(error) => {
                let kind = error.kind();
                let _ = self.error.borrow_mut().get_or_insert(error);
                Err(io::Error::new(kind, "Captured video read failed"))
            }
        }
    }
}

impl Seek for CheckedSource<'_> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.source.seek(from)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Reading,
    Draining,
    Ended,
    Failed,
}

/// Single-owner, single-open decoder. Returned frames own their bounded pixels; no output queue
/// is retained. Each call permits at most 4096 decode/packet steps, including skipped preroll.
pub struct Cursor<'a> {
    input: &'a mut ffmpeg::format::context::Input,
    video: Video,
    output: OutputBounds,
    start: i64,
    minimum_time: i64,
    decoded: ffmpeg::frame::Video,
    pending: Option<ffmpeg::Packet>,
    phase: Phase,
    eof_sent: bool,
    read_error: Rc<RefCell<Option<io::Error>>>,
}

impl Cursor<'_> {
    pub fn audio_track(&self) -> crate::playback_audio::AudioTrack {
        crate::playback_audio::track(self.input)
    }

    /// Return container timing and the selected decoder's current original dimensions,
    /// independent of the bounded/scaled output. Quarter-turn orientation swaps the axes.
    pub fn metadata(&self) -> Metadata {
        let (width, height) = if matches!(self.video.rotation, 90 | 270) {
            (self.video.decoder.height(), self.video.decoder.width())
        } else {
            (self.video.decoder.width(), self.video.decoder.height())
        };
        Metadata {
            duration_micros: u64::try_from(self.input.duration()).ok(),
            width,
            height,
        }
    }

    /// Decode the next presentation frame. EOF is sticky until a successful seek. Native read,
    /// timestamp, color, allocation-bound or decode failures poison this cursor permanently.
    pub fn next_frame(&mut self) -> io::Result<Option<Frame>> {
        match self.phase {
            Phase::Failed => return Err(ended()),
            Phase::Ended => return Ok(None),
            Phase::Reading | Phase::Draining => {}
        }
        self.check_source()?;
        let result = self.next_inner();
        let result = self.check_source().and(result);
        if result.is_err() {
            self.phase = Phase::Failed;
            self.pending = None;
        }
        result
    }

    /// Seek in container-relative microseconds, flush the decoder, then suppress all frames
    /// before the target. A non-frame-aligned target returns the first frame at/after it.
    /// Invalid caller ranges leave the cursor intact; a native seek failure is terminal.
    pub fn seek(&mut self, target_micros: u64) -> io::Result<()> {
        if self.phase == Phase::Failed {
            return Err(ended());
        }
        self.check_source()?;
        let target = i64::try_from(target_micros)
            .ok()
            .and_then(|target| {
                self.start
                    .checked_add(target)
                    .map(|absolute| (target, absolute))
            })
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Video seek exceeds timestamp range",
                )
            })?;
        let result = native::seek_video(self.input, self.video.index, target.1);
        if let Err(error) = self.check_source().and(result) {
            // Native seek errors are terminal. Checked conversion/range refusals happen before
            // touching the demuxer and leave the next consecutive frame available.
            if error.kind() != io::ErrorKind::InvalidInput {
                self.phase = Phase::Failed;
                self.pending = None;
            }
            return Err(error);
        }
        self.video.decoder.flush();
        self.pending = None;
        self.decoded = ffmpeg::frame::Video::empty();
        self.minimum_time = target.0;
        self.phase = Phase::Reading;
        self.eof_sent = false;
        Ok(())
    }

    fn next_inner(&mut self) -> io::Result<Option<Frame>> {
        for _ in 0..4096 {
            match self.video.decoder.receive_frame(&mut self.decoded) {
                Ok(()) => {
                    let time = self.presentation_time()?;
                    if time < self.minimum_time {
                        continue;
                    }
                    let pixels = crate::pixels(
                        &self.decoded,
                        self.output.width,
                        self.output.height,
                        self.video.rotation,
                    )?;
                    return Ok(Some(Frame {
                        presentation_micros: time,
                        width: pixels.width,
                        height: pixels.height,
                        rgba: pixels.rgba,
                    }));
                }
                Err(ffmpeg::Error::Eof) => {
                    self.phase = Phase::Ended;
                    return Ok(None);
                }
                Err(error) if again(error) => {}
                Err(error) => return Err(io::Error::other(error)),
            }

            if let Some(packet) = &self.pending {
                match self.video.decoder.send_packet(packet) {
                    Ok(()) => self.pending = None,
                    // Keep the unsent packet: receiving a frame must never lose its successor.
                    Err(error) if again(error) => {}
                    Err(error) => return Err(io::Error::other(error)),
                }
                continue;
            }
            if self.phase == Phase::Draining {
                if self.eof_sent {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Video decoder stopped making progress after EOF",
                    ));
                }
                match self.video.decoder.send_eof() {
                    Ok(()) => self.eof_sent = true,
                    Err(error) if again(error) => {}
                    Err(error) => return Err(io::Error::other(error)),
                }
                continue;
            }
            let mut packet = ffmpeg::Packet::empty();
            match packet.read(self.input) {
                Ok(()) if packet.stream() == self.video.index => self.pending = Some(packet),
                Ok(()) => {}
                Err(ffmpeg::Error::Eof) => self.phase = Phase::Draining,
                Err(error) => return Err(io::Error::other(error)),
            }
        }
        Err(io::Error::other("Video decode work budget exhausted"))
    }

    fn presentation_time(&self) -> io::Result<i64> {
        let timestamp = self.decoded.timestamp().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Video frame has no presentation timestamp",
            )
        })?;
        let time =
            (i128::from(timestamp) * i128::from(self.video.time_base.numerator()) * 1_000_000)
                .div_euclid(i128::from(self.video.time_base.denominator()))
                - i128::from(self.start);
        i64::try_from(time).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Video presentation timestamp overflow",
            )
        })
    }

    fn check_source(&mut self) -> io::Result<()> {
        let error = self.read_error.borrow_mut().take();
        if let Some(error) = error {
            self.phase = Phase::Failed;
            self.pending = None;
            return Err(error);
        }
        Ok(())
    }
}

fn again(error: ffmpeg::Error) -> bool {
    matches!(error, ffmpeg::Error::Other { errno } if errno == ffmpeg::error::EAGAIN)
}

fn ended() -> io::Error {
    io::Error::other("Playback cursor ended after a failure")
}

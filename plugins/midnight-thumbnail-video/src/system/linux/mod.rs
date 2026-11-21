//! Linux platform decoders: the distribution's own FFmpeg first (libavcodec 58 to 63, loaded at
//! run time with per-series bindings), then GStreamer 1.x with the distribution's plugins.
//!
//! The worker sandbox (seccomp; see the worker SDK) denies every file open, new threads and
//! execution once the helper has entered it, so all libraries and plugins are loaded before
//! `enter`, from root-owned system folders only (see `library`). Nothing is loaded lazily,
//! and both families then decode on the helper's one thread from the brokered capture only:
//! FFmpeg through a custom AVIO context, GStreamer through a source pad this module drives.
//! These helpers never link or load the bundled FFmpeg, so one process maps one FFmpeg.
mod ffmpeg;
mod gstreamer;
mod library;
mod picture;
// The tests load the distribution's FFmpeg into the test process, which a build that also
// links the bundled FFmpeg (feature `bundled`) refuses by design: run them with
// `--no-default-features --features system,playback-worker`, as the helpers are built.
#[cfg(all(test, not(feature = "bundled")))]
mod tests;

use crate::{
    media::{AudioTrack, Block, Codec, Frame, Metadata, NoDecoder, OutputBounds, Tried},
    playback_worker::{PlaybackDecoder, Session, Sources},
};
use library::Trust;
use midnight_thumbnail_sdk::{ReadSeek, api};
use std::{fmt, io};

/// Where a decoder family is looked for.
enum Search {
    /// The system loader's own search by soname: the only production setting.
    System,
    /// Absolute paths in one folder (tests: an FFmpeg built for the test).
    #[cfg(all(test, not(feature = "bundled")))]
    Folder(std::path::PathBuf),
    /// Treat the family as absent (tests: the GStreamer fallback, or neither).
    #[cfg(all(test, not(feature = "bundled")))]
    Hidden,
}

struct Options {
    ffmpeg: Search,
    gstreamer: Search,
    trust: Trust,
}

impl Options {
    const SYSTEM: Self = Self {
        ffmpeg: Search::System,
        gstreamer: Search::System,
        trust: Trust::ROOT,
    };
}

/// The decoder families found before lockdown.
struct Decoders {
    ffmpeg: Option<&'static ffmpeg::Ffmpeg>,
    gstreamer: Option<&'static gstreamer::Gstreamer>,
}

/// A source the decoder families do not recognize as media they can open.
#[derive(Debug)]
struct Unrecognized;

impl fmt::Display for Unrecognized {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("The source is not a recognized video container")
    }
}

impl std::error::Error for Unrecognized {}

pub(super) fn unrecognized() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, Unrecognized)
}

fn is_unrecognized(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.is::<Unrecognized>())
}

/// A container a family opened that has no video stream it can see.
#[derive(Debug)]
struct NoVideo;

impl fmt::Display for NoVideo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("No non-attached video track")
    }
}

impl std::error::Error for NoVideo {}

pub(super) fn no_video() -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, NoVideo)
}

/// How one family's attempt ended when it did not decode.
enum Miss {
    /// No decoder for this codec: the next family may have one.
    NoDecoder(Codec),
    /// The family sees no video stream (it may not know the stream's type).
    NoVideo,
    /// The container was not recognized: the next family may know it.
    Unrecognized(io::Error),
}

fn miss(error: io::Error) -> Result<Miss, io::Error> {
    if let Some(missing) = NoDecoder::find(&error) {
        return Ok(Miss::NoDecoder(missing.codec));
    }
    if is_unrecognized(&error) {
        return Ok(Miss::Unrecognized(error));
    }
    if error.get_ref().is_some_and(|inner| inner.is::<NoVideo>()) {
        return Ok(Miss::NoVideo);
    }
    Err(error)
}

fn gstreamer_miss(refusal: gstreamer::Refusal) -> Result<Miss, io::Error> {
    match refusal {
        gstreamer::Refusal::NoDecoder(codec) => Ok(Miss::NoDecoder(codec)),
        // GStreamer is installed without a demuxer for this video container: report the
        // codec the other attempts named, as missing plugins.
        gstreamer::Refusal::NoDemuxer => Ok(Miss::NoDecoder(Codec::Unknown)),
        gstreamer::Refusal::Unrecognized => Ok(Miss::Unrecognized(unrecognized())),
        gstreamer::Refusal::NoStream => Ok(Miss::NoVideo),
        gstreamer::Refusal::Failed(error) => Err(error),
    }
}

/// Every attempt missed. A family that recognized the container but had no decoder (or no
/// family at all) makes this `NoDecoder` with the first codec named and what was tried; a
/// container in which no family sees video is unsupported; only a source that no family
/// recognized is invalid data.
fn conclude(misses: Vec<Miss>, tried: Tried) -> io::Error {
    let mut codec = Codec::Unknown;
    let mut recognized = false;
    let mut opened = false;
    let mut unrecognized = None;
    for miss in misses {
        match miss {
            Miss::NoDecoder(found) => {
                recognized = true;
                if codec == Codec::Unknown {
                    codec = found;
                }
            }
            Miss::NoVideo => opened = true,
            Miss::Unrecognized(error) => {
                unrecognized.get_or_insert(error);
            }
        }
    }
    if recognized {
        return NoDecoder { codec, tried }.into();
    }
    if opened {
        return no_video();
    }
    match unrecognized {
        Some(error) => error,
        None => NoDecoder { codec, tried }.into(),
    }
}

impl Decoders {
    /// Find the families. Call before lockdown, on the process's only thread.
    fn load(options: &Options) -> Self {
        let ffmpeg = ffmpeg::load(&options.ffmpeg, options.trust).ok();
        let gstreamer = gstreamer::load(&options.gstreamer, options.trust, ffmpeg.is_none()).ok();
        // Any mapped object that does not pass the read rules disqualifies both families: its
        // constructors already ran, and whatever loaded it cannot be trusted to decode.
        if library::audit(options.trust).is_err() {
            return Self {
                ffmpeg: None,
                gstreamer: None,
            };
        }
        Self { ffmpeg, gstreamer }
    }

    fn tried(&self) -> Tried {
        let mut tried = Tried::default();
        if self.ffmpeg.is_some() {
            tried = tried.union(Tried::DISTRIBUTION_FFMPEG);
        }
        if self.gstreamer.is_some() {
            tried = tried.union(Tried::GSTREAMER);
        }
        tried
    }

    fn thumbnail(
        &self,
        request: &api::Request,
        source: &mut dyn ReadSeek,
    ) -> io::Result<api::Frame> {
        api::rgba_length(request.width, request.height)?;
        if request.frame >= api::MAX_ANIMATION_FRAMES || request.source_length > i64::MAX as u64 {
            return Err(io::Error::other("Invalid video request"));
        }
        let bounds = OutputBounds {
            width: request.width,
            height: request.height,
        };
        let length = request.source_length;
        let mut misses = Vec::new();
        if let Some(ffmpeg) = self.ffmpeg {
            source.seek(io::SeekFrom::Start(0))?;
            let decoded = ffmpeg::Input::open(ffmpeg, Box::new(&mut *source), length)
                .and_then(|mut input| ffmpeg::Thumbnail::decode(&mut input, bounds, request.frame));
            match decoded {
                Ok(pixels) => return Ok(thumbnail_frame(request, pixels)),
                Err(error) => misses.push(miss(error)?),
            }
        }
        if let Some(gstreamer) = self.gstreamer {
            source.seek(io::SeekFrom::Start(0))?;
            match gstreamer::thumbnail(
                gstreamer,
                Box::new(&mut *source),
                length,
                bounds,
                request.frame,
            ) {
                Ok(pixels) => return Ok(thumbnail_frame(request, pixels)),
                Err(refusal) => misses.push(gstreamer_miss(refusal)?),
            }
        }
        Err(conclude(misses, self.tried()))
    }

    fn playback(
        &self,
        sources: &dyn Sources,
        bounds: OutputBounds,
        session: Session<'_>,
    ) -> io::Result<()> {
        let length = sources.length();
        let mut misses = Vec::new();
        if let Some(ffmpeg) = self.ffmpeg {
            let opened = ffmpeg::Input::open(ffmpeg, sources.cursor(), length)
                .and_then(|input| ffmpeg::Video::open(input, bounds));
            match opened {
                Ok(video) => {
                    let audio = match ffmpeg::Input::open(ffmpeg, sources.cursor(), length) {
                        Ok(input) => ffmpeg::Audio::open(input, video.metadata().duration_micros),
                        Err(_) => ffmpeg::AudioOpened::Unqualified,
                    };
                    return session(&mut FfmpegPlayback { video, audio });
                }
                Err(error) => misses.push(miss(error)?),
            }
        }
        if let Some(gstreamer) = self.gstreamer {
            match gstreamer::Playback::open(
                gstreamer,
                sources.cursor(),
                || sources.cursor(),
                length,
                bounds,
            ) {
                Ok(mut playback) => return session(&mut playback),
                Err(refusal) => misses.push(gstreamer_miss(refusal)?),
            }
        }
        Err(conclude(misses, self.tried()))
    }
}

fn thumbnail_frame(request: &api::Request, pixels: picture::Pixels) -> api::Frame {
    api::Frame {
        id: request.id,
        frame: request.frame,
        width: pixels.width,
        height: pixels.height,
        rgba: pixels.rgba,
    }
}

/// Playback through the distribution's FFmpeg: one demuxer for the video, one for the audio.
struct FfmpegPlayback<'s> {
    video: ffmpeg::Video<'s>,
    audio: ffmpeg::AudioOpened<'s>,
}

impl PlaybackDecoder for FfmpegPlayback<'_> {
    fn metadata(&self) -> Metadata {
        self.video.metadata()
    }

    fn audio_track(&self) -> AudioTrack {
        ffmpeg::Audio::track(&self.audio)
    }

    fn next_frame(&mut self) -> io::Result<Option<Frame>> {
        self.video.next_frame()
    }

    fn seek(&mut self, position_micros: u64) -> io::Result<()> {
        self.video.seek(position_micros)?;
        if let ffmpeg::AudioOpened::Qualified(audio) = &mut self.audio {
            audio.seek(position_micros)?;
        }
        Ok(())
    }

    fn pcm(&mut self, credit: u16) -> io::Result<Option<Block>> {
        match &mut self.audio {
            ffmpeg::AudioOpened::Qualified(audio) => audio.next(credit),
            _ => Ok(None),
        }
    }
}

impl PlaybackDecoder for gstreamer::Playback<'_> {
    fn metadata(&self) -> Metadata {
        gstreamer::Playback::metadata(self)
    }

    fn audio_track(&self) -> AudioTrack {
        gstreamer::Playback::audio_track(self)
    }

    fn next_frame(&mut self) -> io::Result<Option<Frame>> {
        gstreamer::Playback::next_frame(self)
    }

    fn seek(&mut self, position_micros: u64) -> io::Result<()> {
        gstreamer::Playback::seek(self, position_micros)
    }

    fn pcm(&mut self, credit: u16) -> io::Result<Option<Block>> {
        gstreamer::Playback::pcm(self, credit)
    }
}

/// The locked-down thumbnail helper: load the families, then enter the sandbox and serve one
/// MTH1 request.
pub fn thumbnail_main() -> io::Result<()> {
    let decoders = Decoders::load(&Options::SYSTEM);
    midnight_thumbnail_sdk::serve(|request, source| decoders.thumbnail(request, source))
}

/// The locked-down playback helper: load the families, then enter the sandbox and serve one
/// MMV2 session.
#[cfg(feature = "playback-worker")]
pub fn playback_main() -> io::Result<()> {
    let decoders = Decoders::load(&Options::SYSTEM);
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    midnight_worker_sdk::enter(&mut input, &mut output)?;
    crate::playback_worker::serve(input, output, |sources, bounds, session| {
        decoders.playback(sources, bounds, session)
    })
}

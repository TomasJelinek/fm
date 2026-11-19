//! A second seek cursor over the same brokered source keeps PCM demand independent
//! of video queue pressure. Only the qualified built-in Opus decoder is used.
use crate::{ffmpeg, native};
use ffmpeg::{ChannelLayout, codec, format::Sample, frame, media, software};
use midnight_thumbnail_sdk::ReadSeek;
use std::io;

const RATE: i128 = 48_000;
const CHANNELS: usize = 2;
pub const MAX_PCM_FRAMES: u16 = 4_800;
const MAX_DECODED_FRAMES: usize = 5_760;
const PCM: Sample = Sample::I16(ffmpeg::format::sample::Type::Packed);

#[derive(Clone, Copy)]
#[repr(u8)]
pub enum AudioTrack {
    Absent = 0,
    Qualified = 1,
    Unqualified = 2,
}

pub struct Block {
    pub first_frame: u64,
    pub samples: Vec<i16>,
}

pub(crate) fn track(input: &ffmpeg::format::context::Input) -> AudioTrack {
    if opus(input).is_some() {
        AudioTrack::Qualified
    } else if input
        .streams()
        .any(|stream| stream.parameters().medium() == media::Type::Audio)
    {
        AudioTrack::Unqualified
    } else {
        AudioTrack::Absent
    }
}

fn opus<'a>(input: &'a ffmpeg::format::context::Input) -> Option<ffmpeg::Stream<'a>> {
    let qualifies = |stream: &ffmpeg::Stream<'_>| {
        let parameters = stream.parameters();
        parameters.medium() == media::Type::Audio && parameters.id() == codec::Id::OPUS
    };
    input
        .streams()
        .best(media::Type::Audio)
        .filter(qualifies)
        .or_else(|| input.streams().find(qualifies))
}

pub fn with_pcm<T>(
    source: &mut dyn ReadSeek,
    length: u64,
    duration_micros: Option<u64>,
    consume: impl FnOnce(&mut PcmCursor<'_>) -> io::Result<T>,
) -> io::Result<T> {
    // Called inside with_cursor's decoder lock. Both AVIO cursors obtain bytes
    // through the same retained source authority and one ordered read stream.
    native::with_input(source, length, |input| {
        let stream = opus(input)
            .ok_or_else(|| unsupported("the captured input has no qualified Opus track"))?;
        let index = stream.index();
        let time_base = stream.time_base();
        if time_base.numerator() <= 0 || time_base.denominator() <= 0 {
            return Err(invalid("invalid audio time base"));
        }
        let mut context = codec::context::Context::from_parameters(stream.parameters())
            .map_err(io::Error::other)?;
        native::limit_decoder(&mut context);
        // FFmpeg applies Opus pre-skip to the frame PTS in this time base.
        let mut decoder = context.decoder();
        decoder.set_packet_time_base(time_base);
        let decoder = decoder.audio().map_err(io::Error::other)?;
        let origin_micros = native::start_time(input);
        consume(&mut PcmCursor {
            input,
            decoder,
            index,
            time_base,
            origin_micros,
            duration_micros,
            target_micros: 0,
            submitted: 0,
            held: None,
            resampler: None,
            pending: None,
            draining: false,
            ended: false,
        })
    })
}

struct Clip {
    start: i128,
    samples: Vec<i16>,
}

pub struct PcmCursor<'a> {
    input: &'a mut ffmpeg::format::context::Input,
    decoder: codec::decoder::Audio,
    index: usize,
    time_base: ffmpeg::Rational,
    origin_micros: i64,
    duration_micros: Option<u64>,
    target_micros: u64,
    submitted: u64,
    held: Option<Clip>,
    resampler: Option<software::resampling::Context>,
    pending: Option<ffmpeg::Packet>,
    draining: bool,
    ended: bool,
}

impl PcmCursor<'_> {
    pub fn seek(&mut self, target: u64) -> io::Result<()> {
        // Decode preroll before the requested sample, then trim by the
        // adjusted timestamp when producing the new epoch's PCM.
        let absolute = i64::try_from(target.saturating_sub(80_000))
            .ok()
            .and_then(|relative| self.origin_micros.checked_add(relative))
            .ok_or_else(|| invalid("audio seek timestamp is out of range"))?;
        native::seek_video(self.input, self.index, absolute)?;
        self.decoder.flush();
        self.target_micros = target;
        self.submitted = 0;
        self.held = None;
        self.resampler = None;
        self.pending = None;
        self.draining = false;
        self.ended = false;
        Ok(())
    }

    pub fn next(&mut self, credit: u16) -> io::Result<Option<Block>> {
        if credit == 0 || credit > MAX_PCM_FRAMES {
            return Err(invalid("audio credit exceeds the PCM block limit"));
        }
        let first_frame = self.submitted;
        let mut samples = Vec::with_capacity(usize::from(credit) * CHANNELS);
        for _ in 0..4096 {
            let remaining = usize::from(credit) - samples.len() / CHANNELS;
            if remaining == 0 {
                break;
            }
            if self.held.is_none() && !self.ended {
                self.held = self.decode_clip()?;
            }
            let position = i128::from(self.submitted);
            let frames = if let Some(clip) = &self.held {
                if clip.start > position {
                    let gap = (clip.start - position).min(remaining as i128) as usize;
                    samples.resize(samples.len() + gap * CHANNELS, 0);
                    gap
                } else {
                    let total = clip.samples.len() / CHANNELS;
                    let offset = (position - clip.start).min(total as i128) as usize;
                    let frames = remaining.min(total - offset);
                    samples.extend_from_slice(
                        &clip.samples[offset * CHANNELS..(offset + frames) * CHANNELS],
                    );
                    if offset + frames == total {
                        self.held = None;
                    }
                    frames
                }
            } else {
                // The media timeline can extend beyond the audio stream.
                // Explicit source silence keeps video aligned through that tail.
                let end = self.duration_micros.map_or(position, |duration| {
                    micros_frames(duration.saturating_sub(self.target_micros))
                });
                let frames = (end - position).clamp(0, remaining as i128) as usize;
                samples.resize(samples.len() + frames * CHANNELS, 0);
                if frames == 0 {
                    break;
                }
                frames
            };
            self.submitted = self
                .submitted
                .checked_add(frames as u64)
                .ok_or_else(|| invalid("audio sequence exhausted"))?;
        }
        if samples.is_empty() && !self.ended {
            return Err(invalid("audio preroll exceeded its decode-step budget"));
        }
        Ok((!samples.is_empty()).then_some(Block {
            first_frame,
            samples,
        }))
    }

    fn decode_clip(&mut self) -> io::Result<Option<Clip>> {
        let mut decoded = frame::Audio::empty();
        for _ in 0..4096 {
            match self.decoder.receive_frame(&mut decoded) {
                Ok(()) => return self.convert(&decoded).map(Some),
                Err(ffmpeg::Error::Eof) => {
                    self.ended = true;
                    return Ok(None);
                }
                Err(error) if again(error) => {}
                Err(error) => return Err(io::Error::other(error)),
            }
            if let Some(packet) = self.pending.take() {
                match self.decoder.send_packet(&packet) {
                    Ok(()) => (),
                    Err(error) if again(error) => self.pending = Some(packet),
                    Err(error) => return Err(io::Error::other(error)),
                }
            } else if self.draining {
                return Err(invalid("audio decoder did not drain"));
            } else {
                let mut packet = ffmpeg::Packet::empty();
                match packet.read(self.input) {
                    Ok(()) if packet.stream() == self.index => self.pending = Some(packet),
                    Ok(()) => (),
                    Err(ffmpeg::Error::Eof) => {
                        self.decoder.send_eof().map_err(io::Error::other)?;
                        self.draining = true;
                    }
                    Err(error) => return Err(io::Error::other(error)),
                }
            }
        }
        Err(invalid(
            "audio packet traversal exceeded its decode-step budget",
        ))
    }

    fn convert(&mut self, input: &frame::Audio) -> io::Result<Clip> {
        let layout = input.channel_layout();
        if input.rate() != RATE as u32
            || input.samples() == 0
            || input.samples() > MAX_DECODED_FRAMES
            || !(1..=8).contains(&input.channels())
            || layout.channels() != i32::from(input.channels())
        {
            return Err(invalid(
                "Opus frame has an unqualified sample rate, layout or size",
            ));
        }
        if self.resampler.as_ref().is_none_or(|context| {
            context.input().format != input.format() || context.input().channel_layout != layout
        }) {
            self.resampler = Some(
                software::resampling::Context::get(
                    input.format(),
                    layout,
                    RATE as u32,
                    PCM,
                    ChannelLayout::STEREO,
                    RATE as u32,
                )
                .map_err(io::Error::other)?,
            );
        }
        let context = self.resampler.as_mut().expect("qualified PCM resampler");
        let mut output = frame::Audio::empty();
        context.run(input, &mut output).map_err(io::Error::other)?;
        if output.samples() != input.samples() || output.format() != PCM || output.channels() != 2 {
            return Err(invalid(
                "Opus conversion did not preserve the sample timeline",
            ));
        }
        let bytes = output
            .data(0)
            .get(..output.samples() * 4)
            .ok_or_else(|| invalid("Opus conversion returned short PCM"))?;
        let samples = bytes
            .chunks_exact(2)
            .map(|pair| i16::from_ne_bytes([pair[0], pair[1]]))
            .collect();
        let timestamp = input
            .timestamp()
            .or_else(|| input.pts())
            .ok_or_else(|| invalid("Opus frame lacks a presentation timestamp"))?;
        let raw = i128::from(timestamp) * i128::from(self.time_base.numerator()) * RATE;
        let start = raw.div_euclid(i128::from(self.time_base.denominator()))
            - (i128::from(self.origin_micros) * RATE).div_euclid(1_000_000)
            - micros_frames(self.target_micros);
        Ok(Clip { start, samples })
    }
}

fn micros_frames(micros: u64) -> i128 {
    (i128::from(micros) * RATE).div_euclid(1_000_000)
}

fn again(error: ffmpeg::Error) -> bool {
    matches!(error, ffmpeg::Error::Other {errno} if errno == ffmpeg::error::EAGAIN)
}

fn invalid(detail: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, detail)
}

fn unsupported(detail: &str) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, detail)
}

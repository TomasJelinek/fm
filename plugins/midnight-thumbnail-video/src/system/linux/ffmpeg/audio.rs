//! Audio through the distribution's FFmpeg: decoded, mixed down and resampled to 48 kHz stereo
//! 16-bit PCM by its libswresample. The shared timeline turns the clips into playback blocks.
use super::{
    AV_CH_LAYOUT_STEREO, AV_CHANNEL_ORDER_NATIVE, AV_SAMPLE_FMT_S16, AVERROR_EAGAIN, AVERROR_EOF,
    Api, Channels, Input, Selected, field, input::Stream, video::Decoder, video::seek_ticks,
};
use crate::{
    media::{AudioTrack, Block},
    system::pcm::{self, Clip, Timeline},
};
use std::{
    ffi::{c_int, c_void},
    io, ptr,
};

/// Packets or decoder steps one decode may take.
const STEPS: usize = 4096;
/// Largest decoded frame accepted, in samples per channel (FLAC allows 65535).
const MAX_FRAME_SAMPLES: c_int = 65_536;
/// Decoding restarts this long before a seek target; the timeline trims the preroll.
const PREROLL_MICROS: i64 = 80_000;

struct Resampler {
    api: &'static Api,
    context: *mut c_void,
    key: (c_int, c_int, c_int, u64),
}

impl Drop for Resampler {
    fn drop(&mut self) {
        // SAFETY: a context this resampler owns.
        unsafe { (self.api.swr_free)(&mut self.context) };
    }
}

/// `AVChannelLayout`, 24 bytes in every series that has it (checked in the layout tests).
#[repr(C, align(8))]
struct ChannelLayout([u8; 24]);

impl ChannelLayout {
    fn native(channels: c_int, mask: u64) -> Self {
        let mut layout = Self([0; 24]);
        layout.0[0..4].copy_from_slice(&AV_CHANNEL_ORDER_NATIVE.to_ne_bytes());
        layout.0[4..8].copy_from_slice(&channels.to_ne_bytes());
        layout.0[8..16].copy_from_slice(&mask.to_ne_bytes());
        layout
    }
}

/// The track's status and, when this FFmpeg decodes it, a decoder.
pub(in super::super) enum Opened<'s> {
    Absent,
    Unqualified,
    Qualified(Box<Audio<'s>>),
}

pub(in super::super) struct Audio<'s> {
    state: State<'s>,
    timeline: Timeline,
}

struct State<'s> {
    input: Input<'s>,
    stream: Stream,
    decoder: Decoder,
    resampler: Option<Resampler>,
    start: i64,
    pending: bool,
    draining: bool,
    drained: bool,
    flushed: bool,
    next_nanos: i64,
    failed: bool,
}

impl<'s> Audio<'s> {
    pub(in super::super) fn open(input: Input<'s>, duration_micros: Option<u64>) -> Opened<'s> {
        let Selected::Stream(stream) = input.audio() else {
            return Opened::Absent;
        };
        if stream.time_base.0 <= 0 || stream.time_base.1 <= 0 {
            return Opened::Unqualified;
        }
        let Ok(decoder) = Decoder::open(input.api, &stream) else {
            return Opened::Unqualified;
        };
        let start = input.start_micros();
        Opened::Qualified(Box::new(Self {
            state: State {
                input,
                stream,
                decoder,
                resampler: None,
                start,
                pending: false,
                draining: false,
                drained: false,
                flushed: false,
                next_nanos: 0,
                failed: false,
            },
            timeline: Timeline::new(duration_micros),
        }))
    }

    pub(in super::super) fn track(opened: &Opened<'_>) -> AudioTrack {
        match opened {
            Opened::Absent => AudioTrack::Absent,
            Opened::Unqualified => AudioTrack::Unqualified,
            Opened::Qualified(_) => AudioTrack::Qualified,
        }
    }

    pub(in super::super) fn seek(&mut self, target_micros: u64) -> io::Result<()> {
        let state = &mut self.state;
        if state.failed {
            return Err(io::Error::other("Audio ended after a failure"));
        }
        let absolute = i64::try_from(target_micros)
            .ok()
            .and_then(|target| {
                state
                    .start
                    .checked_add(target.saturating_sub(PREROLL_MICROS))
            })
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "Audio seek is out of range")
            })?;
        let ticks = seek_ticks(absolute, state.stream.time_base)?;
        let index = c_int::try_from(state.stream.index).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "Audio stream index too large")
        })?;
        if let Err(error) = state.input.seek(index, i64::MIN, ticks) {
            state.failed = true;
            return Err(error);
        }
        state.decoder.flush();
        if state.pending {
            state.decoder.unref_packet();
            state.pending = false;
        }
        state.resampler = None;
        state.draining = false;
        state.drained = false;
        state.flushed = false;
        self.timeline.seek(target_micros);
        Ok(())
    }

    pub(in super::super) fn next(&mut self, credit: u16) -> io::Result<Option<Block>> {
        let Self { state, timeline } = self;
        if state.failed {
            return Err(io::Error::other("Audio ended after a failure"));
        }
        let result = timeline.next(credit, || state.decode_clip());
        if result.is_err() {
            state.failed = true;
        }
        result
    }
}

impl State<'_> {
    fn decode_clip(&mut self) -> io::Result<Option<Clip>> {
        for _ in 0..STEPS {
            if self.drained {
                return self.flush_resampler();
            }
            let status = self.decoder.receive();
            if status == 0 {
                if let Some(clip) = self.convert()? {
                    return Ok(Some(clip));
                }
                continue;
            }
            if status == AVERROR_EOF {
                self.drained = true;
                continue;
            }
            if status != AVERROR_EAGAIN {
                return Err(io::Error::other("Audio decoding failed"));
            }
            if self.pending {
                match self.decoder.send(self.decoder.packet) {
                    0 => {
                        self.decoder.unref_packet();
                        self.pending = false;
                    }
                    AVERROR_EAGAIN => {}
                    _ => return Err(io::Error::other("Audio decoding failed")),
                }
                continue;
            }
            if self.draining {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Audio decoder did not drain",
                ));
            }
            match self.input.read(self.decoder.packet)? {
                0 if self.decoder.packet_stream() == Some(self.stream.index) => self.pending = true,
                0 => self.decoder.unref_packet(),
                AVERROR_EOF => {
                    self.decoder.send(ptr::null());
                    self.draining = true;
                }
                _ => return Err(io::Error::other("Audio demuxing failed")),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Audio packet traversal exceeded its decode-step budget",
        ))
    }

    /// Samples the resampler still holds at the end of the stream.
    fn flush_resampler(&mut self) -> io::Result<Option<Clip>> {
        if self.flushed {
            return Ok(None);
        }
        self.flushed = true;
        let Some(resampler) = &self.resampler else {
            return Ok(None);
        };
        let api = resampler.api;
        let mut samples = vec![0i16; 4096 * pcm::CHANNELS];
        let output = [samples.as_mut_ptr().cast::<u8>()];
        // SAFETY: an initialized resampler; a null input drains it into the 4096-frame buffer.
        let count =
            unsafe { (api.swr_convert)(resampler.context, output.as_ptr(), 4096, ptr::null(), 0) };
        if count <= 0 {
            return Ok(None);
        }
        samples.truncate(count as usize * pcm::CHANNELS);
        Ok(Some(Clip {
            start_nanos: self.next_nanos,
            samples,
        }))
    }

    fn convert(&mut self) -> io::Result<Option<Clip>> {
        let api = self.decoder_api();
        let layout = api.layout;
        let frame = self.decoder.frame;
        // SAFETY: the decoder's frame holds a decoded audio frame; fields through this series'
        // offsets.
        let (format, samples, rate, data) = unsafe {
            (
                field::<c_int>(frame, layout.frame_format).unwrap_or(-1),
                field::<c_int>(frame, layout.frame_nb_samples).unwrap_or(0),
                field::<c_int>(frame, layout.frame_sample_rate).unwrap_or(0),
                field::<*const *const u8>(frame, layout.frame_extended_data).unwrap_or(ptr::null()),
            )
        };
        if samples == 0 {
            return Ok(None);
        }
        if !(1..=MAX_FRAME_SAMPLES).contains(&samples)
            || !(8_000..=384_000).contains(&rate)
            || format < 0
            || data.is_null()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Audio frame has an unsupported rate, format or size",
            ));
        }
        let (channels, mask, native) = self.frame_channels(frame)?;
        let key = (format, rate, channels, mask);
        if self
            .resampler
            .as_ref()
            .is_none_or(|resampler| resampler.key != key)
        {
            self.resampler = Some(self.resampler(key, native)?);
        }
        let resampler = self.resampler.as_ref().expect("resampler created above");
        // SAFETY: an initialized resampler.
        let capacity = unsafe { (api.swr_get_out_samples)(resampler.context, samples) };
        if capacity < 0 {
            return Err(io::Error::other("Audio resampling failed"));
        }
        let capacity = capacity.max(samples) + 256;
        let mut output = vec![0i16; capacity as usize * pcm::CHANNELS];
        let planes = [output.as_mut_ptr().cast::<u8>()];
        // SAFETY: input planes of the decoded frame (its own format and channel count), an
        // interleaved stereo output buffer of `capacity` frames.
        let count = unsafe {
            (api.swr_convert)(resampler.context, planes.as_ptr(), capacity, data, samples)
        };
        if count < 0 {
            return Err(io::Error::other("Audio resampling failed"));
        }
        output.truncate(count as usize * pcm::CHANNELS);
        let timestamp = self.decoder.timestamp().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Audio frame has no presentation timestamp",
            )
        })?;
        let (numerator, denominator) = self.stream.time_base;
        let nanos = (i128::from(timestamp) * i128::from(numerator) * 1_000_000_000)
            .div_euclid(i128::from(denominator))
            - i128::from(self.start) * 1_000;
        let start_nanos = i64::try_from(nanos)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Audio timestamp overflow"))?;
        self.next_nanos =
            start_nanos.saturating_add((i64::from(count) * 1_000_000_000) / i64::from(pcm::RATE));
        if output.is_empty() {
            // The resampler holds the first samples back; they come with the next frame.
            return Ok(None);
        }
        Ok(Some(Clip {
            start_nanos,
            samples: output,
        }))
    }

    fn decoder_api(&self) -> &'static Api {
        self.input.api
    }

    /// Channel count, channel mask (0 when not a native-order layout) and whether the frame's
    /// own layout can be handed to the resampler.
    fn frame_channels(&self, frame: *mut c_void) -> io::Result<(c_int, u64, bool)> {
        let layout = self.decoder_api().layout;
        // SAFETY: a decoded frame; the fields exist in the series the API generation says.
        let (channels, mask, native) = unsafe {
            match layout.frame_ch_layout {
                Some(offset) => {
                    let base = frame.cast::<u8>().add(offset).cast::<c_void>();
                    let order = field::<c_int>(base, layout.ch_layout_order).unwrap_or(0);
                    let channels = field::<c_int>(base, layout.ch_layout_nb_channels).unwrap_or(0);
                    let mask = field::<u64>(base, layout.ch_layout_mask).unwrap_or(0);
                    let native = order == AV_CHANNEL_ORDER_NATIVE;
                    (channels, if native { mask } else { 0 }, native)
                }
                None => {
                    let channels = field::<c_int>(frame, layout.frame_channels).unwrap_or(0);
                    let mask = field::<u64>(frame, layout.frame_channel_layout).unwrap_or(0);
                    (channels, mask, mask != 0)
                }
            }
        };
        if !(1..=8).contains(&channels) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Audio has an unsupported number of channels",
            ));
        }
        Ok((
            channels,
            mask,
            native && mask.count_ones() == channels as u32,
        ))
    }

    fn resampler(
        &self,
        (format, rate, channels, mask): (c_int, c_int, c_int, u64),
        native: bool,
    ) -> io::Result<Resampler> {
        let api = self.decoder_api();
        let mut context: *mut c_void = ptr::null_mut();
        // SAFETY: the channel-layout generation resolved for this series; layouts passed by
        // pointer live across the call and are released after it.
        unsafe {
            match &api.channels {
                Channels::Mask {
                    av_get_default_channel_layout,
                    swr_alloc_set_opts,
                } => {
                    let input = if native {
                        mask as i64
                    } else {
                        av_get_default_channel_layout(channels)
                    };
                    context = swr_alloc_set_opts(
                        ptr::null_mut(),
                        AV_CH_LAYOUT_STEREO as i64,
                        AV_SAMPLE_FMT_S16,
                        pcm::RATE as c_int,
                        input,
                        format,
                        rate,
                        0,
                        ptr::null_mut(),
                    );
                }
                Channels::Layout {
                    av_channel_layout_default,
                    av_channel_layout_uninit,
                    swr_alloc_set_opts2,
                    ..
                } => {
                    let output = ChannelLayout::native(2, AV_CH_LAYOUT_STEREO);
                    let mut input = if native {
                        ChannelLayout::native(channels, mask)
                    } else {
                        let mut layout = ChannelLayout([0; 24]);
                        av_channel_layout_default((&raw mut layout).cast(), channels);
                        layout
                    };
                    let status = swr_alloc_set_opts2(
                        &mut context,
                        (&raw const output).cast(),
                        AV_SAMPLE_FMT_S16,
                        pcm::RATE as c_int,
                        (&raw const input).cast(),
                        format,
                        rate,
                        0,
                        ptr::null_mut(),
                    );
                    av_channel_layout_uninit((&raw mut input).cast());
                    if status < 0 {
                        context = ptr::null_mut();
                    }
                }
            }
        }
        if context.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Audio conversion is unavailable for this format",
            ));
        }
        let resampler = Resampler {
            api,
            context,
            key: (format, rate, channels, mask),
        };
        // SAFETY: a configured resampler.
        if unsafe { (api.swr_init)(resampler.context) } < 0 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Audio conversion is unavailable for this format",
            ));
        }
        Ok(resampler)
    }
}

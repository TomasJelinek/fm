//! Video through the distribution's FFmpeg: the decoder, colour conversion and scaling with
//! its libswscale, thumbnails sampled like the bundled profile, and the playback cursor.
use super::{
    AV_NOPTS_VALUE, AV_PIX_FMT_RGBA, AV_TIME_BASE, AVERROR_EAGAIN, AVERROR_EOF, Api, Input,
    SWS_BILINEAR, SWS_CS_ITU601, SWS_CS_ITU709, Selected, field, input::Stream, set_field,
};
use crate::{
    media::{Codec, Frame, Metadata, NoDecoder, OutputBounds, Tried, output_length},
    system::linux::picture::{self, Colour},
};
use std::{
    ffi::{c_int, c_void},
    io, ptr,
};

/// Decoded pixels may not exceed this many (32 Mi), as in the bundled profile.
const MAX_SOURCE_PIXELS: u64 = 32 * 1024 * 1024;
/// Packets or decoder steps one call may take, including skipped preroll.
const STEPS: usize = 4096;

/// An opened decoder for one stream, with its frames, packet and cached scaler.
pub(super) struct Decoder {
    api: &'static Api,
    pub(super) context: *mut c_void,
    pub(super) frame: *mut c_void,
    /// The latest frame before a thumbnail target, kept by moving references, not pixels.
    earlier: *mut c_void,
    pub(super) packet: *mut c_void,
    scaler: Option<Scaler>,
}

struct Scaler {
    key: (c_int, c_int, c_int, u32, u32, i32),
    context: *mut c_void,
}

/// The source's video codec has no usable decoder in the distribution's FFmpeg.
pub(super) fn no_decoder(api: &Api, codec: c_int) -> io::Error {
    NoDecoder {
        codec: Codec::from_ffmpeg_name(&api.codec_name(codec)),
        tried: Tried::DISTRIBUTION_FFMPEG,
    }
    .into()
}

impl Decoder {
    /// Open the decoder for `stream`: single-threaded (the sandbox has no threads), bounded
    /// pixels, packet time base set so decoders apply priming and skip samples.
    pub(super) fn open(api: &'static Api, stream: &Stream) -> io::Result<Self> {
        let layout = api.layout;
        // SAFETY: each native object is owned by `decoder` as soon as it exists; Drop frees it.
        unsafe {
            let codec = (api.avcodec_find_decoder)(stream.codec);
            if codec.is_null() {
                return Err(no_decoder(api, stream.codec));
            }
            let decoder = Self {
                api,
                context: (api.avcodec_alloc_context3)(codec),
                frame: (api.av_frame_alloc)(),
                earlier: (api.av_frame_alloc)(),
                packet: (api.av_packet_alloc)(),
                scaler: None,
            };
            if decoder.context.is_null()
                || decoder.frame.is_null()
                || decoder.earlier.is_null()
                || decoder.packet.is_null()
            {
                return Err(io::Error::other("Decoder allocation failed"));
            }
            if (api.avcodec_parameters_to_context)(decoder.context, stream.parameters) < 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Invalid stream codec parameters",
                ));
            }
            set_field::<c_int>(decoder.context, layout.codec_thread_count, 1);
            set_field::<c_int>(decoder.context, layout.codec_thread_type, 0);
            set_field::<i64>(
                decoder.context,
                layout.codec_max_pixels,
                MAX_SOURCE_PIXELS as i64,
            );
            set_field::<[c_int; 2]>(
                decoder.context,
                layout.codec_pkt_timebase,
                [stream.time_base.0, stream.time_base.1],
            );
            if (api.avcodec_open2)(decoder.context, codec, ptr::null_mut()) < 0 {
                // A decoder that cannot open here (for example one that needs hardware) is as
                // good as absent: another decoder may still play the file.
                return Err(no_decoder(api, stream.codec));
            }
            Ok(decoder)
        }
    }

    pub(super) fn send(&mut self, packet: *const c_void) -> c_int {
        // SAFETY: an opened context; packet is null (drain) or a live packet.
        unsafe { (self.api.avcodec_send_packet)(self.context, packet) }
    }

    pub(super) fn receive(&mut self) -> c_int {
        // SAFETY: an opened context and its allocated frame.
        unsafe { (self.api.avcodec_receive_frame)(self.context, self.frame) }
    }

    pub(super) fn flush(&mut self) {
        // SAFETY: an opened context.
        unsafe { (self.api.avcodec_flush_buffers)(self.context) };
    }

    pub(super) fn unref_packet(&mut self) {
        // SAFETY: an allocated packet.
        unsafe { (self.api.av_packet_unref)(self.packet) };
    }

    pub(super) fn packet_stream(&self) -> Option<usize> {
        // SAFETY: an allocated packet that av_read_frame filled.
        let index = unsafe { field::<c_int>(self.packet, self.api.layout.packet_stream_index) };
        index.and_then(|index| usize::try_from(index).ok())
    }

    /// The decoded frame's timestamp: FFmpeg's best effort, else its pts.
    pub(super) fn timestamp(&self) -> Option<i64> {
        let layout = self.api.layout;
        // SAFETY: the decoder's frame holds the frame avcodec_receive_frame returned.
        let best = unsafe { field::<i64>(self.frame, layout.frame_best_effort_timestamp) };
        // SAFETY: as above.
        let pts = unsafe { field::<i64>(self.frame, layout.frame_pts) };
        best.filter(|time| *time != AV_NOPTS_VALUE)
            .or(pts.filter(|time| *time != AV_NOPTS_VALUE))
    }

    /// Keep the decoded frame as the latest one before the target (references move; the
    /// previous one is released).
    fn keep_earlier(&mut self) {
        // SAFETY: both frames are allocated and owned by this decoder.
        unsafe {
            (self.api.av_frame_unref)(self.earlier);
            (self.api.av_frame_move_ref)(self.earlier, self.frame);
        }
    }

    /// The decoded frame as oriented sRGB RGBA within `bounds`.
    pub(super) fn pixels(
        &mut self,
        bounds: OutputBounds,
        rotation: i32,
    ) -> io::Result<picture::Pixels> {
        self.convert(self.frame, bounds, rotation)
    }

    fn convert(
        &mut self,
        frame: *mut c_void,
        bounds: OutputBounds,
        rotation: i32,
    ) -> io::Result<picture::Pixels> {
        let api = self.api;
        let layout = api.layout;
        // SAFETY: `frame` is one of this decoder's frames holding a decoded picture; the prefix
        // fields and the colour fields are read through this series' offsets.
        let (width, height, format, data, linesize, colour) = unsafe {
            (
                field::<c_int>(frame, layout.frame_width).unwrap_or(0),
                field::<c_int>(frame, layout.frame_height).unwrap_or(0),
                field::<c_int>(frame, layout.frame_format).unwrap_or(-1),
                field::<[*const u8; 8]>(frame, layout.frame_data).unwrap_or([ptr::null(); 8]),
                field::<[c_int; 8]>(frame, layout.frame_linesize).unwrap_or([0; 8]),
                Colour {
                    primaries: field::<c_int>(frame, layout.frame_color_primaries).unwrap_or(2),
                    transfer: field::<c_int>(frame, layout.frame_color_trc).unwrap_or(2),
                    matrix: field::<c_int>(frame, layout.frame_colorspace).unwrap_or(2),
                    full_range: field::<c_int>(frame, layout.frame_color_range).unwrap_or(0) == 2,
                },
            )
        };
        let (Ok(source_width), Ok(source_height)) = (u32::try_from(width), u32::try_from(height))
        else {
            return Err(io::Error::other("Decoded video dimensions are invalid"));
        };
        if source_width == 0
            || source_height == 0
            || u64::from(source_width) * u64::from(source_height) > MAX_SOURCE_PIXELS
            || data[0].is_null()
        {
            return Err(io::Error::other("Decoded video dimensions exceed bound"));
        }
        let rgb = matches!(format, 2 | 3 | 25 | 26 | 27 | 28);
        let conversion = picture::conversion(colour, rgb, source_height)?;
        let (target_width, target_height) =
            picture::fit(source_width, source_height, bounds, rotation)?;
        let key = (
            width,
            height,
            format,
            target_width,
            target_height,
            conversion.matrix,
        );
        if self.scaler.as_ref().is_none_or(|scaler| scaler.key != key) {
            if let Some(old) = self.scaler.take() {
                // SAFETY: a scaler this decoder created.
                unsafe { (api.sws_free_context)(old.context) };
            }
            // SAFETY: valid dimensions and formats; no filters or parameters.
            let context = unsafe {
                (api.sws_get_context)(
                    width,
                    height,
                    format,
                    target_width as c_int,
                    target_height as c_int,
                    AV_PIX_FMT_RGBA,
                    SWS_BILINEAR,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null(),
                )
            };
            if context.is_null() {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "This video's pixel format cannot be converted",
                ));
            }
            self.scaler = Some(Scaler { key, context });
            let coefficients = if conversion.matrix == 601 {
                SWS_CS_ITU601
            } else {
                SWS_CS_ITU709
            };
            // SAFETY: a fresh scaler; the coefficient table is a static libswscale table.
            let status = unsafe {
                let table = (api.sws_get_coefficients)(coefficients);
                (api.sws_set_colorspace_details)(
                    context,
                    table,
                    c_int::from(conversion.full_range),
                    table,
                    1,
                    0,
                    1 << 16,
                    1 << 16,
                )
            };
            if status < 0 {
                return Err(io::Error::other("Video colour conversion failed"));
            }
        }
        let scaler = self.scaler.as_ref().expect("scaler created above").context;
        // Rows padded to 64 bytes plus slack: libswscale may write whole SIMD blocks.
        let stride = (target_width as usize * 4).next_multiple_of(64);
        let mut scaled = vec![0u8; stride * target_height as usize + 64];
        let destination = [
            scaled.as_mut_ptr(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
        ];
        let destination_stride = [stride as c_int, 0, 0, 0];
        // SAFETY: source planes and strides of the decoded frame; destination rows fit the
        // padded buffer above.
        let rows = unsafe {
            (api.sws_scale)(
                scaler,
                data.as_ptr(),
                linesize.as_ptr(),
                0,
                height,
                destination.as_ptr(),
                destination_stride.as_ptr(),
            )
        };
        if rows != target_height as c_int {
            return Err(io::Error::other("Video scaling failed"));
        }
        picture::finish(
            &scaled,
            stride,
            target_width,
            target_height,
            rotation,
            conversion.transfer_709,
        )
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        let api = self.api;
        // SAFETY: each pointer is null or exclusively owned by this decoder.
        unsafe {
            if let Some(scaler) = self.scaler.take() {
                (api.sws_free_context)(scaler.context);
            }
            if !self.packet.is_null() {
                (api.av_packet_free)(&mut self.packet);
            }
            if !self.frame.is_null() {
                (api.av_frame_free)(&mut self.frame);
            }
            if !self.earlier.is_null() {
                (api.av_frame_free)(&mut self.earlier);
            }
            if !self.context.is_null() {
                (api.avcodec_free_context)(&mut self.context);
            }
        }
    }
}

/// The selected video stream of an input, its decoder and orientation.
struct Opened {
    stream: Stream,
    decoder: Decoder,
    rotation: i32,
    width: u32,
    height: u32,
}

fn open_video(input: &Input<'_>) -> io::Result<Opened> {
    let api = input.api;
    let Selected::Stream(stream) = input.video() else {
        return Err(super::super::no_video());
    };
    // SAFETY: live codec parameters of this input.
    let (width, height) = unsafe {
        (
            field::<c_int>(stream.parameters, api.layout.par_width).unwrap_or(0),
            field::<c_int>(stream.parameters, api.layout.par_height).unwrap_or(0),
        )
    };
    let decoder = Decoder::open(api, &stream)?;
    let rotation = input.rotation(&stream)?;
    let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height)) else {
        return Err(io::Error::other("Video dimensions exceed decoder bound"));
    };
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_SOURCE_PIXELS {
        return Err(io::Error::other("Video dimensions exceed decoder bound"));
    }
    if stream.time_base.0 <= 0 || stream.time_base.1 <= 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Video stream has no usable presentation time base",
        ));
    }
    Ok(Opened {
        stream,
        decoder,
        rotation,
        width,
        height,
    })
}

/// One sampled thumbnail, as the bundled profile samples it: between 10% and 85% of the
/// duration counted from the container start, the first frame at or after the target, else
/// the latest frame decoded before it.
pub(in super::super) struct Thumbnail;

impl Thumbnail {
    pub(in super::super) fn decode(
        input: &mut Input<'_>,
        bounds: OutputBounds,
        sample: u8,
    ) -> io::Result<picture::Pixels> {
        let Opened {
            stream,
            mut decoder,
            rotation,
            ..
        } = open_video(input)?;
        let duration = input.duration_micros().unwrap_or(0) as i64;
        let offset = duration.saturating_mul(10 + i64::from(sample) * 15) / 100;
        let target = input.start_micros().saturating_add(offset);
        if offset > 0 {
            input.seek(-1, i64::MIN, target)?;
            decoder.flush();
        }
        let (numerator, denominator) = stream.time_base;
        let target_ticks = i64::try_from(
            i128::from(target) * i128::from(denominator)
                / (i128::from(AV_TIME_BASE) * i128::from(numerator)),
        )
        .map_err(|_| io::Error::other("Video timestamp exceeds supported range"))?;
        let mut has_earlier = false;
        let mut ended = false;
        for _ in 0..STEPS {
            if !ended {
                let read = input.read(decoder.packet)?;
                if read == 0 {
                    let ours = decoder.packet_stream() == Some(stream.index);
                    let status = if ours {
                        decoder.send(decoder.packet)
                    } else {
                        0
                    };
                    decoder.unref_packet();
                    if status < 0 && status != AVERROR_EAGAIN {
                        return Err(io::Error::other("Video decoding failed"));
                    }
                } else if read == AVERROR_EOF {
                    // Signal the end once; the decoder then drains its frames.
                    decoder.send(ptr::null());
                    ended = true;
                } else {
                    return Err(io::Error::other("Video demuxing failed"));
                }
            }
            loop {
                let status = decoder.receive();
                if status == AVERROR_EAGAIN || status == AVERROR_EOF {
                    break;
                }
                if status < 0 {
                    return Err(io::Error::other("Video decoding failed"));
                }
                if decoder.timestamp().is_some_and(|time| time < target_ticks) {
                    decoder.keep_earlier();
                    has_earlier = true;
                    continue;
                }
                return decoder.pixels(bounds, rotation);
            }
            if ended {
                break;
            }
        }
        // A header can overstate the decodable content, and sparse keyframes can exhaust the
        // packet budget: show the latest frame decoded instead.
        if has_earlier {
            let earlier = decoder.earlier;
            return decoder.convert(earlier, bounds, rotation);
        }
        Err(io::Error::other(
            "No representative frame within decode budget",
        ))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Reading,
    Draining,
    Ended,
    Failed,
}

/// Consecutive presentation frames of the selected video, for playback.
pub(in super::super) struct Video<'s> {
    input: Input<'s>,
    stream: Stream,
    decoder: Decoder,
    rotation: i32,
    bounds: OutputBounds,
    metadata: Metadata,
    start: i64,
    minimum_time: i64,
    pending: bool,
    phase: Phase,
    eof_sent: bool,
}

impl<'s> Video<'s> {
    pub(in super::super) fn open(input: Input<'s>, bounds: OutputBounds) -> io::Result<Self> {
        output_length(bounds.width, bounds.height)?;
        let Opened {
            stream,
            decoder,
            rotation,
            width,
            height,
        } = open_video(&input)?;
        let (width, height) = if matches!(rotation, 90 | 270) {
            (height, width)
        } else {
            (width, height)
        };
        let metadata = Metadata {
            duration_micros: input.duration_micros(),
            width,
            height,
        };
        let start = input.start_micros();
        Ok(Self {
            input,
            stream,
            decoder,
            rotation,
            bounds,
            metadata,
            start,
            minimum_time: 0,
            pending: false,
            phase: Phase::Reading,
            eof_sent: false,
        })
    }

    pub(in super::super) fn metadata(&self) -> Metadata {
        self.metadata
    }

    /// The next presentation frame. End of media is sticky until a seek; any error is final.
    pub(in super::super) fn next_frame(&mut self) -> io::Result<Option<Frame>> {
        match self.phase {
            Phase::Failed => return Err(ended()),
            Phase::Ended => return Ok(None),
            Phase::Reading | Phase::Draining => {}
        }
        let result = self.next_inner();
        if result.is_err() {
            self.fail();
        }
        result
    }

    fn fail(&mut self) {
        self.phase = Phase::Failed;
        if self.pending {
            self.decoder.unref_packet();
            self.pending = false;
        }
    }

    fn next_inner(&mut self) -> io::Result<Option<Frame>> {
        for _ in 0..STEPS {
            let status = self.decoder.receive();
            if status == 0 {
                let time = self.presentation_time()?;
                if time < self.minimum_time {
                    continue;
                }
                let pixels = self.decoder.pixels(self.bounds, self.rotation)?;
                return Ok(Some(Frame {
                    presentation_micros: time,
                    width: pixels.width,
                    height: pixels.height,
                    rgba: pixels.rgba,
                }));
            }
            if status == AVERROR_EOF {
                self.phase = Phase::Ended;
                return Ok(None);
            }
            if status != AVERROR_EAGAIN {
                return Err(io::Error::other("Video decoding failed"));
            }
            if self.pending {
                match self.decoder.send(self.decoder.packet) {
                    0 => {
                        self.decoder.unref_packet();
                        self.pending = false;
                    }
                    // Keep the unsent packet: receiving a frame must never lose its successor.
                    AVERROR_EAGAIN => {}
                    _ => return Err(io::Error::other("Video decoding failed")),
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
                match self.decoder.send(ptr::null()) {
                    0 => self.eof_sent = true,
                    AVERROR_EAGAIN => {}
                    _ => return Err(io::Error::other("Video decoding failed")),
                }
                continue;
            }
            match self.input.read(self.decoder.packet)? {
                0 if self.decoder.packet_stream() == Some(self.stream.index) => self.pending = true,
                0 => self.decoder.unref_packet(),
                AVERROR_EOF => self.phase = Phase::Draining,
                _ => return Err(io::Error::other("Video demuxing failed")),
            }
        }
        Err(io::Error::other("Video decode work budget exhausted"))
    }

    fn presentation_time(&self) -> io::Result<i64> {
        let timestamp = self.decoder.timestamp().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Video frame has no presentation timestamp",
            )
        })?;
        let (numerator, denominator) = self.stream.time_base;
        let time = (i128::from(timestamp) * i128::from(numerator) * 1_000_000)
            .div_euclid(i128::from(denominator))
            - i128::from(self.start);
        i64::try_from(time).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Video presentation timestamp overflow",
            )
        })
    }

    /// Seek in container-relative microseconds, then suppress every frame before the target.
    /// Range refusals leave the cursor intact; a native seek failure is final.
    pub(in super::super) fn seek(&mut self, target_micros: u64) -> io::Result<()> {
        if self.phase == Phase::Failed {
            return Err(ended());
        }
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
        let ticks = seek_ticks(target.1, self.stream.time_base)?;
        let index = c_int::try_from(self.stream.index).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Video stream index exceeds native range",
            )
        })?;
        if let Err(error) = self.input.seek(index, i64::MIN, ticks) {
            self.fail();
            return Err(error);
        }
        self.decoder.flush();
        if self.pending {
            self.decoder.unref_packet();
            self.pending = false;
        }
        self.minimum_time = target.0;
        self.phase = Phase::Reading;
        self.eof_sent = false;
        Ok(())
    }
}

/// Microseconds of the container timeline to stream ticks, rounded down so that a seek never
/// lands after the requested time.
pub(super) fn seek_ticks(micros: i64, time_base: (c_int, c_int)) -> io::Result<i64> {
    let (numerator, denominator) = time_base;
    if numerator <= 0 || denominator <= 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid selected stream time base",
        ));
    }
    let ticks = (i128::from(micros) * i128::from(denominator))
        .div_euclid(1_000_000 * i128::from(numerator));
    i64::try_from(ticks).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Seek exceeds the selected stream timestamp range",
        )
    })
}

fn ended() -> io::Error {
    io::Error::other("Playback cursor ended after a failure")
}

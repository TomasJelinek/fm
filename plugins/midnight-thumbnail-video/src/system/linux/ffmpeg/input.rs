//! One demuxer over a brokered cursor, through a custom AVIO context. No URL, protocol or
//! secondary file is ever opened: the protocol whitelist is empty and `io_open` refuses.
use super::{
    AV_DISPOSITION_ATTACHED_PIC, AV_NOPTS_VALUE, AV_PKT_DATA_DISPLAYMATRIX, AVERROR_EOF,
    AVMEDIA_TYPE_AUDIO, AVMEDIA_TYPE_VIDEO, AVSEEK_FORCE, AVSEEK_SIZE, Api, Ffmpeg, SideData,
    field, set_field,
};
use midnight_thumbnail_sdk::ReadSeek;
use std::{
    ffi::{c_char, c_int, c_uint, c_void},
    io::{self, SeekFrom},
    panic::{AssertUnwindSafe, catch_unwind},
    ptr,
};

/// AVIO buffer; FFmpeg may ask the read callback for more than this at once.
const BUFFER: usize = 64 * 1024;
/// Largest single brokered read.
const MAX_READ: usize = 256 * 1024;
/// Containers of the extensions the video helpers are registered for (`thumbnailer.json`):
/// MP4/MOV, Matroska/WebM, AVI, Ogg and ASF (`.wmv`, `.asf`).
pub(in super::super) const FORMATS: &std::ffi::CStr =
    c"mov,mp4,m4a,3gp,3g2,mj2,matroska,webm,avi,ogg,asf";

struct Bridge<'s> {
    source: Box<dyn ReadSeek + 's>,
    length: u64,
    /// The first failed source read. A demuxer may turn it into an end of file, so it is kept
    /// here and reported instead of whatever FFmpeg makes of it.
    error: Option<io::Error>,
}

unsafe extern "C" fn read(opaque: *mut c_void, buffer: *mut u8, length: c_int) -> c_int {
    if opaque.is_null() || buffer.is_null() || length <= 0 {
        return -22;
    }
    // A short count is valid for read_packet; FFmpeg keeps reading.
    let length = (length as usize).min(MAX_READ);
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: Input owns the bridge for the AVIO context's whole life; FFmpeg passes a
        // writable buffer of at least the requested length, clamped above.
        let bridge = unsafe { &mut *opaque.cast::<Bridge<'_>>() };
        let buffer = unsafe { std::slice::from_raw_parts_mut(buffer, length) };
        if bridge.error.is_some() {
            return -5;
        }
        match bridge.source.read(buffer) {
            Ok(0) => AVERROR_EOF,
            Ok(count) => count as c_int,
            Err(error) => {
                bridge.error = Some(error);
                -5
            }
        }
    }))
    .unwrap_or(-5)
}

unsafe extern "C" fn seek(opaque: *mut c_void, offset: i64, whence: c_int) -> i64 {
    if opaque.is_null() {
        return -22;
    }
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: the exclusive callback borrows the live bridge owned by Input.
        let bridge = unsafe { &mut *opaque.cast::<Bridge<'_>>() };
        if whence == AVSEEK_SIZE {
            return bridge.length as i64;
        }
        let position = match whence & !AVSEEK_FORCE {
            0 if offset >= 0 => SeekFrom::Start(offset as u64),
            1 => SeekFrom::Current(offset),
            2 => SeekFrom::End(offset),
            _ => return -22,
        };
        match bridge.source.seek(position) {
            Ok(position) if position <= bridge.length => position as i64,
            _ => -5,
        }
    }))
    .unwrap_or(-5)
}

unsafe extern "C" fn deny_open(
    _: *mut c_void,
    _: *mut *mut c_void,
    _: *const c_char,
    _: c_int,
    _: *mut *mut c_void,
) -> c_int {
    -13
}

/// A stream's properties, read through the series' generated offsets.
#[derive(Clone, Copy, Debug)]
pub(in super::super) struct Stream {
    pub(super) pointer: *mut c_void,
    pub(super) parameters: *mut c_void,
    pub(in super::super) index: usize,
    pub(super) medium: c_int,
    pub(super) codec: c_int,
    pub(super) time_base: (c_int, c_int),
    pub(super) disposition: c_int,
}

/// The stream a decoder would use.
pub(in super::super) enum Selected {
    Stream(Stream),
    None,
}

pub(in super::super) struct Input<'s> {
    pub(super) api: &'static Api,
    context: *mut c_void,
    io: *mut c_void,
    bridge: Box<Bridge<'s>>,
}

impl<'s> Input<'s> {
    pub(in super::super) fn open(
        ffmpeg: &'static Ffmpeg,
        source: Box<dyn ReadSeek + 's>,
        length: u64,
    ) -> io::Result<Self> {
        let api = &ffmpeg.api;
        let layout = api.layout;
        if length > i64::MAX as u64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Video source length exceeds the native seek range",
            ));
        }
        let mut bridge = Box::new(Bridge {
            source,
            length,
            error: None,
        });
        // SAFETY: each allocation is owned exactly once: by `input` once it exists (its Drop
        // releases the context, the AVIO buffer and context), before that by the error paths.
        unsafe {
            let buffer = (api.av_malloc)(BUFFER).cast::<u8>();
            if buffer.is_null() {
                return Err(io::Error::other("AVIO allocation failed"));
            }
            let io = (api.avio_alloc_context)(
                buffer,
                BUFFER as c_int,
                0,
                (&raw mut *bridge).cast(),
                Some(read),
                None,
                Some(seek),
            );
            if io.is_null() {
                (api.av_free)(buffer.cast());
                return Err(io::Error::other("AVIO context allocation failed"));
            }
            let mut input = Self {
                api,
                context: ptr::null_mut(),
                io,
                bridge,
            };
            let mut context = (api.avformat_alloc_context)();
            if context.is_null() {
                return Err(io::Error::other("Format context allocation failed"));
            }
            // Setting pb before opening makes avformat_open_input mark the context custom-IO,
            // so FFmpeg never closes or replaces our AVIO context.
            set_field(context, layout.format_pb, io);
            let deny: unsafe extern "C" fn(
                *mut c_void,
                *mut *mut c_void,
                *const c_char,
                c_int,
                *mut *mut c_void,
            ) -> c_int = deny_open;
            set_field(context, layout.format_io_open, deny);
            let mut options = ptr::null_mut();
            for (key, value) in [
                (c"format_whitelist", FORMATS),
                (c"protocol_whitelist", c""),
                (c"probesize", c"1048576"),
                (c"analyzeduration", c"2000000"),
            ] {
                (api.av_dict_set)(&mut options, key.as_ptr(), value.as_ptr(), 0);
            }
            let opened =
                (api.avformat_open_input)(&mut context, ptr::null(), ptr::null(), &mut options);
            (api.av_dict_free)(&mut options);
            if opened < 0 {
                // avformat_open_input frees the context on failure.
                input.check_source()?;
                return Err(super::super::unrecognized());
            }
            input.context = context;
            let info = (api.avformat_find_stream_info)(context, ptr::null_mut());
            input.check_source()?;
            if info < 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "The container's streams could not be read",
                ));
            }
            Ok(input)
        }
    }

    /// Report a failed source read as itself (and keep it for later calls).
    pub(in super::super) fn check_source(&mut self) -> io::Result<()> {
        match &self.bridge.error {
            Some(error) => Err(io::Error::new(error.kind(), "Captured video read failed")),
            None => Ok(()),
        }
    }

    pub(super) fn streams(&self) -> Vec<Stream> {
        let layout = self.api.layout;
        // SAFETY: a live opened context; nb_streams/streams describe its stream array, whose
        // AVStream and codec parameters stay valid while the context lives.
        unsafe {
            let count = field::<c_uint>(self.context, layout.format_nb_streams).unwrap_or(0);
            let array = field::<*mut *mut c_void>(self.context, layout.format_streams)
                .unwrap_or(ptr::null_mut());
            if array.is_null() {
                return Vec::new();
            }
            (0..count as usize)
                .filter_map(|index| {
                    let pointer = *array.add(index);
                    if pointer.is_null() {
                        return None;
                    }
                    let parameters = field::<*mut c_void>(pointer, layout.stream_codecpar)?;
                    if parameters.is_null() {
                        return None;
                    }
                    let time_base = field::<[c_int; 2]>(pointer, layout.stream_time_base)?;
                    Some(Stream {
                        pointer,
                        parameters,
                        index,
                        medium: field::<c_int>(parameters, layout.par_codec_type)?,
                        codec: field::<c_int>(parameters, layout.par_codec_id)?,
                        time_base: (time_base[0], time_base[1]),
                        disposition: field::<c_int>(pointer, layout.stream_disposition)?,
                    })
                })
                .collect()
        }
    }

    fn best(&self, medium: c_int) -> Option<usize> {
        // SAFETY: a live opened context; no decoder is requested.
        let best = unsafe {
            (self.api.av_find_best_stream)(self.context, medium, -1, -1, ptr::null_mut(), 0)
        };
        usize::try_from(best).ok()
    }

    /// The main video: FFmpeg's best video stream, else the first; never an attached picture.
    pub(in super::super) fn video(&self) -> Selected {
        let streams = self.streams();
        let eligible = |stream: &&Stream| {
            stream.medium == AVMEDIA_TYPE_VIDEO
                && stream.disposition & AV_DISPOSITION_ATTACHED_PIC == 0
        };
        let best = self.best(AVMEDIA_TYPE_VIDEO);
        streams
            .iter()
            .filter(eligible)
            .find(|stream| Some(stream.index) == best)
            .or_else(|| streams.iter().find(eligible))
            .map_or(Selected::None, |stream| Selected::Stream(*stream))
    }

    /// The main audio: FFmpeg's best audio stream, else the first.
    pub(in super::super) fn audio(&self) -> Selected {
        let streams = self.streams();
        let best = self.best(AVMEDIA_TYPE_AUDIO);
        streams
            .iter()
            .filter(|stream| stream.medium == AVMEDIA_TYPE_AUDIO)
            .find(|stream| Some(stream.index) == best)
            .or_else(|| {
                streams
                    .iter()
                    .find(|stream| stream.medium == AVMEDIA_TYPE_AUDIO)
            })
            .map_or(Selected::None, |stream| Selected::Stream(*stream))
    }

    /// Container duration in microseconds; `None` when unknown or negative.
    pub(in super::super) fn duration_micros(&self) -> Option<u64> {
        // SAFETY: a live opened context.
        let duration = unsafe { field::<i64>(self.context, self.api.layout.format_duration) };
        duration
            .filter(|duration| *duration != AV_NOPTS_VALUE)
            .and_then(|duration| u64::try_from(duration).ok())
    }

    /// Container start time in microseconds; zero when unset.
    pub(super) fn start_micros(&self) -> i64 {
        // SAFETY: a live opened context.
        let start = unsafe { field::<i64>(self.context, self.api.layout.format_start_time) };
        start.filter(|start| *start != AV_NOPTS_VALUE).unwrap_or(0)
    }

    /// Clockwise rotation from the stream's display matrix (0, 90, 180 or 270).
    pub(super) fn rotation(&self, stream: &Stream) -> io::Result<i32> {
        let api = self.api;
        let layout = api.layout;
        // SAFETY: live stream and codec parameters of this context; the side-data entry
        // points are the ones resolved for this series, with their own size types.
        let matrix: Option<[i32; 9]> = unsafe {
            let (data, size) = match api.side_data {
                SideData::StreamInt(get) => {
                    let mut size: c_int = 0;
                    let data = get(stream.pointer, AV_PKT_DATA_DISPLAYMATRIX, &mut size);
                    (data.cast_const(), usize::try_from(size).unwrap_or(0))
                }
                SideData::StreamSize(get) => {
                    let mut size = 0usize;
                    let data = get(stream.pointer, AV_PKT_DATA_DISPLAYMATRIX, &mut size);
                    (data.cast_const(), size)
                }
                SideData::Parameters(get) => {
                    let side =
                        field::<*const c_void>(stream.parameters, layout.par_coded_side_data)
                            .unwrap_or(ptr::null());
                    let count = field::<c_int>(stream.parameters, layout.par_nb_coded_side_data)
                        .unwrap_or(0);
                    let entry = if side.is_null() || count <= 0 {
                        ptr::null()
                    } else {
                        get(side, count, AV_PKT_DATA_DISPLAYMATRIX)
                    };
                    if entry.is_null() {
                        (ptr::null(), 0)
                    } else {
                        (
                            field::<*const u8>(entry, layout.side_data_data).unwrap_or(ptr::null()),
                            field::<usize>(entry, layout.side_data_size).unwrap_or(0),
                        )
                    }
                }
            };
            if data.is_null() {
                None
            } else if size != 36 {
                return Err(io::Error::other("Invalid video display matrix"));
            } else {
                Some(ptr::read_unaligned(data.cast()))
            }
        };
        let Some(matrix) = matrix else {
            return Ok(0);
        };
        // Perspective, reflections and nonuniform scaling are refused, not silently ignored.
        let [a, b, c, d] = [matrix[0], matrix[1], matrix[3], matrix[4]].map(i128::from);
        let determinant = a * d - b * c;
        if matrix[2] != 0
            || matrix[5] != 0
            || matrix[8] != 1 << 30
            || determinant <= 0
            || a * a + b * b != c * c + d * d
            || a * c + b * d != 0
        {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Unsupported video display transform",
            ));
        }
        // SAFETY: nine validated matrix integers.
        let angle = -unsafe { (api.av_display_rotation_get)(matrix.as_ptr()) };
        if !angle.is_finite() || (angle - angle.round()).abs() > 0.01 {
            return Err(io::Error::other("Invalid video rotation"));
        }
        let rotation = (angle.round() as i32).rem_euclid(360);
        if !matches!(rotation, 0 | 90 | 180 | 270) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Unsupported video orientation",
            ));
        }
        Ok(rotation)
    }

    /// The next packet into `packet`: 0, AVERROR_EOF or another negative error.
    pub(super) fn read(&mut self, packet: *mut c_void) -> io::Result<c_int> {
        // SAFETY: a live context and an allocated, unreferenced packet.
        let result = unsafe { (self.api.av_read_frame)(self.context, packet) };
        self.check_source()?;
        Ok(result)
    }

    /// Seek `stream` to the keyframe at or before `timestamp` (in its time base); with stream
    /// -1, `timestamp` is in microseconds of the container timeline.
    pub(super) fn seek(&mut self, stream: c_int, minimum: i64, timestamp: i64) -> io::Result<()> {
        // SAFETY: a live context; min <= target == max permits only an earlier keyframe.
        let result = unsafe {
            (self.api.avformat_seek_file)(self.context, stream, minimum, timestamp, timestamp, 0)
        };
        self.check_source()?;
        if result < 0 {
            return Err(io::Error::other("Video seek failed"));
        }
        Ok(())
    }
}

impl Drop for Input<'_> {
    fn drop(&mut self) {
        let api = self.api;
        // SAFETY: the context (custom IO, so FFmpeg never frees pb) is closed first; then the
        // AVIO buffer FFmpeg may have replaced, then the AVIO context. The bridge outlives both.
        unsafe {
            if !self.context.is_null() {
                (api.avformat_close_input)(&mut self.context);
            }
            if !self.io.is_null() {
                if let Some(buffer) = field::<*mut c_void>(self.io, api.layout.io_buffer) {
                    (api.av_free)(buffer);
                }
                (api.avio_context_free)(&mut self.io);
            }
        }
    }
}

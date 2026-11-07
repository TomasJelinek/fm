//! Audited native ownership and custom AVIO bridge. No protocol or secondary-file opens are allowed.
use ffmpeg_next::{self as ffmpeg, ffi};
use midnight_thumbnail_sdk::ReadSeek;
use std::{
    ffi::{c_char, c_int, c_void},
    io::{self, SeekFrom},
    panic::{AssertUnwindSafe, catch_unwind},
    ptr,
};

struct Bridge<'a> {
    source: &'a mut dyn ReadSeek,
    length: u64,
}
unsafe extern "C" fn read(opaque: *mut c_void, buffer: *mut u8, length: c_int) -> c_int {
    if opaque.is_null() || buffer.is_null() || length <= 0 || length > 256 * 1024 {
        return -22;
    }
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: with_input retains Bridge and the decoder-owned writable AVIO buffer until input drop.
        let bridge = unsafe { &mut *opaque.cast::<Bridge<'_>>() };
        let buffer = unsafe { std::slice::from_raw_parts_mut(buffer, length as usize) };
        match bridge.source.read(buffer) {
            Ok(0) => ffi::AVERROR_EOF,
            Ok(count) => count as c_int,
            Err(_) => -5,
        }
    }))
    .unwrap_or(-5)
}
unsafe extern "C" fn seek(opaque: *mut c_void, offset: i64, whence: c_int) -> i64 {
    if opaque.is_null() {
        return -22;
    }
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: the exclusive callback borrows the live Bridge owned by with_input.
        let bridge = unsafe { &mut *opaque.cast::<Bridge<'_>>() };
        if whence == ffi::AVSEEK_SIZE {
            return bridge.length as i64;
        }
        let position = match whence & !ffi::AVSEEK_FORCE {
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
    _: *mut ffi::AVFormatContext,
    _: *mut *mut ffi::AVIOContext,
    _: *const c_char,
    _: c_int,
    _: *mut *mut ffi::AVDictionary,
) -> c_int {
    -13
}

struct IoGuard(*mut ffi::AVIOContext);
impl Drop for IoGuard {
    fn drop(&mut self) {
        // SAFETY: avio_alloc_context transferred the av_malloc buffer into this exclusively owned
        // context. FFmpeg may replace its buffer, so release the current buffer before the context.
        unsafe {
            ffi::av_free((*self.0).buffer.cast());
            ffi::avio_context_free(&mut self.0);
        }
    }
}
pub(super) fn with_input<T>(
    source: &mut dyn ReadSeek,
    length: u64,
    decode: impl FnOnce(&mut ffmpeg::format::context::Input) -> io::Result<T>,
) -> io::Result<T> {
    let mut bridge = Bridge { source, length };
    // SAFETY: each successful allocation is immediately given exactly one owner; failure paths
    // release only allocations they own. Bridge and IO outlive the format Input and all callbacks.
    unsafe {
        let buffer = ffi::av_malloc(64 * 1024).cast::<u8>();
        if buffer.is_null() {
            return Err(io::Error::other("AVIO allocation failed"));
        }
        let io = ffi::avio_alloc_context(
            buffer,
            64 * 1024,
            0,
            (&mut bridge as *mut Bridge<'_>).cast(),
            Some(read),
            None,
            Some(seek),
        );
        if io.is_null() {
            ffi::av_free(buffer.cast());
            return Err(io::Error::other("AVIO context allocation failed"));
        }
        let io = IoGuard(io);
        let mut context = ffi::avformat_alloc_context();
        if context.is_null() {
            return Err(io::Error::other("Format context allocation failed"));
        }
        (*context).pb = io.0;
        (*context).flags |= ffi::AVFMT_FLAG_CUSTOM_IO;
        (*context).io_open = Some(deny_open);
        (*context).probesize = 1024 * 1024;
        (*context).max_analyze_duration = 2_000_000;
        let mut options = ptr::null_mut();
        ffi::av_dict_set(
            &mut options,
            c"format_whitelist".as_ptr(),
            c"matroska,webm,avi,ogg".as_ptr(),
            0,
        );
        ffi::av_dict_set(
            &mut options,
            c"codec_whitelist".as_ptr(),
            c"vp8,vp9,theora,ffv1,rawvideo".as_ptr(),
            0,
        );
        ffi::av_dict_set(
            &mut options,
            c"protocol_whitelist".as_ptr(),
            c"".as_ptr(),
            0,
        );
        let opened = ffi::avformat_open_input(&mut context, ptr::null(), ptr::null(), &mut options);
        ffi::av_dict_free(&mut options);
        if opened < 0 {
            if !context.is_null() {
                ffi::avformat_free_context(context);
            }
            return Err(io::Error::other(ffmpeg::Error::from(opened)));
        }
        let mut input = ffmpeg::format::context::Input::wrap(context);
        let info = ffi::avformat_find_stream_info(input.as_mut_ptr(), ptr::null_mut());
        if info < 0 {
            return Err(io::Error::other(ffmpeg::Error::from(info)));
        }
        decode(&mut input)
    }
}
pub(super) fn limit_decoder(context: &mut ffmpeg::codec::context::Context) {
    // SAFETY: the caller exclusively owns this unopened codec context. Native decode is single
    // threaded; the host supplies bounded CPU parallelism across independent restricted helpers.
    unsafe {
        let context = context.as_mut_ptr();
        (*context).thread_count = 1;
        (*context).thread_type = 0;
        (*context).max_pixels = 32 * 1024 * 1024;
    }
}

pub(super) fn rotation(stream: &ffmpeg::Stream<'_>) -> io::Result<i32> {
    // SAFETY: stream owns valid codec parameters. Copy exactly nine validated matrix integers;
    // read_unaligned avoids assuming side-data alignment before calling FFmpeg's matrix helper.
    unsafe {
        let parameters = (*stream.as_ptr()).codecpar;
        let side = ffi::av_packet_side_data_get(
            (*parameters).coded_side_data,
            (*parameters).nb_coded_side_data,
            ffi::AVPacketSideDataType::AV_PKT_DATA_DISPLAYMATRIX,
        );
        if !side.is_null() {
            if (*side).size != 36 || (*side).data.is_null() {
                return Err(io::Error::other("Invalid video display matrix"));
            }
            let matrix: [i32; 9] = ptr::read_unaligned((*side).data.cast());
            // Perspective, reflections and nonuniform scaling are unsupported, rather than silently ignored.
            let [a, b, c, d] = [matrix[0], matrix[1], matrix[3], matrix[4]].map(i128::from);
            let determinant = a * d - b * c;
            if matrix[2] != 0
                || matrix[5] != 0
                || matrix[8] != 1 << 30
                || determinant <= 0
                || a * a + b * b != c * c + d * d
                || a * c + b * d != 0
            {
                return Err(io::Error::other("Unsupported video display transform"));
            }
            let angle = -ffi::av_display_rotation_get(matrix.as_ptr());
            if !angle.is_finite() || (angle - angle.round()).abs() > 0.01 {
                return Err(io::Error::other("Invalid video rotation"));
            }
            return Ok((angle.round() as i32).rem_euclid(360));
        }
    }
    Ok(stream
        .metadata()
        .get("rotate")
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(0)
        .rem_euclid(360))
}

pub(super) fn color_conversion(
    frame: &ffmpeg::frame::Video,
    scaling: &mut ffmpeg::software::scaling::context::Context,
) -> io::Result<bool> {
    // SAFETY: both borrowed objects are live and scaling is exclusively owned. Coefficient pointers
    // are static FFmpeg tables. Reject wide-gamut/HDR input until a reviewed tone mapper is available.
    unsafe {
        let frame = &*frame.as_ptr();
        let primaries = frame.color_primaries as i32;
        let transfer = frame.color_trc as i32;
        if !matches!(primaries, 1 | 2) || !matches!(transfer, 1 | 2 | 6 | 13) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Video color space requires unsupported gamut or HDR conversion",
            ));
        }
        let rgb = matches!(frame.format, value if value == ffi::AVPixelFormat::AV_PIX_FMT_RGB24 as i32
            || value == ffi::AVPixelFormat::AV_PIX_FMT_BGR24 as i32 || value == ffi::AVPixelFormat::AV_PIX_FMT_RGBA as i32
            || value == ffi::AVPixelFormat::AV_PIX_FMT_BGRA as i32 || value == ffi::AVPixelFormat::AV_PIX_FMT_ARGB as i32
            || value == ffi::AVPixelFormat::AV_PIX_FMT_ABGR as i32);
        let coefficients = match frame.colorspace as i32 {
            0 if rgb => ffi::SWS_CS_ITU709,
            1 => ffi::SWS_CS_ITU709,
            2 => {
                if frame.height <= 576 {
                    ffi::SWS_CS_ITU601
                } else {
                    ffi::SWS_CS_ITU709
                }
            }
            5 | 6 => ffi::SWS_CS_ITU601,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "Unsupported video color matrix",
                ));
            }
        };
        let coefficients = ffi::sws_getCoefficients(coefficients);
        if ffi::sws_setColorspaceDetails(
            scaling.as_mut_ptr(),
            coefficients,
            i32::from(rgb || frame.color_range as i32 == 2),
            coefficients,
            1,
            0,
            1 << 16,
            1 << 16,
        ) < 0
        {
            return Err(io::Error::other("Video color conversion failed"));
        }
        Ok(transfer != 13 && !(transfer == 2 && rgb))
    }
}

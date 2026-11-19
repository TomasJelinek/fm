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
/// Largest single brokered read.
const MAX_READ: usize = 256 * 1024;
unsafe extern "C" fn read(opaque: *mut c_void, buffer: *mut u8, length: c_int) -> c_int {
    if opaque.is_null() || buffer.is_null() || length <= 0 {
        return -22;
    }
    // avio_read hands a request larger than its 64 KiB buffer (a whole AVI chunk or
    // Matroska block) directly to this callback. A short count is valid for
    // read_packet and FFmpeg continues reading, so clamp instead of failing.
    let length = (length as usize).min(MAX_READ);
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: with_input retains Bridge and the decoder-owned writable AVIO buffer until
        // input drop. FFmpeg provides at least the requested length; `length` is clamped below it.
        let bridge = unsafe { &mut *opaque.cast::<Bridge<'_>>() };
        let buffer = unsafe { std::slice::from_raw_parts_mut(buffer, length) };
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
            c"matroska,webm,avi,ogg,mov,mp4,m4a,3gp,3g2,mj2".as_ptr(),
            0,
        );
        ffi::av_dict_set(
            &mut options,
            c"codec_whitelist".as_ptr(),
            c"vp8,vp9,theora,ffv1,rawvideo,opus".as_ptr(),
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
/// Container start time in AV_TIME_BASE units; zero when the container leaves it unset.
pub(super) fn start_time(input: &ffmpeg::format::context::Input) -> i64 {
    // SAFETY: Input owns a live, opened format context for the duration of this borrow.
    let start = unsafe { (*input.as_ptr()).start_time };
    if start == ffi::AV_NOPTS_VALUE {
        0
    } else {
        start
    }
}

fn video_seek_timestamp(micros: i64, time_base: ffmpeg::Rational) -> io::Result<i64> {
    if time_base.numerator() <= 0 || time_base.denominator() <= 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid selected video time base",
        ));
    }
    // Floor toward the preceding stream tick, including negative container start times. A
    // rounded-up tick could seek past the caller's target before preroll is decoded.
    let ticks = (i128::from(micros) * i128::from(time_base.denominator()))
        .div_euclid(1_000_000 * i128::from(time_base.numerator()));
    i64::try_from(ticks).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Video seek exceeds the selected stream timestamp range",
        )
    })
}

/// Seek the selected video, leaving the ordinary thumbnail/global seek path unchanged.
/// `absolute_micros` is in the container's AV_TIME_BASE timeline, including its start time.
pub(super) fn seek_video(
    input: &mut ffmpeg::format::context::Input,
    stream_index: usize,
    absolute_micros: i64,
) -> io::Result<()> {
    let index = c_int::try_from(stream_index).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Video stream index exceeds native range",
        )
    })?;
    let time_base = input
        .stream(stream_index)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Video stream index is unavailable",
            )
        })?
        .time_base();
    let target = video_seek_timestamp(absolute_micros, time_base)?;
    // SAFETY: `input` exclusively owns a live opened context and the audited AVIO bridge for
    // this synchronous call. The selected stream exists and fits c_int; its positive time base
    // and all converted timestamps were checked before native mutation. min <= target == max
    // permits an earlier keyframe but never a later seek timestamp; decoding skips preroll.
    let result =
        unsafe { ffi::avformat_seek_file(input.as_mut_ptr(), index, i64::MIN, target, target, 0) };
    if result < 0 {
        return Err(io::Error::other(ffmpeg::Error::from(result)));
    }
    Ok(())
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

/// AVColorPrimaries/AVColorTransferCharacteristic values shown without gamut or tone
/// mapping: BT.709 (1), unspecified (2) and the SD gamuts BT.470BG (5) and SMPTE 170M
/// (6), which differ only slightly from BT.709; BT.709/unspecified/BT.601 and sRGB
/// transfers, plus the BT.470M/BG tags (4, 5) that PAL and NTSC-M SD streams carry for
/// the same BT.601 camera curve and that are decoded like it. Wide-gamut and HDR input
/// waits for a reviewed tone mapper.
fn displayable(primaries: i32, transfer: i32) -> bool {
    matches!(primaries, 1 | 2 | 5 | 6) && matches!(transfer, 1 | 2 | 4 | 5 | 6 | 13)
}
pub(super) fn color_conversion(
    frame: &ffmpeg::frame::Video,
    scaling: &mut ffmpeg::software::scaling::context::Context,
) -> io::Result<bool> {
    // SAFETY: both borrowed objects are live and scaling is exclusively owned. Coefficient pointers
    // are static FFmpeg tables.
    unsafe {
        let frame = &*frame.as_ptr();
        let primaries = frame.color_primaries as i32;
        let transfer = frame.color_trc as i32;
        if !displayable(primaries, transfer) {
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

#[cfg(test)]
mod tests {
    #[test]
    fn selected_video_seek_ticks_never_round_beyond_the_requested_timestamp() {
        let time_base = ffmpeg_next::Rational::new(1, 6);
        for (micros, expected) in [
            (-1, -1),
            (0, 0),
            (200_000, 1),
            (333_333, 1),
            (333_334, 2),
            (500_000, 3),
        ] {
            assert_eq!(
                super::video_seek_timestamp(micros, time_base).unwrap(),
                expected
            );
        }
        assert_eq!(
            super::video_seek_timestamp(i64::MAX, ffmpeg_next::Rational::new(1, 1_000_000))
                .unwrap(),
            i64::MAX
        );
    }

    #[test]
    fn invalid_time_bases_and_unrepresentable_selected_video_ticks_are_rejected() {
        for time_base in [
            ffmpeg_next::Rational::new(0, 6),
            ffmpeg_next::Rational::new(1, 0),
            ffmpeg_next::Rational::new(-1, 6),
        ] {
            assert_eq!(
                super::video_seek_timestamp(1, time_base)
                    .unwrap_err()
                    .kind(),
                std::io::ErrorKind::InvalidInput
            );
        }
        assert_eq!(
            super::video_seek_timestamp(i64::MAX, ffmpeg_next::Rational::new(1, i32::MAX))
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn sd_tagged_streams_are_displayable_but_wide_gamut_and_hdr_are_not() {
        // The full BT.601 triplet that ffmpeg/libvpx write for smpte170m sources.
        assert!(super::displayable(6, 6));
        assert!(super::displayable(5, 6));
        assert!(super::displayable(1, 1) && super::displayable(2, 13));
        // DVD-PAL and NTSC-M transcodes that keep their BT.470BG/M transfer tags.
        assert!(super::displayable(5, 5) && super::displayable(6, 4));
        // BT.2020 gamut, linear light, PQ and HLG transfers.
        assert!(!super::displayable(9, 1));
        assert!(!super::displayable(1, 8));
        assert!(!super::displayable(1, 16));
        assert!(!super::displayable(9, 18));
    }
}

//! The video stream: one source reader that selects only the first video stream, decodes it in
//! software to a YCbCr layout this helper converts itself, and drops frames before a seek target.
use super::{
    convert::{self, Format, Layout, Matrix},
    failure,
    remote::Reads,
    source_reader,
};
use crate::media::{Codec, Frame, Metadata, NoDecoder, OutputBounds, Tried, output_length};
use std::io;
use windows::{
    Win32::{
        Media::MediaFoundation::{
            IMFActivate, IMFMediaType, IMFSample, IMFSourceReader, MF_MT_DEFAULT_STRIDE,
            MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE, MF_MT_MINIMUM_DISPLAY_APERTURE, MF_MT_SUBTYPE,
            MF_MT_TRANSFER_FUNCTION, MF_MT_VIDEO_NOMINAL_RANGE, MF_MT_VIDEO_PRIMARIES,
            MF_MT_VIDEO_ROTATION, MF_MT_YUV_MATRIX, MF_PD_DURATION, MF_SOURCE_READER_ALL_STREAMS,
            MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_SOURCE_READER_MEDIASOURCE,
            MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED, MF_SOURCE_READERF_ENDOFSTREAM,
            MF_SOURCE_READERF_ERROR, MFCreateMediaType, MFGetStrideForBitmapInfoHeader,
            MFMediaType_Video, MFNominalRange_0_255, MFT_CATEGORY_VIDEO_DECODER,
            MFT_ENUM_FLAG_ASYNCMFT, MFT_ENUM_FLAG_LOCALMFT, MFT_ENUM_FLAG_SORTANDFILTER,
            MFT_ENUM_FLAG_SYNCMFT, MFT_REGISTER_TYPE_INFO, MFTEnumEx, MFVideoArea,
            MFVideoFormat_I420, MFVideoFormat_MPEG2, MFVideoFormat_NV12, MFVideoFormat_P010,
            MFVideoFormat_YUY2, MFVideoFormat_YV12, MFVideoPrimaries_BT2020, MFVideoTransFunc_2084,
            MFVideoTransFunc_HLG, MFVideoTransFunc_sRGB, MFVideoTransferMatrix_BT601,
            MFVideoTransferMatrix_BT709,
        },
        System::{
            Com::{
                CoTaskMemFree,
                StructuredStorage::{PROPVARIANT, PropVariantClear},
            },
            Variant::{VT_I8, VT_UI8},
        },
    },
    core::GUID,
};

const ALL: u32 = MF_SOURCE_READER_ALL_STREAMS.0 as u32;
const VIDEO: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
const MEDIA_SOURCE: u32 = MF_SOURCE_READER_MEDIASOURCE.0 as u32;
/// Decoder outputs this helper converts, in order of preference.
const OUTPUTS: [(GUID, Layout); 5] = [
    (MFVideoFormat_NV12, Layout::Nv12),
    (MFVideoFormat_P010, Layout::P010),
    (MFVideoFormat_YUY2, Layout::Yuy2),
    (MFVideoFormat_I420, Layout::I420),
    (MFVideoFormat_YV12, Layout::Yv12),
];
/// Each call reads at most this many samples, preroll included.
const BUDGET: usize = 4096;

pub(super) struct VideoReader {
    reader: IMFSourceReader,
    layout: Layout,
    rotation: u32,
    format: Format,
    output: (u32, u32),
    /// Container duration in 100 ns units, when known.
    duration: Option<u64>,
    /// Frames before this time (100 ns units) are preroll.
    minimum: i64,
    ended: bool,
}

impl VideoReader {
    pub(super) fn open(reads: &Reads, output: OutputBounds) -> io::Result<Self> {
        output_length(output.width, output.height)?;
        let reader = source_reader(reads)?;
        // SAFETY: every call below uses the live reader on its owning thread.
        unsafe {
            reader.SetStreamSelection(ALL, false).map_err(failure)?;
            if reader.SetStreamSelection(VIDEO, true).is_err() {
                // Playback and thumbnails need a video stream.
                return Err(NoDecoder {
                    codec: Codec::Unknown,
                    tried: Tried::MEDIA_FOUNDATION,
                }
                .into());
            }
            let native = reader.GetNativeMediaType(VIDEO, 0).map_err(failure)?;
            let subtype = native.GetGUID(&MF_MT_SUBTYPE).map_err(failure)?;
            displayable(&native)?;
            let layout = choose_output(&reader, &subtype)?;
            let rotation = upright(native.GetUINT32(&MF_MT_VIDEO_ROTATION).unwrap_or(0))?;
            let current = reader.GetCurrentMediaType(VIDEO).map_err(failure)?;
            displayable(&current)?;
            let format = format(&current, &native, layout, rotation)?;
            Ok(Self {
                duration: duration(&reader),
                reader,
                layout,
                rotation,
                format,
                output: (output.width, output.height),
                minimum: 0,
                ended: false,
            })
        }
    }

    pub(super) fn metadata(&self) -> Metadata {
        let (width, height) = if matches!(self.rotation, 90 | 270) {
            (self.format.height, self.format.width)
        } else {
            (self.format.width, self.format.height)
        };
        Metadata {
            duration_micros: self.duration.map(|duration| duration / 10),
            width,
            height,
        }
    }

    /// The next frame at or after the last seek target.
    pub(super) fn next(&mut self) -> io::Result<Option<Frame>> {
        for _ in 0..BUDGET {
            let Some((sample, time)) = self.read()? else {
                return Ok(None);
            };
            if time >= self.minimum {
                return self.frame(&sample, time).map(Some);
            }
        }
        Err(io::Error::other("Video decode work budget exhausted"))
    }

    pub(super) fn seek(&mut self, position_micros: u64) -> io::Result<()> {
        let target = i64::try_from(position_micros)
            .ok()
            .and_then(|micros| micros.checked_mul(10))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Video seek exceeds timestamp range",
                )
            })?;
        seek(&self.reader, target)?;
        self.minimum = target;
        self.ended = false;
        Ok(())
    }

    /// The first frame at or after 10% to 85% of the duration, or the latest earlier frame when
    /// none decodes there (a header that overstates a partial download).
    pub(super) fn thumbnail(&mut self, sample: u8) -> io::Result<Frame> {
        let duration = self
            .duration
            .and_then(|duration| i64::try_from(duration).ok())
            .unwrap_or(0);
        let target = duration.saturating_mul(10 + i64::from(sample) * 15) / 100;
        if target > 0 {
            seek(&self.reader, target)?;
        }
        let mut earlier = None;
        for _ in 0..BUDGET {
            match self.read()? {
                None => break,
                Some((sample, time)) if time < target => earlier = Some((sample, time)),
                Some((sample, time)) => return self.frame(&sample, time),
            }
        }
        match earlier {
            Some((sample, time)) => self.frame(&sample, time),
            None => Err(io::Error::other(
                "No representative frame within decode budget",
            )),
        }
    }

    /// The next decoded sample and its time, or `None` at the end of the stream.
    fn read(&mut self) -> io::Result<Option<(IMFSample, i64)>> {
        if self.ended {
            return Ok(None);
        }
        for _ in 0..BUDGET {
            let (mut flags, mut time, mut sample) = (0u32, 0i64, None);
            // SAFETY: valid output pointers for one synchronous read on the owning thread.
            unsafe {
                self.reader.ReadSample(
                    VIDEO,
                    0,
                    None,
                    Some(&raw mut flags),
                    Some(&raw mut time),
                    Some(&raw mut sample),
                )
            }
            .map_err(failure)?;
            if flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
                return Err(io::Error::other("Media Foundation reported a stream error"));
            }
            if flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32 != 0 {
                self.reload()?;
            }
            if let Some(sample) = sample {
                return Ok(Some((sample, time)));
            }
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                self.ended = true;
                return Ok(None);
            }
            // A stream tick: a gap without a sample.
        }
        Err(io::Error::other("Video decode work budget exhausted"))
    }

    /// The decoder changed its output (a new picture size mid-stream).
    fn reload(&mut self) -> io::Result<()> {
        // SAFETY: the live reader on its owning thread.
        unsafe {
            let current = self.reader.GetCurrentMediaType(VIDEO).map_err(failure)?;
            let native = self.reader.GetNativeMediaType(VIDEO, 0).map_err(failure)?;
            displayable(&current)?;
            self.format = format(&current, &native, self.layout, self.rotation)?;
        }
        Ok(())
    }

    fn frame(&self, sample: &IMFSample, time: i64) -> io::Result<Frame> {
        // SAFETY: the sample is live; its contiguous buffer is locked only for this conversion and
        // the slice is dropped before the unlock.
        let pixels = unsafe {
            let buffer = sample.ConvertToContiguousBuffer().map_err(failure)?;
            let mut data = std::ptr::null_mut();
            let mut length = 0u32;
            buffer
                .Lock(&mut data, None, Some(&raw mut length))
                .map_err(failure)?;
            let converted = if data.is_null() {
                Err(io::Error::other("Decoded video buffer is empty"))
            } else {
                convert::convert(
                    std::slice::from_raw_parts(data, length as usize),
                    &self.format,
                    self.output,
                )
            };
            let _ = buffer.Unlock();
            converted?
        };
        Ok(Frame {
            presentation_micros: time.max(0) / 10,
            width: pixels.width,
            height: pixels.height,
            rgba: pixels.rgba,
        })
    }
}

/// Ask the decoder for each layout this helper converts. When none is accepted, tell a missing
/// decoder (none registered for the subtype) from a decoder that cannot produce these layouts.
fn choose_output(reader: &IMFSourceReader, subtype: &GUID) -> io::Result<Layout> {
    for (output, layout) in OUTPUTS {
        // SAFETY: a new media type owned here and the live reader on its owning thread.
        let accepted = unsafe {
            let wanted = MFCreateMediaType().map_err(failure)?;
            wanted
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                .map_err(failure)?;
            wanted.SetGUID(&MF_MT_SUBTYPE, &output).map_err(failure)?;
            reader.SetCurrentMediaType(VIDEO, None, &wanted).is_ok()
        };
        if accepted {
            return Ok(layout);
        }
    }
    if !decoder_registered(subtype)? {
        return Err(NoDecoder {
            codec: codec(subtype),
            tried: Tried::MEDIA_FOUNDATION,
        }
        .into());
    }
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "The Windows video decoder produces no picture format this viewer converts",
    ))
}

/// Whether any software decoder (local, system or Store) is registered for `subtype`.
fn decoder_registered(subtype: &GUID) -> io::Result<bool> {
    let input = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: *subtype,
    };
    let mut activates: *mut Option<IMFActivate> = std::ptr::null_mut();
    let mut count = 0u32;
    // SAFETY: valid input and output pointers. Every returned activation object is released and
    // the array is freed with the COM allocator, as MFTEnumEx requires.
    unsafe {
        MFTEnumEx(
            MFT_CATEGORY_VIDEO_DECODER,
            MFT_ENUM_FLAG_SYNCMFT
                | MFT_ENUM_FLAG_ASYNCMFT
                | MFT_ENUM_FLAG_LOCALMFT
                | MFT_ENUM_FLAG_SORTANDFILTER,
            Some(&raw const input),
            None,
            &mut activates,
            &mut count,
        )
        .map_err(failure)?;
        if !activates.is_null() {
            for index in 0..count as usize {
                drop(std::ptr::read(activates.add(index)));
            }
            CoTaskMemFree(Some(activates.cast_const().cast()));
        }
    }
    Ok(count > 0)
}

/// The wire codec for a Media Foundation video subtype (FourCC-based GUIDs and MPEG-2).
pub(super) fn codec(subtype: &GUID) -> Codec {
    if *subtype == MFVideoFormat_MPEG2 {
        return Codec::Mpeg2;
    }
    let fourcc = subtype.data1.to_le_bytes();
    match &fourcc {
        b"H264" | b"h264" | b"AVC1" | b"avc1" => Codec::H264,
        b"HEVC" | b"H265" | b"hvc1" | b"hev1" => Codec::Hevc,
        b"AV01" | b"av01" => Codec::Av1,
        b"VP90" | b"vp09" => Codec::Vp9,
        b"VP80" | b"vp08" => Codec::Vp8,
        b"MP4V" | b"M4S2" | b"MP4S" | b"MP43" | b"MP42" | b"DIVX" | b"XVID" | b"mp4v" => {
            Codec::Mpeg4Part2
        }
        b"WMV1" | b"WMV2" | b"WMV3" | b"WVC1" | b"WMVA" => Codec::Vc1,
        b"MJPG" | b"mjpg" => Codec::MotionJpeg,
        b"H263" | b"h263" | b"s263" => Codec::H263,
        b"MPG1" => Codec::Mpeg1,
        b"apch" | b"apcn" | b"apcs" | b"apco" | b"ap4h" | b"ap4x" => Codec::ProRes,
        b"theo" | b"THEO" => Codec::Theora,
        _ => Codec::Unknown,
    }
}

/// Wide-gamut and HDR pictures wait for a reviewed tone mapper, as in the bundled profile.
fn displayable(media: &IMFMediaType) -> io::Result<()> {
    // SAFETY: a live media type; missing attributes read as their defaults.
    let (transfer, primaries, matrix) = unsafe {
        (
            media.GetUINT32(&MF_MT_TRANSFER_FUNCTION).unwrap_or(0),
            media.GetUINT32(&MF_MT_VIDEO_PRIMARIES).unwrap_or(0),
            media.GetUINT32(&MF_MT_YUV_MATRIX).unwrap_or(0),
        )
    };
    let hdr = [MFVideoTransFunc_2084.0, MFVideoTransFunc_HLG.0].contains(&(transfer as i32));
    // BT.2020 matrices (4, 5) and primaries, and SMPTE 240M (3), have no reviewed conversion.
    if hdr || primaries as i32 == MFVideoPrimaries_BT2020.0 || matches!(matrix, 3..=5) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Video color space requires unsupported gamut or HDR conversion",
        ));
    }
    Ok(())
}

/// Clockwise turn that shows the picture upright. Media Foundation states how far the stored
/// picture is turned counterclockwise (from the MP4 track matrix): a phone's portrait movie,
/// whose matrix turns it a quarter clockwise for display, reads 90. This turn undoes it.
fn upright(rotation: u32) -> io::Result<u32> {
    match rotation {
        0 | 90 | 180 | 270 => Ok(rotation),
        _ => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Unsupported video orientation",
        )),
    }
}

/// The decoder output's geometry and color, from the current type with the stream's own type
/// as fallback for color attributes the decoder leaves out.
fn format(
    current: &IMFMediaType,
    native: &IMFMediaType,
    layout: Layout,
    rotation: u32,
) -> io::Result<Format> {
    // SAFETY: live media types; missing attributes read as errors or defaults.
    unsafe {
        let size = current.GetUINT64(&MF_MT_FRAME_SIZE).map_err(failure)?;
        let (columns, rows) = ((size >> 32) as u32, size as u32);
        if columns == 0 || rows == 0 || u64::from(columns) * u64::from(rows) > 32 * 1024 * 1024 {
            return Err(io::Error::other("Video dimensions exceed decoder bound"));
        }
        let stride = match current.GetUINT32(&MF_MT_DEFAULT_STRIDE) {
            Ok(stride) => (stride as i32).unsigned_abs(),
            Err(_) => {
                let subtype = current.GetGUID(&MF_MT_SUBTYPE).map_err(failure)?;
                MFGetStrideForBitmapInfoHeader(subtype.data1, columns)
                    .map_err(failure)?
                    .unsigned_abs()
            }
        };
        let (mut left, mut top, mut width, mut height) = (0, 0, columns, rows);
        let mut area = [0u8; size_of::<MFVideoArea>()];
        let mut written = 0u32;
        if current
            .GetBlob(
                &MF_MT_MINIMUM_DISPLAY_APERTURE,
                &mut area,
                Some(&raw mut written),
            )
            .is_ok()
            && written as usize == area.len()
        {
            let area: MFVideoArea = std::ptr::read_unaligned(area.as_ptr().cast());
            let (x, y) = (area.OffsetX.value, area.OffsetY.value);
            let (cx, cy) = (area.Area.cx, area.Area.cy);
            if x >= 0 && y >= 0 && cx > 0 && cy > 0 {
                (left, top, width, height) = (x as u32, y as u32, cx as u32, cy as u32);
            }
        }
        let attribute = |key: &GUID| {
            current
                .GetUINT32(key)
                .or_else(|_| native.GetUINT32(key))
                .unwrap_or(0)
        };
        let matrix = match attribute(&MF_MT_YUV_MATRIX) as i32 {
            value if value == MFVideoTransferMatrix_BT709.0 => Matrix::Bt709,
            value if value == MFVideoTransferMatrix_BT601.0 => Matrix::Bt601,
            _ if height <= 576 => Matrix::Bt601,
            _ => Matrix::Bt709,
        };
        Ok(Format {
            layout,
            rows,
            stride,
            left,
            top,
            width,
            height,
            matrix,
            full_range: attribute(&MF_MT_VIDEO_NOMINAL_RANGE) as i32 == MFNominalRange_0_255.0,
            transfer_709: attribute(&MF_MT_TRANSFER_FUNCTION) as i32 != MFVideoTransFunc_sRGB.0,
            rotation,
        })
    }
}

/// The container duration in 100 ns units, when the source states one.
fn duration(reader: &IMFSourceReader) -> Option<u64> {
    // SAFETY: the live reader; the returned variant is cleared after its value is read.
    unsafe {
        let mut value = reader
            .GetPresentationAttribute(MEDIA_SOURCE, &MF_PD_DURATION)
            .ok()?;
        let duration = (value.Anonymous.Anonymous.vt == VT_UI8)
            .then(|| value.Anonymous.Anonymous.Anonymous.uhVal);
        let _ = PropVariantClear(&mut value);
        duration
    }
}

/// Reposition a reader to `target` (100 ns units). Media Foundation lands on a key frame at or
/// before it; the callers drop the preroll.
pub(super) fn seek(reader: &IMFSourceReader, target: i64) -> io::Result<()> {
    let mut position = PROPVARIANT::default();
    // SAFETY: writes the active union member of a zeroed variant, which needs no clearing.
    unsafe {
        (*position.Anonymous.Anonymous).vt = VT_I8;
        (*position.Anonymous.Anonymous).Anonymous.hVal = target.max(0);
        reader
            .SetCurrentPosition(&GUID::zeroed(), &position)
            .map_err(failure)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fourcc(code: &[u8; 4]) -> GUID {
        GUID::from_values(
            u32::from_le_bytes(*code),
            0x0000,
            0x0010,
            [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
        )
    }

    #[test]
    fn video_subtypes_map_to_their_wire_codecs() {
        for (code, expected) in [
            (b"H264", Codec::H264),
            (b"HEVC", Codec::Hevc),
            (b"AV01", Codec::Av1),
            (b"VP90", Codec::Vp9),
            (b"WVC1", Codec::Vc1),
            (b"MP4V", Codec::Mpeg4Part2),
            (b"MJPG", Codec::MotionJpeg),
            (b"apcn", Codec::ProRes),
            (b"ZZZZ", Codec::Unknown),
        ] {
            assert_eq!(codec(&fourcc(code)), expected, "{code:?}");
        }
        assert_eq!(codec(&MFVideoFormat_MPEG2), Codec::Mpeg2);
    }

    #[test]
    fn only_quarter_turns_are_shown_upright() {
        assert_eq!(upright(0).unwrap(), 0);
        assert_eq!(upright(90).unwrap(), 90);
        assert_eq!(upright(270).unwrap(), 270);
        assert!(upright(45).is_err());
    }
}

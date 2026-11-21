//! Decoder-independent media types shared by the bundled FFmpeg profile and the platform
//! decoders. Nothing here links a decoder: the system helpers build this crate without FFmpeg.
use std::{fmt, io};

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

/// Whether the opened media has audio and whether this decoder produces PCM for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum AudioTrack {
    Absent = 0,
    /// `pcm` produces the track.
    Qualified = 1,
    /// An audio track exists but this decoder cannot produce it; video plays with a notice.
    Unqualified = 2,
}

/// Interleaved 48 kHz stereo PCM. `first_frame` counts PCM frames from the last seek target.
#[derive(Debug)]
pub struct Block {
    pub first_frame: u64,
    pub samples: Vec<i16>,
}

/// Largest PCM block one pull may request, in frames.
pub const MAX_PCM_FRAMES: u16 = 4_800;

/// RGBA byte length of an output frame within the edge and pixel budget.
pub fn output_length(width: u32, height: u32) -> io::Result<usize> {
    let pixels = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || width > 4096 || height > 4096 || pixels > 4 * 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Video output exceeds the edge or pixel budget",
        ));
    }
    Ok((pixels * 4) as usize)
}

/// Codec identities on the MMV2 wire (failure cause 5). The values are fixed: the host words
/// the message from them, and helpers never send text. Values 16 and above are audio codecs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Codec {
    Unknown = 0,
    H264 = 1,
    Hevc = 2,
    Mpeg4Part2 = 3,
    Mpeg2 = 4,
    Mpeg1 = 5,
    Av1 = 6,
    /// VC-1 and the Windows Media Video codecs.
    Vc1 = 7,
    Vp6 = 8,
    Vp9 = 9,
    Vp8 = 10,
    Theora = 11,
    MotionJpeg = 12,
    ProRes = 13,
    H263 = 14,
    Aac = 16,
    Mp3 = 17,
    Ac3 = 18,
    Eac3 = 19,
    Wma = 20,
    Vorbis = 21,
    Flac = 22,
    Dts = 23,
    Opus = 24,
}

impl Codec {
    const ALL: [Self; 24] = [
        Self::H264,
        Self::Hevc,
        Self::Mpeg4Part2,
        Self::Mpeg2,
        Self::Mpeg1,
        Self::Av1,
        Self::Vc1,
        Self::Vp6,
        Self::Vp9,
        Self::Vp8,
        Self::Theora,
        Self::MotionJpeg,
        Self::ProRes,
        Self::H263,
        Self::Aac,
        Self::Mp3,
        Self::Ac3,
        Self::Eac3,
        Self::Wma,
        Self::Vorbis,
        Self::Flac,
        Self::Dts,
        Self::Opus,
        Self::Unknown,
    ];

    /// The codec for a wire value; values outside the table are `Unknown`.
    pub fn from_u8(value: u8) -> Self {
        Self::ALL
            .into_iter()
            .find(|codec| *codec as u8 == value)
            .unwrap_or(Self::Unknown)
    }

    /// The codec for an FFmpeg codec name (`avcodec_get_name`), which names codecs even
    /// when no decoder for them is compiled in.
    pub fn from_ffmpeg_name(name: &str) -> Self {
        match name {
            "h264" => Self::H264,
            "hevc" => Self::Hevc,
            "mpeg4" | "msmpeg4v1" | "msmpeg4v2" | "msmpeg4v3" => Self::Mpeg4Part2,
            "mpeg2video" => Self::Mpeg2,
            "mpeg1video" => Self::Mpeg1,
            "av1" => Self::Av1,
            "vc1" | "wmv1" | "wmv2" | "wmv3" | "vc1image" | "wmv3image" => Self::Vc1,
            "vp6" | "vp6f" | "vp6a" => Self::Vp6,
            "vp9" => Self::Vp9,
            "vp8" => Self::Vp8,
            "theora" => Self::Theora,
            "mjpeg" => Self::MotionJpeg,
            "prores" => Self::ProRes,
            "h263" | "h263p" | "h263i" => Self::H263,
            "aac" | "aac_latm" => Self::Aac,
            "mp3" => Self::Mp3,
            "ac3" => Self::Ac3,
            "eac3" => Self::Eac3,
            "wmav1" | "wmav2" | "wmapro" | "wmalossless" | "wmavoice" => Self::Wma,
            "vorbis" => Self::Vorbis,
            "flac" => Self::Flac,
            "dts" => Self::Dts,
            "opus" => Self::Opus,
            _ => Self::Unknown,
        }
    }
}

/// The decoders a helper tried before reporting `NoDecoder` (a bit set on the wire).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tried(pub u8);

impl Tried {
    pub const BUNDLED: Self = Self(1);
    pub const DISTRIBUTION_FFMPEG: Self = Self(2);
    pub const GSTREAMER: Self = Self(4);
    pub const MEDIA_FOUNDATION: Self = Self(8);

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

/// No decoder this helper can use supports the source's video. A helper reports it before
/// playback starts (MMV2 cause 5), so the host can try the platform helper; the host words
/// the message from the codec and what was tried.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoDecoder {
    pub codec: Codec,
    pub tried: Tried,
}

impl NoDecoder {
    /// The `NoDecoder` an error carries, if any.
    pub fn find(error: &io::Error) -> Option<Self> {
        error
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<Self>())
            .copied()
    }
}

impl fmt::Display for NoDecoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "No available decoder supports this video (codec {}, tried {:#04x})",
            self.codec as u8, self.tried.0
        )
    }
}

impl std::error::Error for NoDecoder {}

impl From<NoDecoder> for io::Error {
    fn from(missing: NoDecoder) -> Self {
        io::Error::new(io::ErrorKind::Unsupported, missing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_codec_values_are_fixed_and_unknown_values_stay_unknown() {
        for (codec, value) in [
            (Codec::Unknown, 0),
            (Codec::H264, 1),
            (Codec::Hevc, 2),
            (Codec::Mpeg4Part2, 3),
            (Codec::Mpeg2, 4),
            (Codec::Mpeg1, 5),
            (Codec::Av1, 6),
            (Codec::Vc1, 7),
            (Codec::Vp6, 8),
            (Codec::Vp9, 9),
            (Codec::Vp8, 10),
            (Codec::Theora, 11),
            (Codec::MotionJpeg, 12),
            (Codec::ProRes, 13),
            (Codec::H263, 14),
            (Codec::Aac, 16),
            (Codec::Mp3, 17),
            (Codec::Ac3, 18),
            (Codec::Eac3, 19),
            (Codec::Wma, 20),
            (Codec::Vorbis, 21),
            (Codec::Flac, 22),
            (Codec::Dts, 23),
            (Codec::Opus, 24),
        ] {
            assert_eq!(codec as u8, value);
            assert_eq!(Codec::from_u8(value), codec);
        }
        for value in [15, 25, 255] {
            assert_eq!(Codec::from_u8(value), Codec::Unknown);
        }
        assert_eq!(Codec::from_ffmpeg_name("hevc"), Codec::Hevc);
        assert_eq!(Codec::from_ffmpeg_name("msmpeg4v3"), Codec::Mpeg4Part2);
        assert_eq!(Codec::from_ffmpeg_name("wmv3"), Codec::Vc1);
        assert_eq!(Codec::from_ffmpeg_name("unheard-of"), Codec::Unknown);
    }

    #[test]
    fn a_missing_decoder_survives_the_io_error_that_carries_it() {
        let missing = NoDecoder {
            codec: Codec::Hevc,
            tried: Tried::BUNDLED.union(Tried::MEDIA_FOUNDATION),
        };
        let error = io::Error::from(missing);
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        assert_eq!(NoDecoder::find(&error), Some(missing));
        assert!(missing.tried.contains(Tried::MEDIA_FOUNDATION));
        assert_eq!(NoDecoder::find(&io::Error::other("other")), None);
    }

    #[test]
    fn output_bounds_keep_the_edge_and_pixel_budget() {
        assert_eq!(output_length(2, 2).unwrap(), 16);
        assert_eq!(output_length(4096, 1024).unwrap(), 4096 * 1024 * 4);
        for (width, height) in [(0, 1), (1, 0), (4097, 1), (4096, 4096)] {
            assert_eq!(
                output_length(width, height).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }
}

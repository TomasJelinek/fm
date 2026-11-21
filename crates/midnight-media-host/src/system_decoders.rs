//! A video that no available decoder supports, worded for the viewer's user. Helpers parse
//! untrusted bytes, so they report only a codec number and what they tried (MMV2 failure
//! cause 5); the host chooses the words. The codec numbers are the video plugin's
//! `media::Codec` values.
use std::{fmt, io};

#[cfg(windows)]
#[path = "system_decoders_windows.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "system_decoders_linux.rs"]
mod platform;

/// The decoders a helper tried (a bit set on the wire).
pub const TRIED_BUNDLED: u8 = 1;
pub const TRIED_DISTRIBUTION_FFMPEG: u8 = 2;
pub const TRIED_GSTREAMER: u8 = 4;
pub const TRIED_MEDIA_FOUNDATION: u8 = 8;

/// The message for codec 0 (unknown) and for content no decoder recognized.
pub const UNSUPPORTED: &str = "Media format or codec is unsupported";

/// No decoder a helper could use supports the video.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MissingDecoder {
    pub codec: u8,
    pub tried: u8,
}

impl MissingDecoder {
    /// The missing decoder an error carries, if any.
    pub fn find(error: &io::Error) -> Option<Self> {
        error
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<Self>())
            .copied()
    }

    /// The bundled helper's report followed by the platform helper's: the codec whichever
    /// helper named it, and everything that was tried.
    pub fn then(self, later: Self) -> Self {
        Self {
            codec: if later.codec != 0 {
                later.codec
            } else {
                self.codec
            },
            tried: self.tried | later.tried,
        }
    }
}

impl fmt::Display for MissingDecoder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&platform::missing_decoder(self.codec, self.tried))
    }
}

impl std::error::Error for MissingDecoder {}

impl From<MissingDecoder> for io::Error {
    fn from(missing: MissingDecoder) -> Self {
        io::Error::new(io::ErrorKind::Unsupported, missing)
    }
}

/// The codec's name for its wire number.
pub fn codec_name(codec: u8) -> Option<&'static str> {
    Some(match codec {
        1 => "H.264",
        2 => "HEVC",
        3 => "MPEG-4 Part 2",
        4 => "MPEG-2",
        5 => "MPEG-1",
        6 => "AV1",
        7 => "VC-1 (Windows Media Video)",
        8 => "VP6",
        9 => "VP9",
        10 => "VP8",
        11 => "Theora",
        12 => "Motion JPEG",
        13 => "Apple ProRes",
        14 => "H.263",
        16 => "AAC",
        17 => "MP3",
        18 => "AC-3",
        19 => "E-AC-3",
        20 => "Windows Media Audio",
        21 => "Vorbis",
        22 => "FLAC",
        23 => "DTS",
        24 => "Opus",
        _ => return None,
    })
}

// Other systems have no platform decoders.
#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    pub(super) fn missing_decoder(codec: u8, _tried: u8) -> String {
        match super::codec_name(codec) {
            Some(name) => format!("This video uses {name}; no installed decoder supports it"),
            None => super::UNSUPPORTED.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_later_report_keeps_a_named_codec_and_adds_what_it_tried() {
        let bundled = MissingDecoder {
            codec: 2,
            tried: TRIED_BUNDLED,
        };
        let platform = MissingDecoder {
            codec: 0,
            tried: TRIED_MEDIA_FOUNDATION,
        };
        assert_eq!(
            bundled.then(platform),
            MissingDecoder {
                codec: 2,
                tried: TRIED_BUNDLED | TRIED_MEDIA_FOUNDATION
            }
        );
        let named = MissingDecoder {
            codec: 6,
            tried: TRIED_MEDIA_FOUNDATION,
        };
        assert_eq!(bundled.then(named).codec, 6);
    }

    #[test]
    fn the_message_names_the_codec_and_unknown_codecs_stay_generic() {
        let error = io::Error::from(MissingDecoder {
            codec: 0,
            tried: TRIED_BUNDLED,
        });
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        assert_eq!(error.to_string(), UNSUPPORTED);
        let error = io::Error::from(MissingDecoder {
            codec: 1,
            tried: TRIED_BUNDLED,
        });
        assert!(error.to_string().starts_with("This video uses H.264; "));
        assert_eq!(
            MissingDecoder::find(&error),
            Some(MissingDecoder {
                codec: 1,
                tried: TRIED_BUNDLED
            })
        );
        assert_eq!(codec_name(15), None);
        assert_eq!(codec_name(13), Some("Apple ProRes"));
    }
}

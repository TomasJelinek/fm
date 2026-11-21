//! GStreamer caps to the shared codec identities, and GStreamer colorimetry to the bundled
//! profile's colour rules.
use super::api::{Api, Colorimetry};
use crate::{media::Codec, system::linux::picture};
use std::{
    ffi::{CStr, c_int, c_void},
    io,
};

/// The codec a demuxed caps structure carries (`Unknown` when the table has no entry).
pub(super) fn from_caps(api: &Api, structure: *const c_void) -> Codec {
    if structure.is_null() {
        return Codec::Unknown;
    }
    // SAFETY: a live caps structure; the name is owned by it.
    let name = unsafe { CStr::from_ptr((api.gst_structure_get_name)(structure)) };
    let int = |field: &CStr| {
        let mut value: c_int = 0;
        // SAFETY: a live structure; one int is written.
        let found = unsafe { (api.gst_structure_get_int)(structure, field.as_ptr(), &mut value) };
        (found != 0).then_some(value)
    };
    from_name(name.to_bytes(), int(c"mpegversion"), int(c"layer"))
}

pub(super) fn from_name(name: &[u8], mpeg_version: Option<c_int>, layer: Option<c_int>) -> Codec {
    match name {
        b"video/x-h264" => Codec::H264,
        b"video/x-h265" => Codec::Hevc,
        b"video/x-divx" | b"video/x-xvid" | b"video/x-msmpeg" => Codec::Mpeg4Part2,
        b"video/mpeg" => match mpeg_version {
            Some(1) => Codec::Mpeg1,
            Some(2) => Codec::Mpeg2,
            Some(4) => Codec::Mpeg4Part2,
            _ => Codec::Unknown,
        },
        b"video/x-av1" => Codec::Av1,
        b"video/x-wmv" | b"video/x-vc1" => Codec::Vc1,
        b"video/x-vp6" | b"video/x-vp6-flash" | b"video/x-vp6-alpha" => Codec::Vp6,
        b"video/x-vp9" => Codec::Vp9,
        b"video/x-vp8" => Codec::Vp8,
        b"video/x-theora" => Codec::Theora,
        b"image/jpeg" => Codec::MotionJpeg,
        b"video/x-prores" => Codec::ProRes,
        b"video/x-h263" => Codec::H263,
        b"audio/mpeg" => match (mpeg_version, layer) {
            (Some(2 | 4), _) => Codec::Aac,
            (Some(1), Some(3)) => Codec::Mp3,
            _ => Codec::Unknown,
        },
        b"audio/x-ac3" | b"audio/ac3" => Codec::Ac3,
        b"audio/x-eac3" => Codec::Eac3,
        b"audio/x-wma" => Codec::Wma,
        b"audio/x-vorbis" => Codec::Vorbis,
        b"audio/x-flac" => Codec::Flac,
        b"audio/x-dts" => Codec::Dts,
        b"audio/x-opus" => Codec::Opus,
        _ => Codec::Unknown,
    }
}

/// GStreamer's colour enumerations in FFmpeg's (ISO/IEC 23091-2) code points, so that the
/// bundled profile's rules decide: unknown values map to "unspecified" (2), anything without
/// an equivalent there to a value those rules refuse.
fn primaries(value: c_int) -> i32 {
    match value {
        0 => 2, // unknown
        1 => 1, // BT.709
        3 => 5, // BT.470BG
        4 => 6, // SMPTE 170M
        _ => 9, // BT.470M, SMPTE 240M, film, BT.2020, Adobe RGB, P3, ...: refused
    }
}

fn transfer(value: c_int) -> i32 {
    match value {
        0 => 2,  // unknown
        4 => 4,  // gamma 2.2 (BT.470M)
        5 => 1,  // BT.709
        7 => 13, // sRGB
        8 => 5,  // gamma 2.8 (BT.470BG)
        16 => 6, // BT.601
        _ => 16, // linear, log, BT.2020 10/12-bit, PQ, HLG, ...: refused like PQ
    }
}

/// Whether the decoded picture's colour is shown, and whether the BT.709 camera transfer is
/// decoded to sRGB afterwards (GStreamer's converter applies the YCbCr matrix and range).
pub(super) fn transfer_709(colorimetry: Option<Colorimetry>, rgb: bool) -> io::Result<bool> {
    let Some(colorimetry) = colorimetry else {
        return Ok(!rgb);
    };
    let (primaries, transfer) = (
        primaries(colorimetry.primaries),
        transfer(colorimetry.transfer),
    );
    if !picture::displayable(primaries, transfer) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Video color space requires unsupported gamut or HDR conversion",
        ));
    }
    Ok(transfer != 13 && !(transfer == 2 && rgb))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_names_map_to_the_shared_codec_table() {
        assert_eq!(from_name(b"video/x-h264", None, None), Codec::H264);
        assert_eq!(from_name(b"video/x-h265", None, None), Codec::Hevc);
        assert_eq!(from_name(b"video/mpeg", Some(4), None), Codec::Mpeg4Part2);
        assert_eq!(from_name(b"video/mpeg", Some(2), None), Codec::Mpeg2);
        assert_eq!(from_name(b"video/x-msmpeg", None, None), Codec::Mpeg4Part2);
        assert_eq!(from_name(b"audio/mpeg", Some(4), None), Codec::Aac);
        assert_eq!(from_name(b"audio/mpeg", Some(1), Some(3)), Codec::Mp3);
        assert_eq!(from_name(b"audio/mpeg", Some(1), Some(2)), Codec::Unknown);
        assert_eq!(from_name(b"text/x-raw", None, None), Codec::Unknown);
    }

    #[test]
    fn colorimetry_follows_the_bundled_rules() {
        let bt709 = Colorimetry {
            range: 2,
            matrix: 3,
            transfer: 5,
            primaries: 1,
        };
        assert!(transfer_709(Some(bt709), false).unwrap());
        let srgb = Colorimetry {
            range: 1,
            matrix: 1,
            transfer: 7,
            primaries: 1,
        };
        assert!(!transfer_709(Some(srgb), true).unwrap());
        let sd = Colorimetry {
            range: 2,
            matrix: 4,
            transfer: 16,
            primaries: 4,
        };
        assert!(transfer_709(Some(sd), false).unwrap());
        for refused in [
            Colorimetry {
                range: 2,
                matrix: 6,
                transfer: 14,
                primaries: 7,
            },
            Colorimetry {
                range: 2,
                matrix: 6,
                transfer: 5,
                primaries: 7,
            },
            Colorimetry {
                range: 2,
                matrix: 3,
                transfer: 15,
                primaries: 1,
            },
        ] {
            assert_eq!(
                transfer_709(Some(refused), false).unwrap_err().kind(),
                io::ErrorKind::Unsupported
            );
        }
        assert!(transfer_709(None, false).unwrap());
    }
}

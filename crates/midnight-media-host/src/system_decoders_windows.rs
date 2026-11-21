//! Windows wording: Media Foundation decodes what the bundled profile does not, and the
//! codecs it lacks come from free Microsoft Store extensions or, on N editions, the Media
//! Feature Pack.
use super::{TRIED_MEDIA_FOUNDATION, UNSUPPORTED, codec_name};

pub(super) fn missing_decoder(codec: u8, tried: u8) -> String {
    let Some(name) = codec_name(codec) else {
        return UNSUPPORTED.to_owned();
    };
    if tried & TRIED_MEDIA_FOUNDATION == 0 {
        // The Windows decoders' helper is not installed, so only the bundled profile was tried.
        return format!("This video uses {name}; the installed video decoders do not support it");
    }
    match extension(codec) {
        Some(extension) => {
            format!("This video uses {name}; Windows has no decoder for it installed ({extension})")
        }
        None => format!("This video uses {name}; Windows has no decoder for it"),
    }
}

/// What adds a Windows decoder for the codec.
fn extension(codec: u8) -> Option<&'static str> {
    match codec {
        2 => Some("HEVC Video Extensions"),
        6 => Some("AV1 Video Extension"),
        9 => Some("VP9 Video Extensions"),
        4 | 5 => Some("MPEG-2 Video Extension"),
        10 | 11 => Some("Web Media Extensions"),
        // Windows includes these; N and KN editions add them with the Media Feature Pack.
        1 | 3 | 7 | 12 | 14 => Some("Media Feature Pack"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::TRIED_BUNDLED;
    use super::*;

    #[test]
    fn a_codec_windows_lacks_names_what_installs_its_decoder() {
        assert_eq!(
            missing_decoder(2, TRIED_BUNDLED | TRIED_MEDIA_FOUNDATION),
            "This video uses HEVC; Windows has no decoder for it installed (HEVC Video Extensions)"
        );
        assert_eq!(
            missing_decoder(6, TRIED_MEDIA_FOUNDATION),
            "This video uses AV1; Windows has no decoder for it installed (AV1 Video Extension)"
        );
        assert_eq!(
            missing_decoder(1, TRIED_MEDIA_FOUNDATION),
            "This video uses H.264; Windows has no decoder for it installed (Media Feature Pack)"
        );
        assert_eq!(
            missing_decoder(13, TRIED_MEDIA_FOUNDATION),
            "This video uses Apple ProRes; Windows has no decoder for it"
        );
    }

    #[test]
    fn without_the_windows_helper_or_a_codec_the_message_claims_nothing_about_windows() {
        assert_eq!(
            missing_decoder(2, TRIED_BUNDLED),
            "This video uses HEVC; the installed video decoders do not support it"
        );
        assert_eq!(missing_decoder(0, TRIED_MEDIA_FOUNDATION), UNSUPPORTED);
        assert_eq!(missing_decoder(15, TRIED_MEDIA_FOUNDATION), UNSUPPORTED);
    }
}

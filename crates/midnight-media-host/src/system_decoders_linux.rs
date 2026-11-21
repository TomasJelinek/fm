//! Linux wording: the platform helper decodes with the distribution's FFmpeg, then GStreamer.
//! A missing decoder names the codec and what to install, with the package names of the
//! running distribution family (read once from os-release). The helper reports only numbers;
//! every word here is the host's.
use super::{TRIED_DISTRIBUTION_FFMPEG, TRIED_GSTREAMER, UNSUPPORTED, codec_name};
use std::sync::OnceLock;

/// Distribution families whose packages the message can name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    /// Debian, Ubuntu and their derivatives: FFmpeg there includes every decoder.
    Debian,
    /// Fedora and the RHEL family: the distribution's `ffmpeg-free` omits patent-encumbered
    /// decoders; RPM Fusion has the full FFmpeg.
    Fedora,
    /// openSUSE: its FFmpeg omits them too; Packman has the full one.
    Suse,
    /// Arch and its derivatives: FFmpeg includes every decoder.
    Arch,
    Other,
}

/// The family from os-release's `ID` and `ID_LIKE`.
fn family(os_release: &str) -> Family {
    let mut names = Vec::new();
    for line in os_release.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == "ID" || key.trim() == "ID_LIKE" {
            let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
            names.extend(value.split_whitespace().map(str::to_ascii_lowercase));
        }
    }
    let has = |wanted: &[&str]| names.iter().any(|name| wanted.contains(&name.as_str()));
    if has(&["debian", "ubuntu"]) {
        Family::Debian
    } else if has(&["fedora", "rhel", "centos", "rocky", "almalinux"]) {
        Family::Fedora
    } else if has(&[
        "suse",
        "opensuse",
        "opensuse-tumbleweed",
        "opensuse-leap",
        "sles",
    ]) {
        Family::Suse
    } else if has(&["arch", "manjaro", "endeavouros"]) {
        Family::Arch
    } else {
        Family::Other
    }
}

fn running_family() -> Family {
    static FAMILY: OnceLock<Family> = OnceLock::new();
    *FAMILY.get_or_init(|| {
        ["/etc/os-release", "/usr/lib/os-release"]
            .iter()
            .find_map(|path| std::fs::read_to_string(path).ok())
            .map_or(Family::Other, |text| family(&text))
    })
}

/// What provides the distribution's FFmpeg libraries, and its full build where the
/// distribution's own omits patent-encumbered decoders.
fn ffmpeg_packages(family: Family) -> &'static str {
    match family {
        Family::Debian => "package ffmpeg",
        Family::Fedora => "ffmpeg-libs from RPM Fusion",
        Family::Suse => "ffmpeg from Packman",
        Family::Arch => "package ffmpeg",
        Family::Other => "libavcodec, libavformat, libswscale and libswresample",
    }
}

/// What adds GStreamer decoders for `codec`.
fn gstreamer_packages(family: Family, codec: u8) -> &'static str {
    match (family, codec) {
        (Family::Debian, _) => "package gstreamer1.0-libav",
        // Fedora's own gst-libav uses ffmpeg-free; OpenH264 comes from Cisco's repository.
        (Family::Fedora, 1) => "gstreamer1-plugin-openh264",
        (Family::Fedora, _) => "gstreamer1-plugin-libav with RPM Fusion's FFmpeg",
        (Family::Suse, 1) => "gstreamer-plugin-openh264",
        (Family::Suse, _) => "gstreamer-plugins-libav from Packman",
        (Family::Arch, _) => "package gst-libav",
        (Family::Other, _) => "gst-libav",
    }
}

pub(super) fn missing_decoder(codec: u8, tried: u8) -> String {
    message(codec, tried, running_family())
}

fn message(codec: u8, tried: u8, family: Family) -> String {
    let ffmpeg = tried & TRIED_DISTRIBUTION_FFMPEG != 0;
    let gstreamer = tried & TRIED_GSTREAMER != 0;
    // No helper named the codec (the shared rule for every platform): unsupported content.
    let Some(name) = codec_name(codec) else {
        return UNSUPPORTED.to_owned();
    };
    match (ffmpeg, gstreamer, family) {
        (false, _, _) => format!(
            "This video uses {name}; install your distribution's FFmpeg libraries ({}) or \
             GStreamer's gst-libav plugin ({}) to view it.",
            ffmpeg_packages(family),
            gstreamer_packages(family, codec)
        ),
        // The distribution's FFmpeg lacks this decoder (Fedora's and openSUSE's own builds
        // omit patent-encumbered codecs).
        (true, false, Family::Fedora | Family::Suse) => format!(
            "This video uses {name}; your distribution's FFmpeg does not decode it. Install the \
             full FFmpeg ({}) or GStreamer's plugins ({}) to view it.",
            ffmpeg_packages(family),
            gstreamer_packages(family, codec)
        ),
        (true, false, _) => format!(
            "This video uses {name}; your distribution's FFmpeg does not decode it. Install \
             GStreamer's plugins ({}) to try them.",
            gstreamer_packages(family, codec)
        ),
        (true, true, Family::Fedora | Family::Suse) => format!(
            "This video uses {name}; neither your distribution's FFmpeg nor GStreamer decodes \
             it. Install the full FFmpeg ({}) or {} to view it.",
            ffmpeg_packages(family),
            gstreamer_packages(family, codec)
        ),
        (true, true, _) => format!(
            "This video uses {name}; neither your distribution's FFmpeg nor GStreamer decodes it."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::super::TRIED_BUNDLED;
    use super::*;

    #[test]
    fn os_release_names_the_family() {
        assert_eq!(
            family("NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\n"),
            Family::Debian
        );
        assert_eq!(
            family("ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n"),
            Family::Debian
        );
        assert_eq!(family("ID=fedora\nVERSION_ID=43\n"), Family::Fedora);
        assert_eq!(
            family("ID=\"rocky\"\nID_LIKE=\"rhel centos fedora\"\n"),
            Family::Fedora
        );
        assert_eq!(
            family("ID=\"opensuse-tumbleweed\"\nID_LIKE=\"opensuse suse\"\n"),
            Family::Suse
        );
        assert_eq!(family("ID=endeavouros\nID_LIKE=arch\n"), Family::Arch);
        assert_eq!(family("ID=nixos\n"), Family::Other);
        assert_eq!(family(""), Family::Other);
    }

    #[test]
    fn with_no_system_decoder_the_message_names_the_codec_and_the_packages() {
        assert_eq!(
            message(1, TRIED_BUNDLED, Family::Debian),
            "This video uses H.264; install your distribution's FFmpeg libraries (package ffmpeg) \
             or GStreamer's gst-libav plugin (package gstreamer1.0-libav) to view it."
        );
        assert_eq!(
            message(2, TRIED_BUNDLED | TRIED_GSTREAMER, Family::Arch),
            "This video uses HEVC; install your distribution's FFmpeg libraries (package ffmpeg) \
             or GStreamer's gst-libav plugin (package gst-libav) to view it."
        );
        assert_eq!(
            message(1, TRIED_BUNDLED, Family::Other),
            "This video uses H.264; install your distribution's FFmpeg libraries (libavcodec, \
             libavformat, libswscale and libswresample) or GStreamer's gst-libav plugin \
             (gst-libav) to view it."
        );
    }

    #[test]
    fn a_distribution_ffmpeg_without_the_codec_points_to_the_full_build() {
        assert_eq!(
            message(1, TRIED_BUNDLED | TRIED_DISTRIBUTION_FFMPEG, Family::Fedora),
            "This video uses H.264; your distribution's FFmpeg does not decode it. Install the \
             full FFmpeg (ffmpeg-libs from RPM Fusion) or GStreamer's plugins \
             (gstreamer1-plugin-openh264) to view it."
        );
        assert_eq!(
            message(
                2,
                TRIED_BUNDLED | TRIED_DISTRIBUTION_FFMPEG | TRIED_GSTREAMER,
                Family::Suse
            ),
            "This video uses HEVC; neither your distribution's FFmpeg nor GStreamer decodes it. \
             Install the full FFmpeg (ffmpeg from Packman) or gstreamer-plugins-libav from Packman \
             to view it."
        );
        assert_eq!(
            message(
                13,
                TRIED_DISTRIBUTION_FFMPEG | TRIED_GSTREAMER,
                Family::Debian
            ),
            "This video uses Apple ProRes; neither your distribution's FFmpeg nor GStreamer \
             decodes it."
        );
        assert_eq!(
            message(3, TRIED_DISTRIBUTION_FFMPEG, Family::Debian),
            "This video uses MPEG-4 Part 2; your distribution's FFmpeg does not decode it. \
             Install GStreamer's plugins (package gstreamer1.0-libav) to try them."
        );
    }

    #[test]
    fn an_unnamed_codec_stays_generic() {
        for tried in [TRIED_BUNDLED, TRIED_DISTRIBUTION_FFMPEG | TRIED_GSTREAMER] {
            assert_eq!(message(0, tried, Family::Debian), UNSUPPORTED);
        }
    }

    #[test]
    fn the_running_system_has_a_family_and_every_message_is_one_line() {
        let family = running_family();
        for codec in 0..=24 {
            for tried in 0..8 {
                let text = message(codec, tried, family);
                assert!(!text.is_empty() && !text.contains('\n'), "{text}");
            }
        }
    }
}

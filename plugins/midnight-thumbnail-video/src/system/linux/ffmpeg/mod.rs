//! The distribution's FFmpeg, loaded at run time by soname: libavcodec 58 to 63 (FFmpeg 4 to
//! 9). Each major series has its own struct layout, generated from that series' public headers
//! (`layouts.rs`), and its own API generation for channel layouts and stream side data. Only
//! one complete library set of one series is ever mapped: the newest series whose
//! libavformat loads (with its libavcodec and libavutil) is chosen, its libswscale and
//! libswresample must match it, and the reported versions must match the file names.
mod audio;
mod input;
#[allow(dead_code)]
#[rustfmt::skip]
mod layouts;
mod video;

pub(super) use audio::{Audio, Opened as AudioOpened};
pub(super) use input::{Input, Selected};
pub(super) use video::{Thumbnail, Video};

use super::{
    Search,
    library::{self, Library, LoadError, Trust},
};
use layouts::{LAYOUTS, Layout};
use std::{
    ffi::{CStr, OsString, c_char, c_int, c_uint, c_void},
    fmt,
};

pub(super) const AVERROR_EOF: c_int = -541_478_725;
pub(super) const AVERROR_EAGAIN: c_int = -11;
const AVSEEK_SIZE: c_int = 0x10000;
const AVSEEK_FORCE: c_int = 0x20000;
pub(super) const AV_NOPTS_VALUE: i64 = i64::MIN;
pub(super) const AV_TIME_BASE: i64 = 1_000_000;
const AV_DISPOSITION_ATTACHED_PIC: c_int = 0x0400;
const AVMEDIA_TYPE_VIDEO: c_int = 0;
const AVMEDIA_TYPE_AUDIO: c_int = 1;
const AV_PKT_DATA_DISPLAYMATRIX: c_int = 5;
const AV_PIX_FMT_RGBA: c_int = 26;
const AV_SAMPLE_FMT_S16: c_int = 1;
const SWS_BILINEAR: c_int = 2;
const SWS_CS_ITU709: c_int = 1;
const SWS_CS_ITU601: c_int = 5;
const AV_CHANNEL_ORDER_NATIVE: c_int = 1;
const AV_CH_LAYOUT_STEREO: u64 = 3;
const AV_LOG_QUIET: c_int = -8;

type ReadPacket = unsafe extern "C" fn(*mut c_void, *mut u8, c_int) -> c_int;
type SeekPacket = unsafe extern "C" fn(*mut c_void, i64, c_int) -> i64;

/// The FFmpeg functions the decoder calls. Every signature is the same in all supported series
/// (recorded per series at the end of `layouts.rs`) except the two generations below.
pub(super) struct Api {
    layout: &'static Layout,
    #[cfg_attr(not(all(test, not(feature = "bundled"))), allow(dead_code))]
    pub(super) versions: Versions,
    av_malloc: unsafe extern "C" fn(usize) -> *mut c_void,
    av_free: unsafe extern "C" fn(*mut c_void),
    av_dict_set:
        unsafe extern "C" fn(*mut *mut c_void, *const c_char, *const c_char, c_int) -> c_int,
    av_dict_free: unsafe extern "C" fn(*mut *mut c_void),
    av_frame_alloc: unsafe extern "C" fn() -> *mut c_void,
    av_frame_free: unsafe extern "C" fn(*mut *mut c_void),
    av_frame_unref: unsafe extern "C" fn(*mut c_void),
    av_frame_move_ref: unsafe extern "C" fn(*mut c_void, *mut c_void),
    av_display_rotation_get: unsafe extern "C" fn(*const i32) -> f64,
    avcodec_find_decoder: unsafe extern "C" fn(c_int) -> *const c_void,
    avcodec_alloc_context3: unsafe extern "C" fn(*const c_void) -> *mut c_void,
    avcodec_parameters_to_context: unsafe extern "C" fn(*mut c_void, *const c_void) -> c_int,
    avcodec_open2: unsafe extern "C" fn(*mut c_void, *const c_void, *mut *mut c_void) -> c_int,
    avcodec_send_packet: unsafe extern "C" fn(*mut c_void, *const c_void) -> c_int,
    avcodec_receive_frame: unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int,
    avcodec_flush_buffers: unsafe extern "C" fn(*mut c_void),
    avcodec_free_context: unsafe extern "C" fn(*mut *mut c_void),
    avcodec_get_name: unsafe extern "C" fn(c_int) -> *const c_char,
    av_packet_alloc: unsafe extern "C" fn() -> *mut c_void,
    av_packet_free: unsafe extern "C" fn(*mut *mut c_void),
    av_packet_unref: unsafe extern "C" fn(*mut c_void),
    avformat_alloc_context: unsafe extern "C" fn() -> *mut c_void,
    avformat_open_input: unsafe extern "C" fn(
        *mut *mut c_void,
        *const c_char,
        *const c_void,
        *mut *mut c_void,
    ) -> c_int,
    avformat_find_stream_info: unsafe extern "C" fn(*mut c_void, *mut *mut c_void) -> c_int,
    avformat_close_input: unsafe extern "C" fn(*mut *mut c_void),
    av_find_best_stream:
        unsafe extern "C" fn(*mut c_void, c_int, c_int, c_int, *mut *const c_void, c_int) -> c_int,
    av_read_frame: unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int,
    avformat_seek_file: unsafe extern "C" fn(*mut c_void, c_int, i64, i64, i64, c_int) -> c_int,
    avio_alloc_context: unsafe extern "C" fn(
        *mut u8,
        c_int,
        c_int,
        *mut c_void,
        Option<ReadPacket>,
        Option<ReadPacket>,
        Option<SeekPacket>,
    ) -> *mut c_void,
    avio_context_free: unsafe extern "C" fn(*mut *mut c_void),
    sws_get_context: unsafe extern "C" fn(
        c_int,
        c_int,
        c_int,
        c_int,
        c_int,
        c_int,
        c_int,
        *mut c_void,
        *mut c_void,
        *const f64,
    ) -> *mut c_void,
    sws_scale: unsafe extern "C" fn(
        *mut c_void,
        *const *const u8,
        *const c_int,
        c_int,
        c_int,
        *const *mut u8,
        *const c_int,
    ) -> c_int,
    sws_free_context: unsafe extern "C" fn(*mut c_void),
    sws_set_colorspace_details: unsafe extern "C" fn(
        *mut c_void,
        *const c_int,
        c_int,
        *const c_int,
        c_int,
        c_int,
        c_int,
        c_int,
    ) -> c_int,
    sws_get_coefficients: unsafe extern "C" fn(c_int) -> *const c_int,
    swr_init: unsafe extern "C" fn(*mut c_void) -> c_int,
    swr_convert:
        unsafe extern "C" fn(*mut c_void, *const *mut u8, c_int, *const *const u8, c_int) -> c_int,
    swr_get_out_samples: unsafe extern "C" fn(*mut c_void, c_int) -> c_int,
    swr_free: unsafe extern "C" fn(*mut *mut c_void),
    pub(super) channels: Channels,
    pub(super) side_data: SideData,
}

/// Audio channel layouts: a 64-bit mask before FFmpeg 5.1, `AVChannelLayout` from then on.
/// A series uses the newer one only when every release of it has the frame field.
pub(super) enum Channels {
    Mask {
        av_get_default_channel_layout: unsafe extern "C" fn(c_int) -> i64,
        swr_alloc_set_opts: unsafe extern "C" fn(
            *mut c_void,
            i64,
            c_int,
            c_int,
            i64,
            c_int,
            c_int,
            c_int,
            *mut c_void,
        ) -> *mut c_void,
    },
    Layout {
        av_channel_layout_default: unsafe extern "C" fn(*mut c_void, c_int),
        av_channel_layout_uninit: unsafe extern "C" fn(*mut c_void),
        swr_alloc_set_opts2: unsafe extern "C" fn(
            *mut *mut c_void,
            *const c_void,
            c_int,
            c_int,
            *const c_void,
            c_int,
            c_int,
            c_int,
            *mut c_void,
        ) -> c_int,
    },
}

/// The display matrix: stream side data before FFmpeg 7, codec parameters from then on.
pub(super) enum SideData {
    /// `av_stream_get_side_data`, whose size is an `int *` in FFmpeg 4 and a `size_t *` after.
    StreamInt(unsafe extern "C" fn(*const c_void, c_int, *mut c_int) -> *mut u8),
    StreamSize(unsafe extern "C" fn(*const c_void, c_int, *mut usize) -> *mut u8),
    Parameters(unsafe extern "C" fn(*const c_void, c_int, c_int) -> *const c_void),
}

/// AV_VERSION_INT of each loaded library.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Versions {
    pub(super) avcodec: u32,
    pub(super) avformat: u32,
    pub(super) avutil: u32,
    pub(super) swscale: u32,
    pub(super) swresample: u32,
}

impl fmt::Display for Versions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let part = |version: u32| {
            format!(
                "{}.{}.{}",
                version >> 16,
                (version >> 8) & 0xff,
                version & 0xff
            )
        };
        write!(
            f,
            "libavcodec {}, libavformat {}, libavutil {}, libswscale {}, libswresample {}",
            part(self.avcodec),
            part(self.avformat),
            part(self.avutil),
            part(self.swscale),
            part(self.swresample)
        )
    }
}

/// The loaded FFmpeg. It lives as long as the helper process (libraries are never unloaded).
pub(super) struct Ffmpeg {
    pub(super) api: Api,
    _libraries: Vec<Library>,
}

/// Why the distribution's FFmpeg is not used.
#[derive(Debug)]
pub(super) enum Unavailable {
    /// No supported libavformat (58 to 63) loads.
    NotFound,
    /// A series was found but cannot be used (incomplete, untrusted or not what it claims).
    Unusable { avcodec: u32, error: LoadError },
    /// The process already maps another FFmpeg series.
    Mixed(Vec<(String, u32)>),
}

impl fmt::Display for Unavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => {
                f.write_str("no supported FFmpeg (libavformat 58 to 63) is installed")
            }
            Self::Unusable { avcodec, error } => {
                write!(
                    f,
                    "the FFmpeg with libavcodec {avcodec} cannot be used: {error}"
                )
            }
            Self::Mixed(found) => write!(f, "another FFmpeg is already loaded: {found:?}"),
        }
    }
}

fn soname(stem: &str, major: u32) -> String {
    format!("{stem}.so.{major}")
}

/// Load the newest supported FFmpeg series. Call before worker lockdown, once per process.
pub(super) fn load(search: &Search, trust: Trust) -> Result<&'static Ffmpeg, Unavailable> {
    let already = library::ffmpeg_sonames();
    if !already.is_empty() {
        return Err(Unavailable::Mixed(already));
    }
    for layout in LAYOUTS.iter().rev() {
        let names = [
            soname("libavformat", layout.avformat),
            soname("libavcodec", layout.avcodec),
            soname("libavutil", layout.avutil),
            soname("libswscale", layout.swscale),
            soname("libswresample", layout.swresample),
        ];
        let paths: Vec<OsString> = match search {
            Search::System => names.iter().map(OsString::from).collect(),
            #[cfg(all(test, not(feature = "bundled")))]
            Search::Folder(folder) => {
                // A private build has no RUNPATH: load dependencies first, by absolute path,
                // so each later library finds them by soname among the loaded objects.
                if !folder.join(&names[0]).exists() {
                    continue;
                }
                [2, 4, 3, 1, 0]
                    .iter()
                    .map(|index| folder.join(&names[*index]).into_os_string())
                    .collect()
            }
            #[cfg(all(test, not(feature = "bundled")))]
            Search::Hidden => return Err(Unavailable::NotFound),
        };
        // The first library decides the series: libavformat loads atomically with its
        // libavcodec and libavutil, or not at all, so a missing series maps nothing.
        let first = match Library::open(&paths[0], trust) {
            Ok(library) => library,
            Err(LoadError::Missing(_)) => continue,
            Err(error) => {
                return Err(Unavailable::Unusable {
                    avcodec: layout.avcodec,
                    error,
                });
            }
        };
        // From here this series is committed: trying another would map two of them.
        let mut libraries = vec![first];
        for path in &paths[1..] {
            libraries.push(
                Library::open(path, trust).map_err(|error| Unavailable::Unusable {
                    avcodec: layout.avcodec,
                    error,
                })?,
            );
        }
        let api = Api::resolve(layout, &libraries).map_err(|error| Unavailable::Unusable {
            avcodec: layout.avcodec,
            error,
        })?;
        let mapped = library::ffmpeg_sonames();
        let expected = [
            ("libavformat", layout.avformat),
            ("libavcodec", layout.avcodec),
            ("libavutil", layout.avutil),
            ("libswscale", layout.swscale),
            ("libswresample", layout.swresample),
        ];
        if mapped
            .iter()
            .any(|(stem, major)| !expected.contains(&(stem.as_str(), *major)))
        {
            return Err(Unavailable::Mixed(mapped));
        }
        // SAFETY: the series is loaded; logging is process-global and only silences output.
        unsafe { set_quiet(&libraries) };
        return Ok(Box::leak(Box::new(Ffmpeg {
            api,
            _libraries: libraries,
        })));
    }
    Err(Unavailable::NotFound)
}

/// Silence FFmpeg's log (stderr is discarded anyway; formatting costs time).
unsafe fn set_quiet(libraries: &[Library]) {
    for library in libraries {
        // SAFETY: av_log_set_level(int) in every supported libavutil.
        if let Some(set) =
            unsafe { library.optional::<unsafe extern "C" fn(c_int)>(c"av_log_set_level") }
        {
            // SAFETY: plain process-global setter.
            unsafe { set(AV_LOG_QUIET) };
            return;
        }
    }
}

impl Api {
    fn resolve(layout: &'static Layout, libraries: &[Library]) -> Result<Self, LoadError> {
        // Resolve each name in whichever loaded library defines it (dlsym on one handle also
        // searches that library's dependencies, which are the same series).
        let find = |name: &CStr| -> Result<*mut c_void, LoadError> {
            for library in libraries {
                // SAFETY: only the address is read here; it is typed at its single use below.
                if let Some(symbol) = unsafe { library.optional::<*mut c_void>(name) } {
                    return Ok(symbol);
                }
            }
            Err(LoadError::Symbol(name.to_string_lossy().into_owned()))
        };
        macro_rules! symbol {
            ($name:literal) => {{
                let address = find($name)?;
                // SAFETY: the declared field type is the symbol's C signature in every
                // supported series (see the declarations recorded in layouts.rs).
                unsafe { cast(address) }
            }};
        }
        let version = |name: &CStr| -> Result<u32, LoadError> {
            let address = find(name)?;
            // SAFETY: `unsigned name(void)` in every library.
            let version: unsafe extern "C" fn() -> c_uint = unsafe { cast(address) };
            // SAFETY: no arguments, no preconditions.
            Ok(unsafe { version() })
        };
        let versions = Versions {
            avcodec: version(c"avcodec_version")?,
            avformat: version(c"avformat_version")?,
            avutil: version(c"avutil_version")?,
            swscale: version(c"swscale_version")?,
            swresample: version(c"swresample_version")?,
        };
        check_versions(layout, versions)?;
        let channels = if layout.frame_ch_layout.is_some() {
            Channels::Layout {
                av_channel_layout_default: symbol!(c"av_channel_layout_default"),
                av_channel_layout_uninit: symbol!(c"av_channel_layout_uninit"),
                swr_alloc_set_opts2: symbol!(c"swr_alloc_set_opts2"),
            }
        } else {
            Channels::Mask {
                av_get_default_channel_layout: symbol!(c"av_get_default_channel_layout"),
                swr_alloc_set_opts: symbol!(c"swr_alloc_set_opts"),
            }
        };
        let side_data =
            if layout.par_coded_side_data.is_some() && layout.has_av_packet_side_data_get {
                SideData::Parameters(symbol!(c"av_packet_side_data_get"))
            } else {
                match layout.stream_side_data_int_size {
                    Some(true) => SideData::StreamInt(symbol!(c"av_stream_get_side_data")),
                    Some(false) => SideData::StreamSize(symbol!(c"av_stream_get_side_data")),
                    None => return Err(LoadError::Symbol("av_stream_get_side_data".into())),
                }
            };
        Ok(Self {
            layout,
            versions,
            av_malloc: symbol!(c"av_malloc"),
            av_free: symbol!(c"av_free"),
            av_dict_set: symbol!(c"av_dict_set"),
            av_dict_free: symbol!(c"av_dict_free"),
            av_frame_alloc: symbol!(c"av_frame_alloc"),
            av_frame_free: symbol!(c"av_frame_free"),
            av_frame_unref: symbol!(c"av_frame_unref"),
            av_frame_move_ref: symbol!(c"av_frame_move_ref"),
            av_display_rotation_get: symbol!(c"av_display_rotation_get"),
            avcodec_find_decoder: symbol!(c"avcodec_find_decoder"),
            avcodec_alloc_context3: symbol!(c"avcodec_alloc_context3"),
            avcodec_parameters_to_context: symbol!(c"avcodec_parameters_to_context"),
            avcodec_open2: symbol!(c"avcodec_open2"),
            avcodec_send_packet: symbol!(c"avcodec_send_packet"),
            avcodec_receive_frame: symbol!(c"avcodec_receive_frame"),
            avcodec_flush_buffers: symbol!(c"avcodec_flush_buffers"),
            avcodec_free_context: symbol!(c"avcodec_free_context"),
            avcodec_get_name: symbol!(c"avcodec_get_name"),
            av_packet_alloc: symbol!(c"av_packet_alloc"),
            av_packet_free: symbol!(c"av_packet_free"),
            av_packet_unref: symbol!(c"av_packet_unref"),
            avformat_alloc_context: symbol!(c"avformat_alloc_context"),
            avformat_open_input: symbol!(c"avformat_open_input"),
            avformat_find_stream_info: symbol!(c"avformat_find_stream_info"),
            avformat_close_input: symbol!(c"avformat_close_input"),
            av_find_best_stream: symbol!(c"av_find_best_stream"),
            av_read_frame: symbol!(c"av_read_frame"),
            avformat_seek_file: symbol!(c"avformat_seek_file"),
            avio_alloc_context: symbol!(c"avio_alloc_context"),
            avio_context_free: symbol!(c"avio_context_free"),
            sws_get_context: symbol!(c"sws_getContext"),
            sws_scale: symbol!(c"sws_scale"),
            sws_free_context: symbol!(c"sws_freeContext"),
            sws_set_colorspace_details: symbol!(c"sws_setColorspaceDetails"),
            sws_get_coefficients: symbol!(c"sws_getCoefficients"),
            swr_init: symbol!(c"swr_init"),
            swr_convert: symbol!(c"swr_convert"),
            swr_get_out_samples: symbol!(c"swr_get_out_samples"),
            swr_free: symbol!(c"swr_free"),
            channels,
            side_data,
        })
    }

    /// The codec's FFmpeg name (known for every codec id, decoder or not).
    pub(super) fn codec_name(&self, id: c_int) -> String {
        // SAFETY: avcodec_get_name returns a static string for any id ("unknown_codec" too).
        let name = unsafe { (self.avcodec_get_name)(id) };
        if name.is_null() {
            return String::new();
        }
        // SAFETY: static NUL-terminated string owned by libavcodec.
        unsafe { CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned()
    }
}

/// A resolved symbol address as the pointer type of its declaration.
///
/// # Safety
/// `T` must be the symbol's C type (a function pointer) in the loaded series.
unsafe fn cast<T: Copy>(address: *mut c_void) -> T {
    const { assert!(std::mem::size_of::<T>() == std::mem::size_of::<*mut c_void>()) };
    // SAFETY: same size; the caller guarantees the type.
    unsafe { std::mem::transmute_copy(&address) }
}

/// The loaded libraries are the series the layout describes: same majors, and not older than
/// the oldest release whose headers the layout was generated from.
fn check_versions(layout: &Layout, versions: Versions) -> Result<(), LoadError> {
    let majors = [
        ("libavcodec", versions.avcodec, layout.avcodec),
        ("libavformat", versions.avformat, layout.avformat),
        ("libavutil", versions.avutil, layout.avutil),
        ("libswscale", versions.swscale, layout.swscale),
        ("libswresample", versions.swresample, layout.swresample),
    ];
    for (name, version, major) in majors {
        if version >> 16 != major {
            return Err(LoadError::Version(format!(
                "{name} reports major {} for soname major {major}",
                version >> 16
            )));
        }
    }
    if versions.avcodec < layout.first_avcodec_version {
        return Err(LoadError::Version(format!(
            "libavcodec {:#x} predates the oldest supported release {:#x} of its series",
            versions.avcodec, layout.first_avcodec_version
        )));
    }
    Ok(())
}

/// Read a field of a native object at a generated offset.
///
/// # Safety
/// `object` must point to a live object of the struct the offset belongs to, in the loaded
/// series, and `T` must be the field's type.
pub(super) unsafe fn field<T: Copy>(object: *const c_void, offset: Option<usize>) -> Option<T> {
    // SAFETY: caller contract; unaligned reads tolerate any packing.
    offset.map(|offset| unsafe { object.cast::<u8>().add(offset).cast::<T>().read_unaligned() })
}

/// Write a field of a native object at a generated offset. Returns whether it exists.
///
/// # Safety
/// As for `field`, and the object must be exclusively owned by the caller.
pub(super) unsafe fn set_field<T: Copy>(
    object: *mut c_void,
    offset: Option<usize>,
    value: T,
) -> bool {
    match offset {
        Some(offset) => {
            // SAFETY: caller contract.
            unsafe {
                object
                    .cast::<u8>()
                    .add(offset)
                    .cast::<T>()
                    .write_unaligned(value)
            };
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_series_has_the_fields_the_decoder_requires() {
        for layout in &LAYOUTS {
            for (name, offset) in [
                ("format_pb", layout.format_pb),
                ("format_nb_streams", layout.format_nb_streams),
                ("format_streams", layout.format_streams),
                ("format_start_time", layout.format_start_time),
                ("format_duration", layout.format_duration),
                ("format_io_open", layout.format_io_open),
                ("stream_index", layout.stream_index),
                ("stream_time_base", layout.stream_time_base),
                ("stream_codecpar", layout.stream_codecpar),
                ("stream_disposition", layout.stream_disposition),
                ("par_codec_type", layout.par_codec_type),
                ("par_codec_id", layout.par_codec_id),
                ("par_width", layout.par_width),
                ("par_height", layout.par_height),
                ("codec_thread_count", layout.codec_thread_count),
                ("codec_thread_type", layout.codec_thread_type),
                ("codec_max_pixels", layout.codec_max_pixels),
                ("codec_pkt_timebase", layout.codec_pkt_timebase),
                ("frame_data", layout.frame_data),
                ("frame_linesize", layout.frame_linesize),
                ("frame_extended_data", layout.frame_extended_data),
                ("frame_width", layout.frame_width),
                ("frame_height", layout.frame_height),
                ("frame_nb_samples", layout.frame_nb_samples),
                ("frame_format", layout.frame_format),
                ("frame_pts", layout.frame_pts),
                (
                    "frame_best_effort_timestamp",
                    layout.frame_best_effort_timestamp,
                ),
                ("frame_sample_rate", layout.frame_sample_rate),
                ("frame_color_range", layout.frame_color_range),
                ("frame_color_primaries", layout.frame_color_primaries),
                ("frame_color_trc", layout.frame_color_trc),
                ("frame_colorspace", layout.frame_colorspace),
                ("packet_pts", layout.packet_pts),
                ("packet_stream_index", layout.packet_stream_index),
                ("io_buffer", layout.io_buffer),
            ] {
                assert!(offset.is_some(), "libavcodec {}: {name}", layout.avcodec);
            }
            // Exactly one channel-layout generation and one side-data path per series.
            let mask = layout.frame_channels.is_some() && layout.frame_channel_layout.is_some();
            assert!(
                mask || layout.frame_ch_layout.is_some(),
                "{}",
                layout.avcodec
            );
            if layout.frame_ch_layout.is_some() {
                assert!(layout.has_swr_alloc_set_opts2 && layout.has_av_channel_layout_default);
                assert_eq!(layout.ch_layout_size_of, Some(24));
            } else {
                assert!(layout.has_swr_alloc_set_opts && layout.has_av_get_default_channel_layout);
            }
            let parameters =
                layout.par_coded_side_data.is_some() && layout.has_av_packet_side_data_get;
            assert!(
                parameters || layout.stream_side_data_int_size.is_some(),
                "{}",
                layout.avcodec
            );
        }
    }

    #[test]
    fn series_are_ordered_and_their_sonames_match_the_ffmpeg_releases() {
        let sets: Vec<_> = LAYOUTS
            .iter()
            .map(|layout| {
                (
                    layout.avcodec,
                    layout.avformat,
                    layout.avutil,
                    layout.swscale,
                    layout.swresample,
                )
            })
            .collect();
        assert_eq!(
            sets,
            [
                (58, 58, 56, 5, 3),
                (59, 59, 57, 6, 4),
                (60, 60, 58, 7, 4),
                (61, 61, 59, 8, 5),
                (62, 62, 60, 9, 6),
                (63, 63, 61, 10, 7),
            ]
        );
        // Layout changes the generator found between series, which a single hand-written
        // binding would get wrong: AVStream gained an AVClass in 60, AVFormatContext dropped
        // its 1 KiB filename in 59, and codec parameters grew in 61.
        assert_eq!(LAYOUTS[1].stream_index, Some(0));
        assert_eq!(LAYOUTS[2].stream_index, Some(8));
        assert_eq!(LAYOUTS[0].format_duration, Some(1096));
        assert_eq!(LAYOUTS[1].format_duration, Some(72));
        assert_eq!(LAYOUTS[3].par_width, Some(72));
        assert_eq!(LAYOUTS[0].stream_side_data_int_size, Some(true));
        assert_eq!(LAYOUTS[1].stream_side_data_int_size, Some(false));
    }

    #[test]
    fn versions_must_match_the_soname_series_and_its_oldest_release() {
        let layout = &LAYOUTS[2];
        let good = Versions {
            avcodec: (60 << 16) | (31 << 8) | 102,
            avformat: (60 << 16) | (16 << 8) | 100,
            avutil: (58 << 16) | (29 << 8) | 100,
            swscale: (7 << 16) | (5 << 8) | 100,
            swresample: (4 << 16) | (12 << 8) | 100,
        };
        assert!(check_versions(layout, good).is_ok());
        let wrong_major = Versions {
            avformat: 61 << 16,
            ..good
        };
        assert!(matches!(
            check_versions(layout, wrong_major),
            Err(LoadError::Version(_))
        ));
        let prerelease = Versions {
            avcodec: 60 << 16,
            ..good
        };
        assert!(matches!(
            check_versions(layout, prerelease),
            Err(LoadError::Version(_))
        ));
    }
}

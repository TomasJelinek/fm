//! GStreamer 1.x, loaded at run time: the fallback for codecs the distribution's FFmpeg does
//! not decode. Everything is prepared before lockdown, without the plugin registry:
//!
//! - `GST_REGISTRY_DISABLE=yes` (with `GST_REGISTRY_UPDATE=no`, `GST_REGISTRY_FORK=no`, an
//!   empty plugin path and `/dev/null` as registry file, for releases that predate it): no
//!   registry is read or written, nothing is scanned, no `gst-plugin-scanner` is started and
//!   nothing is written into the user's home.
//! - A fixed list of plugins is loaded explicitly from the plugin folder beside libgstreamer,
//!   each checked like any library (root-owned, writable by nobody else). Plugins that open
//!   devices (VA-API, V4L2, NVIDIA), reach the network, or draw are never loaded. gst-libav is
//!   loaded only when no distribution FFmpeg was loaded, so one process maps one FFmpeg.
//!
//! After lockdown the graph runs on the helper's one thread (see `pipeline`).
mod api;
mod codec;
mod pipeline;

use super::{
    Search,
    library::{self, Library, LoadError, Trust},
    picture,
};
use crate::{
    media::{AudioTrack, Block, Frame, Metadata, OutputBounds},
    system::pcm::{Clip, Timeline},
};
use api::Api;
use midnight_thumbnail_sdk::ReadSeek;
use pipeline::{Kind, Output, Pipeline};
use std::{
    ffi::{CStr, CString, OsStr, c_uint},
    fmt, io,
    os::unix::ffi::OsStrExt,
    path::PathBuf,
    ptr,
    sync::OnceLock,
};

pub(super) use pipeline::Refusal;

/// Upstream plugin names (`libgst<name>.so`) that may be loaded, and what they provide.
const PLUGINS: &[&str] = &[
    // GStreamer core: capsfilter.
    "coreelements",
    // gst-plugins-base: typefinding, converters, Ogg, Theora, Vorbis, Opus.
    "typefindfunctions",
    "videoconvertscale",
    "videoconvert",
    "videoscale",
    "audioconvert",
    "audioresample",
    "ogg",
    "theora",
    "vorbis",
    "opus",
    // gst-plugins-good: MP4/MOV, Matroska/WebM, AVI demuxers, parsers, VP8/VP9, FLAC, MP3.
    "isomp4",
    "matroska",
    "avi",
    "audioparsers",
    "vpx",
    "flac",
    "mpg123",
    // gst-plugins-bad: video parsers, OpenH264, libde265 (HEVC), AV1, AAC, DTS.
    "videoparsersbad",
    "openh264",
    "de265",
    "dav1d",
    "aom",
    "faad",
    "fdkaac",
    "dtsdec",
    // gst-plugins-ugly: AC-3.
    "a52dec",
    // gst-libav: the FFmpeg decoders (only when no distribution FFmpeg was loaded).
    "libav",
];

/// Elements the graph needs whatever the codec.
const REQUIRED: &[&CStr] = &[
    c"capsfilter",
    c"videoconvert",
    c"videoscale",
    c"audioconvert",
    c"audioresample",
];

pub(super) struct Gstreamer {
    api: Api,
    // What was found and loaded: read by the tests and kept for diagnosis.
    #[cfg_attr(not(all(test, not(feature = "bundled"))), allow(dead_code))]
    pub(super) version: (c_uint, c_uint, c_uint),
    #[cfg_attr(not(all(test, not(feature = "bundled"))), allow(dead_code))]
    pub(super) plugin_folder: PathBuf,
    #[cfg_attr(not(all(test, not(feature = "bundled"))), allow(dead_code))]
    pub(super) plugins: Vec<&'static str>,
    #[cfg_attr(not(all(test, not(feature = "bundled"))), allow(dead_code))]
    pub(super) skipped: Vec<(&'static str, String)>,
    /// Required elements that no loaded plugin provides.
    #[cfg_attr(not(all(test, not(feature = "bundled"))), allow(dead_code))]
    pub(super) incomplete: Vec<String>,
    _libraries: Vec<Library>,
}

#[derive(Debug)]
pub(super) enum Unavailable {
    NotFound,
    Unusable(LoadError),
    Init(String),
}

impl fmt::Display for Unavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("GStreamer 1.x is not installed"),
            Self::Unusable(error) => write!(f, "GStreamer cannot be used: {error}"),
            Self::Init(detail) => write!(f, "GStreamer did not initialize: {detail}"),
        }
    }
}

static LOADED: OnceLock<&'static Gstreamer> = OnceLock::new();

/// The loaded GStreamer's functions, for callbacks that only receive GStreamer objects.
fn loaded_api() -> Option<&'static Api> {
    LOADED.get().map(|gstreamer| &gstreamer.api)
}

/// Point GStreamer away from the registry, the user's plugin folders and debug output.
///
/// # Safety
/// Changes the process environment: call only while the process has one thread.
unsafe fn isolate_environment() {
    for (name, value) in [
        ("GST_REGISTRY_DISABLE", "yes"),
        ("GST_REGISTRY_UPDATE", "no"),
        ("GST_REGISTRY_FORK", "no"),
        ("GST_REGISTRY_1_0", "/dev/null"),
        ("GST_REGISTRY", "/dev/null"),
        ("GST_PLUGIN_SYSTEM_PATH_1_0", ""),
        ("GST_PLUGIN_SYSTEM_PATH", ""),
        ("GST_PLUGIN_PATH_1_0", ""),
        ("GST_PLUGIN_PATH", ""),
    ] {
        // SAFETY: the caller guarantees a single-threaded process.
        unsafe { std::env::set_var(name, value) };
    }
    for name in [
        "GST_PLUGIN_SCANNER",
        "GST_PLUGIN_SCANNER_1_0",
        "GST_PLUGIN_LOADING_WHITELIST",
        "GST_DEBUG",
        "GST_DEBUG_FILE",
        "GST_DEBUG_DUMP_DOT_DIR",
        "GST_TRACERS",
        "GST_TRACE",
    ] {
        // SAFETY: as above.
        unsafe { std::env::remove_var(name) };
    }
}

/// Load GStreamer and the allowed plugins. Call before lockdown, once per process, while the
/// process has one thread. `with_libav`: also load gst-libav (no distribution FFmpeg loaded).
pub(super) fn load(
    search: &Search,
    trust: Trust,
    with_libav: bool,
) -> Result<&'static Gstreamer, Unavailable> {
    if let Some(loaded) = LOADED.get() {
        return Ok(loaded);
    }
    match search {
        Search::System => {}
        #[cfg(all(test, not(feature = "bundled")))]
        Search::Folder(_) | Search::Hidden => return Err(Unavailable::NotFound),
    }
    let open = |name: &str| Library::open(OsStr::new(name), trust);
    let core = match open("libgstreamer-1.0.so.0") {
        Ok(core) => core,
        Err(LoadError::Missing(_)) => return Err(Unavailable::NotFound),
        Err(error) => return Err(Unavailable::Unusable(error)),
    };
    let mut libraries = vec![core];
    for name in [
        "libgstbase-1.0.so.0",
        "libgobject-2.0.so.0",
        "libglib-2.0.so.0",
    ] {
        libraries.push(open(name).map_err(Unavailable::Unusable)?);
    }
    // libgstvideo comes with gst-plugins-base; without it the converters are missing anyway.
    let video = open("libgstvideo-1.0.so.0").ok();
    let api = Api::resolve(&libraries, video.as_ref()).map_err(Unavailable::Unusable)?;
    let plugin_folder = libraries[0]
        .path
        .parent()
        .map(|folder| folder.join("gstreamer-1.0"))
        .ok_or_else(|| Unavailable::Init("libgstreamer has no folder".into()))?;
    // SAFETY: workers load decoders on their only thread, before lockdown.
    unsafe { isolate_environment() };
    // SAFETY: gst_init_check with no arguments; the error, if any, is ours to free.
    let initialized = unsafe {
        let mut error = ptr::null_mut();
        let ok = (api.gst_init_check)(ptr::null_mut(), ptr::null_mut(), &mut error);
        let detail = take_error(&api, error);
        if ok == 0 { Err(detail) } else { Ok(()) }
    };
    initialized.map_err(Unavailable::Init)?;
    // SAFETY: GLib's thread-pool functions with their declared signatures, on this thread.
    unsafe { prime_thread_pools(&libraries[3]) }.map_err(Unavailable::Unusable)?;
    let (mut major, mut minor, mut micro, mut nano) = (0, 0, 0, 0);
    // SAFETY: four out-parameters.
    unsafe { (api.gst_version)(&mut major, &mut minor, &mut micro, &mut nano) };
    if major != 1 {
        return Err(Unavailable::Init(format!(
            "GStreamer {major}.{minor} is not 1.x"
        )));
    }
    let mut plugins = Vec::new();
    let mut skipped = Vec::new();
    let folder_trusted = trust.check(&plugin_folder);
    for name in PLUGINS {
        if *name == "libav" && !with_libav {
            skipped.push((*name, "a distribution FFmpeg is loaded".into()));
            continue;
        }
        let path = plugin_folder.join(format!("libgst{name}.so"));
        if !path.exists() {
            skipped.push((*name, "not installed".into()));
            continue;
        }
        if let Err(error) = &folder_trusted {
            skipped.push((*name, error.to_string()));
            continue;
        }
        if let Err(error) = trust.check(&path) {
            skipped.push((*name, error.to_string()));
            continue;
        }
        let Ok(c_path) = CString::new(path.as_os_str().as_bytes()) else {
            continue;
        };
        // SAFETY: an allowed plugin file that passed the read rules; the returned reference
        // is released (the registry keeps its own).
        let loaded = unsafe {
            let mut error = ptr::null_mut();
            let plugin = (api.gst_plugin_load_file)(c_path.as_ptr(), &mut error);
            let detail = take_error(&api, error);
            if plugin.is_null() {
                Err(detail)
            } else {
                (api.gst_object_unref)(plugin);
                Ok(())
            }
        };
        match loaded {
            Ok(()) => plugins.push(*name),
            Err(detail) => skipped.push((*name, detail)),
        }
    }
    let incomplete = REQUIRED
        .iter()
        .filter(|name| {
            // SAFETY: a factory lookup; the reference is released.
            unsafe {
                let factory = (api.gst_element_factory_find)(name.as_ptr());
                if factory.is_null() {
                    true
                } else {
                    (api.gst_object_unref)(factory);
                    false
                }
            }
        })
        .map(|name| name.to_string_lossy().into_owned())
        .collect();
    if let Some(video) = video {
        libraries.push(video);
    }
    let gstreamer: &'static Gstreamer = Box::leak(Box::new(Gstreamer {
        api,
        version: (major, minor, micro),
        plugin_folder,
        plugins,
        skipped,
        incomplete,
        _libraries: libraries,
    }));
    let _ = LOADED.set(gstreamer);
    library::audit(trust).map_err(Unavailable::Unusable)?;
    Ok(gstreamer)
}

/// GLib's first shared thread pool records the creating thread's scheduler settings with
/// `sched_getattr`, or else starts a "pool-spawner" thread. After lockdown both are denied and
/// GLib aborts the process. GStreamer's video converter creates such a pool for every graph
/// (with one thread it never runs a task on it), so create and free one pool now, while GLib
/// can still read the settings: later pools then start no thread.
unsafe fn prime_thread_pools(glib: &Library) -> Result<(), LoadError> {
    type Task = unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void);
    unsafe extern "C" fn idle(_: *mut std::ffi::c_void, _: *mut std::ffi::c_void) {}
    // SAFETY: GThreadPool *g_thread_pool_new(GFunc, gpointer, gint, gboolean, GError **) and
    // void g_thread_pool_free(GThreadPool *, gboolean, gboolean) in every GLib 2.x.
    unsafe {
        let new: unsafe extern "C" fn(
            Task,
            *mut std::ffi::c_void,
            std::ffi::c_int,
            std::ffi::c_int,
            *mut *mut api::Error,
        ) -> *mut std::ffi::c_void = glib.symbol(c"g_thread_pool_new")?;
        let free: unsafe extern "C" fn(*mut std::ffi::c_void, std::ffi::c_int, std::ffi::c_int) =
            glib.symbol(c"g_thread_pool_free")?;
        let pool = new(idle, ptr::null_mut(), 1, 0, ptr::null_mut());
        if !pool.is_null() {
            free(pool, 1, 0);
        }
        // Buffer pools cannot create their wake-up descriptors after lockdown; in a graph on
        // one thread nothing ever waits on them, but each acquire and release then logs a
        // critical warning. Workers discard stderr anyway: drop the messages before formatting.
        // ERROR-level messages still abort, as GLib always does.
        type Handler = unsafe extern "C" fn(
            *const std::ffi::c_char,
            std::ffi::c_int,
            *const std::ffi::c_char,
            *mut std::ffi::c_void,
        );
        unsafe extern "C" fn discard(
            _: *const std::ffi::c_char,
            _: std::ffi::c_int,
            _: *const std::ffi::c_char,
            _: *mut std::ffi::c_void,
        ) {
        }
        let set: unsafe extern "C" fn(Handler, *mut std::ffi::c_void) -> Option<Handler> =
            glib.symbol(c"g_log_set_default_handler")?;
        set(discard, ptr::null_mut());
    }
    Ok(())
}

/// A GError's message, freeing it.
unsafe fn take_error(api: &Api, error: *mut api::Error) -> String {
    if error.is_null() {
        return "unknown GStreamer error".into();
    }
    // SAFETY: a GError we own; its message is a NUL-terminated string.
    unsafe {
        let message = (*error).message;
        let text = if message.is_null() {
            "unknown GStreamer error".into()
        } else {
            CStr::from_ptr(message).to_string_lossy().into_owned()
        };
        (api.g_error_free)(error);
        text
    }
}

fn frame_of(
    output: Output,
    rotation: i32,
    transfer_709: bool,
) -> io::Result<(u64, picture::Pixels)> {
    match output {
        Output::Video {
            nanos,
            width,
            height,
            rgba,
        } => Ok((
            nanos,
            picture::finish(
                &rgba,
                width as usize * 4,
                width,
                height,
                rotation,
                transfer_709,
            )?,
        )),
        Output::Audio { .. } => Err(io::Error::other("Audio output in a video graph")),
    }
}

/// One thumbnail sampled like the bundled profile samples it.
pub(super) fn thumbnail(
    gstreamer: &'static Gstreamer,
    source: Box<dyn ReadSeek + '_>,
    length: u64,
    bounds: OutputBounds,
    sample: u8,
) -> Result<picture::Pixels, Refusal> {
    let mut graph = Pipeline::open(gstreamer, source, length, Kind::Video, Some(bounds))?;
    let duration = graph.duration_nanos().unwrap_or(0);
    let target = duration / 100 * (10 + u64::from(sample) * 15);
    if target > 0 {
        graph.seek(target)?;
    }
    let mut earlier = None;
    for _ in 0..4096 {
        match graph.next()? {
            Some(output @ Output::Video { nanos, .. }) if nanos < target => earlier = Some(output),
            Some(output) => {
                return Ok(frame_of(output, graph.rotation(), graph.transfer_709())?.1);
            }
            None => break,
        }
    }
    // A header can overstate the decodable content: show the latest frame decoded instead.
    let output =
        earlier.ok_or_else(|| io::Error::other("No representative frame within decode budget"))?;
    Ok(frame_of(output, graph.rotation(), graph.transfer_709())?.1)
}

/// The playback decoder over two graphs: one for the video, one for the audio.
pub(super) struct Playback<'s> {
    video: Pipeline<'s>,
    audio: Option<(Pipeline<'s>, Timeline)>,
    track: AudioTrack,
    metadata: Metadata,
    minimum_nanos: u64,
    ended: bool,
    failed: bool,
}

impl<'s> Playback<'s> {
    pub(super) fn open(
        gstreamer: &'static Gstreamer,
        video_source: Box<dyn ReadSeek + 's>,
        audio_source: impl FnOnce() -> Box<dyn ReadSeek + 's>,
        length: u64,
        bounds: OutputBounds,
    ) -> Result<Self, Refusal> {
        crate::media::output_length(bounds.width, bounds.height)?;
        let video = Pipeline::open(gstreamer, video_source, length, Kind::Video, Some(bounds))?;
        let (width, height, rotation) = video
            .picture()
            .ok_or_else(|| io::Error::other("The decoded video has no size"))?;
        let (width, height) = if matches!(rotation, 90 | 270) {
            (height, width)
        } else {
            (width, height)
        };
        let duration_micros = video.duration_nanos().map(|nanos| nanos / 1000);
        let metadata = Metadata {
            duration_micros,
            width,
            height,
        };
        let (audio, track) = match video.discovered(Kind::Audio) {
            None => (None, AudioTrack::Absent),
            Some(found) if !found.decodable => (None, AudioTrack::Unqualified),
            Some(_) => match Pipeline::open(gstreamer, audio_source(), length, Kind::Audio, None) {
                Ok(graph) => (
                    Some((graph, Timeline::new(duration_micros))),
                    AudioTrack::Qualified,
                ),
                Err(_) => (None, AudioTrack::Unqualified),
            },
        };
        Ok(Self {
            video,
            audio,
            track,
            metadata,
            minimum_nanos: 0,
            ended: false,
            failed: false,
        })
    }

    pub(super) fn metadata(&self) -> Metadata {
        self.metadata
    }

    pub(super) fn audio_track(&self) -> AudioTrack {
        self.track
    }

    pub(super) fn next_frame(&mut self) -> io::Result<Option<Frame>> {
        if self.failed {
            return Err(io::Error::other("Playback cursor ended after a failure"));
        }
        if self.ended {
            return Ok(None);
        }
        let result = self.next_inner();
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn next_inner(&mut self) -> io::Result<Option<Frame>> {
        for _ in 0..4096 {
            let Some(output) = self.video.next()? else {
                self.ended = true;
                return Ok(None);
            };
            let (nanos, pixels) =
                frame_of(output, self.video.rotation(), self.video.transfer_709())?;
            if nanos < self.minimum_nanos {
                continue;
            }
            let presentation_micros = i64::try_from(nanos / 1000).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Video presentation timestamp overflow",
                )
            })?;
            return Ok(Some(Frame {
                presentation_micros,
                width: pixels.width,
                height: pixels.height,
                rgba: pixels.rgba,
            }));
        }
        Err(io::Error::other("Video decode work budget exhausted"))
    }

    pub(super) fn seek(&mut self, position_micros: u64) -> io::Result<()> {
        if self.failed {
            return Err(io::Error::other("Playback cursor ended after a failure"));
        }
        let nanos = position_micros.checked_mul(1000).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Video seek exceeds timestamp range",
            )
        })?;
        let result = (|| {
            self.video.seek(nanos)?;
            if let Some((graph, timeline)) = &mut self.audio {
                // Restart a little early; the timeline drops what precedes the target.
                graph.seek(nanos.saturating_sub(80_000_000))?;
                timeline.seek(position_micros);
            }
            Ok(())
        })();
        if result.is_err() {
            self.failed = true;
        } else {
            self.minimum_nanos = nanos;
            self.ended = false;
        }
        result
    }

    pub(super) fn pcm(&mut self, credit: u16) -> io::Result<Option<Block>> {
        if self.failed {
            return Err(io::Error::other("Playback cursor ended after a failure"));
        }
        let Some((graph, timeline)) = &mut self.audio else {
            return Ok(None);
        };
        let result = timeline.next(credit, || {
            for _ in 0..4096 {
                match graph.next()? {
                    Some(Output::Audio { nanos, samples }) => {
                        if samples.is_empty() {
                            continue;
                        }
                        let start_nanos = i64::try_from(nanos).map_err(|_| {
                            io::Error::new(io::ErrorKind::InvalidData, "Audio timestamp overflow")
                        })?;
                        return Ok(Some(Clip {
                            start_nanos,
                            samples,
                        }));
                    }
                    Some(Output::Video { .. }) => {
                        return Err(io::Error::other("Video output in an audio graph"));
                    }
                    None => return Ok(None),
                }
            }
            Err(io::Error::other("Audio decode work budget exhausted"))
        });
        if result.is_err() {
            self.failed = true;
        }
        result
    }
}

/// Whether a loaded plugin provides the element `name`.
#[cfg(all(test, not(feature = "bundled")))]
pub(super) fn has_element(gstreamer: &Gstreamer, name: &CStr) -> bool {
    // SAFETY: a factory lookup; the reference is released.
    unsafe {
        let factory = (gstreamer.api.gst_element_factory_find)(name.as_ptr());
        if factory.is_null() {
            return false;
        }
        (gstreamer.api.gst_object_unref)(factory);
        true
    }
}

/// Check the hand-declared constants and struct offsets against the installed GStreamer.
#[cfg(all(test, not(feature = "bundled")))]
pub(super) fn check_abi(gstreamer: &Gstreamer) -> Result<String, String> {
    use std::ffi::{c_char, c_int, c_void};
    let api = &gstreamer.api;
    let core = Library::open(OsStr::new("libgstreamer-1.0.so.0"), Trust::ROOT)
        .map_err(|error| error.to_string())?;
    // SAFETY: the declared signatures of these GStreamer 1.x functions.
    let (event_name, query_name, new_duration_query): (
        unsafe extern "C" fn(c_int) -> *const c_char,
        unsafe extern "C" fn(c_int) -> *const c_char,
        unsafe extern "C" fn(c_int) -> *mut c_void,
    ) = unsafe {
        (
            core.symbol(c"gst_event_type_get_name")
                .map_err(|error| error.to_string())?,
            core.symbol(c"gst_query_type_get_name")
                .map_err(|error| error.to_string())?,
            core.symbol(c"gst_query_new_duration")
                .map_err(|error| error.to_string())?,
        )
    };
    let name = |text: *const c_char| -> String {
        // SAFETY: GStreamer's static type names.
        if text.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(text) }
                .to_string_lossy()
                .into_owned()
        }
    };
    for (value, expected) in [
        (api::GST_EVENT_FLUSH_START, "flush-start"),
        (api::GST_EVENT_FLUSH_STOP, "flush-stop"),
        (api::GST_EVENT_CAPS, "caps"),
        (api::GST_EVENT_SEGMENT, "segment"),
        (api::GST_EVENT_TAG, "tag"),
        (api::GST_EVENT_EOS, "eos"),
        (api::GST_EVENT_SEEK, "seek"),
    ] {
        // SAFETY: a name lookup.
        let found = name(unsafe { event_name(value) });
        if found != expected {
            return Err(format!("event {value:#x} is {found:?}, not {expected}"));
        }
    }
    for (value, expected) in [
        (api::GST_QUERY_DURATION, "duration"),
        (api::GST_QUERY_SEEKING, "seeking"),
        (api::GST_QUERY_ALLOCATION, "allocation"),
        (api::GST_QUERY_SCHEDULING, "scheduling"),
        (api::GST_QUERY_ACCEPT_CAPS, "accept-caps"),
        (api::GST_QUERY_CAPS, "caps"),
    ] {
        // SAFETY: a name lookup.
        let found = name(unsafe { query_name(value) });
        if found != expected {
            return Err(format!("query {value:#x} is {found:?}, not {expected}"));
        }
    }
    let read = |object: *mut c_void, offset: usize| -> u64 {
        // SAFETY: offsets inside the public structs checked here.
        unsafe {
            object
                .cast::<u8>()
                .add(offset)
                .cast::<u64>()
                .read_unaligned()
        }
    };
    // SAFETY: objects created, inspected through their public layouts and released here.
    unsafe {
        let eos = (api.gst_event_new_eos)();
        let kind = read(eos, api::MINI_OBJECT_TYPE_OFFSET) as u32 as c_int;
        (api.gst_mini_object_unref)(eos);
        if kind != api::GST_EVENT_EOS {
            return Err(format!("GstEvent.type reads {kind:#x}"));
        }
        let query = new_duration_query(api::GST_FORMAT_TIME);
        let kind = read(query, api::MINI_OBJECT_TYPE_OFFSET) as u32 as c_int;
        (api.gst_mini_object_unref)(query);
        if kind != api::GST_QUERY_DURATION {
            return Err(format!("GstQuery.type reads {kind:#x}"));
        }
        let buffer = (api.gst_buffer_new_allocate)(std::ptr::null_mut(), 16, std::ptr::null_mut());
        // A new buffer has no timestamps and no offsets: five GST_CLOCK_TIME_NONE words
        // (pts, dts, duration, offset, offset_end) after the mini object and pool pointer.
        let words: Vec<u64> = (0..5)
            .map(|index| read(buffer, api::BUFFER_PTS_OFFSET + index * 8))
            .collect();
        let mut map = api::MapInfo::empty();
        let mapped = (api.gst_buffer_map)(buffer, &mut map, api::GST_MAP_READ);
        let size = map.size;
        if mapped != 0 {
            (api.gst_buffer_unmap)(buffer, &mut map);
        }
        (api.gst_mini_object_unref)(buffer);
        if words.iter().any(|word| *word != api::GST_CLOCK_TIME_NONE) {
            return Err(format!("GstBuffer timestamps read {words:x?}"));
        }
        if mapped == 0 || size != 16 || map.data.is_null() {
            return Err(format!("GstMapInfo reads size {size}"));
        }
        // GParamSpec.value_type: capsfilter's "caps" holds GstCaps.
        let factory = (api.gst_element_factory_find)(c"capsfilter".as_ptr());
        if factory.is_null() {
            return Err("capsfilter is missing".into());
        }
        let element =
            (api.gst_object_ref_sink)((api.gst_element_factory_create)(factory, std::ptr::null()));
        (api.gst_object_unref)(factory);
        let class = element.cast::<*mut c_void>().read();
        let spec = (api.g_object_class_find_property)(class, c"caps".as_ptr());
        let value_type = if spec.is_null() {
            0
        } else {
            read(spec, api::PARAM_SPEC_VALUE_TYPE_OFFSET) as usize
        };
        (api.gst_object_unref)(element);
        if value_type != (api.gst_caps_get_type)() {
            return Err(format!("GParamSpec.value_type reads {value_type:#x}"));
        }
    }
    Ok(format!(
        "GStreamer {:?}: event and query types, GstEvent/GstQuery/GstBuffer/GstMapInfo/GParamSpec offsets match",
        gstreamer.version
    ))
}

//! The GLib, GObject and GStreamer 1.x functions the decoder calls, declared by hand.
//! GStreamer keeps its C ABI stable across 1.x, so one declaration serves every 1.x release;
//! the few struct fields read here (GstMiniObject, GstBuffer, GstEvent, GstQuery, GstMapInfo,
//! GstPadProbeInfo, GList, GError, GParamSpec) are public and checked against the installed
//! library by the tests.
use super::super::library::{Library, LoadError};
use std::ffi::{CStr, c_char, c_int, c_uint, c_void};

pub(super) type Gboolean = c_int;
pub(super) type GType = usize;

// Enumerations (gstreamer-1.0 headers; all values fixed since 1.0).
pub(super) const GST_PAD_SRC: c_int = 1;
pub(super) const GST_PAD_SINK: c_int = 2;
pub(super) const GST_STATE_NULL: c_int = 1;
pub(super) const GST_STATE_PLAYING: c_int = 4;
pub(super) const GST_STATE_CHANGE_FAILURE: c_int = 0;
pub(super) const GST_FORMAT_BYTES: c_int = 2;
pub(super) const GST_FORMAT_TIME: c_int = 3;
pub(super) const GST_FLOW_OK: c_int = 0;
pub(super) const GST_FLOW_NOT_LINKED: c_int = -1;
pub(super) const GST_FLOW_FLUSHING: c_int = -2;
pub(super) const GST_FLOW_EOS: c_int = -3;
pub(super) const GST_SEEK_FLAG_FLUSH: c_int = 1 << 0;
pub(super) const GST_SEEK_FLAG_KEY_UNIT: c_int = 1 << 2;
pub(super) const GST_SEEK_FLAG_SNAP_BEFORE: c_int = 1 << 5;
pub(super) const GST_SEEK_TYPE_NONE: c_int = 0;
pub(super) const GST_SEEK_TYPE_SET: c_int = 1;
pub(super) const GST_MAP_READ: c_int = 1;
pub(super) const GST_PAD_MODE_PUSH: c_int = 1;
pub(super) const GST_SCHEDULING_FLAG_SEEKABLE: c_int = 1;
pub(super) const GST_PAD_PROBE_TYPE_EVENT_DOWNSTREAM: c_int = 1 << 6;
pub(super) const GST_PAD_PROBE_OK: c_int = 1;
pub(super) const GST_RANK_MARGINAL: c_uint = 64;
pub(super) const GST_ELEMENT_FACTORY_TYPE_DECODER: u64 = 1 << 0;
pub(super) const GST_ELEMENT_FACTORY_TYPE_DEMUXER: u64 = 1 << 5;
pub(super) const GST_ELEMENT_FACTORY_TYPE_PARSER: u64 = 1 << 6;
pub(super) const GST_CLOCK_TIME_NONE: u64 = u64::MAX;

/// GST_EVENT_MAKE_TYPE(num, flags) = num << 8 | flags; flags: upstream 1, downstream 2,
/// serialized 4, sticky 8, sticky-multi 16.
const fn event(number: c_int, flags: c_int) -> c_int {
    (number << 8) | flags
}
pub(super) const GST_EVENT_FLUSH_START: c_int = event(10, 1 | 2);
pub(super) const GST_EVENT_FLUSH_STOP: c_int = event(20, 1 | 2 | 4);
pub(super) const GST_EVENT_CAPS: c_int = event(50, 2 | 4 | 8);
pub(super) const GST_EVENT_SEGMENT: c_int = event(70, 2 | 4 | 8);
pub(super) const GST_EVENT_TAG: c_int = event(80, 2 | 4 | 8 | 16);
pub(super) const GST_EVENT_EOS: c_int = event(110, 2 | 4 | 8);
pub(super) const GST_EVENT_SEEK: c_int = event(200, 1);

/// GST_QUERY_MAKE_TYPE(num, flags) = num << 8 | flags; flags: upstream 1, downstream 2,
/// serialized 4.
const fn query(number: c_int, flags: c_int) -> c_int {
    (number << 8) | flags
}
pub(super) const GST_QUERY_DURATION: c_int = query(20, 1 | 2);
pub(super) const GST_QUERY_SEEKING: c_int = query(60, 1 | 2);
pub(super) const GST_QUERY_ALLOCATION: c_int = query(140, 2 | 4);
pub(super) const GST_QUERY_SCHEDULING: c_int = query(150, 1);
pub(super) const GST_QUERY_ACCEPT_CAPS: c_int = query(160, 1 | 2);
pub(super) const GST_QUERY_CAPS: c_int = query(170, 1 | 2);

// Public struct offsets on 64-bit Linux.
/// GstMiniObject is 64 bytes; GstEvent.type and GstQuery.type follow it.
pub(super) const MINI_OBJECT_TYPE_OFFSET: usize = 64;
/// GstBuffer: mini_object (64), pool (8), pts, dts, duration, offset, offset_end.
pub(super) const BUFFER_PTS_OFFSET: usize = 72;
pub(super) const BUFFER_OFFSET_OFFSET: usize = 96;
/// GstPadProbeInfo: type (4 + 4 padding), id (8), data (8).
pub(super) const PROBE_INFO_DATA_OFFSET: usize = 16;
/// GParamSpec: g_type_instance (8), name (8), flags (4 + 4), value_type.
pub(super) const PARAM_SPEC_VALUE_TYPE_OFFSET: usize = 24;
/// GstStaticPadTemplate: name_template (8), direction.
pub(super) const STATIC_TEMPLATE_DIRECTION_OFFSET: usize = 8;
/// GLib fundamental types.
pub(super) const G_TYPE_INT: GType = 6 << 2;
pub(super) const G_TYPE_UINT: GType = 7 << 2;

/// GstMapInfo (104 bytes): memory, flags, data, size, maxsize, user_data[4], reserved[4].
#[repr(C)]
pub(super) struct MapInfo {
    pub(super) memory: *mut c_void,
    pub(super) flags: c_int,
    pub(super) data: *mut u8,
    pub(super) size: usize,
    pub(super) maxsize: usize,
    pub(super) user_data: [*mut c_void; 4],
    pub(super) reserved: [*mut c_void; 4],
}

impl MapInfo {
    pub(super) fn empty() -> Self {
        Self {
            memory: std::ptr::null_mut(),
            flags: 0,
            data: std::ptr::null_mut(),
            size: 0,
            maxsize: 0,
            user_data: [std::ptr::null_mut(); 4],
            reserved: [std::ptr::null_mut(); 4],
        }
    }
}

/// GValue: the type, then two 8-byte data words.
#[repr(C)]
pub(super) struct Value {
    pub(super) type_: GType,
    pub(super) data: [u64; 2],
}

/// GList.
#[repr(C)]
pub(super) struct List {
    pub(super) data: *mut c_void,
    pub(super) next: *mut List,
    pub(super) prev: *mut List,
}

/// GError.
#[repr(C)]
pub(super) struct Error {
    pub(super) domain: u32,
    pub(super) code: c_int,
    pub(super) message: *const c_char,
}

/// GstVideoColorimetry (libgstvideo): range, matrix, transfer, primaries.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Colorimetry {
    pub(super) range: c_int,
    pub(super) matrix: c_int,
    pub(super) transfer: c_int,
    pub(super) primaries: c_int,
}

pub(super) type ChainFunction =
    unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> c_int;
pub(super) type EventFunction =
    unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> Gboolean;
pub(super) type QueryFunction =
    unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> Gboolean;
pub(super) type ProbeFunction =
    unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> c_int;

macro_rules! api {
    ($($field:ident: $type:ty = $symbol:literal,)*) => {
        pub(super) struct Api {
            $(pub(super) $field: $type,)*
            /// libgstvideo's colorimetry parser, when GStreamer's base libraries are installed.
            pub(super) gst_video_colorimetry_from_string:
                Option<unsafe extern "C" fn(*mut Colorimetry, *const c_char) -> Gboolean>,
        }

        impl Api {
            pub(super) fn resolve(libraries: &[Library], video: Option<&Library>) -> Result<Self, LoadError> {
                fn find(libraries: &[Library], name: &CStr) -> Result<*mut c_void, LoadError> {
                    for library in libraries {
                        // SAFETY: only the address is read; it is typed at its single use.
                        if let Some(symbol) = unsafe { library.optional::<*mut c_void>(name) } {
                            return Ok(symbol);
                        }
                    }
                    Err(LoadError::Symbol(name.to_string_lossy().into_owned()))
                }
                Ok(Self {
                    // SAFETY: each declared type is the symbol's GLib/GStreamer 1.x signature.
                    $($field: unsafe { std::mem::transmute::<*mut c_void, $type>(find(libraries, $symbol)?) },)*
                    gst_video_colorimetry_from_string: video.and_then(|library| {
                        // SAFETY: gboolean gst_video_colorimetry_from_string(GstVideoColorimetry *, const gchar *).
                        unsafe { library.optional(c"gst_video_colorimetry_from_string") }
                    }),
                })
            }
        }
    };
}

api! {
    g_free: unsafe extern "C" fn(*mut c_void) = c"g_free",
    g_error_free: unsafe extern "C" fn(*mut Error) = c"g_error_free",
    g_object_set_property: unsafe extern "C" fn(*mut c_void, *const c_char, *const Value) = c"g_object_set_property",
    g_object_class_find_property: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void = c"g_object_class_find_property",
    g_object_set_data: unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_void) = c"g_object_set_data",
    g_object_get_data: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void = c"g_object_get_data",
    g_value_init: unsafe extern "C" fn(*mut Value, GType) -> *mut Value = c"g_value_init",
    g_value_unset: unsafe extern "C" fn(*mut Value) = c"g_value_unset",
    g_value_set_int: unsafe extern "C" fn(*mut Value, c_int) = c"g_value_set_int",
    g_value_set_uint: unsafe extern "C" fn(*mut Value, c_uint) = c"g_value_set_uint",
    g_value_set_boxed: unsafe extern "C" fn(*mut Value, *const c_void) = c"g_value_set_boxed",
    g_signal_connect_data: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_void, *mut c_void, *const c_void, c_int) -> u64 = c"g_signal_connect_data",
    gst_init_check: unsafe extern "C" fn(*mut c_int, *mut *mut *mut c_char, *mut *mut Error) -> Gboolean = c"gst_init_check",
    gst_version: unsafe extern "C" fn(*mut c_uint, *mut c_uint, *mut c_uint, *mut c_uint) = c"gst_version",
    gst_plugin_load_file: unsafe extern "C" fn(*const c_char, *mut *mut Error) -> *mut c_void = c"gst_plugin_load_file",
    gst_object_unref: unsafe extern "C" fn(*mut c_void) = c"gst_object_unref",
    gst_object_ref_sink: unsafe extern "C" fn(*mut c_void) -> *mut c_void = c"gst_object_ref_sink",
    gst_element_factory_find: unsafe extern "C" fn(*const c_char) -> *mut c_void = c"gst_element_factory_find",
    gst_element_factory_create: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void = c"gst_element_factory_create",
    gst_element_factory_list_get_elements: unsafe extern "C" fn(u64, c_uint) -> *mut List = c"gst_element_factory_list_get_elements",
    gst_element_factory_list_filter: unsafe extern "C" fn(*mut List, *const c_void, c_int, Gboolean) -> *mut List = c"gst_element_factory_list_filter",
    gst_element_factory_get_static_pad_templates: unsafe extern "C" fn(*mut c_void) -> *const List = c"gst_element_factory_get_static_pad_templates",
    gst_static_pad_template_get_caps: unsafe extern "C" fn(*mut c_void) -> *mut c_void = c"gst_static_pad_template_get_caps",
    gst_plugin_feature_list_free: unsafe extern "C" fn(*mut List) = c"gst_plugin_feature_list_free",
    gst_plugin_feature_get_rank: unsafe extern "C" fn(*mut c_void) -> c_uint = c"gst_plugin_feature_get_rank",
    gst_element_get_static_pad: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void = c"gst_element_get_static_pad",
    gst_element_link: unsafe extern "C" fn(*mut c_void, *mut c_void) -> Gboolean = c"gst_element_link",
    gst_element_set_state: unsafe extern "C" fn(*mut c_void, c_int) -> c_int = c"gst_element_set_state",
    gst_element_send_event: unsafe extern "C" fn(*mut c_void, *mut c_void) -> Gboolean = c"gst_element_send_event",
    gst_element_query_duration: unsafe extern "C" fn(*mut c_void, c_int, *mut i64) -> Gboolean = c"gst_element_query_duration",
    gst_pad_new: unsafe extern "C" fn(*const c_char, c_int) -> *mut c_void = c"gst_pad_new",
    gst_pad_set_active: unsafe extern "C" fn(*mut c_void, Gboolean) -> Gboolean = c"gst_pad_set_active",
    gst_pad_set_chain_function_full: unsafe extern "C" fn(*mut c_void, ChainFunction, *mut c_void, *const c_void) = c"gst_pad_set_chain_function_full",
    gst_pad_set_event_function_full: unsafe extern "C" fn(*mut c_void, EventFunction, *mut c_void, *const c_void) = c"gst_pad_set_event_function_full",
    gst_pad_set_query_function_full: unsafe extern "C" fn(*mut c_void, QueryFunction, *mut c_void, *const c_void) = c"gst_pad_set_query_function_full",
    gst_pad_link: unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int = c"gst_pad_link",
    gst_pad_push: unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int = c"gst_pad_push",
    gst_pad_push_event: unsafe extern "C" fn(*mut c_void, *mut c_void) -> Gboolean = c"gst_pad_push_event",
    gst_pad_get_current_caps: unsafe extern "C" fn(*mut c_void) -> *mut c_void = c"gst_pad_get_current_caps",
    gst_pad_query_caps: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void = c"gst_pad_query_caps",
    gst_pad_add_probe: unsafe extern "C" fn(*mut c_void, c_int, ProbeFunction, *mut c_void, *const c_void) -> u64 = c"gst_pad_add_probe",
    gst_pad_query_default: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> Gboolean = c"gst_pad_query_default",
    gst_mini_object_unref: unsafe extern "C" fn(*mut c_void) = c"gst_mini_object_unref",
    gst_buffer_new_allocate: unsafe extern "C" fn(*mut c_void, usize, *mut c_void) -> *mut c_void = c"gst_buffer_new_allocate",
    gst_buffer_fill: unsafe extern "C" fn(*mut c_void, usize, *const c_void, usize) -> usize = c"gst_buffer_fill",
    gst_buffer_map: unsafe extern "C" fn(*mut c_void, *mut MapInfo, c_int) -> Gboolean = c"gst_buffer_map",
    gst_buffer_unmap: unsafe extern "C" fn(*mut c_void, *mut MapInfo) = c"gst_buffer_unmap",
    gst_caps_from_string: unsafe extern "C" fn(*const c_char) -> *mut c_void = c"gst_caps_from_string",
    gst_caps_get_type: unsafe extern "C" fn() -> GType = c"gst_caps_get_type",
    gst_caps_get_structure: unsafe extern "C" fn(*const c_void, c_uint) -> *mut c_void = c"gst_caps_get_structure",
    gst_caps_can_intersect: unsafe extern "C" fn(*const c_void, *const c_void) -> Gboolean = c"gst_caps_can_intersect",
    gst_structure_get_name: unsafe extern "C" fn(*const c_void) -> *const c_char = c"gst_structure_get_name",
    gst_structure_get_int: unsafe extern "C" fn(*const c_void, *const c_char, *mut c_int) -> Gboolean = c"gst_structure_get_int",
    gst_structure_get_fraction: unsafe extern "C" fn(*const c_void, *const c_char, *mut c_int, *mut c_int) -> Gboolean = c"gst_structure_get_fraction",
    gst_structure_get_string: unsafe extern "C" fn(*const c_void, *const c_char) -> *const c_char = c"gst_structure_get_string",
    gst_structure_get_boolean: unsafe extern "C" fn(*const c_void, *const c_char, *mut Gboolean) -> Gboolean = c"gst_structure_get_boolean",
    gst_event_new_stream_start: unsafe extern "C" fn(*const c_char) -> *mut c_void = c"gst_event_new_stream_start",
    gst_event_new_segment: unsafe extern "C" fn(*const c_void) -> *mut c_void = c"gst_event_new_segment",
    gst_event_new_eos: unsafe extern "C" fn() -> *mut c_void = c"gst_event_new_eos",
    gst_event_new_flush_start: unsafe extern "C" fn() -> *mut c_void = c"gst_event_new_flush_start",
    gst_event_new_flush_stop: unsafe extern "C" fn(Gboolean) -> *mut c_void = c"gst_event_new_flush_stop",
    gst_event_new_seek: unsafe extern "C" fn(f64, c_int, c_int, c_int, i64, c_int, i64) -> *mut c_void = c"gst_event_new_seek",
    gst_event_parse_seek: unsafe extern "C" fn(*mut c_void, *mut f64, *mut c_int, *mut c_int, *mut c_int, *mut i64, *mut c_int, *mut i64) = c"gst_event_parse_seek",
    gst_event_parse_caps: unsafe extern "C" fn(*mut c_void, *mut *mut c_void) = c"gst_event_parse_caps",
    gst_event_parse_segment: unsafe extern "C" fn(*mut c_void, *mut *const c_void) = c"gst_event_parse_segment",
    gst_event_parse_tag: unsafe extern "C" fn(*mut c_void, *mut *mut c_void) = c"gst_event_parse_tag",
    gst_tag_list_get_string: unsafe extern "C" fn(*const c_void, *const c_char, *mut *mut c_char) -> Gboolean = c"gst_tag_list_get_string",
    gst_segment_new: unsafe extern "C" fn() -> *mut c_void = c"gst_segment_new",
    gst_segment_free: unsafe extern "C" fn(*mut c_void) = c"gst_segment_free",
    gst_segment_copy: unsafe extern "C" fn(*const c_void) -> *mut c_void = c"gst_segment_copy",
    gst_segment_init: unsafe extern "C" fn(*mut c_void, c_int) = c"gst_segment_init",
    gst_segment_do_seek: unsafe extern "C" fn(*mut c_void, f64, c_int, c_int, c_int, u64, c_int, u64, *mut Gboolean) -> Gboolean = c"gst_segment_do_seek",
    gst_segment_to_stream_time: unsafe extern "C" fn(*const c_void, c_int, u64) -> u64 = c"gst_segment_to_stream_time",
    gst_query_parse_duration: unsafe extern "C" fn(*mut c_void, *mut c_int, *mut i64) = c"gst_query_parse_duration",
    gst_query_set_duration: unsafe extern "C" fn(*mut c_void, c_int, i64) = c"gst_query_set_duration",
    gst_query_parse_seeking: unsafe extern "C" fn(*mut c_void, *mut c_int, *mut Gboolean, *mut i64, *mut i64) = c"gst_query_parse_seeking",
    gst_query_set_seeking: unsafe extern "C" fn(*mut c_void, c_int, Gboolean, i64, i64) = c"gst_query_set_seeking",
    gst_query_set_scheduling: unsafe extern "C" fn(*mut c_void, c_int, c_int, c_int, c_int) = c"gst_query_set_scheduling",
    gst_query_add_scheduling_mode: unsafe extern "C" fn(*mut c_void, c_int) = c"gst_query_add_scheduling_mode",
    gst_query_set_caps_result: unsafe extern "C" fn(*mut c_void, *mut c_void) = c"gst_query_set_caps_result",
    gst_query_parse_accept_caps: unsafe extern "C" fn(*mut c_void, *mut *mut c_void) = c"gst_query_parse_accept_caps",
    gst_query_set_accept_caps_result: unsafe extern "C" fn(*mut c_void, Gboolean) = c"gst_query_set_accept_caps_result",
    gst_type_find_helper_for_data: unsafe extern "C" fn(*mut c_void, *const u8, usize, *mut c_int) -> *mut c_void = c"gst_type_find_helper_for_data",
}

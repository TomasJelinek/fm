//! A GStreamer graph that runs entirely on the calling thread, inside the worker sandbox,
//! which denies new threads and every file open.
//!
//! No pipeline, bin, bus, queue, decodebin or appsrc is used, because each of them starts
//! threads. This module owns a source pad and pushes the captured bytes into the demuxer
//! itself; GStreamer then demuxes, parses, decodes and converts synchronously inside each
//! push, and the decoded output reaches this module's sink pad before the push returns. The
//! elements are chosen like decodebin chooses them (typefind, the best-ranked demuxer, a
//! parser when one exists, the best-ranked decoder), but only among the plugins loaded before
//! lockdown. Seeking uses the demuxers' push-mode seek: a demuxer turns a time seek into a
//! byte seek on the source pad, which flushes and restarts the pushes at that offset.
use super::{
    Gstreamer,
    api::{self, Api, MapInfo, Value},
    codec,
};
use crate::{
    media::{Codec, OutputBounds},
    system::linux::picture,
};
use midnight_thumbnail_sdk::ReadSeek;
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    ffi::{CStr, CString, c_char, c_int, c_void},
    io::{self, SeekFrom},
    panic::{AssertUnwindSafe, catch_unwind},
    ptr,
};

/// The key under which each of our pads stores its graph.
const DATA_KEY: &CStr = c"midnight-graph";
/// Bytes typefind examines.
const TYPEFIND_BYTES: usize = 64 * 1024;
/// Largest brokered read.
const READ_BYTES: usize = 256 * 1024;
/// Push sizes adapt between these so that one push completes only a few frames.
const MIN_PUSH: usize = 512;
const MAX_PUSH: usize = 64 * 1024;
/// Bytes one decoder call may push, including skipped preroll.
pub(super) const PUSH_BUDGET: u64 = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Video,
    Audio,
}

/// A stream the demuxer exposed, and whether a decoder chain exists for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Discovered {
    pub(super) codec: Codec,
    pub(super) decodable: bool,
}

/// Decoded output in stream time (nanoseconds from the start of the presentation).
pub(super) enum Output {
    Video {
        nanos: u64,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    Audio {
        nanos: u64,
        samples: Vec<i16>,
    },
}

struct State {
    position: u64,
    seek_generation: u64,
    segment_pending: bool,
    eos_sent: bool,
    video: Option<Discovered>,
    audio: Option<Discovered>,
    no_more_pads: bool,
    linked: bool,
    queue: VecDeque<Output>,
    sink_eos: bool,
    error: Option<io::Error>,
    segment: *mut c_void,
    rotation: i32,
    transfer_709: bool,
    capsfilter: *mut c_void,
    decoded: Option<Decoded>,
    /// The converted output size, from the caps reaching our sink.
    output: Option<(u32, u32)>,
    elements: Vec<*mut c_void>,
    pads: Vec<*mut c_void>,
}

/// A decoded picture's width, height and pixel aspect ratio.
type Decoded = (u32, u32, (c_int, c_int));

/// Everything the callbacks reach, at a stable address for the graph's lifetime.
struct Shared {
    api: &'static Api,
    kind: Kind,
    bounds: Option<OutputBounds>,
    length: u64,
    src: Cell<*mut c_void>,
    state: RefCell<State>,
}

impl Shared {
    fn fail(&self, error: io::Error) {
        if let Ok(mut state) = self.state.try_borrow_mut() {
            state.error.get_or_insert(error);
        }
    }
}

pub(super) struct Pipeline<'s> {
    api: &'static Api,
    source: Box<dyn ReadSeek + 's>,
    cache: Vec<u8>,
    cache_offset: u64,
    shared: Box<Shared>,
    src: *mut c_void,
    demuxer: *mut c_void,
    push: usize,
    started: bool,
}

/// Why a graph cannot decode the wanted stream.
pub(in super::super) enum Refusal {
    /// GStreamer does not recognize the data.
    Unrecognized,
    /// No loaded demuxer handles the container.
    NoDemuxer,
    /// The source has no stream of the wanted kind.
    NoStream,
    /// A stream of the wanted kind has no decoder among the loaded plugins.
    NoDecoder(Codec),
    Failed(io::Error),
}

impl From<io::Error> for Refusal {
    fn from(error: io::Error) -> Self {
        Self::Failed(error)
    }
}

fn graph<'a>(api: &Api, pad: *mut c_void) -> Option<&'a Shared> {
    // SAFETY: our pads carry a pointer to the Shared that owns them, which outlives the pads'
    // activity (Drop deactivates every element and pad before freeing it).
    let data = unsafe { (api.g_object_get_data)(pad, DATA_KEY.as_ptr()) };
    (!data.is_null()).then(|| unsafe { &*data.cast::<Shared>() })
}

impl<'s> Pipeline<'s> {
    /// Typefind the source, choose and start a demuxer, and push until the wanted stream's
    /// decoder chain produced its first output.
    pub(super) fn open(
        gst: &'static Gstreamer,
        mut source: Box<dyn ReadSeek + 's>,
        length: u64,
        kind: Kind,
        bounds: Option<OutputBounds>,
    ) -> Result<Self, Refusal> {
        let api = &gst.api;
        source.seek(SeekFrom::Start(0))?;
        let mut head = vec![0; TYPEFIND_BYTES.min(length as usize)];
        read_fully(&mut *source, &mut head)?;
        // SAFETY: typefind reads the bytes of `head` only; the caps are owned here.
        let caps = unsafe {
            let mut probability = 0;
            (api.gst_type_find_helper_for_data)(
                ptr::null_mut(),
                head.as_ptr(),
                head.len(),
                &mut probability,
            )
        };
        if caps.is_null() {
            return Err(Refusal::Unrecognized);
        }
        // SAFETY: our typefind caps; the structure name is owned by them.
        let media = unsafe {
            let structure = (api.gst_caps_get_structure)(caps, 0);
            !structure.is_null()
                && container_like(
                    CStr::from_ptr((api.gst_structure_get_name)(structure)).to_bytes(),
                )
        };
        let demuxers = Factories::matching(
            api,
            api::GST_ELEMENT_FACTORY_TYPE_DEMUXER,
            caps,
            api::GST_PAD_SINK,
        );
        // SAFETY: our reference from typefind.
        unsafe { (api.gst_mini_object_unref)(caps) };
        if !media {
            // Text, images and other data typefind recognizes are not video containers.
            return Err(Refusal::Unrecognized);
        }
        let Some(factory) = demuxers.best() else {
            return Err(Refusal::NoDemuxer);
        };
        let shared = Box::new(Shared {
            api,
            kind,
            bounds,
            length,
            src: Cell::new(ptr::null_mut()),
            state: RefCell::new(State {
                position: 0,
                seek_generation: 0,
                segment_pending: true,
                eos_sent: false,
                video: None,
                audio: None,
                no_more_pads: false,
                linked: false,
                queue: VecDeque::new(),
                sink_eos: false,
                error: None,
                segment: ptr::null_mut(),
                rotation: 0,
                transfer_709: true,
                capsfilter: ptr::null_mut(),
                decoded: None,
                output: None,
                elements: Vec::new(),
                pads: Vec::new(),
            }),
        });
        let mut pipeline = Self {
            api,
            source,
            cache: head,
            cache_offset: 0,
            shared,
            src: ptr::null_mut(),
            demuxer: ptr::null_mut(),
            push: MAX_PUSH,
            started: false,
        };
        let shared_pointer = (&raw const *pipeline.shared).cast_mut().cast::<c_void>();
        // SAFETY: GStreamer objects created here are owned by the pipeline (Drop releases
        // them); callbacks receive the boxed Shared, which outlives them.
        unsafe {
            let demuxer = create(&pipeline.shared, factory)?;
            pipeline.demuxer = demuxer;
            let src = (api.gst_pad_new)(c"src".as_ptr(), api::GST_PAD_SRC);
            if src.is_null() {
                return Err(io::Error::other("GStreamer pad allocation failed").into());
            }
            let src = (api.gst_object_ref_sink)(src);
            pipeline.src = src;
            pipeline.shared.src.set(src);
            (api.g_object_set_data)(src, DATA_KEY.as_ptr(), shared_pointer);
            (api.gst_pad_set_query_function_full)(src, source_query, ptr::null_mut(), ptr::null());
            (api.gst_pad_set_event_function_full)(src, source_event, ptr::null_mut(), ptr::null());
            (api.gst_pad_set_active)(src, 1);
            (api.g_signal_connect_data)(
                demuxer,
                c"pad-added".as_ptr(),
                pad_added as *const c_void,
                shared_pointer,
                ptr::null(),
                0,
            );
            (api.g_signal_connect_data)(
                demuxer,
                c"no-more-pads".as_ptr(),
                no_more_pads as *const c_void,
                shared_pointer,
                ptr::null(),
                0,
            );
            let sink = (api.gst_element_get_static_pad)(demuxer, c"sink".as_ptr());
            if sink.is_null() {
                return Err(io::Error::other("The demuxer has no sink pad").into());
            }
            let linked = (api.gst_pad_link)(src, sink);
            (api.gst_object_unref)(sink);
            if linked != 0 {
                return Err(io::Error::other("The demuxer refused the source").into());
            }
            if (api.gst_element_set_state)(demuxer, api::GST_STATE_PLAYING)
                == api::GST_STATE_CHANGE_FAILURE
            {
                return Err(io::Error::other("The demuxer could not start").into());
            }
        }
        pipeline.preroll()?;
        Ok(pipeline)
    }

    /// Push until the wanted stream produced output, or prove that it cannot.
    fn preroll(&mut self) -> Result<(), Refusal> {
        let mut pushed = 0u64;
        loop {
            {
                let state = self.shared.state.borrow();
                let wanted = match self.shared.kind {
                    Kind::Video => state.video,
                    Kind::Audio => state.audio,
                };
                if !state.queue.is_empty() || (state.linked && state.sink_eos) {
                    return Ok(());
                }
                if let Some(Discovered {
                    codec,
                    decodable: false,
                }) = wanted
                {
                    return Err(Refusal::NoDecoder(codec));
                }
                if wanted.is_none() && (state.no_more_pads || state.eos_sent) {
                    // Every pad exists, or the data ended, without the wanted kind.
                    return Err(Refusal::NoStream);
                }
            }
            if pushed > PUSH_BUDGET {
                return Err(io::Error::other("No decodable stream within the read budget").into());
            }
            match self.pump()? {
                Pumped::Pushed(bytes) => pushed += bytes as u64,
                Pumped::Ended => {
                    let state = self.shared.state.borrow();
                    if state.queue.is_empty() {
                        let wanted = match self.shared.kind {
                            Kind::Video => state.video,
                            Kind::Audio => state.audio,
                        };
                        return Err(match wanted {
                            None => Refusal::NoStream,
                            Some(found) if !found.decodable => Refusal::NoDecoder(found.codec),
                            Some(_) => Refusal::Failed(io::Error::other(
                                "The stream ended before its first decoded output",
                            )),
                        });
                    }
                    return Ok(());
                }
            }
        }
    }

    pub(super) fn discovered(&self, kind: Kind) -> Option<Discovered> {
        let state = self.shared.state.borrow();
        match kind {
            Kind::Video => state.video,
            Kind::Audio => state.audio,
        }
    }

    /// The decoder's picture size and the stream's clockwise rotation.
    pub(super) fn picture(&self) -> Option<(u32, u32, i32)> {
        let state = self.shared.state.borrow();
        state
            .decoded
            .map(|(width, height, _)| (width, height, state.rotation))
    }

    pub(super) fn transfer_709(&self) -> bool {
        self.shared.state.borrow().transfer_709
    }

    pub(super) fn rotation(&self) -> i32 {
        self.shared.state.borrow().rotation
    }

    /// Duration in nanoseconds, when the demuxer knows it.
    pub(super) fn duration_nanos(&self) -> Option<u64> {
        let mut duration = -1i64;
        // SAFETY: a live demuxer; the query writes one gint64.
        let known = unsafe {
            (self.api.gst_element_query_duration)(self.demuxer, api::GST_FORMAT_TIME, &mut duration)
        };
        (known != 0)
            .then_some(duration)
            .and_then(|duration| u64::try_from(duration).ok())
    }

    /// The next decoded output, pushing more data as needed. `None` at the end of the stream.
    pub(super) fn next(&mut self) -> io::Result<Option<Output>> {
        let mut pushed = 0u64;
        loop {
            {
                let mut state = self.shared.state.borrow_mut();
                if let Some(error) = state.error.take() {
                    return Err(error);
                }
                if let Some(output) = state.queue.pop_front() {
                    return Ok(Some(output));
                }
                if state.sink_eos || (state.eos_sent && !state.linked) {
                    return Ok(None);
                }
            }
            if pushed > PUSH_BUDGET {
                return Err(io::Error::other("Decode work budget exhausted"));
            }
            match self.pump()? {
                Pumped::Pushed(bytes) => pushed += bytes as u64,
                Pumped::Ended => {
                    let mut state = self.shared.state.borrow_mut();
                    if state.queue.is_empty() && !state.sink_eos {
                        // The demuxer took the end of stream but nothing reached the sink.
                        state.sink_eos = true;
                    }
                }
            }
        }
    }

    /// Seek to `nanos` of stream time: the demuxer restarts at the keyframe before it.
    pub(super) fn seek(&mut self, nanos: u64) -> io::Result<()> {
        let target = i64::try_from(nanos).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "Seek exceeds the time range")
        })?;
        let flags =
            api::GST_SEEK_FLAG_FLUSH | api::GST_SEEK_FLAG_KEY_UNIT | api::GST_SEEK_FLAG_SNAP_BEFORE;
        // SAFETY: a live demuxer; the event's ownership passes to it.
        let accepted = unsafe {
            let event = (self.api.gst_event_new_seek)(
                1.0,
                api::GST_FORMAT_TIME,
                flags,
                api::GST_SEEK_TYPE_SET,
                target,
                api::GST_SEEK_TYPE_NONE,
                -1,
            );
            (self.api.gst_element_send_event)(self.demuxer, event)
        };
        let mut state = self.shared.state.borrow_mut();
        if let Some(error) = state.error.take() {
            return Err(error);
        }
        if accepted == 0 {
            return Err(io::Error::other("The demuxer cannot seek this source"));
        }
        state.queue.clear();
        state.sink_eos = false;
        Ok(())
    }

    /// Push the next part of the source, or the end of stream once.
    fn pump(&mut self) -> io::Result<Pumped> {
        let api = self.api;
        let (position, generation, segment, eos_sent) = {
            let mut state = self.shared.state.borrow_mut();
            if let Some(error) = state.error.take() {
                return Err(error);
            }
            let segment = std::mem::take(&mut state.segment_pending);
            (
                state.position,
                state.seek_generation,
                segment,
                state.eos_sent,
            )
        };
        if eos_sent {
            return Ok(Pumped::Ended);
        }
        // SAFETY: events are created here and handed to GStreamer, which takes ownership.
        unsafe {
            if !self.started {
                self.started = true;
                (api.gst_pad_push_event)(
                    self.src,
                    (api.gst_event_new_stream_start)(c"midnight".as_ptr()),
                );
            }
            if segment {
                let segment = (api.gst_segment_new)();
                (api.gst_segment_init)(segment, api::GST_FORMAT_BYTES);
                (api.gst_segment_do_seek)(
                    segment,
                    1.0,
                    api::GST_FORMAT_BYTES,
                    0,
                    api::GST_SEEK_TYPE_SET,
                    position,
                    api::GST_SEEK_TYPE_NONE,
                    u64::MAX,
                    ptr::null_mut(),
                );
                (api.gst_pad_push_event)(self.src, (api.gst_event_new_segment)(segment));
                (api.gst_segment_free)(segment);
            }
        }
        if position >= self.shared.length {
            self.shared.state.borrow_mut().eos_sent = true;
            // SAFETY: the event's ownership passes to GStreamer.
            unsafe { (api.gst_pad_push_event)(self.src, (api.gst_event_new_eos)()) };
            return Ok(Pumped::Ended);
        }
        let start = self.read_at(position)?;
        let count = (self.cache.len() - start).min(self.push);
        let queued_before = self.shared.state.borrow().queue.len();
        // SAFETY: a new buffer filled with `count` source bytes; gst_pad_push takes it.
        let flow = unsafe {
            let buffer = (api.gst_buffer_new_allocate)(ptr::null_mut(), count, ptr::null_mut());
            if buffer.is_null() {
                return Err(io::Error::other("GStreamer buffer allocation failed"));
            }
            (api.gst_buffer_fill)(buffer, 0, self.cache[start..].as_ptr().cast(), count);
            buffer
                .cast::<u8>()
                .add(api::BUFFER_OFFSET_OFFSET)
                .cast::<u64>()
                .write_unaligned(position);
            (api.gst_pad_push)(self.src, buffer)
        };
        let mut state = self.shared.state.borrow_mut();
        if let Some(error) = state.error.take() {
            return Err(error);
        }
        if state.seek_generation == generation {
            state.position = position + count as u64;
        }
        // Keep each push to a few frames: decoded frames wait as RGBA until consumed.
        let produced = state.queue.len().saturating_sub(queued_before);
        self.push = if produced > 2 {
            (self.push / 4).max(MIN_PUSH)
        } else if produced == 0 {
            (self.push * 2).min(MAX_PUSH)
        } else {
            self.push
        };
        match flow {
            api::GST_FLOW_OK | api::GST_FLOW_NOT_LINKED | api::GST_FLOW_FLUSHING => {
                Ok(Pumped::Pushed(count))
            }
            api::GST_FLOW_EOS => {
                // The demuxer is done (for example its segment ended): drain the decoders.
                if !state.eos_sent {
                    state.eos_sent = true;
                    drop(state);
                    // SAFETY: the event's ownership passes to GStreamer.
                    unsafe { (api.gst_pad_push_event)(self.src, (api.gst_event_new_eos)()) };
                }
                Ok(Pumped::Ended)
            }
            -4 => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "GStreamer could not negotiate a decodable format",
            )),
            _ => Err(io::Error::other("GStreamer could not decode the stream")),
        }
    }

    /// The cache index of the source byte at `position`, reading it with a new brokered read
    /// when the read-ahead cache does not hold it.
    fn read_at(&mut self, position: u64) -> io::Result<usize> {
        let cached =
            position >= self.cache_offset && position < self.cache_offset + self.cache.len() as u64;
        if !cached {
            self.source.seek(SeekFrom::Start(position))?;
            let size = (self.shared.length - position).min(READ_BYTES as u64) as usize;
            self.cache.resize(size, 0);
            let read = read_fully(&mut *self.source, &mut self.cache)?;
            self.cache.truncate(read);
            self.cache_offset = position;
            if read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "The captured source ended early",
                ));
            }
        }
        Ok((position - self.cache_offset) as usize)
    }
}

enum Pumped {
    Pushed(usize),
    Ended,
}

/// Typefind results that are audio/video containers (a missing demuxer for one of them means
/// a missing plugin); anything else is not media the helpers open.
fn container_like(name: &[u8]) -> bool {
    name.starts_with(b"video/")
        || name.starts_with(b"audio/")
        || matches!(
            name,
            b"application/ogg"
                | b"application/x-ogg"
                | b"application/x-ogm-video"
                | b"application/mxf"
        )
}

fn read_fully(source: &mut dyn ReadSeek, buffer: &mut [u8]) -> io::Result<usize> {
    let mut total = 0;
    while total < buffer.len() {
        match source.read(&mut buffer[total..])? {
            0 => break,
            count => total += count,
        }
    }
    Ok(total)
}

impl Drop for Pipeline<'_> {
    fn drop(&mut self) {
        let api = self.api;
        // Callbacks borrow the state themselves while elements stop, so no borrow is held
        // across the calls below.
        let (elements, pads) = {
            let state = self.shared.state.borrow();
            (state.elements.clone(), state.pads.clone())
        };
        // SAFETY: every element and pad below is owned by this graph. Elements stop first, so
        // no callback can run once they are released.
        unsafe {
            if !self.demuxer.is_null() {
                (api.gst_element_set_state)(self.demuxer, api::GST_STATE_NULL);
            }
            for element in &elements {
                (api.gst_element_set_state)(*element, api::GST_STATE_NULL);
            }
            if !self.src.is_null() {
                (api.gst_pad_set_active)(self.src, 0);
            }
            for pad in &pads {
                (api.gst_pad_set_active)(*pad, 0);
            }
            let mut state = self.shared.state.borrow_mut();
            state.elements.clear();
            state.pads.clear();
            let segment = std::mem::replace(&mut state.segment, ptr::null_mut());
            state.queue.clear();
            drop(state);
            // The demuxer is one of the elements; it is released with them.
            for element in elements {
                (api.gst_object_unref)(element);
            }
            for pad in pads {
                (api.gst_object_unref)(pad);
            }
            if !self.src.is_null() {
                (api.gst_object_unref)(self.src);
            }
            if !segment.is_null() {
                (api.gst_segment_free)(segment);
            }
        }
    }
}

/// An element from `factory`, owned by the graph, limited to one thread where it has a knob.
unsafe fn create(shared: &Shared, factory: *mut c_void) -> io::Result<*mut c_void> {
    let api = shared.api;
    // SAFETY: a live factory; the floating reference is sunk into our ownership.
    let element = unsafe { (api.gst_element_factory_create)(factory, ptr::null()) };
    if element.is_null() {
        return Err(io::Error::other("A GStreamer element could not be created"));
    }
    // SAFETY: as above.
    let element = unsafe { (api.gst_object_ref_sink)(element) };
    shared.state.borrow_mut().elements.push(element);
    for name in [c"max-threads", c"n-threads", c"threads"] {
        // SAFETY: a live element whose class pointer comes first in its instance.
        unsafe { set_int_property(api, element, name, 1) };
    }
    Ok(element)
}

/// Set an int or uint property to `value` if the object has it.
unsafe fn set_int_property(api: &Api, object: *mut c_void, name: &CStr, value: i32) -> bool {
    // SAFETY: GTypeInstance starts with its class pointer; the param spec's value type sits at
    // a fixed offset of the public GParamSpec.
    unsafe {
        let class = object.cast::<*mut c_void>().read();
        let spec = (api.g_object_class_find_property)(class, name.as_ptr());
        if spec.is_null() {
            return false;
        }
        let kind = spec
            .cast::<u8>()
            .add(api::PARAM_SPEC_VALUE_TYPE_OFFSET)
            .cast::<api::GType>()
            .read_unaligned();
        let mut gvalue = Value {
            type_: 0,
            data: [0; 2],
        };
        match kind {
            api::G_TYPE_INT => {
                (api.g_value_init)(&mut gvalue, api::G_TYPE_INT);
                (api.g_value_set_int)(&mut gvalue, value);
            }
            api::G_TYPE_UINT => {
                (api.g_value_init)(&mut gvalue, api::G_TYPE_UINT);
                (api.g_value_set_uint)(&mut gvalue, value as u32);
            }
            _ => return false,
        }
        (api.g_object_set_property)(object, name.as_ptr(), &gvalue);
        (api.g_value_unset)(&mut gvalue);
        true
    }
}

/// Element factories of one type whose pads accept `caps`, best rank first.
pub(super) struct Factories<'a> {
    api: &'a Api,
    list: *mut api::List,
    sorted: Vec<*mut c_void>,
}

impl<'a> Factories<'a> {
    pub(super) fn matching(api: &'a Api, kind: u64, caps: *const c_void, direction: c_int) -> Self {
        // SAFETY: GStreamer returns owned factory lists; the filtered list holds its own refs.
        unsafe {
            let all = (api.gst_element_factory_list_get_elements)(kind, api::GST_RANK_MARGINAL);
            let list = (api.gst_element_factory_list_filter)(all, caps, direction, 0);
            (api.gst_plugin_feature_list_free)(all);
            let mut sorted = Vec::new();
            let mut node = list;
            while !node.is_null() {
                sorted.push((*node).data);
                node = (*node).next;
            }
            sorted.sort_by_key(|factory| {
                std::cmp::Reverse((api.gst_plugin_feature_get_rank)(*factory))
            });
            Self { api, list, sorted }
        }
    }

    pub(super) fn best(&self) -> Option<*mut c_void> {
        self.sorted.first().copied()
    }

    fn all(&self) -> &[*mut c_void] {
        &self.sorted
    }
}

impl Drop for Factories<'_> {
    fn drop(&mut self) {
        // SAFETY: the list this value owns.
        unsafe { (self.api.gst_plugin_feature_list_free)(self.list) };
    }
}

/// The caps a factory's source pad template produces (owned), or null.
unsafe fn source_template_caps(api: &Api, factory: *mut c_void) -> *mut c_void {
    // SAFETY: the factory's static template list is owned by the factory; templates are
    // public structs with the direction after the name pointer.
    unsafe {
        let mut node = (api.gst_element_factory_get_static_pad_templates)(factory);
        while !node.is_null() {
            let template = (*node).data;
            let direction = template
                .cast::<u8>()
                .add(api::STATIC_TEMPLATE_DIRECTION_OFFSET)
                .cast::<c_int>()
                .read_unaligned();
            if direction == api::GST_PAD_SRC {
                return (api.gst_static_pad_template_get_caps)(template);
            }
            node = (*node).next;
        }
        ptr::null_mut()
    }
}

/// The decoding chain for `caps`: a parser when one exists and the stream is not parsed yet,
/// then the best decoder that accepts the parser's (or the demuxer's) output. Raw streams
/// need neither. Empty when no decoder is loaded.
unsafe fn plan(api: &Api, caps: *mut c_void) -> Option<Vec<*mut c_void>> {
    // SAFETY: caps owned by the caller; factory lists owned by `Factories`.
    unsafe {
        let structure = (api.gst_caps_get_structure)(caps, 0);
        if structure.is_null() {
            return None;
        }
        let name = CStr::from_ptr((api.gst_structure_get_name)(structure)).to_bytes();
        if name == b"video/x-raw" || name == b"audio/x-raw" {
            return Some(Vec::new());
        }
        let mut parsed = 0;
        let is_parsed = (api.gst_structure_get_boolean)(structure, c"parsed".as_ptr(), &mut parsed)
            != 0
            && parsed != 0;
        if !is_parsed {
            let parsers = Factories::matching(
                api,
                api::GST_ELEMENT_FACTORY_TYPE_PARSER,
                caps,
                api::GST_PAD_SINK,
            );
            for parser in parsers.all() {
                let output = source_template_caps(api, *parser);
                if output.is_null() {
                    continue;
                }
                let decoders = Factories::matching(
                    api,
                    api::GST_ELEMENT_FACTORY_TYPE_DECODER,
                    output,
                    api::GST_PAD_SINK,
                );
                (api.gst_mini_object_unref)(output);
                if let Some(decoder) = decoders.best() {
                    return Some(vec![*parser, decoder]);
                }
            }
        }
        let decoders = Factories::matching(
            api,
            api::GST_ELEMENT_FACTORY_TYPE_DECODER,
            caps,
            api::GST_PAD_SINK,
        );
        decoders.best().map(|decoder| vec![decoder])
    }
}

fn caps_of(api: &Api, pad: *mut c_void) -> *mut c_void {
    // SAFETY: a live pad; both calls return an owned reference (or null).
    unsafe {
        let caps = (api.gst_pad_get_current_caps)(pad);
        if caps.is_null() {
            (api.gst_pad_query_caps)(pad, ptr::null_mut())
        } else {
            caps
        }
    }
}

unsafe extern "C" fn no_more_pads(_: *mut c_void, data: *mut c_void) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: the Shared registered with the signal outlives the demuxer's activity.
        let shared = unsafe { &*data.cast::<Shared>() };
        if let Ok(mut state) = shared.state.try_borrow_mut() {
            state.no_more_pads = true;
        }
    }));
}

unsafe extern "C" fn pad_added(_: *mut c_void, pad: *mut c_void, data: *mut c_void) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: as for no_more_pads.
        let shared = unsafe { &*data.cast::<Shared>() };
        if let Err(error) = unsafe { attach(shared, pad) } {
            shared.fail(error);
        }
    }));
}

/// Decide what a new demuxer pad feeds: the wanted stream's decoder chain, or a drop sink.
unsafe fn attach(shared: &Shared, pad: *mut c_void) -> io::Result<()> {
    let api = shared.api;
    let caps = caps_of(api, pad);
    if caps.is_null() {
        return unsafe { drop_pad(shared, pad) };
    }
    // SAFETY: owned caps, released at the end.
    let result = unsafe {
        let structure = (api.gst_caps_get_structure)(caps, 0);
        let name = if structure.is_null() {
            String::new()
        } else {
            CStr::from_ptr((api.gst_structure_get_name)(structure))
                .to_string_lossy()
                .into_owned()
        };
        let medium = if name.starts_with("video/") || name == "image/jpeg" {
            Some(Kind::Video)
        } else if name.starts_with("audio/") {
            Some(Kind::Audio)
        } else {
            None
        };
        match medium {
            Some(medium) => {
                let codec = codec::from_caps(api, structure);
                let first = {
                    let state = shared.state.borrow();
                    match medium {
                        Kind::Video => state.video.is_none(),
                        Kind::Audio => state.audio.is_none(),
                    }
                };
                let plan = plan(api, caps);
                if first {
                    let found = Discovered {
                        codec,
                        decodable: plan.is_some(),
                    };
                    let mut state = shared.state.borrow_mut();
                    match medium {
                        Kind::Video => state.video = Some(found),
                        Kind::Audio => state.audio = Some(found),
                    }
                }
                match plan {
                    Some(chain) if first && medium == shared.kind => build(shared, pad, &chain),
                    _ => drop_pad(shared, pad),
                }
            }
            None => drop_pad(shared, pad),
        }
    };
    // SAFETY: our caps reference.
    unsafe { (api.gst_mini_object_unref)(caps) };
    result
}

/// A sink pad of ours, owned by the graph.
unsafe fn our_pad(
    shared: &Shared,
    chain: api::ChainFunction,
    event: api::EventFunction,
    query: api::QueryFunction,
) -> io::Result<*mut c_void> {
    let api = shared.api;
    // SAFETY: a new pad, sunk into our ownership, carrying the graph pointer.
    unsafe {
        let pad = (api.gst_pad_new)(c"sink".as_ptr(), api::GST_PAD_SINK);
        if pad.is_null() {
            return Err(io::Error::other("GStreamer pad allocation failed"));
        }
        let pad = (api.gst_object_ref_sink)(pad);
        shared.state.borrow_mut().pads.push(pad);
        (api.g_object_set_data)(
            pad,
            DATA_KEY.as_ptr(),
            (shared as *const Shared).cast_mut().cast(),
        );
        (api.gst_pad_set_chain_function_full)(pad, chain, ptr::null_mut(), ptr::null());
        (api.gst_pad_set_event_function_full)(pad, event, ptr::null_mut(), ptr::null());
        (api.gst_pad_set_query_function_full)(pad, query, ptr::null_mut(), ptr::null());
        (api.gst_pad_set_active)(pad, 1);
        Ok(pad)
    }
}

unsafe fn drop_pad(shared: &Shared, pad: *mut c_void) -> io::Result<()> {
    // SAFETY: a demuxer pad and a new sink pad that accepts and discards everything.
    unsafe {
        let sink = our_pad(shared, drop_chain, drop_event, drop_query)?;
        (shared.api.gst_pad_link)(pad, sink);
    }
    Ok(())
}

/// Link `pad` through `chain` (parser and decoder, possibly none), the converters and a caps
/// filter to our output pad.
unsafe fn build(shared: &Shared, pad: *mut c_void, chain: &[*mut c_void]) -> io::Result<()> {
    let api = shared.api;
    let (converters, filter_caps): (&[&CStr], CString) = match shared.kind {
        Kind::Video => (
            &[c"videoconvert", c"videoscale"],
            CString::new("video/x-raw,format=RGBA").expect("static caps"),
        ),
        Kind::Audio => (
            &[c"audioconvert", c"audioresample"],
            CString::new("audio/x-raw,format=S16LE,layout=interleaved,rate=48000,channels=2")
                .expect("static caps"),
        ),
    };
    // SAFETY: factories and elements of this graph; every created element is owned by it.
    unsafe {
        let mut elements = Vec::new();
        for factory in chain {
            elements.push(create(shared, *factory)?);
        }
        let decoder = elements.last().copied();
        for name in converters {
            let factory = (api.gst_element_factory_find)(name.as_ptr());
            if factory.is_null() {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "GStreamer's converters are not installed",
                ));
            }
            let element = create(shared, factory);
            (api.gst_object_unref)(factory);
            elements.push(element?);
        }
        let capsfilter = (api.gst_element_factory_find)(c"capsfilter".as_ptr());
        if capsfilter.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "GStreamer's capsfilter is missing",
            ));
        }
        let filter = create(shared, capsfilter);
        (api.gst_object_unref)(capsfilter);
        let filter = filter?;
        set_caps_property(api, filter, &filter_caps);
        elements.push(filter);
        shared.state.borrow_mut().capsfilter = filter;
        for pair in elements.windows(2) {
            if (api.gst_element_link)(pair[0], pair[1]) == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "GStreamer could not link the decoding chain",
                ));
            }
        }
        if shared.kind == Kind::Video {
            // Watch the decoder's output: its caps and orientation tags choose the output size.
            // A raw stream has no decoder: watch the demuxer's pad itself.
            let (watched, owned) = match decoder {
                Some(decoder) => (
                    (api.gst_element_get_static_pad)(decoder, c"src".as_ptr()),
                    true,
                ),
                None => (pad, false),
            };
            if !watched.is_null() {
                (api.gst_pad_add_probe)(
                    watched,
                    api::GST_PAD_PROBE_TYPE_EVENT_DOWNSTREAM,
                    decoder_events,
                    (shared as *const Shared).cast_mut().cast(),
                    ptr::null(),
                );
                if owned {
                    (api.gst_object_unref)(watched);
                }
            }
        }
        let output = our_pad(shared, output_chain, output_event, output_query)?;
        let last = (api.gst_element_get_static_pad)(filter, c"src".as_ptr());
        let linked = (api.gst_pad_link)(last, output);
        (api.gst_object_unref)(last);
        if linked != 0 {
            return Err(io::Error::other("GStreamer could not link the output"));
        }
        for element in elements.iter().rev() {
            if (api.gst_element_set_state)(*element, api::GST_STATE_PLAYING)
                == api::GST_STATE_CHANGE_FAILURE
            {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "A GStreamer decoder could not start",
                ));
            }
        }
        let first = (api.gst_element_get_static_pad)(elements[0], c"sink".as_ptr());
        let linked = (api.gst_pad_link)(pad, first);
        (api.gst_object_unref)(first);
        if linked != 0 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "The decoder refused the demuxed stream",
            ));
        }
        shared.state.borrow_mut().linked = true;
    }
    Ok(())
}

unsafe fn set_caps_property(api: &Api, element: *mut c_void, caps: &CStr) {
    // SAFETY: new caps, passed through a boxed GValue that keeps its own reference.
    unsafe {
        let caps = (api.gst_caps_from_string)(caps.as_ptr());
        if caps.is_null() {
            return;
        }
        let mut gvalue = Value {
            type_: 0,
            data: [0; 2],
        };
        (api.g_value_init)(&mut gvalue, (api.gst_caps_get_type)());
        (api.g_value_set_boxed)(&mut gvalue, caps);
        (api.g_object_set_property)(element, c"caps".as_ptr(), &gvalue);
        (api.g_value_unset)(&mut gvalue);
        (api.gst_mini_object_unref)(caps);
    }
}

fn event_type(event: *mut c_void) -> c_int {
    // SAFETY: GstEvent.type follows the 64-byte GstMiniObject.
    unsafe {
        event
            .cast::<u8>()
            .add(api::MINI_OBJECT_TYPE_OFFSET)
            .cast::<c_int>()
            .read_unaligned()
    }
}

fn query_type(query: *mut c_void) -> c_int {
    // SAFETY: GstQuery.type follows the 64-byte GstMiniObject.
    unsafe {
        query
            .cast::<u8>()
            .add(api::MINI_OBJECT_TYPE_OFFSET)
            .cast::<c_int>()
            .read_unaligned()
    }
}

/// The decoder's caps and tags choose the output size (the caps filter) and orientation.
unsafe extern "C" fn decoder_events(_: *mut c_void, info: *mut c_void, data: *mut c_void) -> c_int {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: the registered Shared; GstPadProbeInfo.data holds the event.
        let shared = unsafe { &*data.cast::<Shared>() };
        let api = shared.api;
        let event = unsafe {
            info.cast::<u8>()
                .add(api::PROBE_INFO_DATA_OFFSET)
                .cast::<*mut c_void>()
                .read_unaligned()
        };
        if event.is_null() {
            return;
        }
        let kind = event_type(event);
        let changed = if kind == api::GST_EVENT_CAPS {
            let mut caps = ptr::null_mut();
            // SAFETY: a caps event; the caps stay owned by the event.
            unsafe { (api.gst_event_parse_caps)(event, &mut caps) };
            let picture = unsafe { decoded_picture(shared, caps) };
            match picture {
                Ok(picture) => {
                    let mut state = shared.state.borrow_mut();
                    state.decoded = Some(picture.0);
                    state.transfer_709 = picture.1;
                    true
                }
                Err(error) => {
                    shared.fail(error);
                    false
                }
            }
        } else if kind == api::GST_EVENT_TAG {
            let mut tags = ptr::null_mut();
            // SAFETY: a tag event; the list stays owned by the event.
            unsafe { (api.gst_event_parse_tag)(event, &mut tags) };
            match unsafe { orientation(api, tags) } {
                Ok(Some(rotation)) => {
                    let mut state = shared.state.borrow_mut();
                    let changed = state.rotation != rotation;
                    state.rotation = rotation;
                    changed
                }
                Ok(None) => false,
                Err(error) => {
                    shared.fail(error);
                    false
                }
            }
        } else {
            false
        };
        if changed {
            unsafe { resize(shared) };
        }
    }));
    api::GST_PAD_PROBE_OK
}

/// Width, height and pixel aspect of decoded caps, and whether the BT.709 transfer applies.
unsafe fn decoded_picture(shared: &Shared, caps: *mut c_void) -> io::Result<(Decoded, bool)> {
    let api = shared.api;
    // SAFETY: caps owned by the event being probed.
    unsafe {
        let structure = (api.gst_caps_get_structure)(caps, 0);
        if structure.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Decoded caps are empty",
            ));
        }
        let (mut width, mut height) = (0, 0);
        (api.gst_structure_get_int)(structure, c"width".as_ptr(), &mut width);
        (api.gst_structure_get_int)(structure, c"height".as_ptr(), &mut height);
        let (mut numerator, mut denominator) = (1, 1);
        if (api.gst_structure_get_fraction)(
            structure,
            c"pixel-aspect-ratio".as_ptr(),
            &mut numerator,
            &mut denominator,
        ) == 0
            || numerator <= 0
            || denominator <= 0
        {
            (numerator, denominator) = (1, 1);
        }
        let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height)) else {
            return Err(io::Error::other("Decoded video dimensions are invalid"));
        };
        if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 32 * 1024 * 1024 {
            return Err(io::Error::other("Decoded video dimensions exceed bound"));
        }
        let format = (api.gst_structure_get_string)(structure, c"format".as_ptr());
        let rgb = !format.is_null()
            && matches!(
                CStr::from_ptr(format).to_bytes(),
                b"RGB"
                    | b"BGR"
                    | b"RGBA"
                    | b"BGRA"
                    | b"ARGB"
                    | b"ABGR"
                    | b"RGBx"
                    | b"BGRx"
                    | b"xRGB"
                    | b"xBGR"
            );
        let colorimetry = (api.gst_structure_get_string)(structure, c"colorimetry".as_ptr());
        let parsed = match (api.gst_video_colorimetry_from_string, colorimetry.is_null()) {
            (Some(parse), false) => {
                let mut value = api::Colorimetry::default();
                (parse(&mut value, colorimetry) != 0).then_some(value)
            }
            _ => None,
        };
        let transfer_709 = codec::transfer_709(parsed, rgb)?;
        Ok(((width, height, (numerator, denominator)), transfer_709))
    }
}

/// `image-orientation` as a clockwise rotation; flips are refused like the bundled profile
/// refuses reflections.
unsafe fn orientation(api: &Api, tags: *mut c_void) -> io::Result<Option<i32>> {
    if tags.is_null() {
        return Ok(None);
    }
    let mut value: *mut c_char = ptr::null_mut();
    // SAFETY: a live tag list; the copied string is freed below.
    let found =
        unsafe { (api.gst_tag_list_get_string)(tags, c"image-orientation".as_ptr(), &mut value) };
    if found == 0 || value.is_null() {
        return Ok(None);
    }
    // SAFETY: a NUL-terminated copy owned by us.
    let text = unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .into_owned();
    unsafe { (api.g_free)(value.cast()) };
    match text.as_str() {
        "rotate-0" => Ok(Some(0)),
        "rotate-90" => Ok(Some(90)),
        "rotate-180" => Ok(Some(180)),
        "rotate-270" => Ok(Some(270)),
        _ => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Unsupported video display transform",
        )),
    }
}

/// Set the caps filter to the output size for the decoded picture and orientation.
unsafe fn resize(shared: &Shared) {
    let (filter, decoded, rotation) = {
        let state = shared.state.borrow();
        (state.capsfilter, state.decoded, state.rotation)
    };
    let (Some(bounds), Some((width, height, (numerator, denominator)))) = (shared.bounds, decoded)
    else {
        return;
    };
    if filter.is_null() {
        return;
    }
    match picture::fit(width, height, bounds, rotation) {
        Ok((target_width, target_height)) => {
            let caps = CString::new(format!(
                "video/x-raw,format=RGBA,width={target_width},height={target_height},pixel-aspect-ratio={numerator}/{denominator}"
            ))
            .expect("formatted caps");
            // SAFETY: a capsfilter element owned by the graph.
            unsafe { set_caps_property(shared.api, filter, &caps) };
        }
        Err(error) => shared.fail(error),
    }
}

unsafe extern "C" fn output_chain(pad: *mut c_void, _: *mut c_void, buffer: *mut c_void) -> c_int {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let Some(shared) = graph_for(pad) else {
            return api::GST_FLOW_OK;
        };
        let api = shared.api;
        // SAFETY: we own the pushed buffer; it is mapped, copied and released here.
        let output = unsafe { take_output(shared, buffer) };
        unsafe { (api.gst_mini_object_unref)(buffer) };
        match output {
            Ok(Some(output)) => {
                shared.state.borrow_mut().queue.push_back(output);
                api::GST_FLOW_OK
            }
            Ok(None) => api::GST_FLOW_OK,
            Err(error) => {
                shared.fail(error);
                -5
            }
        }
    }));
    result.unwrap_or(-5)
}

fn graph_for<'a>(pad: *mut c_void) -> Option<&'a Shared> {
    // The graph pointer lives in pad data; the API to read it comes from the graph itself, so
    // read it with GObject's getter resolved for any loaded graph.
    let api = super::loaded_api()?;
    graph(api, pad)
}

/// A converted buffer as output, in stream time; `None` outside the current segment.
unsafe fn take_output(shared: &Shared, buffer: *mut c_void) -> io::Result<Option<Output>> {
    let api = shared.api;
    // SAFETY: a live buffer; GstBuffer.pts sits after the mini object and pool pointer.
    let pts = unsafe {
        buffer
            .cast::<u8>()
            .add(api::BUFFER_PTS_OFFSET)
            .cast::<u64>()
            .read_unaligned()
    };
    if pts == api::GST_CLOCK_TIME_NONE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Decoded output has no presentation timestamp",
        ));
    }
    let segment = shared.state.borrow().segment;
    if segment.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Decoded output before its segment",
        ));
    }
    // SAFETY: our copy of the current segment.
    let nanos = unsafe { (api.gst_segment_to_stream_time)(segment, api::GST_FORMAT_TIME, pts) };
    if nanos == api::GST_CLOCK_TIME_NONE {
        return Ok(None);
    }
    let mut map = MapInfo::empty();
    // SAFETY: a readable mapping released below.
    if unsafe { (api.gst_buffer_map)(buffer, &mut map, api::GST_MAP_READ) } == 0 {
        return Err(io::Error::other("Decoded output could not be mapped"));
    }
    // SAFETY: the mapping's bytes.
    let bytes = unsafe { std::slice::from_raw_parts(map.data, map.size) };
    let result = match shared.kind {
        Kind::Video => {
            let size = shared.state.borrow().output;
            match size {
                Some((width, height)) if bytes.len() >= width as usize * height as usize * 4 => {
                    Ok(Some(Output::Video {
                        nanos,
                        width,
                        height,
                        rgba: bytes[..width as usize * height as usize * 4].to_vec(),
                    }))
                }
                _ => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Converted video does not match its caps",
                )),
            }
        }
        Kind::Audio => {
            if bytes.len() % 4 != 0 {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Converted audio is not whole frames",
                ))
            } else {
                Ok(Some(Output::Audio {
                    nanos,
                    samples: bytes
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|pair| i16::from_le_bytes(*pair))
                        .collect(),
                }))
            }
        }
    };
    // SAFETY: the mapping made above.
    unsafe { (api.gst_buffer_unmap)(buffer, &mut map) };
    result
}

unsafe extern "C" fn output_event(
    pad: *mut c_void,
    _: *mut c_void,
    event: *mut c_void,
) -> api::Gboolean {
    let handled = catch_unwind(AssertUnwindSafe(|| {
        let Some(shared) = graph_for(pad) else {
            return 1;
        };
        let api = shared.api;
        let kind = event_type(event);
        // SAFETY: events delivered to our pad; parsed values stay owned by the event.
        unsafe {
            if kind == api::GST_EVENT_CAPS {
                let mut caps = ptr::null_mut();
                (api.gst_event_parse_caps)(event, &mut caps);
                let structure = (api.gst_caps_get_structure)(caps, 0);
                if shared.kind == Kind::Video && !structure.is_null() {
                    let (mut width, mut height) = (0, 0);
                    (api.gst_structure_get_int)(structure, c"width".as_ptr(), &mut width);
                    (api.gst_structure_get_int)(structure, c"height".as_ptr(), &mut height);
                    let size = (u32::try_from(width).ok(), u32::try_from(height).ok());
                    let mut state = shared.state.borrow_mut();
                    state.output = match size {
                        (Some(width), Some(height)) if width > 0 && height > 0 => {
                            Some((width, height))
                        }
                        _ => None,
                    };
                }
            } else if kind == api::GST_EVENT_SEGMENT {
                let mut segment: *const c_void = ptr::null();
                (api.gst_event_parse_segment)(event, &mut segment);
                let copy = (api.gst_segment_copy)(segment);
                let mut state = shared.state.borrow_mut();
                if !state.segment.is_null() {
                    (api.gst_segment_free)(state.segment);
                }
                state.segment = copy;
            } else if kind == api::GST_EVENT_EOS {
                shared.state.borrow_mut().sink_eos = true;
            } else if kind == api::GST_EVENT_FLUSH_START || kind == api::GST_EVENT_FLUSH_STOP {
                let mut state = shared.state.borrow_mut();
                state.queue.clear();
                state.sink_eos = false;
            }
            (api.gst_mini_object_unref)(event);
        }
        1
    }));
    handled.unwrap_or(0)
}

unsafe extern "C" fn output_query(
    pad: *mut c_void,
    parent: *mut c_void,
    query: *mut c_void,
) -> api::Gboolean {
    let answered = catch_unwind(AssertUnwindSafe(|| {
        let Some(shared) = graph_for(pad) else {
            return 0;
        };
        let api = shared.api;
        let kind = query_type(query);
        let accepted: &CStr = match shared.kind {
            Kind::Video => c"video/x-raw,format=RGBA",
            Kind::Audio => c"audio/x-raw,format=S16LE,layout=interleaved,rate=48000,channels=2",
        };
        // SAFETY: queries delivered to our pad; created caps are released or handed over.
        unsafe {
            if kind == api::GST_QUERY_CAPS {
                let caps = (api.gst_caps_from_string)(accepted.as_ptr());
                (api.gst_query_set_caps_result)(query, caps);
                (api.gst_mini_object_unref)(caps);
                1
            } else if kind == api::GST_QUERY_ACCEPT_CAPS {
                let mut offered = ptr::null_mut();
                (api.gst_query_parse_accept_caps)(query, &mut offered);
                let caps = (api.gst_caps_from_string)(accepted.as_ptr());
                let fits = (api.gst_caps_can_intersect)(caps, offered);
                (api.gst_mini_object_unref)(caps);
                (api.gst_query_set_accept_caps_result)(query, fits);
                1
            } else if kind == api::GST_QUERY_ALLOCATION {
                // No pool and no video meta: converters then write default, tightly packed rows.
                0
            } else {
                (api.gst_pad_query_default)(pad, parent, query)
            }
        }
    }));
    answered.unwrap_or(0)
}

unsafe extern "C" fn drop_chain(_: *mut c_void, _: *mut c_void, buffer: *mut c_void) -> c_int {
    if let Some(api) = super::loaded_api() {
        // SAFETY: we own the pushed buffer.
        unsafe { (api.gst_mini_object_unref)(buffer) };
    }
    api::GST_FLOW_OK
}

unsafe extern "C" fn drop_event(
    _: *mut c_void,
    _: *mut c_void,
    event: *mut c_void,
) -> api::Gboolean {
    if let Some(api) = super::loaded_api() {
        // SAFETY: we own the delivered event.
        unsafe { (api.gst_mini_object_unref)(event) };
    }
    1
}

unsafe extern "C" fn drop_query(_: *mut c_void, _: *mut c_void, _: *mut c_void) -> api::Gboolean {
    0
}

/// Upstream queries reaching the source pad: push-only scheduling over a seekable byte range.
unsafe extern "C" fn source_query(
    pad: *mut c_void,
    _: *mut c_void,
    query: *mut c_void,
) -> api::Gboolean {
    let answered = catch_unwind(AssertUnwindSafe(|| {
        let Some(shared) = graph_for(pad) else {
            return 0;
        };
        let api = shared.api;
        let length = i64::try_from(shared.length).unwrap_or(i64::MAX);
        let kind = query_type(query);
        // SAFETY: queries delivered to our source pad.
        unsafe {
            if kind == api::GST_QUERY_SCHEDULING {
                (api.gst_query_set_scheduling)(query, api::GST_SCHEDULING_FLAG_SEEKABLE, 1, -1, 0);
                (api.gst_query_add_scheduling_mode)(query, api::GST_PAD_MODE_PUSH);
                1
            } else if kind == api::GST_QUERY_SEEKING {
                let mut format = 0;
                (api.gst_query_parse_seeking)(
                    query,
                    &mut format,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                );
                if format == api::GST_FORMAT_BYTES {
                    (api.gst_query_set_seeking)(query, format, 1, 0, length);
                    1
                } else {
                    0
                }
            } else if kind == api::GST_QUERY_DURATION {
                let mut format = 0;
                (api.gst_query_parse_duration)(query, &mut format, ptr::null_mut());
                if format == api::GST_FORMAT_BYTES {
                    (api.gst_query_set_duration)(query, format, length);
                    1
                } else {
                    0
                }
            } else {
                0
            }
        }
    }));
    answered.unwrap_or(0)
}

/// Upstream events reaching the source pad. A demuxer's byte seek flushes the graph here, on
/// this thread, and the next push starts at the requested offset with a new segment.
unsafe extern "C" fn source_event(
    pad: *mut c_void,
    _: *mut c_void,
    event: *mut c_void,
) -> api::Gboolean {
    let handled = catch_unwind(AssertUnwindSafe(|| {
        let Some(shared) = graph_for(pad) else {
            return 0;
        };
        let api = shared.api;
        let kind = event_type(event);
        let mut handled = 0;
        // SAFETY: an event delivered to our source pad, released at the end.
        unsafe {
            if kind == api::GST_EVENT_SEEK {
                let (mut rate, mut format, mut flags) = (0.0, 0, 0);
                let (mut start_type, mut start, mut stop_type, mut stop) = (0, 0i64, 0, 0i64);
                (api.gst_event_parse_seek)(
                    event,
                    &mut rate,
                    &mut format,
                    &mut flags,
                    &mut start_type,
                    &mut start,
                    &mut stop_type,
                    &mut stop,
                );
                let target = u64::try_from(start)
                    .ok()
                    .filter(|start| *start <= shared.length);
                if let (api::GST_FORMAT_BYTES, api::GST_SEEK_TYPE_SET, Some(target), true) =
                    (format, start_type, target, rate > 0.0)
                {
                    let src = shared.src.get();
                    if flags & api::GST_SEEK_FLAG_FLUSH != 0 {
                        (api.gst_pad_push_event)(src, (api.gst_event_new_flush_start)());
                        (api.gst_pad_push_event)(src, (api.gst_event_new_flush_stop)(1));
                    }
                    let mut state = shared.state.borrow_mut();
                    state.position = target;
                    state.seek_generation += 1;
                    state.segment_pending = true;
                    state.eos_sent = false;
                    handled = 1;
                }
            }
            (api.gst_mini_object_unref)(event);
        }
        handled
    }));
    handled.unwrap_or(0)
}

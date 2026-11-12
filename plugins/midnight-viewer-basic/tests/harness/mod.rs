#![allow(unsafe_code)]
use midnight_plugin_sdk::api::{abi, codec, messages};
use std::{
    collections::{HashMap, VecDeque},
    mem::MaybeUninit,
    sync::Mutex,
};

struct Work {
    context: usize,
    run: unsafe extern "C" fn(*mut std::ffi::c_void),
    drop: unsafe extern "C" fn(*mut std::ffi::c_void),
}
pub struct Service {
    id: u64,
    session: u64,
    generation: u64,
    pub read: messages::ReadAt,
}
struct Buffer {
    bytes: Box<[u8]>,
    refs: usize,
}
struct State {
    jobs: Mutex<VecDeque<Work>>,
    services: Mutex<VecDeque<Service>>,
    buffers: Mutex<HashMap<u64, Buffer>>,
}
pub struct Event {
    pub id: u64,
    pub kind: u32,
    pub payload: Vec<u8>,
    pub content: Vec<u8>,
}
pub struct Host {
    state: Box<State>,
    api: abi::PluginApi,
    instance: u64,
    pub data: Vec<u8>,
    pub base: u64,
    pub short: usize,
    pub reads: Vec<messages::ReadAt>,
    pub events: Vec<Event>,
    next_buffer: u64,
}
fn empty() -> abi::ByteSlice {
    slice(&[])
}
fn slice(bytes: &[u8]) -> abi::ByteSlice {
    abi::ByteSlice {
        data: bytes.as_ptr(),
        len: bytes.len() as u32,
        reserved: 0,
    }
}
unsafe fn state<'a>(id: u64) -> &'a State {
    unsafe { &*(id as *const State) }
}
unsafe extern "C" fn schedule(id: u64, work: *const abi::Work) -> u32 {
    let w = unsafe { &*work };
    unsafe { state(id) }.jobs.lock().unwrap().push_back(Work {
        context: w.context as usize,
        run: w.run.unwrap(),
        drop: w.drop.unwrap(),
    });
    abi::OK
}
unsafe extern "C" fn service(id: u64, r: *const abi::Request) -> u32 {
    let r = unsafe { &*r };
    assert_eq!(r.kind, abi::SERVICE_READ_AT);
    let payload = unsafe { std::slice::from_raw_parts(r.payload.data, r.payload.len as usize) };
    unsafe { state(id) }
        .services
        .lock()
        .unwrap()
        .push_back(Service {
            id: r.id,
            session: r.session,
            generation: r.generation,
            read: codec::decode(payload).unwrap(),
        });
    abi::OK
}
unsafe extern "C" fn retain(id: u64, b: u64) -> u32 {
    unsafe { state(id) }
        .buffers
        .lock()
        .unwrap()
        .get_mut(&b)
        .unwrap()
        .refs += 1;
    abi::OK
}
unsafe extern "C" fn release(id: u64, b: u64) -> u32 {
    let mut buffers = unsafe { state(id) }.buffers.lock().unwrap();
    let buffer = buffers.get_mut(&b).unwrap();
    buffer.refs -= 1;
    if buffer.refs == 0 {
        buffers.remove(&b);
    }
    abi::OK
}
unsafe extern "C" fn cancelled(_: u64, _: u64) -> u32 {
    0
}
unsafe extern "C" fn notify(_: u64) {}
unsafe extern "C" fn diagnostic(_: u64, _: u32, _: abi::ByteSlice) {}

impl Host {
    pub fn new(data: Vec<u8>) -> Self {
        let state = Box::new(State {
            jobs: Mutex::new(VecDeque::new()),
            services: Mutex::new(VecDeque::new()),
            buffers: Mutex::new(HashMap::new()),
        });
        let host = abi::HostApi {
            header: abi::Header {
                struct_size: size_of::<abi::HostApi>() as u32,
                abi_major: 1,
                abi_minor: 0,
                kind: abi::KIND_HOST,
                minimum_host_minor: 0,
                reserved: 0,
            },
            context: &*state as *const State as u64,
            schedule_work: Some(schedule),
            submit_service: Some(service),
            cancelled: Some(cancelled),
            notify_ready: Some(notify),
            retain_buffer: Some(retain),
            release_buffer: Some(release),
            diagnostic: Some(diagnostic),
        };
        let mut api = MaybeUninit::uninit();
        assert_eq!(
            unsafe {
                midnight_viewer_basic::midnight_plugin_entry_v1(
                    api.as_mut_ptr(),
                    size_of::<abi::PluginApi>() as u32,
                )
            },
            abi::OK
        );
        let api = unsafe { api.assume_init() };
        let mut instance = 0;
        assert_eq!(
            unsafe { api.create.unwrap()(&host, empty(), &mut instance) },
            abi::OK
        );
        Self {
            state,
            api,
            instance,
            data,
            base: 0,
            short: usize::MAX,
            reads: vec![],
            events: vec![],
            next_buffer: 1,
        }
    }
    pub fn submit<T: codec::Model>(
        &mut self,
        id: u64,
        session: u64,
        generation: u64,
        kind: u32,
        value: &T,
    ) {
        self.raw(
            id,
            session,
            generation,
            kind,
            &codec::encode(value).unwrap(),
            None,
        );
    }
    fn raw(
        &self,
        id: u64,
        session: u64,
        generation: u64,
        kind: u32,
        payload: &[u8],
        content: Option<abi::Buffer>,
    ) {
        let r = abi::Request {
            id,
            session,
            generation,
            kind,
            flags: 0,
            payload: slice(payload),
            content: content.unwrap_or(abi::Buffer {
                id: 0,
                bytes: empty(),
            }),
        };
        assert_eq!(
            unsafe { self.api.submit.unwrap()(self.instance, &r) },
            abi::OK
        );
    }
    pub fn open(&mut self, mode: u16) {
        self.submit(
            1,
            1,
            1,
            abi::REQUEST_OPEN,
            &messages::Open {
                reader: 7,
                length: Some(self.base + self.data.len() as u64),
                revision: 9,
                mode,
                display_name: "fixture".into(),
            },
        );
        self.pump();
    }
    pub fn close(&mut self, id: u64) {
        self.raw(id, 1, 1, abi::REQUEST_CLOSE, &[], None);
    }
    pub fn cancel(&self, id: u64) {
        assert_eq!(
            unsafe { self.api.cancel.unwrap()(self.instance, id) },
            abi::OK
        );
    }
    pub fn jobs(&mut self) {
        for _ in 0..100000 {
            let work = self.state.jobs.lock().unwrap().pop_front();
            let Some(work) = work else { return };
            unsafe {
                (work.run)(work.context as *mut _);
                (work.drop)(work.context as *mut _);
            }
            self.drain();
        }
        panic!("Unbounded worker loop");
    }
    pub fn pending(&self) -> usize {
        self.state.services.lock().unwrap().len()
    }
    pub fn service(&mut self) -> Service {
        let value = self.state.services.lock().unwrap().pop_front();
        value.expect("Plugin must request content")
    }
    pub fn reply(&mut self, s: Service, bytes: Vec<u8>, eof: bool, status: u32) {
        self.reads.push(s.read.clone());
        let reply = messages::ReadReply {
            revision: s.read.revision,
            offset: s.read.offset,
            bytes: bytes.len() as u32,
            eof,
            status,
        };
        let buffer = if bytes.is_empty() {
            None
        } else {
            let id = self.next_buffer;
            self.next_buffer += 1;
            let buffer = Buffer {
                bytes: bytes.into_boxed_slice(),
                refs: 1,
            };
            let b = abi::Buffer {
                id,
                bytes: slice(&buffer.bytes),
            };
            self.state.buffers.lock().unwrap().insert(id, buffer);
            Some(b)
        };
        self.raw(
            s.id,
            s.session,
            s.generation,
            abi::REQUEST_SERVICE_REPLY,
            &codec::encode(&reply).unwrap(),
            buffer,
        );
        if let Some(b) = buffer {
            unsafe {
                release(&*self.state as *const State as u64, b.id);
            }
        }
    }
    pub fn pump(&mut self) {
        for _ in 0..100000 {
            self.jobs();
            self.drain();
            if self.pending() == 0 {
                return;
            }
            let s = self.service();
            let offset = s.read.offset;
            let end = self.base + self.data.len() as u64;
            let count = (end.saturating_sub(offset) as usize)
                .min(s.read.length as usize)
                .min(self.short);
            let bytes = if offset < self.base {
                vec![b'x'; count]
            } else {
                self.data[(offset - self.base) as usize..(offset - self.base) as usize + count]
                    .to_vec()
            };
            self.reply(s, bytes, offset + count as u64 >= end, abi::OK);
        }
        panic!("Unbounded service loop");
    }
    fn drain(&mut self) {
        loop {
            let mut slots = [MaybeUninit::<abi::Event>::uninit(); 64];
            let mut count = 0;
            assert_eq!(
                unsafe {
                    self.api.drain.unwrap()(
                        self.instance,
                        64,
                        slots.as_mut_ptr().cast(),
                        &mut count,
                    )
                },
                abi::OK
            );
            if count == 0 {
                break;
            }
            for slot in slots.into_iter().take(count as usize) {
                let e = unsafe { slot.assume_init() };
                let copy = |b: abi::Buffer| {
                    if b.bytes.len == 0 {
                        vec![]
                    } else {
                        unsafe { std::slice::from_raw_parts(b.bytes.data, b.bytes.len as usize) }
                            .to_vec()
                    }
                };
                self.events.push(Event {
                    id: e.request_id,
                    kind: e.kind,
                    payload: copy(e.payload),
                    content: copy(e.content),
                });
                for b in [e.payload, e.content] {
                    if b.id != 0 {
                        assert_eq!(
                            unsafe { self.api.release_buffer.unwrap()(self.instance, b.id) },
                            abi::OK
                        );
                    }
                }
            }
        }
    }
    pub fn event(&self, id: u64, kind: u32) -> &Event {
        self.events
            .iter()
            .find(|e| e.id == id && e.kind == kind)
            .unwrap_or_else(|| {
                panic!(
                    "Missing event {kind} for {id}: {:?}",
                    self.events
                        .iter()
                        .map(|e| (e.id, e.kind))
                        .collect::<Vec<_>>()
                )
            })
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        unsafe {
            self.api.begin_shutdown.unwrap()(self.instance);
        }
        self.pump();
        assert_eq!(unsafe { self.api.destroy.unwrap()(self.instance) }, abi::OK);
        assert!(
            self.state.buffers.lock().unwrap().is_empty(),
            "Host content lease leak"
        );
    }
}

#![allow(unsafe_code)]
use midnight_plugin_sdk::api::{abi, codec, messages};
use std::{
    collections::{HashMap, VecDeque},
    mem::MaybeUninit,
    sync::Mutex,
};

#[derive(Clone, Debug)]
pub struct Event {
    pub kind: u32,
    pub id: u64,
    pub payload: Vec<u8>,
}
struct Service {
    id: u64,
    session: u64,
    generation: u64,
    read: messages::ReadAt,
}
struct State {
    jobs: VecDeque<abi::Work>,
    services: VecDeque<Service>,
    buffers: HashMap<u64, Box<[u8]>>,
    refs: HashMap<u64, usize>,
    next_buffer: u64,
    pub data: Vec<u8>,
    pub base: u64,
    pub max_read: usize,
    pub reads: Vec<messages::ReadAt>,
}
pub struct Host {
    state: Box<Mutex<State>>,
    api: abi::PluginApi,
    instance: u64,
    next: u64,
}
unsafe fn state<'a>(id: u64) -> &'a Mutex<State> {
    unsafe { &*(id as *const Mutex<State>) }
}
unsafe extern "C" fn schedule(id: u64, work: *const abi::Work) -> u32 {
    unsafe { state(id) }
        .lock()
        .unwrap()
        .jobs
        .push_back(unsafe { *work });
    abi::OK
}
unsafe extern "C" fn cancelled(_: u64, _: u64) -> u32 {
    0
}
unsafe extern "C" fn notify(_: u64) {}
unsafe extern "C" fn diagnostic(_: u64, _: u32, _: abi::ByteSlice) {}
unsafe extern "C" fn retain(id: u64, buffer: u64) -> u32 {
    *unsafe { state(id) }
        .lock()
        .unwrap()
        .refs
        .entry(buffer)
        .or_default() += 1;
    abi::OK
}
unsafe extern "C" fn release(id: u64, buffer: u64) -> u32 {
    let mut state = unsafe { state(id) }.lock().unwrap();
    let count = state.refs.get_mut(&buffer).unwrap();
    *count -= 1;
    if *count == 0 {
        state.buffers.remove(&buffer);
        state.refs.remove(&buffer);
    }
    abi::OK
}
unsafe extern "C" fn service(id: u64, request: *const abi::Request) -> u32 {
    let request = unsafe { &*request };
    if request.kind != abi::SERVICE_READ_AT {
        return abi::UNSUPPORTED;
    }
    let read = codec::decode::<messages::ReadAt>(unsafe { bytes(request.payload) }).unwrap();
    let mut state = unsafe { state(id) }.lock().unwrap();
    state.reads.push(read.clone());
    state.services.push_back(Service {
        id: request.id,
        session: request.session,
        generation: request.generation,
        read,
    });
    abi::OK
}
fn slice(data: &[u8]) -> abi::ByteSlice {
    abi::ByteSlice {
        data: data.as_ptr(),
        len: data.len() as u32,
        reserved: 0,
    }
}
unsafe fn bytes<'a>(data: abi::ByteSlice) -> &'a [u8] {
    if data.len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(data.data, data.len as usize) }
    }
}
impl Host {
    pub fn new(data: &[u8], entry: abi::Entry) -> Self {
        let state = Box::new(Mutex::new(State {
            jobs: VecDeque::new(),
            services: VecDeque::new(),
            buffers: HashMap::new(),
            refs: HashMap::new(),
            next_buffer: 1,
            data: data.to_vec(),
            base: 0,
            max_read: 65536,
            reads: Vec::new(),
        }));
        let host = abi::HostApi {
            header: abi::Header {
                struct_size: size_of::<abi::HostApi>() as u32,
                abi_major: 1,
                abi_minor: 0,
                kind: abi::KIND_HOST,
                minimum_host_minor: 0,
                reserved: 0,
            },
            context: &*state as *const _ as u64,
            schedule_work: Some(schedule),
            cancelled: Some(cancelled),
            submit_service: Some(service),
            notify_ready: Some(notify),
            retain_buffer: Some(retain),
            release_buffer: Some(release),
            diagnostic: Some(diagnostic),
        };
        let mut api = MaybeUninit::uninit();
        assert_eq!(
            unsafe { entry(api.as_mut_ptr(), size_of::<abi::PluginApi>() as u32) },
            abi::OK
        );
        let api: abi::PluginApi = unsafe { api.assume_init() };
        let mut instance = 0;
        assert_eq!(
            unsafe { api.create.unwrap()(&host, slice(&[]), &mut instance) },
            abi::OK
        );
        Self {
            state,
            api,
            instance,
            next: 1,
        }
    }
    pub fn short_reads(&self, maximum: usize) {
        self.state.lock().unwrap().max_read = maximum;
    }
    pub fn base(&self, base: u64) {
        self.state.lock().unwrap().base = base;
    }
    pub fn reads(&self) -> Vec<messages::ReadAt> {
        self.state.lock().unwrap().reads.clone()
    }
    pub fn submit<T: codec::Model>(&mut self, kind: u32, value: &T) -> u64 {
        self.submit_bytes(kind, &codec::encode(value).unwrap())
    }
    pub fn submit_bytes(&mut self, kind: u32, payload: &[u8]) -> u64 {
        let id = self.next;
        self.next += 1;
        let request = abi::Request {
            id,
            session: 7,
            generation: 2,
            kind,
            flags: 0,
            payload: slice(payload),
            content: abi::Buffer {
                id: 0,
                bytes: slice(&[]),
            },
        };
        assert_eq!(
            unsafe { self.api.submit.unwrap()(self.instance, &request) },
            abi::OK
        );
        id
    }
    pub fn cancel(&self, id: u64) {
        assert_eq!(
            unsafe { self.api.cancel.unwrap()(self.instance, id) },
            abi::OK
        );
    }
    pub fn run_one(&mut self) -> bool {
        let work = self.state.lock().unwrap().jobs.pop_front();
        if let Some(work) = work {
            unsafe {
                work.run.unwrap()(work.context);
                work.drop.unwrap()(work.context);
            }
            return true;
        }
        let service = self.state.lock().unwrap().services.pop_front();
        let Some(service) = service else {
            return false;
        };
        let (reply, buffer) = {
            let mut state = self.state.lock().unwrap();
            // The large-offset fixture is a sparse file: bytes before its
            // materialized segment exist and are zero, rather than premature
            // EOF. Real positional reads can include decoder look-behind.
            let end = state.base + state.data.len() as u64;
            let size = end
                .saturating_sub(service.read.offset)
                .min(service.read.length as u64)
                .min(state.max_read as u64) as usize;
            let padding = state
                .base
                .saturating_sub(service.read.offset)
                .min(size as u64) as usize;
            let mut data = vec![0; padding];
            if size > padding {
                let index = service.read.offset.saturating_sub(state.base) as usize;
                data.extend_from_slice(&state.data[index..index + size - padding]);
            }
            let eof = service.read.offset + size as u64 >= end;
            let id = state.next_buffer;
            state.next_buffer += 1;
            state.buffers.insert(id, data.into_boxed_slice());
            state.refs.insert(id, 1);
            let buffer = abi::Buffer {
                id,
                bytes: slice(&state.buffers[&id]),
            };
            (
                messages::ReadReply {
                    revision: service.read.revision,
                    offset: service.read.offset,
                    bytes: size as u32,
                    eof,
                    status: 0,
                },
                buffer,
            )
        };
        let payload = codec::encode(&reply).unwrap();
        let request = abi::Request {
            id: service.id,
            session: service.session,
            generation: service.generation,
            kind: abi::REQUEST_SERVICE_REPLY,
            flags: 0,
            payload: slice(&payload),
            content: buffer,
        };
        let status = unsafe { self.api.submit.unwrap()(self.instance, &request) };
        assert!(
            [abi::OK, abi::UNKNOWN_HANDLE, abi::SHUTTING_DOWN].contains(&status),
            "reply status {status}"
        );
        unsafe {
            release(&*self.state as *const _ as u64, buffer.id);
        }
        true
    }
    fn drain(&self, result: &mut Vec<Event>) {
        loop {
            let mut event = MaybeUninit::<abi::Event>::uninit();
            let mut count = 0;
            assert_eq!(
                unsafe {
                    self.api.drain.unwrap()(self.instance, 1, event.as_mut_ptr(), &mut count)
                },
                abi::OK
            );
            if count == 0 {
                break;
            }
            let event = unsafe { event.assume_init() };
            result.push(Event {
                kind: event.kind,
                id: event.request_id,
                payload: unsafe { bytes(event.payload.bytes) }.to_vec(),
            });
            for buffer in [event.payload, event.content] {
                if buffer.id != 0 {
                    assert_eq!(
                        unsafe { self.api.release_buffer.unwrap()(self.instance, buffer.id) },
                        abi::OK
                    );
                }
            }
        }
    }
    pub fn finish(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        for _ in 0..10000 {
            let worked = self.run_one();
            self.drain(&mut events);
            if !worked && self.state.lock().unwrap().jobs.is_empty() {
                return events;
            }
        }
        panic!("unbounded plugin work");
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        unsafe {
            self.api.begin_shutdown.unwrap()(self.instance);
        }
        self.finish();
        assert!(self.state.lock().unwrap().buffers.is_empty());
        assert_eq!(unsafe { self.api.destroy.unwrap()(self.instance) }, abi::OK);
    }
}

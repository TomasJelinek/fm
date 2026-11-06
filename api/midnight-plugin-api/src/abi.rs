//! C ABI 1.0. Pointers are borrowed only during a call unless explicitly leased.
use std::ffi::c_void;

pub const ABI_MAJOR: u16 = 1;
pub const ABI_MINOR: u16 = 0;
pub const KIND_HOST: u32 = 0;
pub const KIND_VIEWER: u32 = 1;
pub const KIND_FILESYSTEM: u32 = 2;

pub type Status = u32;
pub const OK: Status = 0;
pub const INVALID: Status = 1;
pub const INCOMPATIBLE: Status = 2;
pub const QUEUE_FULL: Status = 3;
pub const UNSUPPORTED: Status = 4;
pub const SHUTTING_DOWN: Status = 5;
pub const NOT_QUIESCENT: Status = 6;
pub const PANICKED: Status = 7;
pub const UNKNOWN_HANDLE: Status = 8;

pub const REQUEST_OPEN: u32 = 1;
pub const REQUEST_VIEWPORT: u32 = 2;
pub const REQUEST_SEARCH: u32 = 3;
pub const REQUEST_CLOSE: u32 = 4;
pub const REQUEST_SERVICE_REPLY: u32 = 5;
pub const EVENT_OPENED: u32 = 1;
pub const EVENT_TEXT_PAGE: u32 = 2;
pub const EVENT_BINARY_PAGE: u32 = 3;
pub const EVENT_DOCUMENT_PAGE: u32 = 4;
pub const EVENT_SEARCH_MATCH: u32 = 5;
pub const EVENT_PROGRESS: u32 = 6;
pub const EVENT_COMPLETED: u32 = 100;
pub const EVENT_CANCELLED: u32 = 101;
pub const EVENT_FAILED: u32 = 102;
pub const EVENT_QUIESCENT: u32 = 103;
pub const SERVICE_READ_AT: u32 = 1001;
pub const SERVICE_CLOSE_READER: u32 = 1002;
pub const WORK_CPU: u32 = 1;
pub const WORK_IO: u32 = 2;

/// This prefix may be read only when the supplied capacity covers all 16 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Header {
    pub struct_size: u32,
    pub abi_major: u16,
    pub abi_minor: u16,
    pub kind: u32,
    pub minimum_host_minor: u16,
    pub reserved: u16,
}

impl Header {
    /// `capacity` is caller-owned readable memory; optional future tails are ignored.
    pub fn validate(&self, capacity: u32, required: u32, kind: u32) -> Result<(), Status> {
        if required < size_of::<Header>() as u32
            || capacity < required
            || self.struct_size < required
        {
            return Err(INVALID);
        }
        if self.abi_major != ABI_MAJOR
            || self.minimum_host_minor > ABI_MINOR
            || self.kind != kind
            || self.reserved != 0
        {
            return Err(INCOMPATIBLE);
        }
        Ok(())
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ByteSlice {
    pub data: *const u8,
    pub len: u32,
    pub reserved: u32,
}

/// The producing instance owns this allocation until `release_buffer(id)` succeeds.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Buffer {
    pub bytes: ByteSlice,
    pub id: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Request {
    pub id: u64,
    pub session: u64,
    pub generation: u64,
    pub kind: u32,
    pub flags: u32,
    pub payload: ByteSlice,
    /// Optional immutable host buffer. Retain through HostApi before async use.
    pub content: Buffer,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Event {
    pub request_id: u64,
    pub session: u64,
    pub generation: u64,
    pub kind: u32,
    pub flags: u32,
    pub payload: Buffer,
    /// Optional raw bytes; released independently of the CBOR payload buffer.
    pub content: Buffer,
}

/// Rejected scheduling retains job ownership in the plugin. Accepted jobs receive
/// exactly one drop, whether run is invoked or cancelled before starting.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Work {
    pub context: *mut c_void,
    pub run: Option<unsafe extern "C" fn(*mut c_void)>,
    pub drop: Option<unsafe extern "C" fn(*mut c_void)>,
    pub request_id: u64,
    pub class: u32,
    pub affinity: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct HostApi {
    pub header: Header,
    pub context: u64,
    pub schedule_work: Option<unsafe extern "C" fn(u64, *const Work) -> Status>,
    pub cancelled: Option<unsafe extern "C" fn(u64, u64) -> u32>,
    pub submit_service: Option<unsafe extern "C" fn(u64, *const Request) -> Status>,
    pub notify_ready: Option<unsafe extern "C" fn(u64)>,
    pub retain_buffer: Option<unsafe extern "C" fn(u64, u64) -> Status>,
    pub release_buffer: Option<unsafe extern "C" fn(u64, u64) -> Status>,
    pub diagnostic: Option<unsafe extern "C" fn(u64, u32, ByteSlice)>,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PluginApi {
    pub header: Header,
    /// Immutable UTF-8 metadata valid for the lifetime of the loaded library.
    pub id: ByteSlice,
    pub version: ByteSlice,
    pub create: Option<unsafe extern "C" fn(*const HostApi, ByteSlice, *mut u64) -> Status>,
    pub submit: Option<unsafe extern "C" fn(u64, *const Request) -> Status>,
    pub cancel: Option<unsafe extern "C" fn(u64, u64) -> Status>,
    pub drain: Option<unsafe extern "C" fn(u64, u32, *mut Event, *mut u32) -> Status>,
    pub release_buffer: Option<unsafe extern "C" fn(u64, u64) -> Status>,
    pub begin_shutdown: Option<unsafe extern "C" fn(u64) -> Status>,
    pub destroy: Option<unsafe extern "C" fn(u64) -> Status>,
}

/// Exported as `midnight_plugin_entry_v1`. Never writes past capacity.
pub type Entry = unsafe extern "C" fn(*mut PluginApi, u32) -> Status;

impl ByteSlice {
    /// This checks metadata, not whether native memory is mapped or trustworthy.
    pub fn validate(&self, maximum: u32) -> Result<(), Status> {
        if self.reserved != 0 || self.len > maximum || (self.len != 0 && self.data.is_null()) {
            Err(INVALID)
        } else {
            Ok(())
        }
    }
}

impl Buffer {
    pub fn validate(&self, maximum: u32) -> Result<(), Status> {
        self.bytes.validate(maximum)?;
        if self.id == 0 && self.bytes.len != 0 {
            Err(INVALID)
        } else {
            Ok(())
        }
    }
}

impl PluginApi {
    pub fn validate(&self, capacity: u32, kind: u32) -> Result<(), Status> {
        self.header
            .validate(capacity, size_of::<Self>() as u32, kind)?;
        self.id.validate(128)?;
        self.version.validate(64)?;
        if self.id.len == 0
            || self.version.len == 0
            || self.create.is_none()
            || self.submit.is_none()
            || self.cancel.is_none()
            || self.drain.is_none()
            || self.release_buffer.is_none()
            || self.begin_shutdown.is_none()
            || self.destroy.is_none()
        {
            Err(INVALID)
        } else {
            Ok(())
        }
    }
}

impl HostApi {
    pub fn validate(&self, capacity: u32) -> Result<(), Status> {
        self.header
            .validate(capacity, size_of::<Self>() as u32, KIND_HOST)?;
        if self.schedule_work.is_none()
            || self.cancelled.is_none()
            || self.submit_service.is_none()
            || self.notify_ready.is_none()
            || self.retain_buffer.is_none()
            || self.release_buffer.is_none()
            || self.diagnostic.is_none()
        {
            Err(INVALID)
        } else {
            Ok(())
        }
    }
}

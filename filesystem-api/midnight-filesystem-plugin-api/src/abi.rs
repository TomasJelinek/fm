//! Synchronous C declarations; pointers are borrowed only during calls unless leased.
pub const ABI_MAJOR: u16 = 1;
pub const ABI_MINOR: u16 = 3;
pub const KIND_FILESYSTEM: u32 = 2;
pub const ENTRY_SYMBOL: &std::ffi::CStr = c"midnight_filesystem_entry_v1";
pub type Status = u32;
pub const OK: Status = 0;
pub const INVALID: Status = 1;
pub const INCOMPATIBLE: Status = 2;
pub const BUSY: Status = 3;
pub const UNSUPPORTED: Status = 4;
pub const RESOURCE_LIMIT: Status = 5;
pub const UNKNOWN_HANDLE: Status = 6;
pub const SHUTTING_DOWN: Status = 7;
pub const PANICKED: Status = 8;
pub const CAP_BROWSE: u64 = 1;
pub const CAP_READ: u64 = 2;
pub const CAP_STAT: u64 = 4;
pub const CAP_VOLUMES: u64 = 8;
pub const CAP_STORAGE: u64 = 16;
pub const CAP_NATIVE_ADDRESS: u64 = 32;
pub const CAP_VOLUME_DETAILS: u64 = 64;
/// Explicit navigation handoff; requires host minor version 2.
pub const CAP_ACTIVATE: u64 = 128;
pub const CAP_REFRESH: u64 = 256;
pub const CAP_RESOLVE_TEXT: u64 = 512;
/// Prepared same-parent Rename; requires host minor version 3.
pub const CAP_PREPARED_RENAME: u64 = 1024;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Slice {
    pub data: *const u8,
    pub len: u64,
}
impl Slice {
    pub fn validate(&self, maximum: u64) -> Result<(), Status> {
        if self.len > maximum || (self.len != 0 && self.data.is_null()) {
            Err(INVALID)
        } else {
            Ok(())
        }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Buffer {
    pub id: u64,
    pub bytes: Slice,
}
impl Buffer {
    pub fn validate(&self, maximum: u64) -> Result<(), Status> {
        self.bytes.validate(maximum)?;
        if self.id == 0 && self.bytes.len != 0 {
            Err(INVALID)
        } else {
            Ok(())
        }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Header {
    pub struct_size: u32,
    pub abi_major: u16,
    pub abi_minor: u16,
    pub kind: u32,
    pub minimum_host_minor: u16,
    pub reserved: u16,
}
impl Header {
    pub fn v1(struct_size: u32) -> Self {
        Self {
            struct_size,
            abi_major: ABI_MAJOR,
            abi_minor: ABI_MINOR,
            kind: KIND_FILESYSTEM,
            minimum_host_minor: 0,
            reserved: 0,
        }
    }
    pub fn validate(&self, capacity: u32, required: u32) -> Result<(), Status> {
        if required < size_of::<Header>() as u32
            || capacity < required
            || self.struct_size < required
        {
            return Err(INVALID);
        }
        if self.abi_major != ABI_MAJOR
            || self.minimum_host_minor > ABI_MINOR
            || self.kind != KIND_FILESYSTEM
            || self.reserved != 0
        {
            Err(INCOMPATIBLE)
        } else {
            Ok(())
        }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Cancellation {
    pub context: u64,
    pub cancelled: Option<unsafe extern "C" fn(u64) -> u32>,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Call {
    pub struct_size: u32,
    pub operation: u32,
    pub payload: Slice,
    pub cancel: Cancellation,
}
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub struct_size: u32,
    pub reserved: u32,
    pub request_bytes: u64,
    pub reply_bytes: u64,
    pub retained_buffer_bytes: u64,
    pub max_buffers: u32,
    pub max_calls: u32,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            reserved: 0,
            request_bytes: crate::DEFAULT_REQUEST_BYTES,
            reply_bytes: crate::DEFAULT_REPLY_BYTES,
            retained_buffer_bytes: 16 * 1024 * 1024,
            max_buffers: 64,
            max_calls: 8,
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<(), Status> {
        if self.struct_size < size_of::<Self>() as u32
            || self.reserved != 0
            || !(1..=crate::DEFAULT_REQUEST_BYTES).contains(&self.request_bytes)
            || !(1..=crate::DEFAULT_REPLY_BYTES).contains(&self.reply_bytes)
            || !(1..=16 * 1024 * 1024).contains(&self.retained_buffer_bytes)
            || self.retained_buffer_bytes < self.reply_bytes
            || !(1..=64).contains(&self.max_buffers)
            || !(1..=8).contains(&self.max_calls)
        {
            Err(INVALID)
        } else {
            Ok(())
        }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Api {
    pub header: Header,
    pub capabilities: u64,
    pub id: Slice,
    pub version: Slice,
    pub create: Option<unsafe extern "C" fn(*const Config, *mut u64) -> Status>,
    pub call: Option<unsafe extern "C" fn(u64, *const Call, *mut Buffer) -> Status>,
    pub release_buffer: Option<unsafe extern "C" fn(u64, u64) -> Status>,
    pub destroy: Option<unsafe extern "C" fn(u64) -> Status>,
}
pub type Entry = unsafe extern "C" fn(*mut Api, u32) -> Status;
impl Api {
    pub fn validate(&self, capacity: u32) -> Result<(), Status> {
        self.header.validate(capacity, size_of::<Self>() as u32)?;
        if self.capabilities & CAP_PREPARED_RENAME != 0
            && (self.header.minimum_host_minor < 3 || self.header.abi_minor < 3)
        {
            return Err(INCOMPATIBLE);
        }
        self.id.validate(128)?;
        self.version.validate(64)?;
        if self.id.len == 0
            || self.version.len == 0
            || self.create.is_none()
            || self.call.is_none()
            || self.release_buffer.is_none()
            || self.destroy.is_none()
        {
            Err(INVALID)
        } else {
            Ok(())
        }
    }
}

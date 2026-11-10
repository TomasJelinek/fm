use super::*;
use std::{
    ffi::c_void,
    mem::{align_of, size_of},
};
use windows_sys::Win32::NetworkManagement::WNet::NETRESOURCEW;
pub(super) const MAX_BUFFER: usize = 1024 * 1024;
pub(super) struct Buffer(Vec<usize>);
const _: () = assert!(align_of::<usize>() >= align_of::<NETRESOURCEW>());
impl Buffer {
    pub fn new(bytes: usize) -> Result<Self, Failure> {
        if bytes == 0 || bytes > MAX_BUFFER {
            return Err(limit());
        }
        let count = bytes.div_ceil(size_of::<usize>());
        let mut words = Vec::new();
        words.try_reserve_exact(count).map_err(|_| limit())?;
        words.resize(count, 0);
        Ok(Self(words))
    }
    pub fn len(&self) -> usize {
        self.0.len() * size_of::<usize>()
    }
    pub fn clear(&mut self) {
        self.0.fill(0);
    }
    pub fn pointer(&mut self) -> *mut c_void {
        self.0.as_mut_ptr().cast()
    }
    pub fn bytes(&self) -> &[u8] {
        // SAFETY: every word is initialized, all byte patterns are valid, the
        // immutable slice stays within this allocation and cannot outlive it.
        unsafe { std::slice::from_raw_parts(self.0.as_ptr().cast(), self.len()) }
    }
    #[cfg(test)]
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        // SAFETY: initialized allocation, exclusive borrow, exact byte extent;
        // writing arbitrary bytes also leaves valid usize values.
        unsafe { std::slice::from_raw_parts_mut(self.0.as_mut_ptr().cast(), self.len()) }
    }
    pub fn parse(&self, count: u32) -> Result<Vec<resource::Resource>, Failure> {
        if count > crate::api::MAX_BATCH {
            return Err(limit());
        }
        let headers = (count as usize)
            .checked_mul(size_of::<NETRESOURCEW>())
            .ok_or_else(invalid)?;
        if count == 0 || headers > self.len() {
            return Err(invalid());
        }
        let mut records = Vec::new();
        records
            .try_reserve_exact(count as usize)
            .map_err(|_| limit())?;
        let mut retained = 0usize;
        for index in 0..count as usize {
            // SAFETY: the allocation has NETRESOURCEW alignment and `headers`
            // proved every complete record lies within initialized owned bytes.
            // Native pointer fields are copied as opaque values, not dereferenced.
            let native = unsafe { self.0.as_ptr().cast::<NETRESOURCEW>().add(index).read() };
            let value = resource::Resource {
                scope: native.dwScope,
                kind: native.dwType,
                display_type: native.dwDisplayType,
                usage: native.dwUsage,
                local: self.text(native.lpLocalName, headers)?,
                remote: self.text(native.lpRemoteName, headers)?,
                comment: self.text(native.lpComment, headers)?,
                provider: self.text(native.lpProvider, headers)?,
            };
            value.encode()?; // Check the complete identity budget, including all strings.
            retained = retained.saturating_add(value.retained_bytes());
            if retained > MAX_BUFFER {
                return Err(limit());
            }
            records.push(value);
        }
        Ok(records)
    }
    fn text(&self, pointer: *const u16, headers: usize) -> Result<Option<Vec<u16>>, Failure> {
        if pointer.is_null() {
            return Ok(None);
        }
        let address = pointer as usize;
        let offset = address
            .checked_sub(self.0.as_ptr() as usize)
            .ok_or_else(invalid)?;
        if !address.is_multiple_of(align_of::<u16>()) || offset < headers || offset >= self.len() {
            return Err(invalid());
        }
        // Read only checked offsets in our own allocation, never the supplied pointer.
        let pairs = self.bytes()[offset..].as_chunks::<2>().0;
        let mut units = Vec::new();
        for pair in pairs {
            let unit = u16::from_ne_bytes([pair[0], pair[1]]);
            if unit == 0 {
                return Ok(Some(units));
            }
            if units.len() >= resource::MAX_KEY_BYTES / 2 {
                return Err(limit());
            }
            units.try_reserve(1).map_err(|_| limit())?;
            units.push(unit);
        }
        Err(invalid())
    }
}

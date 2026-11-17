//! Opaque NLS collation keys prepared on backend workers.

use midnight_core::MAX_COLLATION_KEY_BYTES;
use std::{ffi::OsStr, io, os::windows::ffi::OsStrExt, ptr};
use windows_sys::Win32::{
    Foundation::ERROR_INSUFFICIENT_BUFFER,
    Globalization::{LCMAP_SORTKEY, LCMapStringEx},
};

pub(super) fn key(locale: &[u16], value: &OsStr) -> io::Result<Option<Vec<u8>>> {
    if locale.last() != Some(&0) || locale[..locale.len() - 1].contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "The captured Windows locale must contain one trailing terminator.",
        ));
    }
    let mut source: Vec<_> = value.encode_wide().collect();
    if source.contains(&0) || char::decode_utf16(source.iter().copied()).any(|unit| unit.is_err()) {
        return Ok(None);
    }
    // NLS rejects a zero source count. Including the terminator also represents
    // empty extensions without changing the native string's collation weights.
    source.push(0);
    let source_len = i32::try_from(source.len())
        .map_err(|_| io::Error::other("The native collation input exceeds the API count range."))?;
    let map = |output: *mut u16, bytes: i32| {
        // SAFETY: the captured locale and terminated source remain live; output
        // is null for sizing or an aligned buffer of at least `bytes` writable bytes.
        // LCMAP_SORTKEY changes the destination count unit from wchar to bytes.
        unsafe {
            LCMapStringEx(
                locale.as_ptr(),
                LCMAP_SORTKEY,
                source.as_ptr(),
                source_len,
                output,
                bytes,
                ptr::null(),
                ptr::null(),
                0,
            )
        }
    };
    sort_key(source.len(), map).map(Some)
}

/// Units of the stack buffer that answers ordinary names without a heap allocation.
const STACK_KEY_UNITS: usize = 512;

/// Collects one sort key from `map` (LCMapStringEx with LCMAP_SORTKEY: a destination
/// byte count in, the written byte count out, zero and the thread error on failure).
///
/// A key normally needs at most about four bytes per source unit plus a small fixed
/// overhead, so the first call fills a buffer of that size (on the stack for ordinary
/// names) and answers almost every name with one native call. Only
/// ERROR_INSUFFICIENT_BUFFER leads to a size probe and an exactly sized second fill.
fn sort_key(source_units: usize, mut map: impl FnMut(*mut u16, i32) -> i32) -> io::Result<Vec<u8>> {
    let guess = source_units
        .saturating_mul(4)
        .saturating_add(64)
        .min(MAX_COLLATION_KEY_BYTES)
        .div_ceil(2);
    let mut stack = [0u16; STACK_KEY_UNITS];
    let mut heap = Vec::new();
    let first: &mut [u16] = if guess <= STACK_KEY_UNITS {
        &mut stack[..guess]
    } else {
        heap.try_reserve_exact(guess).map_err(io::Error::other)?;
        heap.resize(guess, 0);
        &mut heap
    };
    let capacity = native_bytes(first.len())?;
    let written = map(first.as_mut_ptr(), capacity);
    if written > 0 {
        return key_bytes(first, written, capacity);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32) {
        return Err(error);
    }
    let required = map(ptr::null_mut(), 0);
    if required <= 0 {
        return Err(io::Error::last_os_error());
    }
    let byte_count = required as usize;
    if byte_count > MAX_COLLATION_KEY_BYTES {
        return Err(io::Error::other(
            "The native collation key exceeds its 4 MiB resource budget.",
        ));
    }
    // The API exposes a wchar pointer even for byte keys. Preserve its alignment
    // while retaining every returned byte, including internal zeros/odd lengths.
    let units = byte_count.div_ceil(2);
    let mut output = Vec::<u16>::new();
    output.try_reserve_exact(units).map_err(io::Error::other)?;
    output.resize(units, 0);
    let written = map(output.as_mut_ptr(), required);
    if written <= 0 {
        return Err(io::Error::last_os_error());
    }
    key_bytes(&output, written, required)
}

fn native_bytes(units: usize) -> io::Result<i32> {
    units
        .checked_mul(2)
        .and_then(|bytes| i32::try_from(bytes).ok())
        .ok_or_else(|| io::Error::other("The native collation buffer exceeds the API range."))
}

/// The first `written` bytes of `output`, which held `capacity` writable bytes.
fn key_bytes(output: &[u16], written: i32, capacity: i32) -> io::Result<Vec<u8>> {
    if written > capacity {
        return Err(io::Error::other(
            "The native collation key changed size during capture.",
        ));
    }
    let mut key = Vec::new();
    key.try_reserve_exact(written as usize)
        .map_err(io::Error::other)?;
    key.extend(
        output
            .iter()
            .flat_map(|unit| unit.to_ne_bytes())
            .take(written as usize),
    );
    Ok(key)
}

#[cfg(test)]
#[path = "sort_keys_tests.rs"]
mod tests;

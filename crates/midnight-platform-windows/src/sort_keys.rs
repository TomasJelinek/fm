//! Opaque NLS collation keys prepared on backend workers.

use midnight_core::MAX_COLLATION_KEY_BYTES;
use std::{ffi::OsStr, io, os::windows::ffi::OsStrExt, ptr};
use windows_sys::Win32::Globalization::{LCMAP_SORTKEY, LCMapStringEx};

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
        // is null for sizing or an aligned allocation of at least `bytes` bytes.
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
    if written > required {
        return Err(io::Error::other(
            "The native collation key changed size during capture.",
        ));
    }
    let mut key = Vec::new();
    key.try_reserve_exact(written as usize)
        .map_err(io::Error::other)?;
    key.extend(
        output
            .into_iter()
            .flat_map(u16::to_ne_bytes)
            .take(written as usize),
    );
    Ok(Some(key))
}

#[cfg(test)]
#[path = "sort_keys_tests.rs"]
mod tests;

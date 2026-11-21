//! Owned POSIX collation objects and order-preserving keys of their wide-character
//! weights, in a variable-length code that compares byte by byte in weight order.

use midnight_core::{FsError, MAX_COLLATION_KEY_BYTES};
use std::{
    ffi::{CStr, OsStr},
    io,
    path::Path,
    ptr,
};

// libc's Rust bindings do not currently expose this POSIX function. The ABI is
// declared by GNU/Linux wchar.h: size_t, wchar_t pointers, then locale_t.
unsafe extern "C" {
    fn wcsxfrm_l(
        output: *mut libc::wchar_t,
        source: *const libc::wchar_t,
        capacity: usize,
        locale: libc::locale_t,
    ) -> usize;
}

struct Collator(libc::locale_t);

impl Collator {
    fn new(name: &CStr) -> io::Result<Self> {
        // SAFETY: a terminated name and null base create a separately owned
        // object. No global locale or thread locale is installed or modified.
        let locale =
            unsafe { libc::newlocale(libc::LC_COLLATE_MASK, name.as_ptr(), ptr::null_mut()) };
        if locale.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self(locale))
        }
    }

    fn with_fallback(name: &CStr) -> io::Result<Self> {
        match Self::new(name) {
            Err(error) if error.raw_os_error() == Some(libc::ENOENT) => Self::new(c"C"),
            result => result,
        }
    }

    fn key(&self, value: &OsStr) -> io::Result<Option<Vec<u8>>> {
        // A few weights per character and level, plus level separators.
        self.key_estimated(value, |units| units.saturating_mul(6).saturating_add(16))
    }

    fn key_estimated(
        &self,
        value: &OsStr,
        estimate: fn(usize) -> usize,
    ) -> io::Result<Option<Vec<u8>>> {
        let Some(text) = value.to_str() else {
            return Ok(None);
        };
        if text.contains('\0') {
            return Ok(None);
        }
        let count = text
            .chars()
            .count()
            .checked_add(1)
            .ok_or_else(|| io::Error::other("The native collation input size overflowed."))?;
        let mut source = Vec::new();
        source.try_reserve_exact(count).map_err(io::Error::other)?;
        source.extend(text.chars().map(|value| value as libc::wchar_t));
        source.push(0);
        // glibc computes all weights even for a sizing-only call, so transform into
        // an estimated buffer and repeat only when the key turns out to be longer.
        // The wchar terminator is part of the key: on signed-wchar targets a
        // negative next weight must sort before a terminating zero.
        let budget = MAX_COLLATION_KEY_BYTES / 4;
        let mut units = estimate(count).clamp(1, budget);
        let mut output = Vec::new();
        let written = loop {
            output.clear();
            output.try_reserve_exact(units).map_err(io::Error::other)?;
            output.resize(units, 0);
            let required = self.transform(&source, &mut output)?;
            if required < units {
                break required;
            }
            let needed = required
                .checked_add(1)
                .filter(|needed| *needed <= budget)
                .ok_or_else(|| {
                    io::Error::other("The native collation key exceeds its 4 MiB resource budget.")
                })?;
            if needed <= units {
                return Err(io::Error::other(
                    "The native collation key changed size during capture.",
                ));
            }
            units = needed;
        };
        if output[written] != 0 {
            return Err(io::Error::other(
                "The native collation key changed size during capture.",
            ));
        }
        encode(&output[..=written]).map(Some)
    }

    fn transform(
        &self,
        source: &[libc::wchar_t],
        output: &mut [libc::wchar_t],
    ) -> io::Result<usize> {
        if source.last() != Some(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "The native collation input must contain its terminator.",
            ));
        }
        // SAFETY: source is terminated, output capacity matches its live slice,
        // and self uniquely owns this immutable locale. A zero capacity only
        // queries length. errno is per-thread and inspected immediately.
        let (required, problem) = unsafe {
            *libc::__errno_location() = 0;
            let required = wcsxfrm_l(output.as_mut_ptr(), source.as_ptr(), output.len(), self.0);
            (required, *libc::__errno_location())
        };
        if problem == 0 {
            Ok(required)
        } else {
            Err(io::Error::from_raw_os_error(problem))
        }
    }
}

impl Drop for Collator {
    fn drop(&mut self) {
        // SAFETY: only this object owns the locale, and no active borrow remains.
        unsafe { libc::freelocale(self.0) };
    }
}

// Collation weights are small: glibc's largest is about 66,000 (most of a Latin name's
// are below 30,847) and C locales use code points, so most weights take one or two
// bytes instead of four (a 16-character name's en_US key: 312 → 108 bytes). A weight's
// code is a lead byte from its class's range and its offset within the class,
// big-endian. Lead ranges rise with the classes and each lead decides its code's length,
// so comparing keys byte by byte compares their weights one by one, exactly in `wchar_t`
// order, and no weight's code is a prefix of another's. Linux wchar_t is signed i32 on
// x86 and unsigned u32 on AArch64: a negative weight takes lead 0x00, below every other
// code (the terminating zero's included). 0xFF is unused.

/// The end of the one-byte class: leads 0x01..=0x7F alone.
const ONE_BYTE_END: i64 = 0x7f;
/// The end of the two-byte class: leads 0x80..=0xF7 and one byte.
const TWO_BYTES_END: i64 = ONE_BYTE_END + (0x78 << 8);
/// The end of the three-byte class: leads 0xF8..=0xFC and two bytes.
const THREE_BYTES_END: i64 = TWO_BYTES_END + (0x05 << 16);
/// The end of the four-byte class: lead 0xFD and three bytes. Lead 0xFE and four
/// bytes take the rest, up to u32::MAX.
const FOUR_BYTES_END: i64 = THREE_BYTES_END + (1 << 24);

/// The code of `weight` as a big-endian number, and its length in bytes. A class's
/// offsets stay below the next class's leads (the class ends above).
fn weight_code(weight: libc::wchar_t) -> (u64, usize) {
    let value = i64::from(weight);
    if value < 0 {
        // Lead 0x00; two's complement orders negative i32 values like their u32 bits.
        (u64::from(u32::from_ne_bytes(weight.to_ne_bytes())), 5)
    } else if value < ONE_BYTE_END {
        (0x01 + value as u64, 1)
    } else if value < TWO_BYTES_END {
        (0x8000 + (value - ONE_BYTE_END) as u64, 2)
    } else if value < THREE_BYTES_END {
        (0xf8_0000 + (value - TWO_BYTES_END) as u64, 3)
    } else if value < FOUR_BYTES_END {
        (0xfd00_0000 + (value - THREE_BYTES_END) as u64, 4)
    } else {
        (0xfe_0000_0000 + (value - FOUR_BYTES_END) as u64, 5)
    }
}

/// The key of `weights`, allocated exactly and within the key budget.
fn encode(weights: &[libc::wchar_t]) -> io::Result<Vec<u8>> {
    let mut length = 0;
    for weight in weights {
        length += weight_code(*weight).1;
    }
    if length > MAX_COLLATION_KEY_BYTES {
        return Err(io::Error::other(
            "The native collation key exceeds its 4 MiB resource budget.",
        ));
    }
    let mut key = Vec::new();
    key.try_reserve_exact(length).map_err(io::Error::other)?;
    for weight in weights {
        let (code, size) = weight_code(*weight);
        key.extend_from_slice(&code.to_be_bytes()[8 - size..]);
    }
    Ok(key)
}

fn error(error: io::Error) -> FsError {
    FsError::from_io("prepare Linux collation key", Path::new(""), error)
}

thread_local! {
    static CURRENT: Result<Collator, FsError> = crate::locale::collation_name()
        .and_then(|name| Collator::with_fallback(name).map_err(error));
}

pub(super) fn key(value: &OsStr) -> Result<Option<Vec<u8>>, FsError> {
    CURRENT.with(|locale| {
        locale
            .as_ref()
            .map_err(Clone::clone)?
            .key(value)
            .map_err(error)
    })
}

#[cfg(test)]
#[path = "sort_keys_tests.rs"]
mod tests;

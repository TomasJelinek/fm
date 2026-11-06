//! Owned POSIX collation objects and order-preserving wide-character keys.

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
        let required = self.transform(&source, &mut [])?;
        // Include the wchar terminator in byte comparison: on signed-wchar
        // targets a negative next weight must sort before a terminating zero.
        let units = required
            .checked_add(1)
            .ok_or_else(|| io::Error::other("The native collation key size overflowed."))?;
        let bytes = units
            .checked_mul(4)
            .filter(|bytes| *bytes <= MAX_COLLATION_KEY_BYTES)
            .ok_or_else(|| {
                io::Error::other("The native collation key exceeds its 4 MiB resource budget.")
            })?;
        let mut output = Vec::new();
        output.try_reserve_exact(units).map_err(io::Error::other)?;
        output.resize(units, 0);
        let written = self.transform(&source, &mut output)?;
        if written >= output.len() || output[written] != 0 {
            return Err(io::Error::other(
                "The native collation key changed size during capture.",
            ));
        }
        let mut key = Vec::new();
        key.try_reserve_exact(bytes).map_err(io::Error::other)?;
        key.extend(output[..=written].iter().copied().flat_map(weight_bytes));
        Ok(Some(key))
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

fn weight_bytes(weight: libc::wchar_t) -> [u8; 4] {
    // Linux wchar_t is signed i32 on x86 and unsigned u32 on AArch64. Bias
    // signed representations by their sign bit before lexicographic encoding.
    (u32::from_ne_bytes(weight.to_ne_bytes())
        ^ u32::from_ne_bytes(libc::wchar_t::MIN.to_ne_bytes()))
    .to_be_bytes()
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

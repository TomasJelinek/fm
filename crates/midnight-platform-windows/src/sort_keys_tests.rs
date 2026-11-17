use super::*;
use crate::WindowsFileSystem;
use midnight_core::{LocaleService, MAX_COLLATION_KEY_BYTES};
use std::{
    ffi::{OsStr, OsString},
    os::windows::ffi::{OsStrExt, OsStringExt},
    ptr,
};
use windows_sys::Win32::{
    Foundation::SetLastError,
    Globalization::{CSTR_EQUAL, CompareStringEx},
};

fn czech() -> Vec<u16> {
    "cs-CZ".encode_utf16().chain(Some(0)).collect()
}

fn native_compare(locale: &[u16], left: &str, right: &str) -> std::cmp::Ordering {
    let left: Vec<u16> = left.encode_utf16().chain(Some(0)).collect();
    let right: Vec<u16> = right.encode_utf16().chain(Some(0)).collect();
    // SAFETY: terminated locale/text buffers remain live, and optional native parameters are null.
    let compared = unsafe {
        CompareStringEx(
            locale.as_ptr(),
            0,
            left.as_ptr(),
            -1,
            right.as_ptr(),
            -1,
            ptr::null(),
            ptr::null(),
            0,
        )
    };
    assert_ne!(compared, 0);
    compared.cmp(&CSTR_EQUAL)
}

#[test]
fn czech_keys_match_native_case_accent_and_contraction_order() {
    let locale = czech();
    let samples = [
        "", "a", "A", "á", "ä", "b", "c", "č", "d", "h", "ch", "Ch", "i", "z", "ž",
    ];
    let keys: Vec<_> = samples
        .iter()
        .map(|value| key(&locale, OsStr::new(value)).unwrap().unwrap())
        .collect();
    for (i, left) in samples.iter().enumerate() {
        for (j, right) in samples.iter().enumerate() {
            assert_eq!(
                keys[i].cmp(&keys[j]),
                native_compare(&locale, left, right),
                "{left:?} / {right:?}"
            );
        }
    }
    assert!(key(&locale, OsStr::new("h")).unwrap() < key(&locale, OsStr::new("ch")).unwrap());
    assert!(key(&locale, OsStr::new("ch")).unwrap() < key(&locale, OsStr::new("i")).unwrap());
    assert_ne!(
        key(&locale, OsStr::new("A")).unwrap(),
        key(&locale, OsStr::new("a")).unwrap()
    );
}

#[test]
fn native_sort_keys_preserve_invalid_utf16_for_raw_fallback_and_handle_long_names() {
    let locale = czech();
    let raw = OsString::from_wide(&[b'a' as u16, 0xd800, b'b' as u16]);
    assert!(key(&locale, &raw).unwrap().is_none());
    assert_eq!(
        raw.encode_wide().collect::<Vec<_>>(),
        [b'a' as u16, 0xd800, b'b' as u16]
    );
    assert!(key(&locale, OsStr::new("a\0b")).unwrap().is_none());
    let left = format!("{}ch", "á".repeat(4096));
    let right = format!("{}i", "á".repeat(4096));
    let left_key = key(&locale, OsStr::new(&left)).unwrap().unwrap();
    let right_key = key(&locale, OsStr::new(&right)).unwrap().unwrap();
    assert!(left_key.len() > 4096);
    assert_eq!(
        left_key.cmp(&right_key),
        native_compare(&locale, &left, &right)
    );
}

#[test]
fn captured_user_locale_exposes_native_keys_through_the_backend_trait() {
    assert!(
        WindowsFileSystem::new(None)
            .collation_key(OsStr::new("name.txt"))
            .unwrap()
            .is_some()
    );
}

#[test]
fn sort_key_allocation_budget_rejects_oversized_native_output() {
    assert!(key(&czech(), OsStr::new(&"a".repeat(MAX_COLLATION_KEY_BYTES))).is_err());
}

/// A fake `LCMapStringEx(LCMAP_SORTKEY)` producing a key of `length` bytes. It fails
/// with ERROR_INSUFFICIENT_BUFFER when the destination is smaller and records each
/// call's destination byte count (0 = size probe).
fn fake_mapper(length: usize, calls: &mut Vec<i32>) -> impl FnMut(*mut u16, i32) -> i32 + '_ {
    move |output, bytes| {
        calls.push(bytes);
        if output.is_null() {
            return length as i32;
        }
        if (bytes as usize) < length {
            // SAFETY: sets only this thread's last-error value.
            unsafe { SetLastError(ERROR_INSUFFICIENT_BUFFER) };
            return 0;
        }
        // SAFETY: sort_key passes a buffer of at least `bytes` >= `length` writable bytes.
        let target = unsafe { std::slice::from_raw_parts_mut(output.cast::<u8>(), length) };
        for (index, byte) in target.iter_mut().enumerate() {
            *byte = index as u8 ^ 0x5a;
        }
        length as i32
    }
}

fn fake_key(length: usize) -> Vec<u8> {
    (0..length).map(|index| index as u8 ^ 0x5a).collect()
}

/// Before: every key cost a size probe plus a fill (two NLS calls, three allocations).
#[test]
fn ordinary_names_take_one_native_call_and_only_short_buffers_are_size_probed() {
    // A 12-unit name: one call into the stack buffer of 4 * 12 + 64 bytes.
    let mut calls = Vec::new();
    assert_eq!(
        sort_key(12, fake_mapper(31, &mut calls)).unwrap(),
        fake_key(31)
    );
    assert_eq!(calls, [112]);
    // A long name: one call into a heap buffer of the same estimate.
    let mut calls = Vec::new();
    assert_eq!(
        sort_key(1_000, fake_mapper(3_001, &mut calls)).unwrap(),
        fake_key(3_001)
    );
    assert_eq!(calls, [4_064]);
    // A key beyond the estimate: the failed fill, one size probe, one exact fill.
    let mut calls = Vec::new();
    assert_eq!(
        sort_key(12, fake_mapper(2_000, &mut calls)).unwrap(),
        fake_key(2_000)
    );
    assert_eq!(calls, [112, 0, 2_000]);
    // Another failure is reported at once; an oversized key is refused unallocated.
    let mut attempts = 0;
    let error = sort_key(12, |_, _| {
        attempts += 1;
        // SAFETY: sets only this thread's last-error value.
        unsafe { SetLastError(87) };
        0
    })
    .unwrap_err();
    assert_eq!((error.raw_os_error(), attempts), (Some(87), 1));
    let mut calls = Vec::new();
    assert!(sort_key(12, fake_mapper(MAX_COLLATION_KEY_BYTES + 1, &mut calls)).is_err());
    assert_eq!(calls, [112, 0]);
}

#[test]
fn invalid_locale_errors_instead_of_claiming_a_native_key() {
    let locale: Vec<_> = "midnight-invalid-locale"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    assert!(key(&locale, OsStr::new("name")).is_err());
}

#[test]
fn locale_slice_must_contain_its_terminator_before_the_native_call() {
    let locale = czech();
    // Keep a valid backing allocation so the old implementation fails the
    // assertion without asking NLS to read uninitialized or unmapped storage.
    let unterminated = &locale[..locale.len() - 1];
    assert!(key(unterminated, OsStr::new("name")).is_err());
    assert!(key(&[], OsStr::new("name")).is_err());
}

use super::*;
use crate::LinuxFileSystem;
use midnight_core::{LocaleService, MAX_COLLATION_KEY_BYTES};
use std::{
    ffi::{OsStr, OsString},
    os::unix::ffi::OsStringExt,
};

unsafe extern "C" {
    fn wcscoll_l(
        left: *const libc::wchar_t,
        right: *const libc::wchar_t,
        locale: libc::locale_t,
    ) -> libc::c_int;
}

fn native_compare(locale: &Collator, left: &str, right: &str) -> std::cmp::Ordering {
    let left: Vec<_> = left
        .chars()
        .map(|value| value as libc::wchar_t)
        .chain(Some(0))
        .collect();
    let right: Vec<_> = right
        .chars()
        .map(|value| value as libc::wchar_t)
        .chain(Some(0))
        .collect();
    // SAFETY: terminated Unicode wchar buffers and the owned locale remain live for this call.
    unsafe { wcscoll_l(left.as_ptr(), right.as_ptr(), locale.0) }.cmp(&0)
}

#[test]
#[ignore = "requires cs_CZ.UTF-8 installed or an isolated test LOCPATH"]
fn czech_keys_match_native_case_accent_and_contraction_order() {
    let locale = Collator::new(c"cs_CZ.UTF-8").unwrap();
    let samples = [
        "", "a", "A", "á", "ä", "b", "c", "č", "d", "h", "ch", "Ch", "i", "z", "ž",
    ];
    let keys: Vec<_> = samples
        .iter()
        .map(|value| locale.key(OsStr::new(value)).unwrap().unwrap())
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
    assert!(locale.key(OsStr::new("h")).unwrap() < locale.key(OsStr::new("ch")).unwrap());
    assert!(locale.key(OsStr::new("ch")).unwrap() < locale.key(OsStr::new("i")).unwrap());
    assert_ne!(
        locale.key(OsStr::new("A")).unwrap(),
        locale.key(OsStr::new("a")).unwrap()
    );
}

#[test]
fn raw_unix_names_fall_back_without_replacement_and_long_keys_use_dynamic_storage() {
    let locale = Collator::new(c"C").unwrap();
    let raw = OsString::from_vec(vec![b'a', 0xff, b'b']);
    assert!(locale.key(&raw).unwrap().is_none());
    assert_eq!(raw.into_vec(), [b'a', 0xff, b'b']);
    assert!(locale.key(OsStr::new("a\0b")).unwrap().is_none());
    let left = format!("{}a", "文".repeat(4096));
    let right = format!("{}b", "文".repeat(4096));
    let left_key = locale.key(OsStr::new(&left)).unwrap().unwrap();
    let right_key = locale.key(OsStr::new(&right)).unwrap().unwrap();
    assert!(left_key.len() > 4096);
    assert_eq!(
        left_key.cmp(&right_key),
        native_compare(&locale, &left, &right)
    );
}

#[test]
fn missing_configured_locale_uses_posix_fallback() {
    assert!(Collator::new(c"midnight_missing_test_locale").is_err());
    let fallback = Collator::with_fallback(c"midnight_missing_test_locale").unwrap();
    let posix = Collator::new(c"C").unwrap();
    for value in ["a", "A", "ä", "ch", "文"] {
        assert_eq!(
            fallback.key(OsStr::new(value)).unwrap(),
            posix.key(OsStr::new(value)).unwrap()
        );
    }
}

#[test]
fn wide_weight_byte_order_matches_native_wchar_order() {
    let weights = [libc::wchar_t::MIN, 0, 1, libc::wchar_t::MAX];
    for left in weights {
        for right in weights {
            assert_eq!(
                weight_bytes(left).cmp(&weight_bytes(right)),
                left.cmp(&right)
            );
        }
    }
}

#[test]
fn captured_locale_exposes_native_keys_through_the_backend_trait() {
    assert!(
        LinuxFileSystem::new(None)
            .collation_key(OsStr::new("name.txt"))
            .unwrap()
            .is_some()
    );
}

#[test]
fn sort_key_allocation_budget_rejects_oversized_native_output() {
    let locale = Collator::new(c"C").unwrap();
    assert!(
        locale
            .key(OsStr::new(&"a".repeat(MAX_COLLATION_KEY_BYTES / 4 + 1)))
            .is_err()
    );
}

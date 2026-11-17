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

fn exact_two_pass_key(locale: &Collator, value: &str) -> Vec<u8> {
    let source: Vec<_> = value
        .chars()
        .map(|value| value as libc::wchar_t)
        .chain(Some(0))
        .collect();
    let mut output = vec![0; locale.transform(&source, &mut []).unwrap() + 1];
    let written = locale.transform(&source, &mut output).unwrap();
    output[..=written]
        .iter()
        .copied()
        .flat_map(weight_bytes)
        .collect()
}

#[test]
fn single_pass_keys_equal_exactly_sized_transforms_including_the_retry_path() {
    let mut locales = vec![Collator::new(c"C").unwrap()];
    // Real collation rules where available; otherwise C still covers both paths.
    for name in [c"cs_CZ.UTF-8", c"en_US.UTF-8", c"C.UTF-8"] {
        if let Ok(locale) = Collator::new(name) {
            locales.push(locale);
        }
    }
    let long = format!("{}a", "文".repeat(4096));
    let samples = [
        "",
        "a",
        "Ch",
        "ch",
        "Výroční zpráva – finální.pdf",
        "Straße",
        "æble",
        "e\u{301}",
        "👩🏽‍💻 notes",
        "日本語のファイル",
        "עברית",
        &long,
    ];
    for locale in &locales {
        for value in samples {
            let expected = exact_two_pass_key(locale, value);
            assert_eq!(locale.key(OsStr::new(value)).unwrap().unwrap(), expected);
            let retried = locale.key_estimated(OsStr::new(value), |_| 1).unwrap();
            assert_eq!(retried.unwrap(), expected, "{value:?}");
        }
    }
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

/// Run with `MIDNIGHT_COLLATION_TIMING_LOCALE=cs_CZ.UTF-8 cargo test -p
/// midnight-platform-linux --lib collation_key_timing -- --ignored --nocapture`;
/// a missing locale falls back to C.
#[test]
#[ignore = "timing harness for collation keys; prints measurements"]
fn collation_key_timing() {
    let name =
        std::env::var("MIDNIGHT_COLLATION_TIMING_LOCALE").unwrap_or_else(|_| "en_US.UTF-8".into());
    let locale = Collator::with_fallback(&std::ffi::CString::new(name.clone()).unwrap()).unwrap();
    let names: Vec<String> = (0..100_000)
        .map(|index| format!("Výroční zpráva {index:06} – finální verze.pdf"))
        .collect();
    // Interleave the single-pass keys with the former sizing-plus-transform work
    // in one process so machine load affects both alike; report the best round.
    let mut single = std::time::Duration::MAX;
    let mut two_pass = std::time::Duration::MAX;
    for _ in 0..5 {
        let start = std::time::Instant::now();
        for value in &names {
            std::hint::black_box(locale.key(OsStr::new(value)).unwrap());
        }
        single = single.min(start.elapsed());
        let start = std::time::Instant::now();
        for value in &names {
            std::hint::black_box(exact_two_pass_key(&locale, value));
        }
        two_pass = two_pass.min(start.elapsed());
    }
    let count = names.len() as u32;
    eprintln!(
        "TIMING {count} collation keys ({name}; C if missing): single pass {single:?} \
         ({:?}/key), former two passes {two_pass:?} ({:?}/key)",
        single / count,
        two_pass / count
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

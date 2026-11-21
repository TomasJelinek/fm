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

/// The native weights of `value`, its terminating zero included, from an exactly sized
/// transform.
fn two_pass_weights(locale: &Collator, value: &str) -> Vec<libc::wchar_t> {
    let source: Vec<_> = value
        .chars()
        .map(|value| value as libc::wchar_t)
        .chain(Some(0))
        .collect();
    let mut output = vec![0; locale.transform(&source, &mut []).unwrap() + 1];
    let written = locale.transform(&source, &mut output).unwrap();
    output.truncate(written + 1);
    output
}

fn exact_two_pass_key(locale: &Collator, value: &str) -> Vec<u8> {
    encode(&two_pass_weights(locale, value)).unwrap()
}

/// The former fixed-width key: four big-endian bytes per weight, biased by the sign bit
/// of a signed wchar_t. The variable-length keys must order every key exactly as it did.
fn fixed_width_key(weights: &[libc::wchar_t]) -> Vec<u8> {
    let mut key = Vec::with_capacity(weights.len() * 4);
    key.extend(weights.iter().flat_map(|weight| {
        (u32::from_ne_bytes(weight.to_ne_bytes())
            ^ u32::from_ne_bytes(libc::wchar_t::MIN.to_ne_bytes()))
        .to_be_bytes()
    }));
    key
}

fn code(weight: libc::wchar_t) -> Vec<u8> {
    encode(&[weight]).unwrap()
}

/// The extremes of wchar_t and u32, small weights and every class boundary.
fn boundary_weights() -> Vec<libc::wchar_t> {
    let mut values = vec![
        i64::from(libc::wchar_t::MIN),
        -65_536,
        -1,
        i64::from(i32::MAX),
        i64::from(u32::MAX),
        i64::from(libc::wchar_t::MAX),
    ];
    for start in [
        0,
        ONE_BYTE_END,
        TWO_BYTES_END,
        THREE_BYTES_END,
        FOUR_BYTES_END,
    ] {
        values.extend([-1, 0, 1, 2, 3, 255, 256, 65_535, 65_536].map(|delta| start + delta));
    }
    let mut weights: Vec<_> = values
        .into_iter()
        .filter_map(|value| libc::wchar_t::try_from(value).ok())
        .collect();
    weights.sort_unstable();
    weights.dedup();
    weights
}

#[test]
fn weight_codes_keep_wchar_order_and_no_code_is_a_prefix_of_another() {
    let weights = boundary_weights();
    for left in &weights {
        let left_code = code(*left);
        for right in &weights {
            let right_code = code(*right);
            assert_eq!(
                left_code.cmp(&right_code),
                left.cmp(right),
                "{left} / {right}"
            );
            if left != right {
                assert!(!right_code.starts_with(&left_code), "{left} / {right}");
            }
        }
    }
    // Each class from its first to its last weight, and the leads that follow.
    for (weight, expected) in [
        (0, &[0x01][..]),
        (126, &[0x7f]),
        (127, &[0x80, 0x00]),
        (30_846, &[0xf7, 0xff]),
        (30_847, &[0xf8, 0x00, 0x00]),
        (358_526, &[0xfc, 0xff, 0xff]),
        (358_527, &[0xfd, 0x00, 0x00, 0x00]),
        (17_135_742, &[0xfd, 0xff, 0xff, 0xff]),
        (17_135_743, &[0xfe, 0x00, 0x00, 0x00, 0x00]),
    ] {
        assert_eq!(code(weight), expected, "{weight}");
    }
    // Only a signed wchar_t is negative: lead 0x00 and its two's complement.
    if let Ok(weight) = libc::wchar_t::try_from(-1i64) {
        assert_eq!(code(weight), [0x00, 0xff, 0xff, 0xff, 0xff]);
        assert_eq!(code(libc::wchar_t::MIN), [0x00, 0x80, 0x00, 0x00, 0x00]);
    }
    // The largest u32 weight is the last five-byte code.
    if let Ok(weight) = libc::wchar_t::try_from(i64::from(u32::MAX)) {
        assert_eq!(code(weight), [0xfe, 0xfe, 0xfa, 0x87, 0x80]);
    }
}

#[test]
fn variable_keys_order_random_weight_sequences_exactly_as_fixed_width_keys() {
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let boundaries = boundary_weights();
    let sequences: Vec<Vec<libc::wchar_t>> = (0..400)
        .map(|_| {
            let length = next() % 8;
            (0..length)
                .map(|_| match next() % 4 {
                    0 => boundaries[(next() % boundaries.len() as u64) as usize],
                    // Separator-like weights make long shared prefixes common.
                    1 => (next() % 4) as libc::wchar_t,
                    2 => (next() % 70_000) as libc::wchar_t,
                    _ => libc::wchar_t::from_ne_bytes((next() as u32).to_ne_bytes()),
                })
                .collect()
        })
        .collect();
    let fixed: Vec<_> = sequences
        .iter()
        .map(|weights| fixed_width_key(weights))
        .collect();
    let variable: Vec<_> = sequences
        .iter()
        .map(|weights| encode(weights).unwrap())
        .collect();
    for (left, sequence) in sequences.iter().enumerate() {
        assert_eq!(variable[left].capacity(), variable[left].len());
        for right in 0..sequences.len() {
            assert_eq!(
                variable[left].cmp(&variable[right]),
                fixed[left].cmp(&fixed[right]),
                "{sequence:?} / {:?}",
                sequences[right]
            );
        }
    }
}

#[test]
fn real_locale_keys_keep_their_fixed_width_order_and_shrink() {
    let mut locales = vec![("C", Collator::new(c"C").unwrap())];
    // Real collation rules where available (installed or through LOCPATH).
    for (label, name) in [
        ("C.UTF-8", c"C.UTF-8"),
        ("en_US.UTF-8", c"en_US.UTF-8"),
        ("cs_CZ.UTF-8", c"cs_CZ.UTF-8"),
        ("de_DE.UTF-8", c"de_DE.UTF-8"),
    ] {
        if let Ok(locale) = Collator::new(name) {
            locales.push((label, locale));
        }
    }
    let long = format!("{}a", "文".repeat(4096));
    let mut samples: Vec<String> = [
        "",
        "a",
        "A",
        "á",
        "ä",
        "b",
        "ch",
        "Ch",
        "h",
        "i",
        "z",
        "ž",
        "file-0000001.txt",
        "File-0000001.txt",
        "file-0000010.txt",
        "file_0000001.txt",
        "file 0000001.txt",
        "IMG_1234.JPG",
        "img_1234.jpg",
        "Výroční zpráva – finální.pdf",
        "Straße",
        "strasse",
        "æble",
        "e\u{301}",
        "é",
        "👩🏽‍💻 notes",
        "日本語のファイル",
        "עברית",
        "a.b",
        "a-b",
        "a b",
        "über",
        "Uber",
        "Čeština",
        "cestina",
        &long,
    ]
    .map(String::from)
    .to_vec();
    samples.extend((0..40).map(|index| format!("Report {index:03} – Ægir.ODT")));
    for (label, locale) in &locales {
        let weights: Vec<_> = samples
            .iter()
            .map(|value| two_pass_weights(locale, value))
            .collect();
        let fixed: Vec<_> = weights
            .iter()
            .map(|weights| fixed_width_key(weights))
            .collect();
        let variable: Vec<_> = samples
            .iter()
            .map(|value| locale.key(OsStr::new(value)).unwrap().unwrap())
            .collect();
        for (left, value) in samples.iter().enumerate() {
            assert_eq!(variable[left], encode(&weights[left]).unwrap(), "{label}");
            assert!(
                variable[left].len() <= fixed[left].len(),
                "{label} {value:?}"
            );
            for right in 0..samples.len() {
                assert_eq!(
                    variable[left].cmp(&variable[right]),
                    fixed[left].cmp(&fixed[right]),
                    "{label}: {value:?} / {:?}",
                    samples[right]
                );
            }
        }
    }
}

#[test]
fn c_locale_keys_take_one_byte_per_ascii_character() {
    let name = "file-0000001.txt";
    let key = Collator::new(c"C")
        .unwrap()
        .key(OsStr::new(name))
        .unwrap()
        .unwrap();
    // Code points 0..127 and the terminating zero each take one byte, shifted by one.
    let expected: Vec<u8> = name.bytes().map(|byte| byte + 1).chain([0x01]).collect();
    assert_eq!(key, expected);
    assert_eq!(key.capacity(), key.len());
}

#[test]
fn encoded_keys_stay_within_the_key_budget() {
    let largest = libc::wchar_t::MAX;
    assert_eq!(code(largest).len(), 5);
    let count = MAX_COLLATION_KEY_BYTES / 5;
    let key = encode(&vec![largest; count]).unwrap();
    assert_eq!(key.len(), count * 5);
    assert!(encode(&vec![largest; count + 1]).is_err());
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

/// Run with `MIDNIGHT_COLLATION_TIMING_LOCALE=en_US.UTF-8 cargo test -p
/// midnight-platform-linux --lib collation_key_sizes -- --ignored --nocapture`; a missing
/// locale falls back to C. Each case runs in its own process, so neither reuses memory
/// that the other freed.
#[test]
#[ignore = "memory measurement of a million collation keys; prints measurements"]
fn collation_key_sizes() {
    const CASE: &str = "MIDNIGHT_COLLATION_SIZES_CASE";
    const NAMES: usize = 1_000_000;
    let Ok(case) = std::env::var(CASE) else {
        for case in ["fixed-width", "variable-length"] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["collation_key_sizes", "--ignored", "--nocapture"])
                .arg("--test-threads=1")
                .env(CASE, case)
                .output()
                .unwrap();
            let report = String::from_utf8_lossy(&output.stdout);
            let line = report
                .lines()
                .find_map(|line| line.find("KEYS ").map(|at| line[at..].to_owned()));
            assert!(output.status.success() && line.is_some(), "{report}");
            eprintln!("{}", line.unwrap_or_default());
        }
        return;
    };
    let resident = || {
        let status = std::fs::read_to_string("/proc/self/status").unwrap();
        status
            .lines()
            .find_map(|line| line.strip_prefix("VmRSS:"))
            .and_then(|value| {
                value
                    .trim()
                    .trim_end_matches("kB")
                    .trim()
                    .parse::<u64>()
                    .ok()
            })
            .unwrap()
            * 1024
    };
    let name =
        std::env::var("MIDNIGHT_COLLATION_TIMING_LOCALE").unwrap_or_else(|_| "en_US.UTF-8".into());
    let locale = Collator::with_fallback(&std::ffi::CString::new(name.clone()).unwrap()).unwrap();
    // 16-character names, as in the archive panel measurement.
    let names: Vec<String> = (1..=NAMES)
        .map(|index| format!("file-{index:07}.txt"))
        .collect();
    let start = resident();
    let keys: Vec<Vec<u8>> = names
        .iter()
        .map(|value| {
            if case == "fixed-width" {
                fixed_width_key(&two_pass_weights(&locale, value))
            } else {
                locale.key(OsStr::new(value)).unwrap().unwrap()
            }
        })
        .collect();
    let grown = resident().saturating_sub(start);
    let bytes: usize = keys.iter().map(Vec::len).sum();
    println!(
        "KEYS {case} keys of {NAMES} 16-character names ({name}; C if missing): {:.1} key bytes \
         per name; holding them grows the process by {:.1} MiB, {} bytes per name",
        bytes as f64 / NAMES as f64,
        grown as f64 / 1048576.0,
        grown / NAMES as u64
    );
}

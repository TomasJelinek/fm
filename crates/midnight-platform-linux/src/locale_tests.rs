use super::*;
use std::ffi::OsStr;

#[test]
fn archive_calendar_is_formatted_without_normalizing_invalid_dates() {
    use midnight_core::ArchiveTimestamp;
    let service = LinuxFileSystem::new(None);
    let valid = ArchiveTimestamp::Local {
        year: 2024,
        month: 2,
        day: 29,
        hour: 12,
        minute: 34,
        second: 56,
    };
    let rendered = service.format_archive_local_time(valid).unwrap();
    assert!(!rendered.date.is_empty() && !rendered.time.is_empty());
    assert!(
        service
            .format_archive_local_time(ArchiveTimestamp::Local {
                year: 2023,
                month: 2,
                day: 29,
                hour: 12,
                minute: 34,
                second: 56,
            })
            .is_none()
    );
    assert!(
        service
            .format_archive_local_time(ArchiveTimestamp::Utc {
                seconds: 0,
                nanoseconds: 0,
            })
            .is_none()
    );
}

#[test]
fn locale_category_precedence_respects_empty_variables_and_specific_overrides() {
    let all = OsStr::new("cs_CZ.UTF-8");
    let numeric = OsStr::new("en_IN.UTF-8");
    let language = OsStr::new("de_DE.UTF-8");
    assert_eq!(
        selected_locale(Some(all), Some(numeric), Some(language)),
        all
    );
    assert_eq!(
        selected_locale(Some(OsStr::new("")), Some(numeric), Some(language)),
        numeric
    );
    assert_eq!(selected_locale(None, None, Some(language)), language);
    assert_eq!(selected_locale(None, None, None), "C");
}

#[test]
fn posix_grouping_distinguishes_repeat_from_char_max_termination() {
    assert_eq!(grouping(&[3, 2, 0]), (vec![3, 2], true));
    assert_eq!(grouping(&[3, 2, 127, 0]), (vec![3, 2], false));
    assert_eq!(grouping(&[0]), (vec![], false));
    assert_eq!(grouping(&[127, 0]), (vec![], false));
}

#[test]
fn current_native_timestamp_has_separate_complete_date_and_time_fields() {
    let value = LinuxFileSystem::new(None)
        .format_datetime(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000))
        .unwrap();
    assert!(!value.date.is_empty());
    assert!(!value.time.is_empty());
    let seconds: libc::time_t = 1_700_000_000;
    // SAFETY: localtime_r initializes this writable per-call structure from a valid time_t.
    let mut native: libc::tm = unsafe { std::mem::zeroed() };
    assert!(!unsafe { libc::localtime_r(&seconds, &mut native) }.is_null());
    CURRENT.with(|locales| {
        let locale = &locales.as_ref().unwrap().time;
        for (pattern, expected) in [(c"%x", value.date), (c"%X", value.time)] {
            let mut buffer = vec![0u8; 4096];
            // SAFETY: live native locale/calendar and terminated pattern, writable bounded output.
            let count = unsafe {
                libc::strftime_l(
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    pattern.as_ptr(),
                    &native,
                    locale.0,
                )
            };
            assert!(count > 0);
            assert_eq!(
                decode_encoded(locale.value(libc::CODESET), &buffer[..count]).unwrap(),
                expected
            );
        }
    });
}

#[test]
fn non_utf8_locale_text_is_converted_without_replacement_characters() {
    assert_eq!(
        decode_encoded(c"ISO-8859-1", &[b'1', 0xa0, b'0', b'2', b'4']).unwrap(),
        "1\u{a0}024"
    );
    assert_eq!(
        decode_encoded(c"UTF-8", "listopad 🦀".as_bytes()).unwrap(),
        "listopad 🦀"
    );
}

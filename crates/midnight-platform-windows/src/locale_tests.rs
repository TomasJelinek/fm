use super::*;
use std::{ffi::OsStr, os::windows::ffi::OsStrExt, ptr};
use windows_sys::Win32::Globalization::{GetLocaleInfoEx, GetNumberFormatEx, LOCALE_SDECIMAL};

#[test]
fn native_text_is_formatted_once_and_size_probed_only_when_long() {
    use windows_sys::Win32::Foundation::SetLastError;
    let mut capacities = Vec::new();
    let short = wide_output(|buffer, capacity| {
        capacities.push(capacity);
        // SAFETY: the stack buffer advertised by `capacity` holds these three units.
        unsafe { ptr::copy_nonoverlapping([97u16, 98, 0].as_ptr(), buffer, 3) };
        3
    })
    .unwrap();
    assert_eq!(short, [97, 98, 0]);
    assert_eq!(capacities, [128], "one native call for an ordinary value");
    capacities.clear();
    let long = wide_output(|buffer, capacity| {
        capacities.push(capacity);
        if capacity == 0 {
            return 300;
        }
        if capacity < 300 {
            // SAFETY: plain thread error-code store, as the NLS API does on failure.
            unsafe { SetLastError(122) };
            return 0;
        }
        // SAFETY: the sized buffer holds `capacity` (300) writable units.
        unsafe { std::slice::from_raw_parts_mut(buffer, 300).fill(120) };
        300
    })
    .unwrap();
    assert_eq!(long.len(), 300);
    assert_eq!(capacities, [128, 0, 300]);
    capacities.clear();
    let error = wide_output(|_, capacity| {
        capacities.push(capacity);
        // SAFETY: plain thread error-code store.
        unsafe { SetLastError(87) };
        0
    })
    .unwrap_err();
    assert_eq!(error.raw_os_error(), Some(87));
    assert_eq!(capacities, [128], "other failures are not retried");
}

#[test]
fn archive_calendar_is_formatted_without_normalizing_invalid_dates() {
    use midnight_core::ArchiveTimestamp;
    let service = WindowsFileSystem::new(None);
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
fn current_user_number_snapshot_matches_nls_without_losing_u64_precision() {
    let locale = WindowsFileSystem::new(None).number_locale().unwrap();
    let mut decimal = [0u16; 32];
    // SAFETY: bounded writable buffer, user-default locale, and valid locale field.
    let count = unsafe {
        GetLocaleInfoEx(
            ptr::null(),
            LOCALE_SDECIMAL,
            decimal.as_mut_ptr(),
            decimal.len() as i32,
        )
    };
    assert!(count > 0);
    let decimal = String::from_utf16(&decimal[..count as usize - 1]).unwrap();
    for value in [0, 1024, 123456789, u64::MAX] {
        let input: Vec<_> = OsStr::new(&value.to_string())
            .encode_wide()
            .chain(Some(0))
            .collect();
        let mut buffer = [0u16; 256];
        // SAFETY: terminated input and bounded output, null format requests actual user defaults.
        let count = unsafe {
            GetNumberFormatEx(
                ptr::null(),
                0,
                input.as_ptr(),
                ptr::null(),
                buffer.as_mut_ptr(),
                buffer.len() as i32,
            )
        };
        assert!(count > 0);
        let native = String::from_utf16(&buffer[..count as usize - 1]).unwrap();
        let integer = native
            .split_once(&decimal)
            .map_or(native.as_str(), |(whole, _)| whole);
        assert_eq!(locale.format_u64(value), integer);
    }
}

#[test]
fn native_timestamp_is_returned_as_two_complete_nonempty_fields() {
    let parts =
        modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000)).unwrap();
    assert!(!parts.date.is_empty());
    assert!(!parts.time.is_empty());
}

#[test]
fn native_tick_before_epoch_stays_in_the_preceding_native_second() {
    // Windows SystemTime has 100 ns resolution; subtracting 1 ns rounds to the epoch itself.
    assert_eq!(
        modified(UNIX_EPOCH - std::time::Duration::from_nanos(100)),
        modified(UNIX_EPOCH - std::time::Duration::from_secs(1))
    );
}

#[test]
fn timestamp_matches_native_user_overrides_and_historical_timezone_conversion() {
    use windows_sys::Win32::{
        Foundation::SYSTEMTIME,
        Globalization::{DATE_SHORTDATE, GetDateFormatEx, GetTimeFormatEx},
        System::Time::SystemTimeToTzSpecificLocalTimeEx,
    };
    let utc = SYSTEMTIME {
        wYear: 2023,
        wMonth: 11,
        wDay: 14,
        wHour: 22,
        wMinute: 13,
        wSecond: 20,
        ..Default::default()
    };
    let mut local = SYSTEMTIME::default();
    // SAFETY: all structures are live and correctly typed; null selects the user's timezone.
    assert_ne!(
        unsafe { SystemTimeToTzSpecificLocalTimeEx(ptr::null(), &utc, &mut local) },
        0
    );
    let mut date = [0u16; 512];
    let mut time = [0u16; 512];
    // SAFETY: explicit known local time and bounded output buffers; null patterns select the
    // actual current user's overrides independently of the implementation's captured patterns.
    let date_count = unsafe {
        GetDateFormatEx(
            ptr::null(),
            DATE_SHORTDATE,
            &local,
            ptr::null(),
            date.as_mut_ptr(),
            date.len() as i32,
            ptr::null(),
        )
    };
    // SAFETY: same bounded output and valid local-time structure as above.
    let time_count = unsafe {
        GetTimeFormatEx(
            ptr::null(),
            0,
            &local,
            ptr::null(),
            time.as_mut_ptr(),
            time.len() as i32,
        )
    };
    assert!(date_count > 0 && time_count > 0);
    let expected = midnight_core::LocalizedDateTime {
        date: String::from_utf16(&date[..date_count as usize - 1]).unwrap(),
        time: String::from_utf16(&time[..time_count as usize - 1]).unwrap(),
    };
    assert_eq!(
        WindowsFileSystem::new(None)
            .format_datetime(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000)),
        Some(expected)
    );
}

#[test]
fn recent_texts_format_each_distinct_value_once_and_keep_no_failure() {
    let mut recent = RecentTexts::new();
    let calls = std::cell::Cell::new(0);
    let format = |text: &str| {
        let text = text.to_owned();
        let calls = &calls;
        move || {
            calls.set(calls.get() + 1);
            Ok(text)
        }
    };
    assert_eq!(recent.text(7, format("seven")).unwrap(), "seven");
    assert_eq!(recent.text(7, format("other")).unwrap(), "seven");
    assert_eq!(calls.get(), 1, "a kept text is reused");
    // Every key of a full cycle of slots: each is formatted once, and the first is
    // replaced by a later key in its slot, so it is formatted again.
    for key in 0..(RecentTexts::SLOTS as u64 * 4) {
        recent.text(key, format(&key.to_string())).unwrap();
    }
    let before = calls.get();
    assert_eq!(recent.text(7, format("seven")).unwrap(), "seven");
    assert!(calls.get() <= before + 1);
    let failed = recent.text(1 << 40, || Err(io::Error::other("native failure")));
    assert!(failed.is_err());
    assert_eq!(
        recent.text(1 << 40, format("now")).unwrap(),
        "now",
        "a failure is not kept"
    );
}

#[test]
fn reused_modified_texts_equal_native_formatting() {
    let base = UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    let locale = snapshot().unwrap();
    // Distinct seconds over three years, then the same values again (reused texts),
    // and many rows of one minute: every one equals a direct native call.
    let offsets = (0..4_000_u64)
        .map(|row| row.wrapping_mul(2_654_435_761) % (3 * 365 * 86_400))
        .chain((0..4_000_u64).map(|row| row.wrapping_mul(2_654_435_761) % (3 * 365 * 86_400)))
        .chain((0..2_000_u64).map(|row| row % 60));
    for offset in offsets {
        let time = base + std::time::Duration::from_secs(offset);
        let direct = locale.format(&local_system_time(time).unwrap()).unwrap();
        assert_eq!(modified(time), Some(direct), "offset {offset}");
    }
}

#[test]
fn local_calendar_fields_by_day_offset_equal_native_conversion() {
    use std::time::Duration;
    let fields = |value: SYSTEMTIME| {
        (
            value.wYear,
            value.wMonth,
            value.wDayOfWeek,
            value.wDay,
            value.wHour,
            value.wMinute,
            value.wSecond,
            value.wMilliseconds,
        )
    };
    let ticks = |time: SystemTime| {
        let nanos = time.duration_since(UNIX_EPOCH).unwrap().as_nanos();
        u64::try_from(nanos / 100).unwrap() + 116_444_736_000_000_000
    };
    // Every 433.789 s over three years: every day, each daylight saving transition day
    // of this machine's zone, and every millisecond phase; a second pass reuses the kept
    // offsets of every day.
    for _ in 0..2 {
        let mut time = UNIX_EPOCH + Duration::from_secs(1_600_000_000);
        for _ in 0..(3 * 365 * 86_400_000_u64 / 433_789) {
            let expected = native_local_time(ticks(time)).map(fields);
            assert_eq!(local_system_time(time).map(fields), expected, "{time:?}");
            time += Duration::from_millis(433_789);
        }
    }
    // Before the Unix epoch (Windows `SystemTime` cannot hold a time before 1601).
    let before = UNIX_EPOCH - Duration::from_secs(86_400 * 365 * 100 + 12_345);
    assert_eq!(
        local_system_time(before).map(fields),
        native_local_time(116_444_736_000_000_000 - 3_153_612_345 * 10_000_000).map(fields)
    );
    assert!(local_system_time(before).is_some());
}

/// A timing, not a check: per-row cost of the listing's native formatting of
/// modification times and of collation keys, over 100,000 rows. Run in a release build:
/// `cargo test --release -p midnight-platform-windows --lib locale::tests::listing_locale_timing
/// -- --ignored --nocapture --test-threads 1`.
#[test]
#[ignore = "timing of native locale work per listing row; run explicitly"]
fn listing_locale_timing() {
    use std::time::{Duration, Instant};
    const ROWS: u64 = 100_000;
    let base = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    // Three spreads: every row in another second of three years; rows of one working
    // day (a build's outputs); one timestamp for all (an extracted archive).
    type Spread = (&'static str, fn(u64) -> u64);
    let spreads: [Spread; 3] = [
        ("three years", |row| {
            row.wrapping_mul(2_654_435_761) % (3 * 365 * 86_400)
        }),
        ("one day", |row| row.wrapping_mul(2_654_435_761) % 86_400),
        ("one second", |_| 0),
    ];
    let _ = modified(base);
    let mut times = Vec::new();
    for _ in 0..5 {
        let started = Instant::now();
        for row in 0..ROWS {
            let offset = row.wrapping_mul(2_654_435_761) % (3 * 365 * 86_400);
            std::hint::black_box(local_system_time(base + Duration::from_secs(offset)));
        }
        times.push(started.elapsed());
    }
    times.sort();
    println!(
        "local calendar fields only, three years: {:.0} ns per row",
        times[2].as_secs_f64() * 1e9 / ROWS as f64
    );
    for (label, offset) in spreads {
        let mut times = Vec::new();
        for _ in 0..5 {
            let started = Instant::now();
            for row in 0..ROWS {
                std::hint::black_box(modified(base + Duration::from_secs(offset(row))));
            }
            times.push(started.elapsed());
        }
        times.sort();
        println!(
            "format modified, {label}: median {:.3} ms for {ROWS} rows ({:.0} ns per row)",
            times[2].as_secs_f64() * 1000.0,
            times[2].as_secs_f64() * 1e9 / ROWS as f64
        );
    }
    let names: Vec<std::ffi::OsString> = (0..ROWS)
        .map(|row| format!("Report {row} - Čtvrtletí.txt").into())
        .collect();
    let locale = collation_name().unwrap();
    let mut times = Vec::new();
    for _ in 0..5 {
        let started = Instant::now();
        for name in &names {
            std::hint::black_box(crate::sort_keys::key(locale, name).unwrap());
        }
        times.push(started.elapsed());
    }
    times.sort();
    println!(
        "collation keys: median {:.3} ms for {ROWS} names ({:.0} ns per name)",
        times[2].as_secs_f64() * 1000.0,
        times[2].as_secs_f64() * 1e9 / ROWS as f64
    );
}

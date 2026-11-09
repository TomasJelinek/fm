use super::*;
use std::{ffi::OsStr, os::windows::ffi::OsStrExt, ptr};
use windows_sys::Win32::Globalization::{GetLocaleInfoEx, GetNumberFormatEx, LOCALE_SDECIMAL};

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

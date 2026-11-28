//! Captured user overrides and timezone; native formatting only runs on backend workers.
use crate::WindowsFileSystem;
use midnight_core::{FsError, LocaleService, LocalizedDateTime, NumberLocale};
use std::{
    ffi::OsStr,
    io,
    path::Path,
    ptr,
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::{
    Foundation::{FILETIME, SYSTEMTIME},
    Globalization::{
        GetDateFormatEx, GetLocaleInfoEx, GetNumberFormatEx, GetTimeFormatEx, LOCALE_IDIGITS,
        LOCALE_SDECIMAL, LOCALE_SGROUPING, LOCALE_SNAME, LOCALE_SSHORTDATE, LOCALE_STHOUSAND,
        LOCALE_STIMEFORMAT,
    },
    System::Time::{
        DYNAMIC_TIME_ZONE_INFORMATION, FileTimeToSystemTime, GetDynamicTimeZoneInformation,
        SystemTimeToFileTime, SystemTimeToTzSpecificLocalTimeEx, TIME_ZONE_ID_INVALID,
    },
};

struct WindowsLocale {
    numbers: NumberLocale,
    name: Vec<u16>,
    date_pattern: Vec<u16>,
    time_pattern: Vec<u16>,
    timezone: DYNAMIC_TIME_ZONE_INFORMATION,
    input_format: Option<midnight_core::DateTimeInputFormat>,
}

#[path = "locale_input.rs"]
mod input;

static SNAPSHOT: OnceLock<Result<WindowsLocale, FsError>> = OnceLock::new();
static NAME: OnceLock<Result<Vec<u16>, FsError>> = OnceLock::new();

impl LocaleService for WindowsFileSystem {
    fn number_locale(&self) -> Result<NumberLocale, FsError> {
        Ok(snapshot()?.numbers.clone())
    }

    fn datetime_input_format(&self) -> Option<midnight_core::DateTimeInputFormat> {
        snapshot().ok()?.input_format.clone()
    }

    fn format_datetime(&self, value: SystemTime) -> Option<LocalizedDateTime> {
        modified(value)
    }

    fn local_calendar(&self, value: SystemTime) -> Option<midnight_core::LocalCalendarTime> {
        let local = local_system_time(value)?;
        midnight_core::LocalCalendarTime::from_local_fields(
            i32::from(local.wYear),
            u8::try_from(local.wMonth).ok()?,
            u8::try_from(local.wDay).ok()?,
            u8::try_from(local.wHour).ok()?,
            u8::try_from(local.wMinute).ok()?,
            u8::try_from(local.wSecond).ok()?,
            value,
        )
    }

    fn format_archive_local_time(
        &self,
        value: midnight_core::ArchiveTimestamp,
    ) -> Option<LocalizedDateTime> {
        use midnight_core::ArchiveTimestamp;
        if !value.is_valid() {
            return None;
        }
        let ArchiveTimestamp::Local {
            year,
            month,
            day,
            hour,
            minute,
            second,
        } = value
        else {
            return None;
        };
        let calendar = SYSTEMTIME {
            wYear: u16::try_from(year).ok()?,
            wMonth: u16::from(month),
            wDay: u16::from(day),
            wHour: u16::from(hour),
            wMinute: u16::from(minute),
            wSecond: u16::from(second),
            ..Default::default()
        };
        snapshot().ok()?.format(&calendar).ok()
    }

    fn collation_key(&self, value: &OsStr) -> Result<Option<Vec<u8>>, FsError> {
        crate::sort_keys::key(collation_name()?, value).map_err(|error| {
            FsError::from_io("prepare Windows collation key", Path::new(""), error)
        })
    }
}

fn collation_name() -> Result<&'static [u16], FsError> {
    NAME.get_or_init(|| {
        locale_value(LOCALE_SNAME).map_err(|error| {
            FsError::from_io("read Windows collation locale", Path::new(""), error)
        })
    })
    .as_ref()
    .map(Vec::as_slice)
    .map_err(Clone::clone)
}

fn snapshot() -> Result<&'static WindowsLocale, FsError> {
    SNAPSHOT
        .get_or_init(|| {
            WindowsLocale::capture()
                .map_err(|error| FsError::from_io("read Windows locale", Path::new(""), error))
        })
        .as_ref()
        .map_err(Clone::clone)
}

impl WindowsLocale {
    fn capture() -> io::Result<Self> {
        let grouping_value = text(&locale_value(LOCALE_SGROUPING)?)?;
        let mut grouping = grouping_value
            .split(';')
            .map(str::parse::<u8>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(io::Error::other)?;
        let repeat_last_group = grouping.last() == Some(&0);
        if repeat_last_group {
            grouping.pop();
        }
        if grouping.contains(&0) {
            return Err(io::Error::other("invalid Windows digit grouping"));
        }
        let numbers = NumberLocale {
            decimal_separator: text(&locale_value(LOCALE_SDECIMAL)?)?,
            group_separator: text(&locale_value(LOCALE_STHOUSAND)?)?,
            repeat_last_group: repeat_last_group && !grouping.is_empty(),
            grouping,
        };
        // Compare the captured rules with NLS's actual user-override formatter. Decimal text
        // input preserves all 64 bits; neither native nor pure formatting uses floating point.
        let input: Vec<_> = u64::MAX.to_string().encode_utf16().chain(Some(0)).collect();
        let native = text(&wide_output(|buffer, capacity| {
            // SAFETY: terminated input, null format requests user overrides; output capacity
            // and its live allocation are supplied by wide_output.
            unsafe {
                GetNumberFormatEx(
                    ptr::null(),
                    0,
                    input.as_ptr(),
                    ptr::null(),
                    buffer,
                    capacity,
                )
            }
        })?)?;
        let fractional_digits = text(&locale_value(LOCALE_IDIGITS)?)?
            .parse::<usize>()
            .map_err(io::Error::other)?;
        let integer = if fractional_digits > 0 {
            native
                .rsplit_once(&numbers.decimal_separator)
                .map(|(whole, _)| whole)
                .ok_or_else(|| io::Error::other("missing native decimal separator"))?
        } else {
            &native
        };
        if numbers.format_u64(u64::MAX) != integer {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "native number format cannot be represented by the captured grouping rules",
            ));
        }
        let mut timezone = DYNAMIC_TIME_ZONE_INFORMATION::default();
        // SAFETY: timezone is a fully writable native structure; no retained pointers.
        if unsafe { GetDynamicTimeZoneInformation(&mut timezone) } == TIME_ZONE_ID_INVALID {
            return Err(io::Error::last_os_error());
        }
        let mut locale = Self {
            numbers,
            name: collation_name()
                .map_err(|error| io::Error::new(error.kind, error.message))?
                .to_vec(),
            date_pattern: locale_value(LOCALE_SSHORTDATE)?,
            time_pattern: locale_value(LOCALE_STIMEFORMAT)?,
            timezone,
            input_format: None,
        };
        locale.input_format = input::capture(&locale);
        Ok(locale)
    }

    fn format(&self, local: &SYSTEMTIME) -> io::Result<LocalizedDateTime> {
        Ok(LocalizedDateTime {
            date: self.date(local)?,
            time: self.time(local)?,
        })
    }

    fn date(&self, local: &SYSTEMTIME) -> io::Result<String> {
        wide_output_with(
            |buffer, capacity| {
                // SAFETY: captured terminated locale/format strings and local SYSTEMTIME stay
                // alive; wide_output supplies a bounded writable buffer. No override-disabling flag.
                unsafe {
                    GetDateFormatEx(
                        self.name.as_ptr(),
                        0,
                        local,
                        self.date_pattern.as_ptr(),
                        buffer,
                        capacity,
                        ptr::null(),
                    )
                }
            },
            text,
        )
    }

    fn time(&self, local: &SYSTEMTIME) -> io::Result<String> {
        wide_output_with(
            |buffer, capacity| {
                // SAFETY: same owned input/buffer lifetimes as date formatting above.
                unsafe {
                    GetTimeFormatEx(
                        self.name.as_ptr(),
                        0,
                        local,
                        self.time_pattern.as_ptr(),
                        buffer,
                        capacity,
                    )
                }
            },
            text,
        )
    }
}

/// Texts of recently formatted dates or times by their calendar fields: a bounded,
/// direct-mapped cache (a value replaces the one in its slot). Failures are not kept.
struct RecentTexts {
    slots: Vec<Option<(u64, String)>>,
}

impl RecentTexts {
    /// Slots per cache, a power of two: a few hundred distinct dates or times in a
    /// folder stay resident, and a cache never holds more than this many short texts.
    const SLOTS: usize = 512;

    fn new() -> Self {
        Self {
            slots: vec![None; Self::SLOTS],
        }
    }

    fn text(
        &mut self,
        key: u64,
        format: impl FnOnce() -> io::Result<String>,
    ) -> io::Result<String> {
        // Fibonacci hashing spreads neighbouring days and seconds over the slots.
        let slot = (key.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> (64 - Self::SLOTS.trailing_zeros()))
            as usize;
        if let Some((kept, text)) = &self.slots[slot]
            && *kept == key
        {
            return Ok(text.clone());
        }
        let text = format()?;
        self.slots[slot] = Some((key, text.clone()));
        Ok(text)
    }
}

/// The calendar fields that GetDateFormatEx formats (the day of week follows the date).
fn date_key(local: &SYSTEMTIME) -> u64 {
    u64::from(local.wYear) << 48
        | u64::from(local.wMonth) << 32
        | u64::from(local.wDay) << 16
        | u64::from(local.wDayOfWeek)
}

/// The clock fields that GetTimeFormatEx formats; it has no millisecond pattern.
fn time_key(local: &SYSTEMTIME) -> u64 {
    u64::from(local.wHour) << 32 | u64::from(local.wMinute) << 16 | u64::from(local.wSecond)
}

thread_local! {
    /// Per worker thread, so formatting never waits for a lock: dates, then times.
    static RECENT: std::cell::RefCell<(RecentTexts, RecentTexts)> =
        std::cell::RefCell::new((RecentTexts::new(), RecentTexts::new()));
}

/// A listing formats every row's modification time, and the rows of a folder share
/// most dates and many times: the text of a recent date or time is reused, so each
/// distinct value calls NLS once per worker thread. The captured locale snapshot never
/// changes in a process, so a reused text equals what a new call returns.
pub(crate) fn modified(time: SystemTime) -> Option<LocalizedDateTime> {
    let locale = snapshot().ok()?;
    let local = local_system_time(time)?;
    RECENT.with(|recent| {
        let mut recent = recent.borrow_mut();
        let (dates, times) = &mut *recent;
        Some(LocalizedDateTime {
            date: dates.text(date_key(&local), || locale.date(&local)).ok()?,
            time: times.text(time_key(&local), || locale.time(&local)).ok()?,
        })
    })
}

/// FILETIME ticks (100 ns) in a day.
const TICKS_PER_DAY: u64 = 864_000_000_000;

fn local_system_time(time: SystemTime) -> Option<SYSTEMTIME> {
    // FILETIME is an unsigned count of 100 ns ticks since 1601, not a floating-point timestamp.
    let ticks = match time.duration_since(UNIX_EPOCH) {
        Ok(value) => i128::try_from(value.as_nanos()).ok()? / 100,
        Err(error) => (-i128::try_from(error.duration().as_nanos()).ok()?).div_euclid(100),
    }
    .checked_add(116_444_736_000_000_000)?;
    let ticks = u64::try_from(ticks).ok()?;
    let offset = DAY_OFFSETS.with(|offsets| offsets.borrow_mut().offset(ticks / TICKS_PER_DAY));
    if let Some(local) = offset.and_then(|offset| ticks.checked_add_signed(offset)) {
        return calendar(local);
    }
    native_local_time(ticks)
}

/// The calendar fields of FILETIME `ticks`, without a time zone.
fn calendar(ticks: u64) -> Option<SYSTEMTIME> {
    let filetime = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut fields = SYSTEMTIME::default();
    // SAFETY: both pointers refer to live correctly typed structures; the API rejects
    // values outside its representable range.
    (unsafe { FileTimeToSystemTime(&filetime, &mut fields) } != 0).then_some(fields)
}

/// Local calendar fields of the UTC FILETIME `ticks` by the captured time zone's rules.
fn native_local_time(ticks: u64) -> Option<SYSTEMTIME> {
    let locale = snapshot().ok()?;
    let utc = calendar(ticks)?;
    let mut local = SYSTEMTIME::default();
    // SAFETY: all pointers refer to live correctly typed structures; native APIs reject dates
    // outside their representable calendar range without affecting the sorting timestamp.
    if unsafe { SystemTimeToTzSpecificLocalTimeEx(&locale.timezone, &utc, &mut local) } == 0 {
        return None;
    }
    Some(local)
}

/// Local minus UTC FILETIME ticks at the UTC instant `ticks` (a whole millisecond).
fn native_offset(ticks: u64) -> Option<i64> {
    let local = native_local_time(ticks)?;
    let mut filetime = FILETIME::default();
    // SAFETY: both pointers refer to live correctly typed structures.
    if unsafe { SystemTimeToFileTime(&local, &mut filetime) } == 0 {
        return None;
    }
    let local = u64::from(filetime.dwHighDateTime) << 32 | u64::from(filetime.dwLowDateTime);
    i64::try_from(i128::from(local) - i128::from(ticks)).ok()
}

/// The time zone offsets of recent UTC days: converting a row's time to local
/// calendar fields is then arithmetic, not a time zone conversion per row. A day is
/// kept only when its first and last millisecond have the same offset, that is when no
/// daylight saving transition falls inside it; rows of a transition day are converted
/// natively one by one. Direct-mapped and bounded, per worker thread.
struct DayOffsets {
    slots: Vec<Option<(u64, Option<i64>)>>,
}

impl DayOffsets {
    /// Slots, a power of two: three years of days fit with few collisions.
    const SLOTS: usize = 2048;

    fn new() -> Self {
        Self {
            slots: vec![None; Self::SLOTS],
        }
    }

    fn offset(&mut self, day: u64) -> Option<i64> {
        let slot = (day.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> (64 - Self::SLOTS.trailing_zeros()))
            as usize;
        if let Some((kept, offset)) = self.slots[slot]
            && kept == day
        {
            return offset;
        }
        let offset = (|| {
            let start = day.checked_mul(TICKS_PER_DAY)?;
            let first = native_offset(start)?;
            let last = native_offset(start.checked_add(TICKS_PER_DAY - 10_000)?)?;
            (first == last).then_some(first)
        })();
        self.slots[slot] = Some((day, offset));
        offset
    }
}

thread_local! {
    static DAY_OFFSETS: std::cell::RefCell<DayOffsets> =
        std::cell::RefCell::new(DayOffsets::new());
}

fn locale_value(field: u32) -> io::Result<Vec<u16>> {
    wide_output(|buffer, capacity| {
        // SAFETY: field is a documented string locale field; null means current user default
        // including user overrides. Buffer lifetime and capacity come from wide_output.
        unsafe { GetLocaleInfoEx(ptr::null(), field, buffer, capacity) }
    })
}

fn wide_output(call: impl FnMut(*mut u16, i32) -> i32) -> io::Result<Vec<u16>> {
    wide_output_with(call, |wide| Ok(wide.to_vec()))
}

/// `wide_output` finished by `finish`, which reads the written units where they are:
/// text formatted on the stack is converted without an intermediate UTF-16 copy.
fn wide_output_with<T>(
    mut call: impl FnMut(*mut u16, i32) -> i32,
    finish: impl FnOnce(&[u16]) -> io::Result<T>,
) -> io::Result<T> {
    // Dates, times and locale fields normally fit a small buffer: format once on the
    // stack (one NLS call per value on the listing hot path) and size-probe only when
    // the native call reports an insufficient buffer.
    let mut stack = [0u16; 128];
    let written = call(stack.as_mut_ptr(), stack.len() as i32);
    if written > 0 {
        return finish(&stack[..written as usize]);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() != Some(122) {
        // Anything but ERROR_INSUFFICIENT_BUFFER is a real failure.
        return Err(error);
    }
    let required = call(ptr::null_mut(), 0);
    if required <= 0 {
        return Err(io::Error::last_os_error());
    }
    let mut buffer = vec![0u16; required as usize];
    let written = call(buffer.as_mut_ptr(), required);
    if written <= 0 {
        return Err(io::Error::last_os_error());
    }
    finish(&buffer[..written as usize])
}

fn text(wide: &[u16]) -> io::Result<String> {
    let end = wide
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(wide.len());
    String::from_utf16(&wide[..end])
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
#[path = "locale_tests.rs"]
mod tests;

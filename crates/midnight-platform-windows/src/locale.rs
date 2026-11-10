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
        SystemTimeToTzSpecificLocalTimeEx, TIME_ZONE_ID_INVALID,
    },
};

struct WindowsLocale {
    numbers: NumberLocale,
    name: Vec<u16>,
    date_pattern: Vec<u16>,
    time_pattern: Vec<u16>,
    timezone: DYNAMIC_TIME_ZONE_INFORMATION,
}

static SNAPSHOT: OnceLock<Result<WindowsLocale, FsError>> = OnceLock::new();
static NAME: OnceLock<Result<Vec<u16>, FsError>> = OnceLock::new();

impl LocaleService for WindowsFileSystem {
    fn number_locale(&self) -> Result<NumberLocale, FsError> {
        Ok(snapshot()?.numbers.clone())
    }

    fn format_datetime(&self, value: SystemTime) -> Option<LocalizedDateTime> {
        modified(value)
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
        Ok(Self {
            numbers,
            name: collation_name()
                .map_err(|error| io::Error::new(error.kind, error.message))?
                .to_vec(),
            date_pattern: locale_value(LOCALE_SSHORTDATE)?,
            time_pattern: locale_value(LOCALE_STIMEFORMAT)?,
            timezone,
        })
    }

    fn format(&self, local: &SYSTEMTIME) -> io::Result<LocalizedDateTime> {
        let date = wide_output(|buffer, capacity| {
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
        })?;
        let time = wide_output(|buffer, capacity| {
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
        })?;
        Ok(LocalizedDateTime {
            date: text(&date)?,
            time: text(&time)?,
        })
    }
}

pub(crate) fn modified(time: SystemTime) -> Option<LocalizedDateTime> {
    let locale = snapshot().ok()?;
    // FILETIME is an unsigned count of 100 ns ticks since 1601, not a floating-point timestamp.
    let ticks = match time.duration_since(UNIX_EPOCH) {
        Ok(value) => i128::try_from(value.as_nanos()).ok()? / 100,
        Err(error) => (-i128::try_from(error.duration().as_nanos()).ok()?).div_euclid(100),
    }
    .checked_add(116_444_736_000_000_000)?;
    let ticks = u64::try_from(ticks).ok()?;
    let filetime = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    // SAFETY: all pointers refer to live correctly typed structures; native APIs reject dates
    // outside their representable calendar range without affecting the sorting timestamp.
    if unsafe { FileTimeToSystemTime(&filetime, &mut utc) } == 0
        || unsafe { SystemTimeToTzSpecificLocalTimeEx(&locale.timezone, &utc, &mut local) } == 0
    {
        return None;
    }
    locale.format(&local).ok()
}

fn locale_value(field: u32) -> io::Result<Vec<u16>> {
    wide_output(|buffer, capacity| {
        // SAFETY: field is a documented string locale field; null means current user default
        // including user overrides. Buffer lifetime and capacity come from wide_output.
        unsafe { GetLocaleInfoEx(ptr::null(), field, buffer, capacity) }
    })
}

fn wide_output(mut call: impl FnMut(*mut u16, i32) -> i32) -> io::Result<Vec<u16>> {
    let required = call(ptr::null_mut(), 0);
    if required <= 0 {
        return Err(io::Error::last_os_error());
    }
    let mut buffer = vec![0u16; required as usize];
    let written = call(buffer.as_mut_ptr(), required);
    if written <= 0 {
        return Err(io::Error::last_os_error());
    }
    buffer.truncate(written as usize);
    Ok(buffer)
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

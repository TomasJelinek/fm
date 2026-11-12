//! Native POSIX locale objects are thread-local; process-wide setlocale is never called.
use crate::LinuxFileSystem;
use midnight_core::{FsError, LocaleService, LocalizedDateTime, NumberLocale};
use std::{
    ffi::{CStr, CString, OsStr, OsString},
    io,
    os::unix::ffi::OsStrExt,
    path::Path,
    ptr,
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};

#[path = "locale_input.rs"]
mod input;

struct LocaleNames {
    numeric: CString,
    time: CString,
    collation: CString,
}
static NAMES: OnceLock<Result<LocaleNames, FsError>> = OnceLock::new();

struct NativeLocale(libc::locale_t);

impl NativeLocale {
    fn new(name: &CStr) -> io::Result<Self> {
        // SAFETY: name is terminated; a null base creates a new object without mutating any
        // global/thread locale. Ownership is acquired only after successful initialization.
        let locale = unsafe { libc::newlocale(libc::LC_ALL_MASK, name.as_ptr(), ptr::null_mut()) };
        if locale.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self(locale))
        }
    }

    fn value(&self, item: libc::nl_item) -> &CStr {
        // SAFETY: self owns this immutable locale throughout the returned borrow. Native
        // nl_langinfo_l returns a locale-owned terminated string, including for unknown items.
        unsafe { CStr::from_ptr(libc::nl_langinfo_l(item, self.0)) }
    }

    fn text(&self, item: libc::nl_item) -> io::Result<String> {
        decode_encoded(self.value(libc::CODESET), self.value(item).to_bytes())
    }

    fn numbers(&self) -> io::Result<NumberLocale> {
        // GNU/Linux's langinfo numeric category places GROUPING immediately after THOUSEP.
        // libc exposes THOUSEP but does not export GROUPING; this is a native langinfo item.
        let (grouping, repeat_last_group) =
            grouping(self.value(libc::THOUSEP + 1).to_bytes_with_nul());
        Ok(NumberLocale {
            decimal_separator: self.text(libc::RADIXCHAR)?,
            group_separator: self.text(libc::THOUSEP)?,
            grouping,
            repeat_last_group,
        })
    }

    fn format(&self, calendar: &libc::tm, pattern: &CStr) -> Option<String> {
        let mut capacity = 128usize;
        loop {
            let mut buffer = vec![0u8; capacity];
            // SAFETY: pattern and calendar are live, self is a thread-local owned locale, and
            // the writable output allocation has exactly the supplied capacity.
            let count = unsafe {
                libc::strftime_l(
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    pattern.as_ptr(),
                    calendar,
                    self.0,
                )
            };
            if count != 0 {
                buffer.truncate(count);
                return decode_encoded(self.value(libc::CODESET), &buffer).ok();
            }
            // strftime's zero result can also mean an empty result. %x/%X are nonempty;
            // constrain only diagnostic text allocation, never paths or filenames.
            if capacity >= 65_536 {
                return None;
            }
            capacity *= 2;
        }
    }
}

impl Drop for NativeLocale {
    fn drop(&mut self) {
        // SAFETY: this unique owned locale is no longer borrowed or installed as a thread
        // locale; freelocale balances only this object's successful newlocale call.
        unsafe {
            libc::freelocale(self.0);
        }
    }
}

struct Locales {
    numeric: NativeLocale,
    time: NativeLocale,
}
thread_local! { static CURRENT: Result<Locales, FsError> = capture(); }

fn names() -> Result<&'static LocaleNames, FsError> {
    NAMES
        .get_or_init(|| {
            let all = std::env::var_os("LC_ALL");
            let language = std::env::var_os("LANG");
            let numeric = std::env::var_os("LC_NUMERIC");
            let time = std::env::var_os("LC_TIME");
            let collation = std::env::var_os("LC_COLLATE");
            let cstring = |value: OsString| {
                CString::new(value.as_bytes()).map_err(|error| {
                    locale_error(io::Error::new(io::ErrorKind::InvalidInput, error))
                })
            };
            Ok(LocaleNames {
                numeric: cstring(selected_locale(
                    all.as_deref(),
                    numeric.as_deref(),
                    language.as_deref(),
                ))?,
                time: cstring(selected_locale(
                    all.as_deref(),
                    time.as_deref(),
                    language.as_deref(),
                ))?,
                collation: cstring(selected_locale(
                    all.as_deref(),
                    collation.as_deref(),
                    language.as_deref(),
                ))?,
            })
        })
        .as_ref()
        .map_err(Clone::clone)
}

pub(super) fn collation_name() -> Result<&'static CStr, FsError> {
    Ok(names()?.collation.as_c_str())
}

fn capture() -> Result<Locales, FsError> {
    let names = names()?;
    Ok(Locales {
        numeric: NativeLocale::new(&names.numeric).map_err(locale_error)?,
        time: NativeLocale::new(&names.time).map_err(locale_error)?,
    })
}

impl LocaleService for LinuxFileSystem {
    fn datetime_input_format(&self) -> Option<midnight_core::DateTimeInputFormat> {
        CURRENT.with(|value| input::capture(&value.as_ref().ok()?.time))
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
        // SAFETY: zero is valid for every tm field; populate a validated calendar.
        let mut calendar: libc::tm = unsafe { std::mem::zeroed() };
        calendar.tm_year = year - 1900;
        calendar.tm_mon = i32::from(month) - 1;
        calendar.tm_mday = i32::from(day);
        calendar.tm_hour = i32::from(hour);
        calendar.tm_min = i32::from(minute);
        calendar.tm_sec = i32::from(second);
        // SAFETY: live writable tm; GNU timegm fills weekday/year-day without
        // changing the process timezone. Its return value is not a source timestamp.
        unsafe { libc::timegm(&mut calendar) };
        format_calendar(&calendar)
    }

    fn collation_key(&self, value: &OsStr) -> Result<Option<Vec<u8>>, FsError> {
        crate::sort_keys::key(value)
    }

    fn number_locale(&self) -> Result<NumberLocale, FsError> {
        CURRENT.with(|value| {
            value
                .as_ref()
                .map_err(Clone::clone)?
                .numeric
                .numbers()
                .map_err(locale_error)
        })
    }
}

pub(crate) fn modified(time: SystemTime) -> Option<LocalizedDateTime> {
    let seconds = match time.duration_since(UNIX_EPOCH) {
        Ok(value) => i128::from(value.as_secs()),
        Err(error) => {
            -i128::from(error.duration().as_secs())
                - i128::from(error.duration().subsec_nanos() != 0)
        }
    };
    let seconds = libc::time_t::try_from(seconds).ok()?;
    // SAFETY: zero is a valid initial value for every field, overwritten by localtime_r.
    let mut calendar: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: valid input time_t and writable tm. localtime_r uses per-call storage and never
    // changes the process locale; timezone work happens only on the backend metadata worker.
    if unsafe { libc::localtime_r(&seconds, &mut calendar) }.is_null() {
        return None;
    }
    format_calendar(&calendar)
}

fn format_calendar(calendar: &libc::tm) -> Option<LocalizedDateTime> {
    CURRENT.with(|value| {
        let locale = &value.as_ref().ok()?.time;
        Some(LocalizedDateTime {
            date: locale.format(calendar, c"%x")?,
            time: locale.format(calendar, c"%X")?,
        })
    })
}

fn selected_locale(
    all: Option<&OsStr>,
    category: Option<&OsStr>,
    language: Option<&OsStr>,
) -> OsString {
    [all, category, language]
        .into_iter()
        .flatten()
        .find(|name| !name.is_empty())
        .unwrap_or(OsStr::new("C"))
        .to_owned()
}

fn grouping(bytes: &[u8]) -> (Vec<u8>, bool) {
    let mut widths = Vec::new();
    for &width in bytes {
        if width == 0 {
            let repeat = !widths.is_empty();
            return (widths, repeat);
        }
        if width >= 127 {
            return (widths, false);
        }
        widths.push(width);
    }
    (widths, false)
}

fn locale_error(error: io::Error) -> FsError {
    FsError::from_io("read Linux locale", Path::new(""), error)
}

fn decode_encoded(codeset: &CStr, bytes: &[u8]) -> io::Result<String> {
    if codeset.to_bytes().eq_ignore_ascii_case(b"UTF-8")
        || codeset.to_bytes().eq_ignore_ascii_case(b"UTF8")
    {
        return String::from_utf8(bytes.to_vec())
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error));
    }
    // SAFETY: both charset names are terminated strings; the result is owned only on success.
    let converter = unsafe { libc::iconv_open(c"UTF-8".as_ptr(), codeset.as_ptr()) };
    if converter as isize == -1 {
        return Err(io::Error::last_os_error());
    }
    struct Converter(libc::iconv_t);
    impl Drop for Converter {
        fn drop(&mut self) {
            // SAFETY: this is the sole owner of the successfully opened conversion descriptor.
            unsafe {
                libc::iconv_close(self.0);
            }
        }
    }
    let converter = Converter(converter);
    let capacity = bytes
        .len()
        .checked_mul(4)
        .and_then(|size| size.checked_add(32))
        .ok_or_else(|| io::Error::from(io::ErrorKind::OutOfMemory))?;
    let mut output = vec![0u8; capacity];
    let mut input_pointer = bytes.as_ptr().cast::<libc::c_char>().cast_mut();
    let mut input_remaining = bytes.len();
    let mut output_pointer = output.as_mut_ptr().cast::<libc::c_char>();
    let mut output_remaining = output.len();
    // SAFETY: iconv updates the input/output pointer variables and counts, reads input bytes
    // without modifying them, and writes only within the supplied output allocation. Every
    // Unicode scalar occupies at most four UTF-8 bytes per input byte, plus spare state space.
    let converted = unsafe {
        libc::iconv(
            converter.0,
            &mut input_pointer,
            &mut input_remaining,
            &mut output_pointer,
            &mut output_remaining,
        )
    };
    if converted == usize::MAX {
        return Err(io::Error::last_os_error());
    }
    output.truncate(capacity - output_remaining);
    String::from_utf8(output).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
#[path = "locale_tests.rs"]
mod tests;

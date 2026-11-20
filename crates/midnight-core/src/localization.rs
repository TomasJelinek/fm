use crate::FsError;

/// Immutable number preferences read on a worker. Formatting this snapshot does no OS I/O.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NumberLocale {
    pub decimal_separator: String,
    pub group_separator: String,
    /// Group widths from the least significant digits towards the most significant.
    pub grouping: Vec<u8>,
    pub repeat_last_group: bool,
}

impl Default for NumberLocale {
    fn default() -> Self {
        Self {
            decimal_separator: ".".into(),
            group_separator: String::new(),
            grouping: Vec::new(),
            repeat_last_group: false,
        }
    }
}

impl NumberLocale {
    /// Formats the complete integer without floating-point conversion or precision loss.
    pub fn format_u64(&self, value: u64) -> String {
        let digits = value.to_string();
        if self.group_separator.is_empty() || self.grouping.first().is_none_or(|size| *size == 0) {
            return digits;
        }
        let mut pieces = Vec::new();
        let mut end = digits.len();
        let mut group = 0usize;
        while end > 0 {
            let width = self.grouping.get(group).copied().or_else(|| {
                self.repeat_last_group
                    .then(|| self.grouping.last().copied())
                    .flatten()
            });
            let Some(width) = width.filter(|width| *width > 0) else {
                pieces.push(&digits[..end]);
                break;
            };
            let start = end.saturating_sub(usize::from(width));
            pieces.push(&digits[start..end]);
            end = start;
            group += 1;
        }
        pieces.reverse();
        pieces.join(&self.group_separator)
    }
}

/// Date and time stay separate because either localized field can contain spaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalizedDateTime {
    pub date: String,
    pub time: String,
}

/// Wall-clock Gregorian fields of one instant in the native timezone, with the UTC
/// offset in effect at that instant. Captured on a worker; editors use it only as an
/// explicit, editable starting value and never infer offsets for other instants.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LocalCalendarTime {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// The instant's fraction of its (floored) second, so the fields name it exactly.
    pub nanosecond: u32,
    /// Seconds east of UTC.
    pub offset_seconds: i32,
}

impl LocalCalendarTime {
    /// The fraction of an instant's second, counted from the floored second that
    /// native calendar conversions use for instants before 1970 too.
    pub fn nanosecond_of(instant: std::time::SystemTime) -> u32 {
        match instant.duration_since(std::time::UNIX_EPOCH) {
            Ok(after) => after.subsec_nanos(),
            Err(before) => (1_000_000_000 - before.duration().subsec_nanos()) % 1_000_000_000,
        }
    }

    /// Pairs native local fields with the instant they describe, deriving the offset
    /// from their difference. Rejects invalid fields and offsets of a day or more.
    #[allow(clippy::too_many_arguments)]
    pub fn from_local_fields(
        year: i32,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
        instant: std::time::SystemTime,
    ) -> Option<Self> {
        let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        let days_in_month = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if leap => 29,
            2 => 28,
            _ => return None,
        };
        if day == 0 || day > days_in_month || hour > 23 || minute > 59 || second > 59 {
            return None;
        }
        let utc = match instant.duration_since(std::time::UNIX_EPOCH) {
            Ok(value) => i64::try_from(value.as_secs()).ok()?,
            Err(error) => {
                let before = error.duration();
                -i64::try_from(before.as_secs()).ok()? - i64::from(before.subsec_nanos() != 0)
            }
        };
        let local = days_from_civil(i64::from(year), i64::from(month), i64::from(day))
            .checked_mul(86_400)?
            .checked_add(i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second))?;
        let offset = i32::try_from(local.checked_sub(utc)?).ok()?;
        (offset.abs() < 86_400).then_some(Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
            nanosecond: Self::nanosecond_of(instant),
            offset_seconds: offset,
        })
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Immutable Gregorian edit format captured on a worker. The numeric strftime
/// subset is `%Y`, `%m`, `%d`, `%H`, `%I`, `%M`, `%S`, `%p` and `%%` plus literals.
/// Years always have four digits. Editors require a separate explicit UTC offset;
/// this describes calendar text and must never imply a local timezone conversion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DateTimeInputFormat {
    pub pattern: String,
    pub am_pm: [String; 2],
}

/// Optional native locale discovery. Call only on a startup/backend worker.
pub trait LocaleService: Send + Sync {
    fn number_locale(&self) -> Result<NumberLocale, FsError>;

    /// Format an existing timestamp using native date, time and timezone preferences.
    /// Call only on a backend worker. None retains the caller's fallback display;
    /// formatting never requires reopening the file that supplied the timestamp.
    fn format_datetime(&self, _value: std::time::SystemTime) -> Option<LocalizedDateTime> {
        None
    }

    /// Native-timezone wall-clock fields and UTC offset of one instant. Call only on
    /// a backend worker. None leaves editors with an explicit UTC starting value.
    fn local_calendar(&self, _value: std::time::SystemTime) -> Option<LocalCalendarTime> {
        None
    }

    /// Capture an unambiguous numeric Gregorian edit format matching native field
    /// order, separators and clock preferences. Unsupported calendars/patterns
    /// return None so callers retain explicitly labelled ISO input.
    fn datetime_input_format(&self) -> Option<DateTimeInputFormat> {
        None
    }

    /// Format a timezone-unspecified archive calendar without shifting its fields.
    /// Only `ArchiveTimestamp::Local` is accepted. Run on a backend worker.
    fn format_archive_local_time(
        &self,
        _value: crate::ArchiveTimestamp,
    ) -> Option<LocalizedDateTime> {
        None
    }

    /// Produce an opaque, lexicographically comparable key using one immutable
    /// session locale. Call only on a backend worker, never in a sort comparator.
    /// None preserves an unrepresentable native name through deterministic raw
    /// ordering. A native transformation failure is an error, never truncation.
    fn collation_key(&self, _value: &std::ffi::OsStr) -> Result<Option<Vec<u8>>, FsError> {
        Ok(None)
    }
}

#[cfg(test)]
#[path = "localization_tests.rs"]
mod tests;

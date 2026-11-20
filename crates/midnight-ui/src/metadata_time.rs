//! Pure date, time and UTC-offset field logic for the metadata editor: parsing,
//! per-part arrow-key stepping and calendar views. No native timezone calls; the
//! worker supplies local starting values and the system date order, and every
//! offset stays explicit.
use chrono::{Datelike, Duration as Days, FixedOffset, Months, NaiveDate, NaiveTime, TimeZone};
use midnight_core::LocalCalendarTime;
use std::{
    ops::Range,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Which field of a timestamp a piece of text belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Field {
    Date,
    Time,
    Offset,
}

impl Field {
    pub(crate) fn from_index(index: i32) -> Option<Self> {
        match index {
            0 => Some(Self::Date),
            1 => Some(Self::Time),
            2 => Some(Self::Offset),
            _ => None,
        }
    }
}

const MAX_FIELD: usize = 64;
/// Captured native patterns are short; longer text is not a date format.
const MAX_PATTERN: usize = 128;
/// The date of format examples. Its day cannot be mistaken for a month.
const EXAMPLE: [i64; 3] = [2026, 11, 24];

/// The system's numeric date order and separators: the date part of the
/// worker-captured edit format, e.g. `%d.%m.%Y`. Dates typed this way are accepted
/// besides year-month-day.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LocaleDate {
    /// Parts in text order: 0 year, 1 month, 2 day.
    order: [usize; 3],
    /// Text before, between and after the parts; the separators are never empty, and
    /// no literal contains digits.
    literals: [String; 4],
}

impl LocaleDate {
    /// The date part of a captured pattern such as `%d. %m. %Y %H:%M:%S`: its year,
    /// month and day with the text between them, plus the text that ends the date
    /// before the next space or field (`日`, a final `.`). None unless the pattern
    /// starts with all three, once each and separated.
    pub(crate) fn from_pattern(pattern: &str) -> Option<Self> {
        if pattern.len() > MAX_PATTERN {
            return None;
        }
        let mut order = Vec::with_capacity(3);
        let mut literals = vec![String::new()];
        let mut chars = pattern.chars();
        while order.len() < 3 {
            match chars.next()? {
                '%' => {
                    let part = match chars.next()? {
                        '%' => {
                            literals.last_mut()?.push('%');
                            continue;
                        }
                        'Y' => 0,
                        'm' => 1,
                        'd' => 2,
                        // A time, a two-digit year or another field inside the date.
                        _ => return None,
                    };
                    if order.contains(&part) {
                        return None;
                    }
                    order.push(part);
                    literals.push(String::new());
                }
                ch => literals.last_mut()?.push(ch),
            }
        }
        *literals.last_mut()? = chars
            .take_while(|ch| !ch.is_whitespace() && *ch != '%')
            .collect();
        if literals[1..3].iter().any(String::is_empty)
            || literals
                .iter()
                .any(|literal| literal.contains(|ch: char| ch.is_ascii_digit()))
        {
            return None;
        }
        Some(Self {
            order: order.try_into().ok()?,
            literals: literals.try_into().ok()?,
        })
    }

    /// The date pattern alone, e.g. `%d. %m. %Y`.
    pub(crate) fn pattern(&self) -> String {
        let mut pattern = self.literals[0].replace('%', "%%");
        for (part, literal) in self.order.iter().zip(&self.literals[1..]) {
            pattern.push_str(["%Y", "%m", "%d"][*part]);
            pattern.push_str(&literal.replace('%', "%%"));
        }
        pattern
    }

    /// The example date in this order; None when that is year-month-day anyway.
    pub(crate) fn example(&self) -> Option<String> {
        let (text, _) = self.render(EXAMPLE);
        (text != iso_text(EXAMPLE)).then_some(text)
    }

    /// A date typed in this order. Spaces around the separators are optional, and
    /// so is the text before and after the date. Years have four digits, months and
    /// days one or two.
    fn parse(&self, text: &str) -> Option<NaiveDate> {
        let mut input = skip(text, &self.literals[0]).unwrap_or_else(|| text.trim_start());
        let mut values = [0u32; 3];
        for (index, part) in self.order.iter().enumerate() {
            if index > 0 {
                input = skip(input, &self.literals[index])?;
            }
            let digits = input.bytes().take_while(u8::is_ascii_digit).count();
            if (*part == 0 && digits != 4) || (*part != 0 && !(1..=2).contains(&digits)) {
                return None;
            }
            values[*part] = input[..digits].parse().ok()?;
            input = &input[digits..];
        }
        if !skip(input, &self.literals[3])
            .unwrap_or(input)
            .trim()
            .is_empty()
        {
            return None;
        }
        NaiveDate::from_ymd_opt(i32::try_from(values[0]).ok()?, values[1], values[2])
    }

    /// A date in this order, with the byte range of each part in text order.
    fn render(&self, values: [i64; 3]) -> (String, [Range<usize>; 3]) {
        let mut text = self.literals[0].clone();
        let mut ranges: [Range<usize>; 3] = Default::default();
        for (index, part) in self.order.iter().enumerate() {
            let start = text.len();
            if *part == 0 {
                text.push_str(&format!("{:04}", values[0]));
            } else {
                text.push_str(&format!("{:02}", values[*part]));
            }
            ranges[index] = start..text.len();
            text.push_str(&self.literals[index + 1]);
        }
        (text, ranges)
    }
}

/// `input` after `literal`, ignoring whitespace around and within the literal.
fn skip<'a>(input: &'a str, literal: &str) -> Option<&'a str> {
    let mut input = input.trim_start();
    for ch in literal.chars().filter(|ch| !ch.is_whitespace()) {
        input = input.strip_prefix(ch)?.trim_start();
    }
    Some(input)
}

fn iso_text(values: [i64; 3]) -> String {
    format!("{:04}-{:02}-{:02}", values[0], values[1], values[2])
}

/// A date in the system order when `style` is set, else as year-month-day, with the
/// byte range of each part in text order.
fn render_date(values: [i64; 3], style: Option<&LocaleDate>) -> (String, [Range<usize>; 3]) {
    match style {
        Some(locale) => locale.render(values),
        None => (iso_text(values), [0..4, 5..7, 8..10]),
    }
}

/// How a date can be typed, with examples, for hints and errors. Ends with a period.
pub(crate) fn date_forms(locale: Option<&LocaleDate>) -> String {
    let mut forms = format!("year-month-day, e.g. {}", iso_text(EXAMPLE));
    if let Some(example) = locale.and_then(LocaleDate::example) {
        forms.push_str(", or your system's format, e.g. ");
        forms.push_str(&example);
    }
    if !forms.ends_with('.') {
        forms.push('.');
    }
    forms
}

/// The editable starting value of one timestamp, as field texts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FieldTexts {
    pub(crate) date: String,
    pub(crate) time: String,
    pub(crate) offset: String,
}

pub(crate) fn texts(value: &LocalCalendarTime) -> FieldTexts {
    FieldTexts {
        date: format!("{:04}-{:02}-{:02}", value.year, value.month, value.day),
        time: format!("{:02}:{:02}:{:02}", value.hour, value.minute, value.second),
        offset: offset_text(value.offset_seconds / 60),
    }
}

/// The current instant in UTC, used when the worker supplied no local starting value.
pub(crate) fn utc_texts(now: SystemTime) -> FieldTexts {
    let seconds = now
        .duration_since(UNIX_EPOCH)
        .map_or(0, |value| i64::try_from(value.as_secs()).unwrap_or(0));
    chrono::Utc
        .timestamp_opt(seconds, 0)
        .single()
        .map(|value| FieldTexts {
            date: value.format("%Y-%m-%d").to_string(),
            time: value.format("%H:%M:%S").to_string(),
            offset: "+00:00".into(),
        })
        .unwrap_or_default()
}

fn offset_text(minutes: i32) -> String {
    let sign = if minutes < 0 { '-' } else { '+' };
    let minutes = minutes.unsigned_abs();
    format!("{sign}{:02}:{:02}", minutes / 60, minutes % 60)
}

/// Digit runs of a field with their byte ranges in the text.
fn runs(text: &str) -> Vec<Range<usize>> {
    let mut runs = Vec::new();
    let mut start = None;
    for (index, byte) in text.bytes().enumerate() {
        match (byte.is_ascii_digit(), start) {
            (true, None) => start = Some(index),
            (false, Some(begin)) => {
                runs.push(begin..index);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(begin) = start {
        runs.push(begin..text.len());
    }
    runs
}

fn number(text: &str, range: &Range<usize>) -> Option<u32> {
    text.get(range.clone())?.parse().ok()
}

/// A date typed as year-month-day or in the system order.
pub(crate) fn parse_date(text: &str, locale: Option<&LocaleDate>) -> Option<NaiveDate> {
    read_date(text, locale).map(|(date, _)| date)
}

/// A typed date and the system order it was typed in (None: year-month-day). The
/// system order is tried first, so its separators survive steps and picks.
fn read_date<'a>(
    text: &str,
    locale: Option<&'a LocaleDate>,
) -> Option<(NaiveDate, Option<&'a LocaleDate>)> {
    if text.len() > MAX_FIELD {
        return None;
    }
    if let Some(locale) = locale
        && let Some(date) = locale.parse(text)
    {
        return Some((date, Some(locale)));
    }
    parse_iso(text).map(|date| (date, None))
}

/// `YYYY-MM-DD`; `/` and `.` separators and one-digit months or days are accepted.
fn parse_iso(text: &str) -> Option<NaiveDate> {
    let text = text.trim();
    if text.len() > MAX_FIELD
        || !text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'-' | b'/' | b'.'))
    {
        return None;
    }
    let parts = runs(text);
    let [year, month, day] = parts.as_slice() else {
        return None;
    };
    if year.len() != 4 || month.len() > 2 || day.len() > 2 || year.start != 0 {
        return None;
    }
    if day.end != text.len() || text[year.end..month.start].len() != 1 {
        return None;
    }
    if text[year.end..month.start] != text[month.end..day.start] {
        return None;
    }
    NaiveDate::from_ymd_opt(
        i32::try_from(number(text, year)?).ok()?,
        number(text, month)?,
        number(text, day)?,
    )
}

/// Wall-clock time: `H:MM`, `H:MM:SS` or `H:MM:SS.fraction` (24-hour).
pub(crate) fn parse_time(text: &str) -> Result<NaiveTime, TimeError> {
    let text = text.trim();
    if text.len() > MAX_FIELD {
        return Err(TimeError::Invalid);
    }
    let (clock, fraction) = text.split_once('.').unwrap_or((text, ""));
    let parts = clock.split(':').collect::<Vec<_>>();
    let digits = |part: &&str| {
        !part.is_empty() && part.len() <= 2 && part.bytes().all(|b| b.is_ascii_digit())
    };
    if !(2..=3).contains(&parts.len())
        || !parts.iter().all(digits)
        || parts[1..].iter().any(|part| part.len() != 2)
        || (!fraction.is_empty() && parts.len() != 3)
        || fraction.len() > 9
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || (text.contains('.') && fraction.is_empty())
    {
        return Err(TimeError::Invalid);
    }
    let field = |index: usize| parts.get(index).map_or(Ok(0), |part| part.parse::<u32>());
    let (hour, minute, second) = (
        field(0).map_err(|_| TimeError::Invalid)?,
        field(1).map_err(|_| TimeError::Invalid)?,
        field(2).map_err(|_| TimeError::Invalid)?,
    );
    if second == 60 {
        return Err(TimeError::LeapSecond);
    }
    let nanos = format!("{fraction:0<9}")
        .parse::<u32>()
        .map_err(|_| TimeError::Invalid)?;
    NaiveTime::from_hms_nano_opt(hour, minute, second, nanos).ok_or(TimeError::Invalid)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TimeError {
    Invalid,
    LeapSecond,
}

/// `+HH:MM`, `+HHMM`, `+H`, `Z` or `UTC`, optionally prefixed with `UTC`/`GMT`.
pub(crate) fn parse_offset(text: &str) -> Option<FixedOffset> {
    let text = text.trim();
    if text.len() > MAX_FIELD {
        return None;
    }
    let upper = text.to_ascii_uppercase();
    if matches!(upper.as_str(), "Z" | "UTC" | "GMT") {
        return FixedOffset::east_opt(0);
    }
    let rest = upper
        .strip_prefix("UTC")
        .or_else(|| upper.strip_prefix("GMT"))
        .unwrap_or(&upper)
        .trim_start();
    if rest.is_empty() {
        return None;
    }
    let (sign, body) = match rest.as_bytes()[0] {
        b'+' => (1, &rest[1..]),
        b'-' => (-1, &rest[1..]),
        _ => return None,
    };
    let (hours, minutes) = match body.split_once(':') {
        Some((hours, minutes)) => (hours, minutes),
        None if body.len() == 4 => body.split_at(2),
        None => (body, "00"),
    };
    if hours.is_empty()
        || hours.len() > 2
        || minutes.len() != 2
        || !hours
            .bytes()
            .chain(minutes.bytes())
            .all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let (hours, minutes) = (hours.parse::<i32>().ok()?, minutes.parse::<i32>().ok()?);
    if hours > 23 || minutes > 59 {
        return None;
    }
    FixedOffset::east_opt(sign * (hours * 3600 + minutes * 60))
}

/// One timestamp from its three field texts. The error names the field to fix.
pub(crate) fn instant(
    label: &str,
    date: &str,
    time: &str,
    offset: &str,
    locale: Option<&LocaleDate>,
) -> Result<SystemTime, String> {
    let date = parse_date(date, locale)
        .ok_or_else(|| format!("{label} date: use {}", date_forms(locale)))?;
    let time = parse_time(time).map_err(|error| match error {
        TimeError::LeapSecond => {
            "Leap seconds cannot be represented as a filesystem timestamp.".to_string()
        }
        TimeError::Invalid => format!("{label} time: use hours:minutes:seconds, e.g. 14:30:00."),
    })?;
    let offset = parse_offset(offset)
        .ok_or_else(|| format!("{label} UTC offset: use +HH:MM or -HH:MM, e.g. +01:00, or Z."))?;
    let value = offset
        .from_local_datetime(&date.and_time(time))
        .single()
        .ok_or_else(|| format!("{label}: the timestamp is outside the supported range."))?;
    let nanos = value.timestamp_subsec_nanos();
    let seconds = value.timestamp();
    if seconds >= 0 {
        UNIX_EPOCH.checked_add(Duration::new(seconds.unsigned_abs(), nanos))
    } else {
        UNIX_EPOCH
            .checked_sub(Duration::from_secs(seconds.unsigned_abs()))
            .and_then(|time| time.checked_add(Duration::from_nanos(u64::from(nanos))))
    }
    .ok_or_else(|| format!("{label}: the timestamp is outside the supported range."))
}

/// Index of the part under a byte cursor: the digit run containing or ending at the
/// cursor, else the next one. Offsets have hours (0) and minutes (1).
pub(crate) fn part_at(text: &str, cursor: usize) -> usize {
    let parts = runs(text);
    parts
        .iter()
        .position(|run| cursor >= run.start && cursor <= run.end)
        .or_else(|| parts.iter().position(|run| run.start > cursor))
        .unwrap_or(parts.len().saturating_sub(1))
}

/// Step one part by `delta`, wrapping inside its range as a date/time spinner does.
/// `part` counts the parts in text order, so in a date typed in the system order it
/// can be the day. Unparsable text is replaced by `fallback` without stepping.
/// Returns the normalized text, in the form it was typed in, and the byte range of
/// the stepped part, or None when nothing can be shown.
pub(crate) fn step(
    text: &str,
    field: Field,
    part: usize,
    delta: i32,
    fallback: &str,
    locale: Option<&LocaleDate>,
) -> Option<(String, Range<usize>)> {
    let (mut values, style, stepping) = match normalize(text, field, locale) {
        Some((values, style)) => (values, style, true),
        None => {
            let (values, style) = normalize(fallback, field, locale)?;
            (values, style, false)
        }
    };
    let part = part.min(match field {
        Field::Offset => 1,
        _ => 2,
    });
    if stepping {
        match field {
            Field::Date => {
                let wrap = |value: i64, low: i64, high: i64| {
                    low + (value - low + i64::from(delta)).rem_euclid(high - low + 1)
                };
                match style.map_or(part, |locale| locale.order[part]) {
                    0 => values[0] = (values[0] + i64::from(delta)).clamp(1, 9999),
                    1 => values[1] = wrap(values[1], 1, 12),
                    _ => values[2] = wrap(values[2], 1, days_in_month(values[0], values[1])),
                }
                values[2] = values[2].min(days_in_month(values[0], values[1]));
            }
            Field::Time => {
                let high = if part == 0 { 23 } else { 59 };
                values[part] = (values[part] + i64::from(delta)).rem_euclid(high + 1);
            }
            Field::Offset => {
                // Hours keep the minutes; minutes snap to quarter hours. UTC-12..UTC+14.
                let minutes = values[0];
                let next = match (part, delta.signum()) {
                    (0, _) => minutes + 60 * i64::from(delta),
                    (_, 1) => minutes.div_euclid(15) * 15 + 15 * i64::from(delta),
                    (_, -1) => (minutes + 14).div_euclid(15) * 15 + 15 * i64::from(delta),
                    _ => minutes,
                };
                values[0] = next.clamp(-12 * 60, 14 * 60);
            }
        }
    }
    Some(render(field, &values, text, part, style))
}

/// Numeric parts of a valid field: date `[year, month, day]` with the system order it
/// was typed in (None: year-month-day), time `[h, m, s]`, offset `[signed minutes]`.
fn normalize<'a>(
    text: &str,
    field: Field,
    locale: Option<&'a LocaleDate>,
) -> Option<(Vec<i64>, Option<&'a LocaleDate>)> {
    match field {
        Field::Date => read_date(text, locale).map(|(date, style)| {
            let values = vec![date.year().into(), date.month().into(), date.day().into()];
            (values, style)
        }),
        Field::Time => parse_time(text).ok().map(|time| {
            use chrono::Timelike;
            let values = vec![
                time.hour().into(),
                time.minute().into(),
                time.second().into(),
            ];
            (values, None)
        }),
        Field::Offset => {
            parse_offset(text).map(|offset| (vec![i64::from(offset.local_minus_utc() / 60)], None))
        }
    }
}

fn render(
    field: Field,
    values: &[i64],
    original: &str,
    part: usize,
    style: Option<&LocaleDate>,
) -> (String, Range<usize>) {
    match field {
        Field::Date => {
            let (text, ranges) = render_date([values[0], values[1], values[2]], style);
            let range = ranges[part].clone();
            (text, range)
        }
        Field::Time => {
            // A typed fraction survives stepping another part.
            let fraction = original
                .trim()
                .split_once('.')
                .map(|(_, fraction)| format!(".{fraction}"))
                .filter(|_| parse_time(original).is_ok())
                .unwrap_or_default();
            let text = format!(
                "{:02}:{:02}:{:02}{fraction}",
                values[0], values[1], values[2]
            );
            (text, [0..2, 3..5, 6..8][part].clone())
        }
        Field::Offset => {
            let minutes = i32::try_from(values[0]).unwrap_or_default();
            (offset_text(minutes), [1..3, 4..6][part].clone())
        }
    }
}

fn days_in_month(year: i64, month: i64) -> i64 {
    let (Ok(year), Ok(month)) = (i32::try_from(year), u32::try_from(month)) else {
        return 31;
    };
    NaiveDate::from_ymd_opt(year, month, 1)
        .and_then(|first| first.checked_add_months(Months::new(1)))
        .and_then(|next| next.pred_opt())
        .map_or(31, |last| i64::from(last.day()))
}

/// One calendar page of the date picker, weeks starting on Monday.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MonthView {
    pub(crate) title: String,
    pub(crate) year: i32,
    pub(crate) month: u32,
    /// Blank cells before day 1 (0 = Monday).
    pub(crate) leading: u32,
    pub(crate) days: u32,
    /// Day of the month the cursor date is on.
    pub(crate) day: u32,
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

pub(crate) fn month_view(
    cursor: &str,
    fallback: &str,
    locale: Option<&LocaleDate>,
) -> Option<MonthView> {
    let date = parse_date(cursor, locale).or_else(|| parse_date(fallback, locale))?;
    let first = date.with_day(1)?;
    Some(MonthView {
        title: format!("{} {}", MONTHS[date.month0() as usize], date.year()),
        year: date.year(),
        month: date.month(),
        leading: first.weekday().num_days_from_monday(),
        days: days_in_month(date.year().into(), date.month().into()) as u32,
        day: date.day(),
    })
}

/// Move a picker cursor date by days or months; months clamp to the last day. The
/// result is year-month-day; `restyle` gives a picked date the field's form.
pub(crate) fn shift(
    cursor: &str,
    fallback: &str,
    days: i32,
    months: i32,
    locale: Option<&LocaleDate>,
) -> Option<String> {
    let date = parse_date(cursor, locale).or_else(|| parse_date(fallback, locale))?;
    let date = date.checked_add_signed(Days::days(days.into()))?;
    let date = if months >= 0 {
        date.checked_add_months(Months::new(months.unsigned_abs()))?
    } else {
        date.checked_sub_months(Months::new(months.unsigned_abs()))?
    };
    (1..=9999)
        .contains(&date.year())
        .then(|| date.format("%Y-%m-%d").to_string())
}

pub(crate) fn compose(year: i32, month: i32, day: i32) -> Option<String> {
    let date = NaiveDate::from_ymd_opt(year, u32::try_from(month).ok()?, u32::try_from(day).ok()?)?;
    Some(date.format("%Y-%m-%d").to_string())
}

/// `date` written in the form of `like`: in the system order when `like` is typed
/// in it, else as year-month-day. A picked date keeps the form the field was typed in.
pub(crate) fn restyle(date: &str, like: &str, locale: Option<&LocaleDate>) -> Option<String> {
    let (date, _) = read_date(date, locale)?;
    let style = read_date(like, locale).and_then(|(_, style)| style);
    let values = [date.year().into(), date.month().into(), date.day().into()];
    Some(render_date(values, style).0)
}

/// The picker's displayed parts of a field, from its text or else its fallback. A
/// date's parts are its year, month and day, whatever order it was typed in.
pub(crate) fn parts(
    text: &str,
    field: Field,
    fallback: &str,
    locale: Option<&LocaleDate>,
) -> Vec<String> {
    let Some((values, _)) =
        normalize(text, field, locale).or_else(|| normalize(fallback, field, locale))
    else {
        return vec![];
    };
    match field {
        Field::Date => vec![
            format!("{:04}", values[0]),
            format!("{:02}", values[1]),
            format!("{:02}", values[2]),
        ],
        Field::Time => values.iter().map(|value| format!("{value:02}")).collect(),
        Field::Offset => {
            let text = offset_text(i32::try_from(values[0]).unwrap_or_default());
            vec![text[..3].to_string(), text[4..6].to_string()]
        }
    }
}

#[cfg(test)]
#[path = "metadata_time_tests.rs"]
mod tests;

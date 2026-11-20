//! Pure date, time and UTC-offset field logic for the metadata editor: parsing,
//! per-part arrow-key stepping and calendar views. No native timezone calls; the
//! worker supplies local starting values and the system's date and clock forms, and
//! every offset stays explicit.
use chrono::{Datelike, Duration as Days, FixedOffset, Months, NaiveDate, NaiveTime, TimeZone};
use midnight_core::LocalCalendarTime;
use std::{
    ops::{Range, RangeInclusive},
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
const EXAMPLE_DATE: [i64; 3] = [2026, 11, 24];
/// The time of format examples, in the afternoon so a 12-hour clock shows its marker.
const EXAMPLE_TIME: [i64; 3] = [14, 30, 0];

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

    /// The example date in this order; None when that is year-month-day anyway.
    pub(crate) fn example(&self) -> Option<String> {
        let (text, _) = self.render(EXAMPLE_DATE);
        (text != iso_text(EXAMPLE_DATE)).then_some(text)
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
            let width = if *part == 0 { 4..=4 } else { 1..=2 };
            (values[*part], input) = leading_number(input, width)?;
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

/// One part of the system's clock form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Clock {
    Hour,
    Minute,
    Second,
    Marker,
}

impl Clock {
    /// The part's index in steps and picker columns.
    fn index(self) -> usize {
        match self {
            Self::Hour => 0,
            Self::Minute => 1,
            Self::Second => 2,
            Self::Marker => 3,
        }
    }
}

/// The system's clock form: the time part of the worker-captured edit format, e.g.
/// `%I:%M:%S %p` with its AM/PM markers or `%H時%M分%S秒`. Times typed this way are
/// accepted besides the plain 24-hour form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LocaleTime {
    /// Parts in text order; hours, minutes and seconds come in that order.
    parts: Vec<Clock>,
    /// The text before each part, then the text after the last one. No digits, and
    /// never empty between two numbers.
    literals: Vec<String>,
    /// The AM and PM markers of a 12-hour clock.
    markers: Option<[String; 2]>,
}

impl LocaleTime {
    /// The time part of a captured pattern such as `%m/%d/%Y %I:%M:%S %p`: from its
    /// first time field to the end. None unless it has hours, minutes and seconds in
    /// that order, separated, and a marker exactly when the hours are on a 12-hour
    /// clock; the markers must then be distinct, non-empty and free of digits.
    pub(crate) fn from_pattern(pattern: &str, markers: [&str; 2]) -> Option<Self> {
        if pattern.len() > MAX_PATTERN {
            return None;
        }
        let mut parts = Vec::new();
        let mut literals = vec![String::new()];
        let mut twelve = false;
        let mut chars = pattern.chars();
        while let Some(ch) = chars.next() {
            if ch != '%' {
                if !parts.is_empty() {
                    literals.last_mut()?.push(ch);
                }
                continue;
            }
            let part = match chars.next()? {
                '%' => {
                    if !parts.is_empty() {
                        literals.last_mut()?.push('%');
                    }
                    continue;
                }
                // The date comes before the time.
                'Y' | 'm' | 'd' if parts.is_empty() => continue,
                token @ ('H' | 'I') => {
                    twelve = token == 'I';
                    Clock::Hour
                }
                'M' => Clock::Minute,
                'S' => Clock::Second,
                'p' => Clock::Marker,
                _ => return None,
            };
            if parts.contains(&part) {
                return None;
            }
            parts.push(part);
            literals.push(String::new());
        }
        let suffix = literals.last_mut()?;
        *suffix = suffix.trim_end().to_string();
        let position = |part| parts.iter().position(|candidate| *candidate == part);
        let (Some(hour), Some(minute), Some(second)) = (
            position(Clock::Hour),
            position(Clock::Minute),
            position(Clock::Second),
        ) else {
            return None;
        };
        let separated = parts
            .windows(2)
            .zip(&literals[1..])
            .all(|(pair, literal)| pair.contains(&Clock::Marker) || !literal.is_empty());
        if hour > minute
            || minute > second
            || twelve != parts.contains(&Clock::Marker)
            || !separated
            || literals
                .iter()
                .any(|literal| literal.contains(|ch: char| ch.is_ascii_digit()))
        {
            return None;
        }
        let markers = if twelve {
            let [am, pm] = markers.map(str::trim);
            if am.is_empty()
                || pm.is_empty()
                || am.to_lowercase() == pm.to_lowercase()
                || format!("{am}{pm}").contains(|ch: char| ch.is_ascii_digit())
            {
                return None;
            }
            Some([am.to_string(), pm.to_string()])
        } else {
            None
        };
        Some(Self {
            parts,
            literals,
            markers,
        })
    }

    /// The example time in this form; None when that is the plain 24-hour form.
    pub(crate) fn example(&self) -> Option<String> {
        let (text, _) = self.render(EXAMPLE_TIME);
        (text != clock_text(EXAMPLE_TIME)).then_some(text)
    }

    /// A time typed in this form: one- or two-digit hours, two-digit minutes, optional
    /// two-digit seconds and the marker in any letter case, with optional spaces
    /// around the separators. None when the text is not in this form; otherwise the
    /// time and the marker's byte range in `text`, or a leap-second error.
    fn parse(&self, text: &str) -> Option<Result<ClockReading, TimeError>> {
        let mut input = text.trim_start();
        let (mut hour, mut minute, mut second) = (0, 0, 0);
        let (mut pm, mut marker) = (None, None);
        for (index, part) in self.parts.iter().enumerate() {
            let rest = if index == 0 {
                Some(input)
            } else {
                skip(input, &self.literals[index])
            };
            match part {
                Clock::Hour => (hour, input) = leading_number(rest?, 1..=2)?,
                Clock::Minute => (minute, input) = leading_number(rest?, 2..=2)?,
                // Seconds may be left out together with the separator before them.
                Clock::Second => {
                    if let Some((value, after)) = rest.and_then(|rest| leading_number(rest, 2..=2))
                    {
                        (second, input) = (value, after);
                    }
                }
                Clock::Marker => {
                    let rest = rest?;
                    let (is_pm, length) = self.marker_at(rest)?;
                    let start = text.len() - rest.len();
                    (pm, marker) = (Some(is_pm), Some(start..start + length));
                    input = &rest[length..];
                }
            }
        }
        let rest = skip(input, self.literals.last()?).unwrap_or(input);
        if !rest.trim().is_empty() || minute > 59 || second > 60 {
            return None;
        }
        let hour = match pm {
            Some(pm) if (1..=12).contains(&hour) => hour % 12 + if pm { 12 } else { 0 },
            None if hour <= 23 => hour,
            _ => return None,
        };
        if second == 60 {
            return Some(Err(TimeError::LeapSecond));
        }
        Some(
            NaiveTime::from_hms_opt(hour, minute, second)
                .map(|time| ClockReading { time, marker })
                .ok_or(TimeError::Invalid),
        )
    }

    /// Whether `input` starts with the PM marker (true) or the AM one, in any letter
    /// case, and that marker's byte length in `input`.
    fn marker_at(&self, input: &str) -> Option<(bool, usize)> {
        self.markers
            .as_ref()?
            .iter()
            .enumerate()
            .filter_map(|(index, marker)| {
                let length = input
                    .char_indices()
                    .nth(marker.chars().count())
                    .map_or(input.len(), |(at, _)| at);
                (input[..length].to_lowercase() == marker.to_lowercase())
                    .then_some((index == 1, length))
            })
            .max_by_key(|(_, length)| *length)
    }

    /// A time `[hour, minute, second]` in this form, with the byte ranges of its hours,
    /// minutes, seconds and, on a 12-hour clock, marker.
    fn render(&self, values: [i64; 3]) -> (String, Vec<Range<usize>>) {
        let mut text = String::new();
        let mut ranges = vec![0..0; if self.markers.is_some() { 4 } else { 3 }];
        for (part, literal) in self.parts.iter().zip(&self.literals) {
            text.push_str(literal);
            let start = text.len();
            match (part, &self.markers) {
                (Clock::Hour, Some(_)) => {
                    text.push_str(&format!("{:02}", (values[0] + 11) % 12 + 1));
                }
                (Clock::Marker, Some(markers)) => {
                    text.push_str(&markers[usize::from(values[0] >= 12)]);
                }
                (Clock::Marker, None) => {}
                _ => text.push_str(&format!("{:02}", values[part.index()])),
            }
            ranges[part.index()] = start..text.len();
        }
        text.push_str(self.literals.last().map_or("", String::as_str));
        (text, ranges)
    }
}

/// A time read in the system's clock form, with the byte range of its marker.
struct ClockReading {
    time: NaiveTime,
    marker: Option<Range<usize>>,
}

/// The system's own date and clock forms, from the worker-captured edit format.
/// Dates and times typed in them are accepted besides year-month-day and the 24-hour
/// clock, and keep their form when stepped or picked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SystemForms {
    date: Option<LocaleDate>,
    time: Option<LocaleTime>,
}

impl SystemForms {
    /// From a captured edit pattern, e.g. `%m/%d/%Y %I:%M:%S %p`, and its AM and PM
    /// markers. Either form may be missing.
    pub(crate) fn new(pattern: &str, markers: [&str; 2]) -> Self {
        Self {
            date: LocaleDate::from_pattern(pattern),
            time: LocaleTime::from_pattern(pattern, markers),
        }
    }
}

/// The form a field's text was typed in: the system's date or clock form, or the
/// plain year-month-day, 24-hour or offset form.
#[derive(Clone, Copy)]
enum Typed<'a> {
    Plain,
    Date(&'a LocaleDate),
    Time(&'a LocaleTime),
}

/// How a time can be typed, with examples, for hints and errors. Ends with a period.
pub(crate) fn time_forms(forms: &SystemForms) -> String {
    with_system_example(
        format!("hours:minutes:seconds, e.g. {}", clock_text(EXAMPLE_TIME)),
        forms.time.as_ref().and_then(LocaleTime::example),
    )
}

/// The part under a byte cursor as `part_at` finds it, except that on or right after
/// the marker of a time typed on the system's 12-hour clock it is the marker (3).
pub(crate) fn part_under(text: &str, field: Field, cursor: usize, forms: &SystemForms) -> usize {
    if field == Field::Time
        && text.len() <= MAX_FIELD
        && let Some(time) = &forms.time
        && let Some(Ok(ClockReading {
            marker: Some(marker),
            ..
        })) = time.parse(text)
        && (marker.start..=marker.end).contains(&cursor)
    {
        return Clock::Marker.index();
    }
    part_at(text, cursor)
}

/// A number of `width` leading digits and the text after it.
fn leading_number(input: &str, width: RangeInclusive<usize>) -> Option<(u32, &str)> {
    let digits = input.bytes().take_while(u8::is_ascii_digit).count();
    if !width.contains(&digits) {
        return None;
    }
    Some((input[..digits].parse().ok()?, &input[digits..]))
}

fn clock_text(values: [i64; 3]) -> String {
    format!("{:02}:{:02}:{:02}", values[0], values[1], values[2])
}

/// A plain form's description followed by the system's example, ending with a period.
fn with_system_example(mut text: String, example: Option<String>) -> String {
    if let Some(example) = example {
        text.push_str(", or your system's format, e.g. ");
        text.push_str(&example);
    }
    if !text.ends_with('.') {
        text.push('.');
    }
    text
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

/// A date in the form it was typed in, with the byte range of each part in text order.
fn render_date(values: [i64; 3], typed: Typed) -> (String, [Range<usize>; 3]) {
    match typed {
        Typed::Date(order) => order.render(values),
        _ => (iso_text(values), [0..4, 5..7, 8..10]),
    }
}

/// How a date can be typed, with examples, for hints and errors. Ends with a period.
pub(crate) fn date_forms(forms: &SystemForms) -> String {
    with_system_example(
        format!("year-month-day, e.g. {}", iso_text(EXAMPLE_DATE)),
        forms.date.as_ref().and_then(LocaleDate::example),
    )
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
pub(crate) fn parse_date(text: &str, forms: &SystemForms) -> Option<NaiveDate> {
    read_date(text, forms).map(|(date, _)| date)
}

/// A typed date and the form it was typed in. The system order is tried first, so
/// its separators survive steps and picks.
fn read_date<'a>(text: &str, forms: &'a SystemForms) -> Option<(NaiveDate, Typed<'a>)> {
    if text.len() > MAX_FIELD {
        return None;
    }
    if let Some(order) = &forms.date
        && let Some(date) = order.parse(text)
    {
        return Some((date, Typed::Date(order)));
    }
    parse_iso(text).map(|date| (date, Typed::Plain))
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

/// A time typed on the 24-hour clock or in the system's clock form.
pub(crate) fn parse_time(text: &str, forms: &SystemForms) -> Result<NaiveTime, TimeError> {
    read_time(text, forms).map(|(time, _)| time)
}

/// A typed time and the form it was typed in. The system's clock form is tried
/// first, so its separators and markers survive steps and picks.
fn read_time<'a>(text: &str, forms: &'a SystemForms) -> Result<(NaiveTime, Typed<'a>), TimeError> {
    if text.len() > MAX_FIELD {
        return Err(TimeError::Invalid);
    }
    if let Some(clock) = &forms.time
        && let Some(time) = clock.parse(text)
    {
        return time.map(|reading| (reading.time, Typed::Time(clock)));
    }
    parse_clock(text).map(|time| (time, Typed::Plain))
}

/// Wall-clock time: `H:MM`, `H:MM:SS` or `H:MM:SS.fraction` (24-hour).
fn parse_clock(text: &str) -> Result<NaiveTime, TimeError> {
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
    forms: &SystemForms,
) -> Result<SystemTime, String> {
    let date = parse_date(date, forms)
        .ok_or_else(|| format!("{label} date: use {}", date_forms(forms)))?;
    let time = parse_time(time, forms).map_err(|error| match error {
        TimeError::LeapSecond => {
            "Leap seconds cannot be represented as a filesystem timestamp.".to_string()
        }
        TimeError::Invalid => format!("{label} time: use {}", time_forms(forms)),
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
/// can be the day; part 3 of a time on the system's 12-hour clock is its marker,
/// which any step switches between AM and PM. Unparsable text is replaced by
/// `fallback` without stepping.
/// Returns the normalized text, in the form it was typed in, and the byte range of
/// the stepped part, or None when nothing can be shown.
pub(crate) fn step(
    text: &str,
    field: Field,
    part: usize,
    delta: i32,
    fallback: &str,
    forms: &SystemForms,
) -> Option<(String, Range<usize>)> {
    let (mut values, typed, stepping) = match normalize(text, field, forms) {
        Some((values, typed)) => (values, typed, true),
        None => {
            let (values, typed) = normalize(fallback, field, forms)?;
            (values, typed, false)
        }
    };
    let part = part.min(match (field, typed) {
        (Field::Offset, _) => 1,
        (Field::Time, Typed::Time(clock)) if clock.markers.is_some() => 3,
        _ => 2,
    });
    if stepping {
        match field {
            Field::Date => {
                let wrap = |value: i64, low: i64, high: i64| {
                    low + (value - low + i64::from(delta)).rem_euclid(high - low + 1)
                };
                let unit = match typed {
                    Typed::Date(order) => order.order[part],
                    _ => part,
                };
                match unit {
                    0 => values[0] = (values[0] + i64::from(delta)).clamp(1, 9999),
                    1 => values[1] = wrap(values[1], 1, 12),
                    _ => values[2] = wrap(values[2], 1, days_in_month(values[0], values[1])),
                }
                values[2] = values[2].min(days_in_month(values[0], values[1]));
            }
            Field::Time if part == Clock::Marker.index() => {
                if delta != 0 {
                    values[0] = (values[0] + 12) % 24;
                }
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
    Some(render(field, &values, text, part, typed))
}

/// Numeric parts of a valid field and the form it was typed in: date `[year, month,
/// day]`, time `[hour, minute, second]` on the 24-hour clock, offset `[minutes]`.
fn normalize<'a>(
    text: &str,
    field: Field,
    forms: &'a SystemForms,
) -> Option<(Vec<i64>, Typed<'a>)> {
    match field {
        Field::Date => read_date(text, forms).map(|(date, typed)| {
            let values = vec![date.year().into(), date.month().into(), date.day().into()];
            (values, typed)
        }),
        Field::Time => read_time(text, forms).ok().map(|(time, typed)| {
            use chrono::Timelike;
            let values = vec![
                time.hour().into(),
                time.minute().into(),
                time.second().into(),
            ];
            (values, typed)
        }),
        Field::Offset => parse_offset(text)
            .map(|offset| (vec![i64::from(offset.local_minus_utc() / 60)], Typed::Plain)),
    }
}

fn render(
    field: Field,
    values: &[i64],
    original: &str,
    part: usize,
    typed: Typed,
) -> (String, Range<usize>) {
    match (field, typed) {
        (Field::Date, _) => {
            let (text, ranges) = render_date([values[0], values[1], values[2]], typed);
            let range = ranges[part].clone();
            (text, range)
        }
        (Field::Time, Typed::Time(clock)) => {
            let (text, ranges) = clock.render([values[0], values[1], values[2]]);
            let range = ranges[part].clone();
            (text, range)
        }
        (Field::Time, _) => {
            // A typed fraction survives stepping another part.
            let fraction = original
                .trim()
                .split_once('.')
                .map(|(_, fraction)| format!(".{fraction}"))
                .filter(|_| parse_clock(original).is_ok())
                .unwrap_or_default();
            let text = clock_text([values[0], values[1], values[2]]) + &fraction;
            (text, [0..2, 3..5, 6..8][part].clone())
        }
        (Field::Offset, _) => {
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

pub(crate) fn month_view(cursor: &str, fallback: &str, forms: &SystemForms) -> Option<MonthView> {
    let date = parse_date(cursor, forms).or_else(|| parse_date(fallback, forms))?;
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
    forms: &SystemForms,
) -> Option<String> {
    let date = parse_date(cursor, forms).or_else(|| parse_date(fallback, forms))?;
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
pub(crate) fn restyle(date: &str, like: &str, forms: &SystemForms) -> Option<String> {
    let (date, _) = read_date(date, forms)?;
    let typed = read_date(like, forms).map_or(Typed::Plain, |(_, typed)| typed);
    let values = [date.year().into(), date.month().into(), date.day().into()];
    Some(render_date(values, typed).0)
}

/// The picker's displayed parts of a field, from its text or else its fallback. A
/// date's parts are its year, month and day, whatever order it was typed in; a time
/// on the system's 12-hour clock has its marker as a fourth part.
pub(crate) fn parts(text: &str, field: Field, fallback: &str, forms: &SystemForms) -> Vec<String> {
    let Some((values, typed)) =
        normalize(text, field, forms).or_else(|| normalize(fallback, field, forms))
    else {
        return vec![];
    };
    match (field, typed) {
        (Field::Date, _) => vec![
            format!("{:04}", values[0]),
            format!("{:02}", values[1]),
            format!("{:02}", values[2]),
        ],
        (Field::Time, Typed::Time(clock)) => {
            let (text, ranges) = clock.render([values[0], values[1], values[2]]);
            ranges
                .into_iter()
                .map(|range| text[range].to_string())
                .collect()
        }
        (Field::Time, _) => values.iter().map(|value| format!("{value:02}")).collect(),
        (Field::Offset, _) => {
            let text = offset_text(i32::try_from(values[0]).unwrap_or_default());
            vec![text[..3].to_string(), text[4..6].to_string()]
        }
    }
}

#[cfg(test)]
#[path = "metadata_time_tests.rs"]
mod tests;

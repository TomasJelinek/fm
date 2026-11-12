//! Translate the native numeric Gregorian edit pattern without guessing date order.
use super::*;
use midnight_core::DateTimeInputFormat;
use windows_sys::Win32::Globalization::{LOCALE_ICALENDARTYPE, LOCALE_S1159, LOCALE_S2359};

pub(super) fn capture(locale: &WindowsLocale) -> Option<DateTimeInputFormat> {
    // Non-Gregorian year/era rules cannot be represented by the numeric editor.
    if !matches!(
        text(&locale_value(LOCALE_ICALENDARTYPE).ok()?)
            .ok()?
            .as_str(),
        "1" | "2"
    ) {
        return None;
    }
    let date = pattern(&text(&locale.date_pattern).ok()?, true)?;
    let time = pattern(&text(&locale.time_pattern).ok()?, false)?;
    let am_pm = [
        text(&locale_value(LOCALE_S1159).ok()?).ok()?,
        text(&locale_value(LOCALE_S2359).ok()?).ok()?,
    ];
    if time.contains("%p") && (am_pm[0].is_empty() || am_pm[1].is_empty() || am_pm[0] == am_pm[1]) {
        return None;
    }
    Some(DateTimeInputFormat {
        pattern: format!("{date} {time}"),
        am_pm,
    })
}

fn pattern(native: &str, date: bool) -> Option<String> {
    let mut input = native.chars().peekable();
    let mut output = String::new();
    let mut quoted = false;
    let mut fields = 0u8;
    while let Some(ch) = input.next() {
        if ch == '\'' {
            if input.peek() == Some(&'\'') {
                input.next();
                output.push('\'');
            } else {
                quoted = !quoted;
            }
            continue;
        }
        if quoted || !ch.is_ascii_alphabetic() {
            if ch == '%' {
                output.push('%');
            }
            output.push(ch);
            continue;
        }
        let mut count = 1;
        while input.peek() == Some(&ch) {
            input.next();
            count += 1;
        }
        let (token, flag) = match (date, ch, count) {
            (true, 'd', 1..=2) => ("%d", 1),
            (true, 'M', 1..=2) => ("%m", 2),
            (true, 'y', 1..=4) => ("%Y", 4),
            (false, 'H', 1..=2) => ("%H", 1),
            (false, 'h', 1..=2) => ("%I", 1),
            (false, 'm', 1..=2) => ("%M", 2),
            (false, 's', 1..=2) => ("%S", 4),
            // A single-letter native marker is not round-trip safe in all locales.
            (false, 't', 2) => ("%p", 8),
            _ => return None,
        };
        if fields & flag != 0 {
            return None;
        }
        fields |= flag;
        output.push_str(token);
    }
    if quoted
        || (date && fields != 7)
        || (!date && fields & 3 != 3)
        || (!date && output.contains("%I") && fields & 8 == 0)
    {
        return None;
    }
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_native_order_literals_and_unambiguous_four_digit_year() {
        assert_eq!(pattern("d. M. yyyy", true).as_deref(), Some("%d. %m. %Y"));
        assert_eq!(pattern("M/d/yy", true).as_deref(), Some("%m/%d/%Y"));
        assert_eq!(
            pattern("yyyy'年'M'月'd'日'", true).as_deref(),
            Some("%Y年%m月%d日")
        );
        assert_eq!(pattern("h:mm:ss tt", false).as_deref(), Some("%I:%M:%S %p"));
        assert_eq!(pattern("H:mm:ss", false).as_deref(), Some("%H:%M:%S"));
        assert!(pattern("dd MMM yyyy", true).is_none());
        assert!(pattern("h:mm t", false).is_none());
        assert!(pattern("HH:mm 'broken", false).is_none());
    }
}

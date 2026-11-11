//! Pure parsing of the worker-captured numeric locale format; no native locale calls.
use chrono::{DateTime, FixedOffset};
use midnight_core::DateTimeInputFormat;

pub(crate) fn example(format: Option<&DateTimeInputFormat>) -> String {
    let Some(format) = format else {
        return "2026-11-06T12:30:00+01:00".into();
    };
    let value =
        DateTime::parse_from_rfc3339("2026-11-06T12:30:00+01:00").expect("fixed timestamp example");
    let pattern = format
        .pattern
        .replace("%p", &format.am_pm[1].replace('%', "%%"));
    use std::fmt::Write;
    let mut output = String::new();
    if write!(&mut output, "{} +01:00", value.format(&pattern)).is_err() {
        return "2026-11-06T12:30:00+01:00".into();
    }
    output
}

pub(super) fn parse(text: &str, format: &DateTimeInputFormat) -> Option<DateTime<FixedOffset>> {
    let mut input = text.trim();
    let mut normalized = String::new();
    let mut pattern = format.pattern.trim().chars();
    while let Some(ch) = pattern.next() {
        if ch != '%' {
            if ch.is_whitespace() {
                let rest = input.trim_start();
                if rest.len() == input.len() {
                    return None;
                }
                input = rest;
                while pattern.as_str().starts_with(char::is_whitespace) {
                    pattern.next();
                }
            } else {
                input = input.strip_prefix(ch)?;
            }
            normalized.push(ch);
            continue;
        }
        match pattern.next()? {
            '%' => {
                input = input.strip_prefix('%')?;
                normalized.push('%');
            }
            'p' => {
                let (marker, english) = format
                    .am_pm
                    .iter()
                    .zip(["AM", "PM"])
                    .filter(|(marker, _)| !marker.is_empty() && input.starts_with(marker.as_str()))
                    .max_by_key(|(marker, _)| marker.len())?;
                input = &input[marker.len()..];
                normalized.push_str(english);
            }
            token @ ('Y' | 'm' | 'd' | 'H' | 'I' | 'M' | 'S') => {
                let count = input
                    .bytes()
                    .take_while(u8::is_ascii_digit)
                    .count()
                    .min(if token == 'Y' { 4 } else { 2 });
                if (token == 'Y' && count != 4) || (token != 'Y' && !(1..=2).contains(&count)) {
                    return None;
                }
                normalized.push_str(&input[..count]);
                input = &input[count..];
            }
            _ => return None,
        }
    }
    // Offsets are explicit and bounded; there is no local-time DST inference.
    let offset = input.strip_prefix(char::is_whitespace)?.trim();
    let offset = if offset == "Z" { "+00:00" } else { offset };
    let bytes = offset.as_bytes();
    if bytes.len() != 6
        || !matches!(bytes[0], b'+' | b'-')
        || bytes[3] != b':'
        || ![bytes[1], bytes[2], bytes[4], bytes[5]]
            .iter()
            .all(u8::is_ascii_digit)
    {
        return None;
    }
    normalized.push(' ');
    normalized.push_str(offset);
    DateTime::parse_from_str(&normalized, &format!("{} %:z", format.pattern.trim())).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn format(pattern: &str) -> DateTimeInputFormat {
        DateTimeInputFormat {
            pattern: pattern.into(),
            am_pm: ["dop.".into(), "odp.".into()],
        }
    }
    #[test]
    fn native_order_and_clock_round_trip_with_explicit_dst_offset() {
        let czech = format("%d. %m. %Y %H:%M:%S");
        assert_eq!(example(Some(&czech)), "06. 11. 2026 12:30:00 +01:00");
        let earlier = parse("25. 10. 2026 02:30:00 +02:00", &czech).unwrap();
        let later = parse("25. 10. 2026 02:30:00 +01:00", &czech).unwrap();
        assert_eq!(later.timestamp() - earlier.timestamp(), 3600);
        assert!(parse("25. 10. 2026 02:30:00", &czech).is_none());
        assert!(parse("30. 02. 2026 02:30:00 +01:00", &czech).is_none());
        assert!(parse("01. 02. 26 02:30:00 +01:00", &czech).is_none());
        assert!(parse("01. 02. 2026 02:30:00 +2500", &czech).is_none());
        let twelve = format("%m/%d/%Y %I:%M:%S %p");
        let value = parse("11/06/2026 12:30:00 odp. Z", &twelve).unwrap();
        assert_eq!(value.to_rfc3339(), "2026-11-06T12:30:00+00:00");
        assert_eq!(example(Some(&twelve)), "11/06/2026 12:30:00 odp. +01:00");
        assert!(parse("11/06/2026 12:30:00 AM Z", &twelve).is_none());
        let compact = format("%Y%m%d %H%M%S");
        assert_eq!(
            parse("20261106 123000 +01:00", &compact)
                .unwrap()
                .to_rfc3339(),
            "2026-11-06T12:30:00+01:00"
        );
        let minutes = format("%Y-%m-%d %H:%M");
        assert_eq!(
            parse("2026-11-06 12:30 +01:00", &minutes)
                .unwrap()
                .to_rfc3339(),
            "2026-11-06T12:30:00+01:00"
        );
    }
}

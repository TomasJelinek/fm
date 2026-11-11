//! Numeric edit format from the same captured LC_TIME locale as displayed timestamps.
use super::*;
use midnight_core::DateTimeInputFormat;

pub(super) fn capture(locale: &NativeLocale) -> Option<DateTimeInputFormat> {
    let date = pattern(&locale.text(libc::D_FMT).ok()?, locale, 0)?;
    let time = pattern(&locale.text(libc::T_FMT).ok()?, locale, 0)?;
    let combined = format!("{date} {time}");
    if !["%Y", "%m", "%d", "%M"]
        .iter()
        .all(|token| combined.contains(token))
        || !(time.contains("%H") || (time.contains("%I") && time.contains("%p")))
    {
        return None;
    }
    let am_pm = [
        locale.text(libc::AM_STR).ok()?,
        locale.text(libc::PM_STR).ok()?,
    ];
    if time.contains("%p") && (am_pm[0].is_empty() || am_pm[1].is_empty() || am_pm[0] == am_pm[1]) {
        return None;
    }
    Some(DateTimeInputFormat {
        pattern: combined,
        am_pm,
    })
}

fn pattern(native: &str, locale: &NativeLocale, depth: u8) -> Option<String> {
    if depth > 2 {
        return None;
    }
    let mut input = native.chars();
    let mut output = String::new();
    while let Some(ch) = input.next() {
        if ch != '%' {
            output.push(ch);
            continue;
        }
        let token = input.next()?;
        output.push_str(match token {
            'Y' | 'y' => "%Y",
            'm' => "%m",
            'd' | 'e' => "%d",
            'H' | 'k' => "%H",
            'I' | 'l' => "%I",
            'M' => "%M",
            'S' => "%S",
            'p' => "%p",
            '%' => "%%",
            'T' => "%H:%M:%S",
            'R' => "%H:%M",
            'D' => "%m/%d/%Y",
            'F' => "%Y-%m-%d",
            'n' | 't' => " ",
            'r' => {
                output.push_str(&pattern(
                    &locale.text(libc::T_FMT_AMPM).ok()?,
                    locale,
                    depth + 1,
                )?);
                continue;
            }
            _ => return None,
        });
    }
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captures_c_locale_with_explicit_four_digit_year_and_rejects_era_patterns() {
        let locale = NativeLocale::new(c"C").unwrap();
        let format = capture(&locale).unwrap();
        assert_eq!(format.pattern, "%m/%d/%Y %H:%M:%S");
        assert_eq!(
            pattern("%d.%m.%Y %T", &locale, 0).as_deref(),
            Some("%d.%m.%Y %H:%M:%S")
        );
        assert!(pattern("%Ex", &locale, 0).is_none());
        assert!(pattern("%d %B %Y", &locale, 0).is_none());
    }
}

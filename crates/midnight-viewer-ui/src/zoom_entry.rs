//! A zoom percentage typed into the toolbar: "150", "150%", "150 %", "1,5" or "1.5", with
//! the decimal separator of the user's number format. The value is always a percentage.
//! A value outside the viewer's range is clamped to it and reported; text that is no
//! number is refused and reported, never ignored.
use midnight_core::NumberLocale;
use std::{cell::RefCell, rc::Rc};

/// The zoom a typed text asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Entry {
    /// The percentage to apply; `limited` when the typed value lay outside the range.
    Value {
        percent: f64,
        limited: Option<Limit>,
    },
    /// No number: nothing is applied and the field stays open with a hint.
    Invalid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Limit {
    Minimum,
    Maximum,
}

/// Images and documents: one basis point (0.01%) to 6400%.
pub(crate) const RASTER_RANGE: (f64, f64) = (0.01, 6400.0);
/// The HTML viewer's layout zoom: 10% to 1000%.
pub(crate) const WEB_RANGE: (f64, f64) = (10.0, 1000.0);

type LocaleSource = Rc<dyn Fn() -> NumberLocale>;

thread_local! {
    // Viewer windows live on the UI thread; the application supplies its number format.
    static LOCALE: RefCell<Option<LocaleSource>> = const { RefCell::new(None) };
}

/// Use `source` for the number format of typed zoom values (`ViewerWindows::set_number_locale`).
pub(crate) fn set_locale(source: LocaleSource) {
    LOCALE.with(|slot| *slot.borrow_mut() = Some(source));
}

/// The user's number format, or the invariant one before the application supplied it.
pub(crate) fn locale() -> NumberLocale {
    LOCALE
        .with(|slot| slot.borrow().clone())
        .map_or_else(NumberLocale::default, |source| source())
}

fn separator_char(text: &str) -> Option<char> {
    let mut chars = text.chars();
    let first = chars.next()?;
    chars.next().is_none().then_some(first)
}

fn is_space(character: char) -> bool {
    character.is_whitespace() || matches!(character, '\u{a0}' | '\u{202f}' | '\u{2009}')
}

/// Read `text` as a percentage within `range`.
pub(crate) fn parse(text: &str, locale: &NumberLocale, range: (f64, f64)) -> Entry {
    let mut text = text.trim_matches(is_space);
    if let Some(stripped) = text.strip_suffix('%') {
        text = stripped.trim_end_matches(is_space);
    }
    let decimal = separator_char(&locale.decimal_separator).unwrap_or('.');
    let group = separator_char(&locale.group_separator).filter(|group| *group != decimal);
    let mut number = String::with_capacity(text.len());
    let mut separators = 0;
    for (index, character) in text.chars().enumerate() {
        match character {
            '0'..='9' => number.push(character),
            '-' | '+' | '\u{2212}' if index == 0 => {
                number.push(if character == '+' { '+' } else { '-' })
            }
            // The locale's own separator, and the other common one unless it groups digits.
            character
                if character == decimal
                    || (matches!(character, '.' | ',') && Some(character) != group) =>
            {
                separators += 1;
                number.push('.');
            }
            character if Some(character) == group || is_space(character) => {}
            _ => return Entry::Invalid,
        }
    }
    if separators > 1 || !number.chars().any(|character| character.is_ascii_digit()) {
        return Entry::Invalid;
    }
    let Ok(value) = number.parse::<f64>() else {
        return Entry::Invalid;
    };
    if !value.is_finite() {
        return Entry::Invalid;
    }
    if value < range.0 {
        Entry::Value {
            percent: range.0,
            limited: Some(Limit::Minimum),
        }
    } else if value > range.1 {
        Entry::Value {
            percent: range.1,
            limited: Some(Limit::Maximum),
        }
    } else {
        Entry::Value {
            percent: value,
            limited: None,
        }
    }
}

/// `percent` as the field shows it: at most `decimals` decimal places, without trailing
/// zeros, in the user's number format (no digit grouping, so typing over it stays easy).
pub(crate) fn format(percent: f64, decimals: usize, locale: &NumberLocale) -> String {
    let text = format!("{percent:.decimals$}");
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    };
    let decimal = if locale.decimal_separator.is_empty() {
        "."
    } else {
        &locale.decimal_separator
    };
    text.replace('.', decimal)
}

/// The hint shown beside the field after `entry` was applied or refused.
pub(crate) fn hint(entry: Entry, range: (f64, f64), locale: &NumberLocale) -> String {
    let show = |value: f64| format!("{}%", format(value, 2, locale));
    match entry {
        Entry::Invalid => format!(
            "Type a zoom percentage from {} to {}, for example {}",
            show(range.0),
            show(range.1),
            format(150.5, 1, locale)
        ),
        Entry::Value {
            limited: Some(Limit::Minimum),
            ..
        } => format!("The smallest zoom is {}", show(range.0)),
        Entry::Value {
            limited: Some(Limit::Maximum),
            ..
        } => format!("The largest zoom is {}", show(range.1)),
        Entry::Value { limited: None, .. } => String::new(),
    }
}

#[cfg(test)]
#[path = "zoom_entry_tests.rs"]
mod tests;

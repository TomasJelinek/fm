use super::*;

fn number_format(decimal: &str, group: &str) -> NumberLocale {
    NumberLocale {
        decimal_separator: decimal.into(),
        group_separator: group.into(),
        grouping: vec![3],
        repeat_last_group: true,
    }
}

fn value(percent: f64) -> Entry {
    Entry::Value {
        percent,
        limited: None,
    }
}

#[test]
fn typed_percentages_accept_a_sign_spaces_and_the_locales_separator() {
    let english = number_format(".", ",");
    for (text, expected) in [
        ("150", value(150.0)),
        ("150%", value(150.0)),
        (" 150 % ", value(150.0)),
        ("1.5", value(1.5)),
        ("206.7", value(206.7)),
        ("1,500", value(1500.0)),
        ("+75", value(75.0)),
        (".5", value(0.5)),
    ] {
        assert_eq!(parse(text, &english, RASTER_RANGE), expected, "{text}");
    }
}

#[test]
fn a_comma_locale_reads_its_decimal_comma_and_still_a_dot() {
    // Czech: decimal comma, non-breaking space between digit groups.
    let czech = number_format(",", "\u{a0}");
    for (text, expected) in [
        ("1,5", value(1.5)),
        ("1.5", value(1.5)),
        ("206,7 %", value(206.7)),
        ("1\u{a0}500", value(1500.0)),
        ("1 500,25", value(1500.25)),
    ] {
        assert_eq!(parse(text, &czech, RASTER_RANGE), expected, "{text}");
    }
    // German groups digits with the dot, so a dot is never a decimal separator there.
    let german = number_format(",", ".");
    assert_eq!(parse("1.500", &german, RASTER_RANGE), value(1500.0));
    assert_eq!(parse("1,5", &german, RASTER_RANGE), value(1.5));
}

#[test]
fn values_outside_the_range_are_clamped_and_reported() {
    let english = number_format(".", ",");
    assert_eq!(
        parse("9000", &english, RASTER_RANGE),
        Entry::Value {
            percent: 6400.0,
            limited: Some(Limit::Maximum)
        }
    );
    for text in ["0", "-5", "0.001"] {
        assert_eq!(
            parse(text, &english, RASTER_RANGE),
            Entry::Value {
                percent: 0.01,
                limited: Some(Limit::Minimum)
            },
            "{text}"
        );
    }
    assert_eq!(
        parse("5", &english, WEB_RANGE),
        Entry::Value {
            percent: 10.0,
            limited: Some(Limit::Minimum)
        }
    );
    assert_eq!(
        hint(
            parse("9000", &english, RASTER_RANGE),
            RASTER_RANGE,
            &english
        ),
        "The largest zoom is 6400%"
    );
    assert_eq!(
        hint(parse("0", &english, RASTER_RANGE), RASTER_RANGE, &english),
        "The smallest zoom is 0.01%"
    );
    assert!(hint(value(150.0), RASTER_RANGE, &english).is_empty());
}

#[test]
fn text_without_a_number_is_refused_with_a_hint_in_the_users_format() {
    let czech = number_format(",", "\u{a0}");
    for text in [
        "", "%", "abc", "1,5,5", "1.5.5", "12a", "--5", "5-", "∞", "1e3",
    ] {
        assert_eq!(parse(text, &czech, RASTER_RANGE), Entry::Invalid, "{text}");
    }
    assert_eq!(
        hint(Entry::Invalid, RASTER_RANGE, &czech),
        "Type a zoom percentage from 0,01% to 6400%, for example 150,5"
    );
}

#[test]
fn the_field_shows_the_zoom_in_the_users_format_without_trailing_zeros() {
    let czech = number_format(",", "\u{a0}");
    let english = number_format(".", ",");
    assert_eq!(format(206.66, 1, &czech), "206,7");
    assert_eq!(format(150.0, 1, &english), "150");
    assert_eq!(
        format(1500.0, 1, &czech),
        "1500",
        "no digit grouping in the field"
    );
    assert_eq!(format(99.96, 0, &english), "100");
    assert_eq!(format(0.01, 2, &english), "0.01");
    // What the field shows reads back as the same value.
    for percent in [0.01, 12.5, 206.7, 6400.0] {
        for locale in [&czech, &english] {
            assert_eq!(
                parse(&format(percent, 2, locale), locale, RASTER_RANGE),
                value(percent)
            );
        }
    }
}

use super::*;

fn seconds(value: SystemTime) -> i64 {
    match value.duration_since(UNIX_EPOCH) {
        Ok(after) => after.as_secs() as i64,
        Err(before) => -(before.duration().as_secs() as i64),
    }
}

#[test]
fn separate_date_time_and_offset_fields_combine_into_one_instant() {
    let value = instant("Modified", "1970-01-01", "01:00:01.125", "+01:00", &plain()).unwrap();
    assert_eq!(value, UNIX_EPOCH + Duration::from_millis(1125));
    for (date, time, offset) in [
        ("1970-1-1", "1:00:01", "+0100"),
        ("1970/01/01", "01:00:01", "UTC+1"),
        ("1970.01.01", "00:00:01", "Z"),
        ("1970-01-01", "00:00:01", "utc"),
    ] {
        assert_eq!(
            seconds(instant("Modified", date, time, offset, &plain()).unwrap()),
            1,
            "{date} {time} {offset}"
        );
    }
    assert_eq!(
        seconds(instant("Created", "1969-12-31", "19:00", "-05:00", &plain()).unwrap()),
        0,
        "seconds are optional"
    );
}

#[test]
fn repeated_daylight_saving_hour_is_distinguished_by_the_explicit_offset() {
    let earlier = instant("Modified", "2026-10-25", "02:30:00", "+02:00", &plain()).unwrap();
    let later = instant("Modified", "2026-10-25", "02:30:00", "+01:00", &plain()).unwrap();
    assert_eq!(
        later.duration_since(earlier).unwrap(),
        Duration::from_secs(3600)
    );
}

#[test]
fn each_invalid_field_is_named_in_its_error() {
    let date = instant("Modified", "2026-02-30", "12:00", "Z", &plain()).unwrap_err();
    assert!(date.starts_with("Modified date:"), "{date}");
    for bad in [
        "26-11-06",
        "2026-11",
        "2026-11-06-01",
        "2026-11/06",
        "x",
        "",
        "06.11.2026",
    ] {
        assert!(parse_date(bad, &plain()).is_none(), "{bad}");
    }
    let time = instant("Accessed", "2026-11-06", "24:00", "Z", &plain()).unwrap_err();
    assert!(time.starts_with("Accessed time:"), "{time}");
    for bad in [
        "12",
        "12:5",
        "12:60",
        "1:00:00.",
        "12:00.5",
        "",
        "12:00:00:00",
        "123:00",
    ] {
        assert!(parse_time(bad, &plain()).is_err(), "{bad}");
    }
    let leap = instant("Modified", "2016-12-31", "23:59:60", "Z", &plain()).unwrap_err();
    assert!(leap.contains("Leap seconds"), "{leap}");
    let offset = instant("Created", "2026-11-06", "12:00", "", &plain()).unwrap_err();
    assert!(offset.starts_with("Created UTC offset:"), "{offset}");
    for bad in ["", "01:00", "+24:00", "+01:60", "+1:5", "CET", "+", "UTC+"] {
        assert!(parse_offset(bad).is_none(), "{bad}");
    }
    assert!(parse_date(&"1".repeat(65), &plain()).is_none());
}

#[test]
fn arrow_keys_step_the_part_under_the_cursor_and_wrap_like_a_spinner() {
    let date = "2026-01-31";
    assert_eq!(part_at(date, 0), 0);
    assert_eq!(
        part_at(date, 4),
        0,
        "the cursor right after a part still edits it"
    );
    assert_eq!(part_at(date, 6), 1);
    assert_eq!(part_at(date, 10), 2);
    assert_eq!(
        step(date, Field::Date, 2, 1, "", &plain()).unwrap(),
        ("2026-01-01".into(), 8..10)
    );
    assert_eq!(
        step(date, Field::Date, 1, 1, "", &plain()).unwrap(),
        ("2026-02-28".into(), 5..7),
        "the day is clamped to the new month"
    );
    assert_eq!(
        step(date, Field::Date, 1, -1, "", &plain()).unwrap().0,
        "2026-12-31"
    );
    assert_eq!(
        step("2024-02-29", Field::Date, 0, 1, "", &plain())
            .unwrap()
            .0,
        "2025-02-28"
    );
    assert_eq!(
        step("2026-3-5", Field::Date, 2, 0, "", &plain()).unwrap().0,
        "2026-03-05"
    );

    assert_eq!(
        step("23:59:59", Field::Time, 0, 1, "", &plain()).unwrap(),
        ("00:59:59".into(), 0..2)
    );
    assert_eq!(
        step("23:59:59", Field::Time, 2, 1, "", &plain()).unwrap(),
        ("23:59:00".into(), 6..8)
    );
    assert_eq!(
        step("7:05", Field::Time, 1, -1, "", &plain()).unwrap(),
        ("07:04:00".into(), 3..5)
    );
    assert_eq!(
        step("12:00:00.5", Field::Time, 0, 1, "", &plain())
            .unwrap()
            .0,
        "13:00:00.5",
        "a typed fraction survives"
    );

    assert_eq!(
        step(
            "+01:00",
            Field::Offset,
            part_at("+01:00", 2),
            1,
            "",
            &plain()
        )
        .unwrap(),
        ("+02:00".into(), 1..3)
    );
    assert_eq!(
        step("+00:00", Field::Offset, 0, -1, "", &plain())
            .unwrap()
            .0,
        "-01:00"
    );
    assert_eq!(
        step("+05:30", Field::Offset, 1, 1, "", &plain()).unwrap(),
        ("+05:45".into(), 4..6)
    );
    assert_eq!(
        step("+05:20", Field::Offset, 1, -1, "", &plain())
            .unwrap()
            .0,
        "+05:15"
    );
    assert_eq!(
        step("+14:00", Field::Offset, 0, 1, "", &plain()).unwrap().0,
        "+14:00"
    );
    assert_eq!(
        step("Z", Field::Offset, 0, 0, "", &plain()).unwrap().0,
        "+00:00"
    );
}

#[test]
fn stepping_an_empty_or_invalid_field_starts_from_the_fallback() {
    assert_eq!(
        step("", Field::Date, 2, 1, "2026-11-06", &plain()).unwrap(),
        ("2026-11-06".into(), 8..10)
    );
    assert_eq!(
        step("garbage", Field::Time, 0, 1, "12:30:00", &plain())
            .unwrap()
            .0,
        "12:30:00"
    );
    assert!(step("", Field::Offset, 0, 1, "", &plain()).is_none());
}

#[test]
fn calendar_pages_start_on_monday_and_moves_clamp_to_month_ends() {
    let view = month_view("2026-11-06", "", &plain()).unwrap();
    assert_eq!(view.title, "November 2026");
    assert_eq!(
        (view.year, view.month, view.day, view.days),
        (2026, 11, 6, 30)
    );
    assert_eq!(view.leading, 6, "1 November 2026 is a Sunday");
    assert_eq!(month_view("", "2024-02-10", &plain()).unwrap().days, 29);
    assert!(month_view("", "", &plain()).is_none());
    assert_eq!(
        shift("2026-01-31", "", 0, 1, &plain()).unwrap(),
        "2026-02-28"
    );
    assert_eq!(
        shift("2026-03-01", "", -1, 0, &plain()).unwrap(),
        "2026-02-28"
    );
    assert_eq!(
        shift("2026-12-28", "", 7, 0, &plain()).unwrap(),
        "2027-01-04"
    );
    assert_eq!(
        shift("", "2026-11-06", 0, -1, &plain()).unwrap(),
        "2026-10-06"
    );
    assert!(shift("9999-12-31", "", 1, 0, &plain()).is_none());
    assert_eq!(compose(2026, 2, 3).unwrap(), "2026-02-03");
    assert!(compose(2026, 2, 30).is_none());
}

#[test]
fn starting_values_keep_the_worker_offset_and_utc_is_explicit() {
    let value = LocalCalendarTime {
        year: 2026,
        month: 7,
        day: 4,
        hour: 9,
        minute: 5,
        second: 0,
        offset_seconds: -(4 * 3600 + 30 * 60),
    };
    assert_eq!(
        texts(&value),
        FieldTexts {
            date: "2026-07-04".into(),
            time: "09:05:00".into(),
            offset: "-04:30".into(),
        }
    );
    let utc = utc_texts(UNIX_EPOCH + Duration::from_secs(86_400 + 3661));
    assert_eq!(
        (utc.date.as_str(), utc.time.as_str(), utc.offset.as_str()),
        ("1970-01-02", "01:01:01", "+00:00")
    );
    assert_eq!(
        parts("", Field::Time, "08:09:10", &plain()),
        ["08", "09", "10"]
    );
    assert_eq!(parts("-03:30", Field::Offset, "", &plain()), ["-03", "30"]);
}

/// No system forms: year-month-day and the 24-hour clock only.
fn plain() -> SystemForms {
    SystemForms::default()
}

/// The system forms of a captured edit pattern, with English AM/PM markers.
fn system(pattern: &str) -> SystemForms {
    SystemForms::new(pattern, ["AM", "PM"])
}

fn date_order(pattern: &str) -> LocaleDate {
    LocaleDate::from_pattern(pattern).unwrap()
}

#[test]
fn the_system_date_order_comes_from_the_date_part_of_the_captured_pattern() {
    // The time is not part of the date order; text that ends the date is.
    assert_eq!(date_order("%d. %m. %Y %H:%M:%S"), date_order("%d. %m. %Y"));
    assert_eq!(
        date_order("%Y年%m月%d日 %H:%M:%S"),
        date_order("%Y年%m月%d日")
    );
    assert_eq!(
        date_order("%Y. %m. %d. %H:%M:%S"),
        date_order("%Y. %m. %d.")
    );
    assert_ne!(date_order("%Y. %m. %d."), date_order("%Y. %m. %d"));
    assert_eq!(date_order("%m/%d/%Y %I:%M:%S %p"), date_order("%m/%d/%Y"));
    for pattern in [
        "",
        "%H:%M:%S",
        "%d.%m. %H:%M",
        "%d.%m.%d.%Y",
        "%d %H %m %Y",
        "%Y%m%d %H%M%S",
        "%d.%m.%y",
        "%d1%m1%Y",
    ] {
        assert!(LocaleDate::from_pattern(pattern).is_none(), "{pattern}");
    }
}

#[test]
fn dates_in_the_system_order_are_accepted_alongside_year_month_day() {
    let none = plain();
    let czech = system("%d. %m. %Y %H:%M:%S");
    let us = system("%m/%d/%Y %I:%M:%S %p");
    let hungarian = system("%Y. %m. %d. %H:%M:%S");
    let japanese = system("%Y年%m月%d日 %H:%M:%S");
    let november = NaiveDate::from_ymd_opt(2026, 11, 24);
    for (locale, text) in [
        (&czech, "24. 11. 2026"),
        (&czech, "24.11.2026"),
        (&czech, " 24 .11. 2026 "),
        (&czech, "2026-11-24"),
        (&us, "11/24/2026"),
        (&us, "2026/11/24"),
        (&hungarian, "2026. 11. 24."),
        (&hungarian, "2026. 11. 24"),
        (&japanese, "2026年11月24日"),
    ] {
        assert_eq!(parse_date(text, locale), november, "{text}");
    }
    assert_eq!(
        parse_date("6.1.2026", &czech),
        NaiveDate::from_ymd_opt(2026, 1, 6)
    );
    for (locale, text) in [
        (&czech, "24.11.26"),
        (&czech, "24/11/2026"),
        (&czech, "24.11.2026 12:00"),
        (&czech, "31.11.2026"),
        (&us, "24/11/2026"),
        (&none, "24.11.2026"),
    ] {
        assert_eq!(parse_date(text, locale), None, "{text}");
    }
    assert_eq!(
        seconds(instant("Modified", "24. 11. 2026", "12:00", "Z", &czech).unwrap()),
        1_795_521_600
    );
}

#[test]
fn a_date_error_shows_the_system_format_next_to_year_month_day() {
    let us = system("%m/%d/%Y %I:%M:%S %p");
    assert_eq!(
        instant("Modified", "24/11/2026", "12:00", "Z", &us).unwrap_err(),
        "Modified date: use year-month-day, e.g. 2026-11-24, or your system's format, e.g. 11/24/2026."
    );
    assert_eq!(
        date_order("%m/%d/%Y %I:%M:%S %p").example().as_deref(),
        Some("11/24/2026")
    );
    assert_eq!(
        instant("Created", "x", "12:00", "Z", &plain()).unwrap_err(),
        "Created date: use year-month-day, e.g. 2026-11-24."
    );
    // A system format that already reads year-month-day adds nothing to explain.
    let iso = system("%Y-%m-%d %H:%M:%S");
    assert_eq!(date_order("%Y-%m-%d %H:%M:%S").example(), None);
    assert_eq!(
        instant("Created", "x", "12:00", "Z", &iso).unwrap_err(),
        "Created date: use year-month-day, e.g. 2026-11-24."
    );
}

#[test]
fn arrows_and_the_calendar_keep_the_system_date_style() {
    let czech = system("%d. %m. %Y %H:%M:%S");
    let text = "6. 1. 2026";
    // In this order the first part is the day and the last one the year.
    assert_eq!(
        step(text, Field::Date, part_at(text, 0), 1, "", &czech).unwrap(),
        ("07. 01. 2026".into(), 0..2)
    );
    assert_eq!(
        step(text, Field::Date, part_at(text, 3), -1, "", &czech).unwrap(),
        ("06. 12. 2026".into(), 4..6)
    );
    assert_eq!(
        step(text, Field::Date, part_at(text, 10), 1, "", &czech).unwrap(),
        ("06. 01. 2027".into(), 8..12)
    );
    let us = system("%m/%d/%Y %I:%M:%S %p");
    assert_eq!(
        step("01/31/2026", Field::Date, 0, 1, "", &us).unwrap(),
        ("02/28/2026".into(), 0..2),
        "the day is clamped to the new month"
    );
    // Year-month-day text keeps its own style; unparsable text shows the fallback.
    assert_eq!(
        step("2026-01-06", Field::Date, 2, 1, "", &czech).unwrap(),
        ("2026-01-07".into(), 8..10)
    );
    assert_eq!(
        step("x", Field::Date, 0, 1, "2026-11-24", &czech).unwrap(),
        ("2026-11-24".into(), 0..4)
    );
    let view = month_view("24. 11. 2026", "", &czech).unwrap();
    assert_eq!((view.title.as_str(), view.day), ("November 2026", 24));
    assert_eq!(
        shift("24. 11. 2026", "", 1, 0, &czech).unwrap(),
        "2026-11-25"
    );
    // A picked date is written in the style of the field's current text.
    assert_eq!(
        restyle("2026-11-25", "24. 11. 2026", &czech).unwrap(),
        "25. 11. 2026"
    );
    assert_eq!(
        restyle("25. 11. 2026", "2026-11-24", &czech).unwrap(),
        "2026-11-25"
    );
    assert_eq!(restyle("2026-11-25", "", &czech).unwrap(), "2026-11-25");
    assert_eq!(
        restyle("2026-11-25", "24. 11. 2026", &plain()).unwrap(),
        "2026-11-25"
    );
    assert_eq!(
        parts("24. 11. 2026", Field::Date, "", &czech),
        ["2026", "11", "24"]
    );
}

#[test]
fn the_system_clock_comes_from_the_time_part_of_the_captured_pattern() {
    let us = LocaleTime::from_pattern("%m/%d/%Y %I:%M:%S %p", ["AM", "PM"]).unwrap();
    assert_eq!(us.example().as_deref(), Some("02:30:00 PM"));
    let korean = LocaleTime::from_pattern("%Y. %m. %d. %p %I:%M:%S", ["오전", "오후"]).unwrap();
    assert_eq!(korean.example().as_deref(), Some("오후 02:30:00"));
    let japanese = LocaleTime::from_pattern("%Y年%m月%d日 %H時%M分%S秒", ["", ""]).unwrap();
    assert_eq!(japanese.example().as_deref(), Some("14時30分00秒"));
    // A 24-hour clock written like the plain one adds nothing to explain.
    let czech = LocaleTime::from_pattern("%d. %m. %Y %H:%M:%S", ["", ""]).unwrap();
    assert_eq!(czech.example(), None);
    for (pattern, markers) in [
        ("%m/%d/%Y %I:%M:%S", ["AM", "PM"]),
        ("%m/%d/%Y %H:%M:%S %p", ["AM", "PM"]),
        ("%m/%d/%Y %I:%M:%S %p", ["", "PM"]),
        ("%m/%d/%Y %I:%M:%S %p", ["am", "AM"]),
        ("%m/%d/%Y %I:%M:%S %p", ["1", "2"]),
        ("%m/%d/%Y %I:%M %p", ["AM", "PM"]),
        ("%m/%d/%Y %M:%H:%S", ["", ""]),
        ("%m/%d/%Y %H%M%S", ["", ""]),
        ("%H:%M:%S %d.%m.%Y", ["", ""]),
        ("%m/%d/%Y", ["", ""]),
    ] {
        assert!(
            LocaleTime::from_pattern(pattern, markers).is_none(),
            "{pattern} {markers:?}"
        );
    }
}

#[test]
fn times_on_the_system_clock_are_accepted_alongside_the_24_hour_clock() {
    let us = system("%m/%d/%Y %I:%M:%S %p");
    for (text, hour, minute, second) in [
        ("2:30:00 PM", 14, 30, 0),
        ("02:30 pm", 14, 30, 0),
        ("12:05:09 AM", 0, 5, 9),
        ("12:00 PM", 12, 0, 0),
        ("11:59:59PM", 23, 59, 59),
        ("14:30", 14, 30, 0),
        ("2:30", 2, 30, 0),
    ] {
        assert_eq!(
            parse_time(text, &us),
            NaiveTime::from_hms_opt(hour, minute, second).ok_or(TimeError::Invalid),
            "{text}"
        );
    }
    for text in [
        "13:00 PM",
        "0:30 AM",
        "2:3 PM",
        "2:30 XM",
        "2:30 PM later",
        "2:30:00.5 PM",
    ] {
        assert_eq!(parse_time(text, &us), Err(TimeError::Invalid), "{text}");
    }
    assert_eq!(parse_time("11:59:60 PM", &us), Err(TimeError::LeapSecond));
    let korean = SystemForms::new("%Y. %m. %d. %p %I:%M:%S", ["오전", "오후"]);
    assert_eq!(
        parse_time("오후 2:30:00", &korean),
        NaiveTime::from_hms_opt(14, 30, 0).ok_or(TimeError::Invalid)
    );
    // Without a system clock there are no markers to read.
    assert_eq!(parse_time("2:30 PM", &plain()), Err(TimeError::Invalid));
    assert_eq!(
        instant("Modified", "2026-11-24", "25:00", "Z", &us).unwrap_err(),
        "Modified time: use hours:minutes:seconds, e.g. 14:30:00, or your system's format, e.g. 02:30:00 PM."
    );
    assert_eq!(
        instant("Modified", "2026-11-24", "25:00", "Z", &plain()).unwrap_err(),
        "Modified time: use hours:minutes:seconds, e.g. 14:30:00."
    );
}

#[test]
fn arrows_and_the_picker_keep_the_system_clock() {
    let us = system("%m/%d/%Y %I:%M:%S %p");
    let text = "2:30 pm";
    // On or right after the marker, Up/Down switch between AM and PM.
    assert_eq!(part_under(text, Field::Time, 7, &us), 3);
    assert_eq!(part_under(text, Field::Time, 5, &us), 3);
    assert_eq!(part_under(text, Field::Time, 4, &us), 1);
    assert_eq!(part_under("14:30", Field::Time, 5, &us), 1);
    assert_eq!(
        step(text, Field::Time, 3, 1, "", &us).unwrap(),
        ("02:30:00 AM".into(), 9..11)
    );
    assert_eq!(
        step(text, Field::Time, 0, -1, "", &us).unwrap(),
        ("01:30:00 PM".into(), 0..2)
    );
    assert_eq!(
        step("11:59:00 PM", Field::Time, 0, 1, "", &us).unwrap().0,
        "12:59:00 AM"
    );
    assert_eq!(
        step("12:59:00 AM", Field::Time, 1, 1, "", &us).unwrap().0,
        "12:00:00 AM"
    );
    // 24-hour text keeps its form; there is no marker to step.
    assert_eq!(
        step("14:30", Field::Time, 3, 1, "", &us).unwrap(),
        ("14:30:01".into(), 6..8)
    );
    assert_eq!(parts(text, Field::Time, "", &us), ["02", "30", "00", "PM"]);
    assert_eq!(parts("14:30", Field::Time, "", &us), ["14", "30", "00"]);
    let korean = SystemForms::new("%Y. %m. %d. %p %I:%M:%S", ["오전", "오후"]);
    let text = "오후 2:30:00";
    assert_eq!(part_under(text, Field::Time, 0, &korean), 3);
    assert_eq!(part_under(text, Field::Time, 7, &korean), 0);
    assert_eq!(
        step(text, Field::Time, 3, -1, "", &korean).unwrap(),
        ("오전 02:30:00".into(), 0..6)
    );
}

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
    assert!(leap.starts_with("Modified time:"), "{leap}");
    assert!(leap.contains("leap second"), "{leap}");
    let offset = instant("Created", "2026-11-06", "12:00", "", &plain()).unwrap_err();
    assert!(offset.starts_with("Created UTC offset:"), "{offset}");
    for bad in ["", "01:00", "+24:00", "+01:60", "+1:5", "CET", "+", "UTC+"] {
        assert!(parse_offset(bad).is_none(), "{bad}");
    }
    assert!(parse_date(&"1".repeat(65), &plain()).is_none());
}

#[test]
fn non_ascii_offsets_are_invalid_instead_of_panicking() {
    // Four bytes after the sign, but byte 2 is inside a character.
    for bad in [
        "+1€",
        "+€1",
        "+😀",
        "UTC+1€",
        "gmt-€€",
        "−01:00",
        "+０１:００",
    ] {
        assert!(parse_offset(bad).is_none(), "{bad}");
        let error = instant("Modified", "2026-11-06", "12:00", bad, &plain()).unwrap_err();
        assert!(error.starts_with("Modified UTC offset:"), "{error}");
        // Up/Down on the field and the review both parse it.
        assert_eq!(
            step(bad, Field::Offset, part_at(bad, 2), 1, "+01:00", &plain()).unwrap(),
            ("+01:00".into(), 1..3),
            "{bad} is replaced by the fallback"
        );
        assert!(step(bad, Field::Offset, 1, -1, "", &plain()).is_none());
    }
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
fn picker_steps_on_an_empty_or_invalid_field_step_from_the_fallback() {
    assert_eq!(
        step_picker("", Field::Time, 0, 1, "03:04:05", &plain()).unwrap(),
        ("04:04:05".into(), 0..2),
        "the first click already changes the shown hour"
    );
    assert_eq!(
        step_picker("25:00", Field::Time, 2, -1, "03:04:05", &plain())
            .unwrap()
            .0,
        "03:04:04"
    );
    assert_eq!(
        step_picker("12:30:00", Field::Time, 1, 1, "03:04:05", &plain())
            .unwrap()
            .0,
        "12:31:00",
        "a valid field steps from its own value"
    );
    assert!(step_picker("", Field::Time, 0, 1, "", &plain()).is_none());
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
        nanosecond: 250_000_000,
        offset_seconds: -(4 * 3600 + 30 * 60),
    };
    assert_eq!(
        texts(&value),
        FieldTexts {
            date: "2026-07-04".into(),
            time: "09:05:00".into(),
            offset: "-04:30".into(),
        },
        "fields show whole seconds"
    );
    // 2026-07-04 13:35:00.25 UTC
    assert_eq!(
        exact(&value),
        Some(UNIX_EPOCH + Duration::from_millis(1_783_172_100_250))
    );
    let before_epoch = LocalCalendarTime {
        year: 1969,
        month: 12,
        day: 31,
        hour: 23,
        minute: 59,
        second: 59,
        nanosecond: 500_000_000,
        offset_seconds: 0,
    };
    assert_eq!(
        exact(&before_epoch),
        Some(UNIX_EPOCH - Duration::from_millis(500))
    );
    // Now at the worker's offset: the same instant, shown as local fields.
    let moment = UNIX_EPOCH + Duration::from_millis(1_783_172_100_250);
    let local = calendar_at(moment, value.offset_seconds).unwrap();
    assert_eq!(local, value);
    assert_eq!(exact(&local), Some(moment));
    assert_eq!(
        texts(&calendar_at(moment, 3600).unwrap()).time,
        "14:35:00",
        "13:35 UTC at +01:00"
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

/// What a key typed into a time picker column makes of `text`, and whether the column
/// waits for another digit.
fn typed(
    text: &str,
    part: usize,
    pending: &str,
    key: &str,
    forms: &SystemForms,
) -> (String, String, bool) {
    let typed = type_part(text, part, pending, key, "", forms).unwrap();
    (typed.text, typed.pending, typed.complete)
}

#[test]
fn digits_typed_into_a_picker_column_set_it_and_move_on_when_complete() {
    // 1 then 7: the hour shows 01, then 17, and the column is complete.
    assert_eq!(
        typed("03:04:05", 0, "", "1", &plain()),
        ("01:04:05".into(), "1".into(), false)
    );
    assert_eq!(
        type_part("01:04:05", 0, "1", "7", "", &plain()).unwrap(),
        TypedPart {
            text: "17:04:05".into(),
            range: 0..2,
            pending: String::new(),
            complete: true,
        }
    );
    // A digit no second digit could follow completes the column at once.
    assert_eq!(
        typed("03:04:05", 0, "", "5", &plain()),
        ("05:04:05".into(), String::new(), true)
    );
    assert_eq!(
        typed("03:04:05", 1, "", "6", &plain()),
        ("03:06:05".into(), String::new(), true)
    );
    assert_eq!(
        typed("03:04:05", 2, "", "5", &plain()),
        ("03:04:05".into(), "5".into(), false)
    );
    assert_eq!(
        typed("03:04:05", 2, "5", "9", &plain()),
        ("03:04:59".into(), String::new(), true)
    );
    // Two digits that are no hour start the column again with the second.
    assert_eq!(
        typed("03:04:05", 0, "2", "4", &plain()),
        ("04:04:05".into(), String::new(), true)
    );
    assert_eq!(
        typed("03:04:05", 0, "2", "3", &plain()),
        ("23:04:05".into(), String::new(), true)
    );
    // A typed fraction survives, an empty field starts from the fallback, and other
    // keys are not typing.
    assert_eq!(typed("12:00:00.5", 1, "", "7", &plain()).0, "12:07:00.5");
    assert_eq!(
        type_part("", 1, "", "8", "03:04:05", &plain())
            .unwrap()
            .text,
        "03:08:05"
    );
    for key in ["a", "", "12", "٣", " "] {
        assert!(
            type_part("03:04:05", 0, "", key, "", &plain()).is_none(),
            "{key:?}"
        );
    }
}

#[test]
fn a_12_hour_column_types_hours_of_its_half_day_and_the_marker_by_its_letter() {
    let us = system("%m/%d/%Y %I:%M:%S %p");
    assert_eq!(
        typed("02:30:00 PM", 0, "", "1", &us),
        ("01:30:00 PM".into(), "1".into(), false)
    );
    assert_eq!(
        typed("01:30:00 PM", 0, "1", "2", &us),
        ("12:30:00 PM".into(), String::new(), true)
    );
    assert_eq!(
        typed("02:30:00 AM", 0, "1", "1", &us),
        ("11:30:00 AM".into(), String::new(), true)
    );
    // 2 to 9 are complete hours; a lone 0 waits and changes nothing yet.
    assert_eq!(
        typed("02:30:00 PM", 0, "", "9", &us),
        ("09:30:00 PM".into(), String::new(), true)
    );
    assert_eq!(
        typed("02:30:00 PM", 0, "", "0", &us),
        ("02:30:00 PM".into(), "0".into(), false)
    );
    assert_eq!(
        typed("02:30:00 PM", 0, "0", "7", &us),
        ("07:30:00 PM".into(), String::new(), true)
    );
    assert_eq!(
        typed("02:30:00 PM", 0, "1", "3", &us),
        ("03:30:00 PM".into(), String::new(), true)
    );
    // The marker column takes the first letter of a marker, in any letter case.
    for key in ["a", "A"] {
        assert_eq!(
            type_part("02:30:00 PM", 3, "", key, "", &us).unwrap(),
            TypedPart {
                text: "02:30:00 AM".into(),
                range: 9..11,
                pending: String::new(),
                complete: true,
            }
        );
    }
    assert_eq!(typed("02:30:00 AM", 3, "", "p", &us).0, "02:30:00 PM");
    assert!(type_part("02:30:00 AM", 3, "", "x", "", &us).is_none());
    assert!(type_part("02:30:00 AM", 3, "", "5", "", &us).is_none());
    // Both Korean markers start with 오: that letter chooses neither.
    let korean = SystemForms::new("%Y. %m. %d. %p %I:%M:%S", ["오전", "오후"]);
    assert!(type_part("오후 2:30:00", 3, "", "오", "", &korean).is_none());
}

#[test]
fn home_and_end_give_a_picker_column_its_first_and_last_value() {
    for (part, last, wanted, range) in [
        (0, false, "00:34:56", 0..2),
        (0, true, "23:34:56", 0..2),
        (1, false, "12:00:56", 3..5),
        (1, true, "12:59:56", 3..5),
        (2, false, "12:34:00", 6..8),
        (2, true, "12:34:59", 6..8),
    ] {
        assert_eq!(
            edge_part("12:34:56", part, last, "", &plain()).unwrap(),
            (wanted.into(), range),
            "part {part}, last {last}"
        );
    }
    assert_eq!(
        edge_part("", 1, true, "03:04:05", &plain()).unwrap().0,
        "03:59:05"
    );
    let us = system("%m/%d/%Y %I:%M:%S %p");
    // The hours of a 12-hour clock run 12, 01 … 11 in their half of the day.
    assert_eq!(
        edge_part("02:30:00 PM", 0, false, "", &us).unwrap().0,
        "12:30:00 PM"
    );
    assert_eq!(
        edge_part("02:30:00 PM", 0, true, "", &us).unwrap().0,
        "11:30:00 PM"
    );
    assert_eq!(
        edge_part("02:30:00 AM", 0, false, "", &us).unwrap().0,
        "12:30:00 AM"
    );
    assert_eq!(
        edge_part("02:30:00 PM", 3, false, "", &us).unwrap().0,
        "02:30:00 AM"
    );
    assert_eq!(
        edge_part("02:30:00 AM", 3, true, "", &us).unwrap().0,
        "02:30:00 PM"
    );
    assert!(edge_part("", 0, true, "", &plain()).is_none());
}

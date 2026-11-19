use super::*;

fn seconds(value: SystemTime) -> i64 {
    match value.duration_since(UNIX_EPOCH) {
        Ok(after) => after.as_secs() as i64,
        Err(before) => -(before.duration().as_secs() as i64),
    }
}

#[test]
fn separate_date_time_and_offset_fields_combine_into_one_instant() {
    let value = instant("Modified", "1970-01-01", "01:00:01.125", "+01:00").unwrap();
    assert_eq!(value, UNIX_EPOCH + Duration::from_millis(1125));
    for (date, time, offset) in [
        ("1970-1-1", "1:00:01", "+0100"),
        ("1970/01/01", "01:00:01", "UTC+1"),
        ("1970.01.01", "00:00:01", "Z"),
        ("1970-01-01", "00:00:01", "utc"),
    ] {
        assert_eq!(
            seconds(instant("Modified", date, time, offset).unwrap()),
            1,
            "{date} {time} {offset}"
        );
    }
    assert_eq!(
        seconds(instant("Created", "1969-12-31", "19:00", "-05:00").unwrap()),
        0,
        "seconds are optional"
    );
}

#[test]
fn repeated_daylight_saving_hour_is_distinguished_by_the_explicit_offset() {
    let earlier = instant("Modified", "2026-10-25", "02:30:00", "+02:00").unwrap();
    let later = instant("Modified", "2026-10-25", "02:30:00", "+01:00").unwrap();
    assert_eq!(
        later.duration_since(earlier).unwrap(),
        Duration::from_secs(3600)
    );
}

#[test]
fn each_invalid_field_is_named_in_its_error() {
    let date = instant("Modified", "2026-02-30", "12:00", "Z").unwrap_err();
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
        assert!(parse_date(bad).is_none(), "{bad}");
    }
    let time = instant("Accessed", "2026-11-06", "24:00", "Z").unwrap_err();
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
        assert!(parse_time(bad).is_err(), "{bad}");
    }
    let leap = instant("Modified", "2016-12-31", "23:59:60", "Z").unwrap_err();
    assert!(leap.contains("Leap seconds"), "{leap}");
    let offset = instant("Created", "2026-11-06", "12:00", "").unwrap_err();
    assert!(offset.starts_with("Created UTC offset:"), "{offset}");
    for bad in ["", "01:00", "+24:00", "+01:60", "+1:5", "CET", "+", "UTC+"] {
        assert!(parse_offset(bad).is_none(), "{bad}");
    }
    assert!(parse_date(&"1".repeat(65)).is_none());
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
        step(date, Field::Date, 2, 1, "").unwrap(),
        ("2026-01-01".into(), 8..10)
    );
    assert_eq!(
        step(date, Field::Date, 1, 1, "").unwrap(),
        ("2026-02-28".into(), 5..7),
        "the day is clamped to the new month"
    );
    assert_eq!(step(date, Field::Date, 1, -1, "").unwrap().0, "2026-12-31");
    assert_eq!(
        step("2024-02-29", Field::Date, 0, 1, "").unwrap().0,
        "2025-02-28"
    );
    assert_eq!(
        step("2026-3-5", Field::Date, 2, 0, "").unwrap().0,
        "2026-03-05"
    );

    assert_eq!(
        step("23:59:59", Field::Time, 0, 1, "").unwrap(),
        ("00:59:59".into(), 0..2)
    );
    assert_eq!(
        step("23:59:59", Field::Time, 2, 1, "").unwrap(),
        ("23:59:00".into(), 6..8)
    );
    assert_eq!(
        step("7:05", Field::Time, 1, -1, "").unwrap(),
        ("07:04:00".into(), 3..5)
    );
    assert_eq!(
        step("12:00:00.5", Field::Time, 0, 1, "").unwrap().0,
        "13:00:00.5",
        "a typed fraction survives"
    );

    assert_eq!(
        step("+01:00", Field::Offset, part_at("+01:00", 2), 1, "").unwrap(),
        ("+02:00".into(), 1..3)
    );
    assert_eq!(
        step("+00:00", Field::Offset, 0, -1, "").unwrap().0,
        "-01:00"
    );
    assert_eq!(
        step("+05:30", Field::Offset, 1, 1, "").unwrap(),
        ("+05:45".into(), 4..6)
    );
    assert_eq!(
        step("+05:20", Field::Offset, 1, -1, "").unwrap().0,
        "+05:15"
    );
    assert_eq!(step("+14:00", Field::Offset, 0, 1, "").unwrap().0, "+14:00");
    assert_eq!(step("Z", Field::Offset, 0, 0, "").unwrap().0, "+00:00");
}

#[test]
fn stepping_an_empty_or_invalid_field_starts_from_the_fallback() {
    assert_eq!(
        step("", Field::Date, 2, 1, "2026-11-06").unwrap(),
        ("2026-11-06".into(), 8..10)
    );
    assert_eq!(
        step("garbage", Field::Time, 0, 1, "12:30:00").unwrap().0,
        "12:30:00"
    );
    assert!(step("", Field::Offset, 0, 1, "").is_none());
}

#[test]
fn calendar_pages_start_on_monday_and_moves_clamp_to_month_ends() {
    let view = month_view("2026-11-06", "").unwrap();
    assert_eq!(view.title, "November 2026");
    assert_eq!(
        (view.year, view.month, view.day, view.days),
        (2026, 11, 6, 30)
    );
    assert_eq!(view.leading, 6, "1 November 2026 is a Sunday");
    assert_eq!(month_view("", "2024-02-10").unwrap().days, 29);
    assert!(month_view("", "").is_none());
    assert_eq!(shift("2026-01-31", "", 0, 1).unwrap(), "2026-02-28");
    assert_eq!(shift("2026-03-01", "", -1, 0).unwrap(), "2026-02-28");
    assert_eq!(shift("2026-12-28", "", 7, 0).unwrap(), "2027-01-04");
    assert_eq!(shift("", "2026-11-06", 0, -1).unwrap(), "2026-10-06");
    assert!(shift("9999-12-31", "", 1, 0).is_none());
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
    assert_eq!(parts("", Field::Time, "08:09:10"), ["08", "09", "10"]);
    assert_eq!(parts("-03:30", Field::Offset, ""), ["-03", "30"]);
}

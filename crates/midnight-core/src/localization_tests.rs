use super::*;

#[test]
fn czech_narrow_spaces_and_indian_groups_preserve_all_u64_digits() {
    let czech = NumberLocale {
        decimal_separator: ",".into(),
        group_separator: "\u{a0}".into(),
        grouping: vec![3],
        repeat_last_group: true,
    };
    assert_eq!(
        czech.format_u64(u64::MAX),
        "18\u{a0}446\u{a0}744\u{a0}073\u{a0}709\u{a0}551\u{a0}615"
    );
    assert_eq!(czech.format_u64(1024), "1\u{a0}024");
    let indian = NumberLocale {
        group_separator: ",".into(),
        grouping: vec![3, 2],
        ..czech
    };
    assert_eq!(indian.format_u64(u64::MAX), "1,84,46,74,40,73,70,95,51,615");
    assert_eq!(indian.format_u64(123456789), "12,34,56,789");
}

#[test]
fn grouping_termination_small_values_and_empty_grouping_are_exact() {
    let finite = NumberLocale {
        group_separator: "_".into(),
        grouping: vec![3, 2],
        repeat_last_group: false,
        ..Default::default()
    };
    assert_eq!(finite.format_u64(123456789), "1234_56_789");
    assert_eq!(finite.format_u64(0), "0");
    assert_eq!(finite.format_u64(12), "12");
    assert_eq!(
        NumberLocale::default().format_u64(u64::MAX),
        "18446744073709551615"
    );
    let invalid = NumberLocale {
        grouping: vec![0],
        ..finite
    };
    assert_eq!(invalid.format_u64(12345), "12345");
}

#[test]
fn local_calendar_offset_is_the_difference_between_local_fields_and_the_instant() {
    use std::time::{Duration, UNIX_EPOCH};
    // 2026-10-25 00:30:00 UTC is 02:30 in both Central European offsets that day.
    let instant = UNIX_EPOCH + Duration::from_secs(1_792_888_200);
    let summer = LocalCalendarTime::from_local_fields(2026, 10, 25, 2, 30, 0, instant).unwrap();
    assert_eq!(summer.offset_seconds, 7200);
    let later = instant + Duration::from_secs(3600);
    let winter = LocalCalendarTime::from_local_fields(2026, 10, 25, 2, 30, 0, later).unwrap();
    assert_eq!(winter.offset_seconds, 3600);
    let before_epoch = UNIX_EPOCH - Duration::from_millis(500);
    let west = LocalCalendarTime::from_local_fields(1969, 12, 31, 18, 59, 59, before_epoch);
    assert_eq!(west.unwrap().offset_seconds, -5 * 3600);
    for (year, month, day, hour) in [(2026, 2, 29, 0), (2026, 13, 1, 0), (2026, 1, 1, 24)] {
        assert!(
            LocalCalendarTime::from_local_fields(year, month, day, hour, 0, 0, instant).is_none()
        );
    }
    assert!(
        LocalCalendarTime::from_local_fields(2026, 10, 27, 2, 30, 0, instant).is_none(),
        "an offset of a day or more is not a timezone"
    );
}

//! Calendar formatting is fallible independently of the native timestamp used for sorting.
use chrono::{DateTime, Datelike, FixedOffset, Local, TimeZone, Utc};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn modified(time: SystemTime) -> Option<String> {
    let (seconds, nanos) = match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => (i128::from(duration.as_secs()), duration.subsec_nanos()),
        Err(error) => {
            let duration = error.duration();
            let nanos = duration.subsec_nanos();
            (
                -i128::from(duration.as_secs()) - i128::from(nanos != 0),
                if nanos == 0 { 0 } else { 1_000_000_000 - nanos },
            )
        }
    };
    let utc = DateTime::from_timestamp(i64::try_from(seconds).ok()?, nanos)?;
    if !display_year(utc.year()) {
        return None;
    }
    // Chrono's host timezone lookup is infallible in its API but may panic on
    // an unsupported native date. Extreme timestamps are rejected above first.
    let offset =
        std::panic::catch_unwind(|| Local.offset_from_utc_datetime(&utc.naive_utc())).ok()?;
    in_offset(utc, offset)
}

fn in_offset(utc: DateTime<Utc>, offset: FixedOffset) -> Option<String> {
    if !display_year(utc.year()) {
        return None;
    }
    let local = utc.naive_utc().checked_add_offset(offset)?;
    display_year(local.year()).then(|| local.format("%Y-%m-%d %H:%M").to_string())
}

fn display_year(year: i32) -> bool {
    (1..=9999).contains(&year)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn local_offset_cannot_push_a_calendar_date_out_of_the_display_range() {
        use chrono::TimeZone;
        let last = Utc
            .with_ymd_and_hms(9999, 12, 31, 23, 30, 0)
            .single()
            .unwrap();
        let first = Utc.with_ymd_and_hms(1, 1, 1, 0, 30, 0).single().unwrap();
        assert_eq!(in_offset(last, FixedOffset::east_opt(3600).unwrap()), None);
        assert_eq!(in_offset(first, FixedOffset::west_opt(3600).unwrap()), None);
        assert_eq!(
            in_offset(DateTime::<Utc>::MAX_UTC, FixedOffset::east_opt(0).unwrap()),
            None
        );
        assert!(in_offset(last, FixedOffset::east_opt(0).unwrap()).is_some());
    }

    #[test]
    fn ordinary_before_and_after_epoch_dates_keep_the_existing_local_display() {
        for time in [
            UNIX_EPOCH + Duration::from_secs(1_700_000_000),
            UNIX_EPOCH - Duration::from_millis(500),
        ] {
            let expected: DateTime<Local> = time.into();
            assert_eq!(
                modified(time),
                Some(expected.format("%Y-%m-%d %H:%M").to_string())
            );
        }
    }
}

//! Calendar formatting is fallible independently of the native timestamp used for sorting.
use chrono::{DateTime, Datelike};
use midnight_core::{EntryMetadata, LocalizedDateTime};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn modified(metadata: &EntryMetadata) -> Option<LocalizedDateTime> {
    metadata
        .localized_modified
        .clone()
        .or_else(|| utc_fallback(metadata.modified?))
}

// Unavailable native formatting still has a deterministic, explicitly labelled fallback.
// Formatting an existing timestamp never queries the OS timezone or locale on the UI thread.
pub(crate) fn utc_fallback(time: SystemTime) -> Option<LocalizedDateTime> {
    let utc = utc_timestamp(time)?;
    Some(LocalizedDateTime {
        date: utc.format("%Y-%m-%d").to_string(),
        time: utc.format("%H:%M:%S UTC").to_string(),
    })
}

/// Conflict dialogs retain the captured fractional value and explicitly label UTC.
/// This does not infer a filesystem's timestamp resolution or query the OS.
pub(crate) fn utc_precise(time: SystemTime) -> Option<String> {
    Some(
        utc_timestamp(time)?
            .format("%Y-%m-%d %H:%M:%S%.f UTC")
            .to_string(),
    )
}

fn utc_timestamp(time: SystemTime) -> Option<DateTime<chrono::Utc>> {
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
    Some(utc)
}

fn display_year(year: i32) -> bool {
    (1..=9999).contains(&year)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn dates_outside_the_display_calendar_are_unknown() {
        assert_eq!(
            utc_fallback(UNIX_EPOCH + Duration::from_secs(500_000_000_000)),
            None
        );
    }

    #[test]
    fn conflict_utc_keeps_fractional_precision_and_rejects_out_of_calendar_values() {
        assert_eq!(
            utc_precise(UNIX_EPOCH + Duration::from_nanos(123_456_700)).as_deref(),
            Some("1970-01-01 00:00:00.123456700 UTC")
        );
        assert_eq!(
            utc_precise(UNIX_EPOCH - Duration::from_millis(500)).as_deref(),
            Some("1969-12-31 23:59:59.500 UTC")
        );
        assert!(utc_precise(UNIX_EPOCH + Duration::from_secs(500_000_000_000)).is_none());
    }

    #[test]
    fn fallback_before_and_after_epoch_is_pure_and_explicitly_utc() {
        for (time, date, clock) in [
            (
                UNIX_EPOCH + Duration::from_secs(1_700_000_000),
                "2023-11-14",
                "22:13:20 UTC",
            ),
            (
                UNIX_EPOCH - Duration::from_millis(500),
                "1969-12-31",
                "23:59:59 UTC",
            ),
        ] {
            assert_eq!(
                utc_fallback(time),
                Some(LocalizedDateTime {
                    date: date.into(),
                    time: clock.into()
                })
            );
        }
    }

    #[test]
    fn backend_date_and_time_remain_complete_even_when_both_contain_spaces() {
        let localized = LocalizedDateTime {
            date: "5. listopadu 2026".into(),
            time: "1:07 PM".into(),
        };
        let metadata = EntryMetadata {
            modified: Some(UNIX_EPOCH),
            localized_modified: Some(localized.clone()),
            ..Default::default()
        };
        assert_eq!(modified(&metadata), Some(localized));
    }
}

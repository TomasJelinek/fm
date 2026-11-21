//! Bounded form validation only; native inspection, time capture and edits are backend work.
use crate::{MetadataDraft, MetadataTimeDraft, metadata_time};
use midnight_core::*;

/// The reviewed patch of a draft. Dates and times may be typed in the system's forms.
/// `starting` holds the worker values the time fields started from (see
/// `metadata_presentation::set_time_defaults`): a field's own, else now. `now` holds
/// the moments that each row's Now button filled in (created, modified, accessed).
pub(crate) fn patch(
    draft: &MetadataDraft,
    starting: &MetadataTimeDefaults,
    now: &[Option<LocalCalendarTime>; 3],
    forms: &metadata_time::SystemForms,
) -> Result<MetadataPatch, String> {
    let mut edit = MetadataPatch {
        compressed: state(draft.compressed)?,
        created: timestamp(
            "Created",
            &draft.created,
            [starting.created.or(starting.now), now[0]],
            forms,
        )?,
        modified: timestamp(
            "Modified",
            &draft.modified,
            [starting.modified.or(starting.now), now[1]],
            forms,
        )?,
        accessed: timestamp(
            "Accessed",
            &draft.accessed,
            [starting.accessed.or(starting.now), now[2]],
            forms,
        )?,
        recursive: draft.recursive,
        ..Default::default()
    };
    for (choice, flag) in [
        (draft.read_only, EDIT_READ_ONLY),
        (draft.hidden, EDIT_HIDDEN),
        (draft.system, EDIT_SYSTEM),
        (draft.archive, EDIT_ARCHIVE),
    ] {
        if let Some(set) = state(choice)? {
            edit.windows_mask |= flag;
            if set {
                edit.windows_value |= flag;
            }
        }
    }
    (edit.unix_mask, edit.unix_value) =
        crate::unix_permissions::matrix_edit(draft.unix_mask, draft.unix_value)?;
    if draft.unix_edit {
        if edit.unix_mask != 0 {
            return Err("Choose either an octal permission edit or individual bits.".into());
        }
        (edit.unix_mask, edit.unix_value) = crate::unix_permissions::parse_octal(&draft.unix_mode)?;
    }
    if draft.case_enabled {
        let mode = match draft.case_mode {
            0 => CaseMode::Lower,
            1 => CaseMode::Upper,
            2 => CaseMode::Mixed,
            3 => CaseMode::MixedNameLowerExtension,
            _ => return Err("Choose a filename case conversion.".into()),
        };
        let scope = match draft.case_scope {
            0 => CaseScope::Whole,
            1 => CaseScope::Basename,
            2 => CaseScope::Extension,
            _ => return Err("Choose which part of the filename to change.".into()),
        };
        edit.case = Some(CaseChange { mode, scope });
    }
    Ok(edit)
}

fn state(value: i32) -> Result<Option<bool>, String> {
    match value {
        0 => Ok(None),
        1 => Ok(Some(true)),
        2 => Ok(Some(false)),
        _ => Err("Choose Leave unchanged, Set or Clear.".into()),
    }
}

/// A ticked row that still shows a value it was filled with (its starting value, or
/// the moment its Now button captured) keeps that exact instant: the fields show
/// whole seconds and a whole-minute offset, the captured value can be more precise.
/// Other dates and times may be typed in the system's `forms`.
fn timestamp(
    label: &str,
    time: &MetadataTimeDraft,
    filled: [Option<LocalCalendarTime>; 2],
    forms: &metadata_time::SystemForms,
) -> Result<TimeEdit, String> {
    match time.mode {
        0 => Ok(TimeEdit::Keep),
        2 => Ok(TimeEdit::Now),
        1 => {
            let unchanged = filled.into_iter().flatten().rev().find(|value| {
                let shown = metadata_time::texts(value);
                shown.date == time.date.as_str()
                    && shown.time == time.time.as_str()
                    && shown.offset == time.offset.as_str()
            });
            match unchanged.as_ref().and_then(metadata_time::exact) {
                Some(exact) => Ok(TimeEdit::Set(exact)),
                None => metadata_time::instant(label, &time.date, &time.time, &time.offset, forms)
                    .map(TimeEdit::Set),
            }
        }
        _ => Err("Choose a timestamp action.".into()),
    }
}

/// The UTC offsets typed for set times (created, modified, accessed), in seconds east
/// of UTC. The review compares them with the local offset at each resulting instant.
pub(crate) fn typed_offsets(draft: &MetadataDraft) -> [Option<i32>; 3] {
    [&draft.created, &draft.modified, &draft.accessed].map(|time| {
        if time.mode != 1 {
            return None;
        }
        metadata_time::parse_offset(&time.offset).map(|offset| offset.local_minus_utc())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    /// A draft without worker starting values or Now captures.
    fn patch(
        draft: &MetadataDraft,
        forms: &metadata_time::SystemForms,
    ) -> Result<MetadataPatch, String> {
        super::patch(draft, &MetadataTimeDefaults::default(), &[None; 3], forms)
    }

    #[test]
    fn opening_attributes_has_no_edits_and_case_has_explicit_opt_in() {
        assert!(
            patch(&MetadataDraft::default(), &plain())
                .unwrap()
                .is_empty()
        );
        let draft = MetadataDraft {
            case_enabled: true,
            ..Default::default()
        };
        assert_eq!(
            patch(&draft, &plain()).unwrap().case,
            Some(CaseChange::default())
        );
    }

    #[test]
    fn tri_state_attributes_and_compression_preserve_untouched_fields() {
        let draft = MetadataDraft {
            read_only: 1,
            hidden: 2,
            archive: 1,
            compressed: 2,
            ..Default::default()
        };
        let edit = patch(&draft, &plain()).unwrap();
        assert_eq!(
            edit.windows_mask,
            EDIT_READ_ONLY | EDIT_HIDDEN | EDIT_ARCHIVE
        );
        assert_eq!(edit.windows_value, EDIT_READ_ONLY | EDIT_ARCHIVE);
        assert_eq!(edit.compressed, Some(false));
        assert_eq!(edit.modified, TimeEdit::Keep);
    }

    #[test]
    fn unix_three_digit_modes_keep_special_bits_four_digits_edit_them() {
        let mut draft = MetadataDraft {
            unix_edit: true,
            unix_mode: "640".into(),
            ..Default::default()
        };
        let edit = patch(&draft, &plain()).unwrap();
        assert_eq!((edit.unix_mask, edit.unix_value), (0o777, 0o640));
        draft.unix_mode = "2750".into();
        let edit = patch(&draft, &plain()).unwrap();
        assert_eq!((edit.unix_mask, edit.unix_value), (0o7777, 0o2750));
        for invalid in ["", "75", "888", "10000", "-755", "0x755"] {
            draft.unix_mode = invalid.into();
            assert!(patch(&draft, &plain()).is_err(), "{invalid}");
        }
    }

    #[test]
    fn unix_matrix_changes_only_requested_bits_and_rejects_ambiguous_edits() {
        let mut draft = MetadataDraft {
            unix_mask: 0o220,
            unix_value: 0o020,
            ..Default::default()
        };
        let edit = patch(&draft, &plain()).unwrap();
        assert_eq!((edit.unix_mask, edit.unix_value), (0o220, 0o020));
        draft.unix_edit = true;
        draft.unix_mode = "755".into();
        assert!(patch(&draft, &plain()).is_err());
        draft.unix_edit = false;
        for (mask, value) in [(-1, 0), (0o10000, 0), (0, 1), (0o7777, -1)] {
            draft.unix_mask = mask;
            draft.unix_value = value;
            assert!(patch(&draft, &plain()).is_err());
        }
    }

    fn time(mode: i32, date: &str, time: &str, offset: &str) -> MetadataTimeDraft {
        MetadataTimeDraft {
            mode,
            date: date.into(),
            time: time.into(),
            offset: offset.into(),
        }
    }

    #[test]
    fn typed_date_time_and_offset_become_the_patch_and_now_is_captured_by_backend() {
        let mut draft = MetadataDraft {
            modified: time(1, "2026-11-06", "12:30:15", "+01:00"),
            created: time(1, "1970-01-01", "01:00:01.125", "+01:00"),
            accessed: time(2, "", "", ""),
            ..Default::default()
        };
        let edit = patch(&draft, &plain()).unwrap();
        assert_eq!(
            edit.modified,
            TimeEdit::Set(UNIX_EPOCH + Duration::from_secs(1_793_964_615))
        );
        assert_eq!(
            edit.created,
            TimeEdit::Set(UNIX_EPOCH + Duration::from_millis(1125))
        );
        assert_eq!(edit.accessed, TimeEdit::Now);
        // Unchecked fields keep their native value whatever their text says.
        draft.modified.mode = 0;
        draft.modified.date = "not a date".into();
        assert_eq!(patch(&draft, &plain()).unwrap().modified, TimeEdit::Keep);
        draft.modified.mode = 1;
        assert!(
            patch(&draft, &plain())
                .unwrap_err()
                .starts_with("Modified date:")
        );
        draft.modified = time(1, "2026-10-25", "02:30:00", "");
        assert!(
            patch(&draft, &plain()).unwrap_err().contains("UTC offset"),
            "there is no guessed local offset"
        );
    }

    #[test]
    fn a_ticked_row_left_at_its_starting_value_keeps_the_exact_instant() {
        // 2026-07-15 10:00:00.789 at +02:00, as the worker captured it.
        let original = UNIX_EPOCH + Duration::from_millis(1_784_102_400_789);
        let starting = MetadataTimeDefaults {
            modified: Some(LocalCalendarTime {
                year: 2026,
                month: 7,
                day: 15,
                hour: 10,
                minute: 0,
                second: 0,
                nanosecond: 789_000_000,
                offset_seconds: 7200,
            }),
            ..Default::default()
        };
        let mut draft = MetadataDraft {
            modified: time(1, "2026-07-15", "10:00:00", "+02:00"),
            ..Default::default()
        };
        assert_eq!(
            super::patch(&draft, &starting, &[None; 3], &plain())
                .unwrap()
                .modified,
            TimeEdit::Set(original),
            "ticking without editing must not drop the sub-seconds"
        );
        draft.modified.date = "2026-07-16".into();
        assert_eq!(
            super::patch(&draft, &starting, &[None; 3], &plain())
                .unwrap()
                .modified,
            TimeEdit::Set(original + Duration::from_secs(86_400) - Duration::from_millis(789)),
            "an edited row is exactly what its fields say"
        );
        // A seconds-precision historical offset is shown truncated but kept exactly.
        let lmt = LocalCalendarTime {
            year: 1850,
            month: 1,
            day: 1,
            hour: 12,
            minute: 0,
            second: 0,
            nanosecond: 0,
            offset_seconds: 3464,
        };
        let starting = MetadataTimeDefaults {
            now: Some(lmt),
            ..Default::default()
        };
        draft.modified = time(1, "1850-01-01", "12:00:00", "+00:57");
        assert_eq!(
            super::patch(&draft, &starting, &[None; 3], &plain())
                .unwrap()
                .modified,
            TimeEdit::Set(metadata_time::exact(&lmt).unwrap()),
            "a field without its own value starts from now"
        );
        assert_ne!(
            patch(&draft, &plain()).unwrap().modified,
            TimeEdit::Set(metadata_time::exact(&lmt).unwrap())
        );
    }

    #[test]
    fn a_row_filled_by_now_applies_the_captured_instant_until_it_is_edited() {
        let moment = UNIX_EPOCH + Duration::from_nanos(1_795_000_000_123_456_789);
        let captured = metadata_time::calendar_at(moment, 3600).unwrap();
        let shown = metadata_time::texts(&captured);
        let mut draft = MetadataDraft {
            accessed: time(1, &shown.date, &shown.time, &shown.offset),
            ..Default::default()
        };
        let now = [None, None, Some(captured)];
        assert_eq!(
            super::patch(&draft, &MetadataTimeDefaults::default(), &now, &plain())
                .unwrap()
                .accessed,
            TimeEdit::Set(moment),
            "the moment Now captured, sub-seconds included"
        );
        // Another row's capture does not apply, and an edit applies what it shows.
        assert_ne!(
            super::patch(
                &draft,
                &MetadataTimeDefaults::default(),
                &[Some(captured), None, None],
                &plain()
            )
            .unwrap()
            .accessed,
            TimeEdit::Set(moment)
        );
        draft.accessed.time = "00:00:00".into();
        assert_ne!(
            super::patch(&draft, &MetadataTimeDefaults::default(), &now, &plain())
                .unwrap()
                .accessed,
            TimeEdit::Set(moment)
        );
    }

    #[test]
    fn only_ticked_rows_are_applied_and_unticking_keeps_the_typed_value() {
        // The fields of an unticked row show values, starting or typed; none applies.
        let mut draft = MetadataDraft {
            modified: time(0, "2025-12-24", "18:30:00", "-05:00"),
            accessed: time(0, "2026-11-19", "10:00:00", "+01:00"),
            ..Default::default()
        };
        let edit = patch(&draft, &plain()).unwrap();
        assert_eq!(
            (edit.modified, edit.accessed),
            (TimeEdit::Keep, TimeEdit::Keep)
        );
        assert!(edit.is_empty());
        assert_eq!(typed_offsets(&draft), [None; 3]);
        // Ticked again, the row applies the value it kept.
        draft.modified.mode = 1;
        let edit = patch(&draft, &plain()).unwrap();
        assert_eq!(
            edit.modified,
            TimeEdit::Set(UNIX_EPOCH + Duration::from_secs(1_766_619_000))
        );
        assert_eq!(edit.accessed, TimeEdit::Keep);
        assert_eq!(typed_offsets(&draft), [None, Some(-5 * 3600), None]);
    }

    #[test]
    fn only_typed_rows_report_their_offset_for_the_review() {
        let draft = MetadataDraft {
            created: time(1, "2026-12-15", "10:00", "UTC+2"),
            modified: time(2, "", "", "+02:00"),
            accessed: time(0, "2026-12-15", "10:00", "-05:30"),
            ..Default::default()
        };
        assert_eq!(typed_offsets(&draft), [Some(7200), None, None]);
    }

    #[test]
    fn out_of_range_modes_and_oversized_fields_are_rejected() {
        for draft in [
            MetadataDraft {
                read_only: 3,
                ..Default::default()
            },
            MetadataDraft {
                modified: time(-1, "", "", ""),
                ..Default::default()
            },
            MetadataDraft {
                case_enabled: true,
                case_mode: 4,
                ..Default::default()
            },
            MetadataDraft {
                case_enabled: true,
                case_scope: 3,
                ..Default::default()
            },
            MetadataDraft {
                modified: time(1, &"1".repeat(4097), "12:00", "Z"),
                ..Default::default()
            },
        ] {
            assert!(patch(&draft, &plain()).is_err());
        }
    }

    #[test]
    fn dates_and_times_typed_in_the_system_forms_become_the_same_patch() {
        let us = metadata_time::SystemForms::new("%m/%d/%Y %I:%M:%S %p", ["AM", "PM"]);
        let draft = MetadataDraft {
            modified: time(1, "11/6/2026", "12:30:15 pm", "+01:00"),
            created: time(1, "2026-11-06", "12:30:15", "+01:00"),
            ..Default::default()
        };
        let edit = patch(&draft, &us).unwrap();
        let expected = TimeEdit::Set(UNIX_EPOCH + Duration::from_secs(1_793_964_615));
        assert_eq!((edit.modified, edit.created), (expected, expected));
        let error = patch(&draft, &plain()).unwrap_err();
        assert!(error.starts_with("Modified date:"), "{error}");
    }

    /// No system forms: year-month-day and the 24-hour clock only.
    fn plain() -> metadata_time::SystemForms {
        metadata_time::SystemForms::default()
    }
}

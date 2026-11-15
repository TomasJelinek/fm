//! Bounded form validation only; native inspection, time capture and edits are backend work.
use crate::MetadataDraft;
use chrono::DateTime;
use midnight_core::*;
use std::time::{Duration, UNIX_EPOCH};

#[path = "metadata_datetime.rs"]
mod datetime;
pub(crate) use datetime::example as timestamp_example;

pub(crate) fn patch(
    draft: &MetadataDraft,
    format: Option<&DateTimeInputFormat>,
) -> Result<MetadataPatch, String> {
    let mut edit = MetadataPatch {
        compressed: state(draft.compressed)?,
        created: timestamp(draft.created_mode, &draft.created_text, format)?,
        modified: timestamp(draft.modified_mode, &draft.modified_text, format)?,
        accessed: timestamp(draft.accessed_mode, &draft.accessed_text, format)?,
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

fn timestamp(
    mode: i32,
    text: &str,
    format: Option<&DateTimeInputFormat>,
) -> Result<TimeEdit, String> {
    match mode {
        0 => return Ok(TimeEdit::Keep),
        2 => return Ok(TimeEdit::Now),
        1 => {}
        _ => return Err("Choose a timestamp action.".into()),
    }
    if text.len() > 256 {
        return Err("Timestamp is too long.".into());
    }
    let value = DateTime::parse_from_rfc3339(text.trim()).ok()
        .or_else(|| format.and_then(|format| datetime::parse(text, format)))
        .ok_or_else(|| format!("Use a four-digit year and explicit UTC offset, e.g. {}. ISO timestamps are also accepted.", timestamp_example(format)))?;
    let nanos = value.timestamp_subsec_nanos();
    if nanos >= 1_000_000_000 {
        return Err("Leap seconds cannot be represented as a filesystem timestamp.".into());
    }
    let seconds = value.timestamp();
    let instant = if seconds >= 0 {
        UNIX_EPOCH.checked_add(Duration::new(seconds as u64, nanos))
    } else {
        UNIX_EPOCH
            .checked_sub(Duration::from_secs(seconds.unsigned_abs()))
            .and_then(|time| time.checked_add(Duration::from_nanos(u64::from(nanos))))
    }
    .ok_or_else(|| "Timestamp is outside the supported range.".to_string())?;
    Ok(TimeEdit::Set(instant))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn opening_attributes_has_no_edits_and_case_has_explicit_opt_in() {
        assert!(patch(&MetadataDraft::default(), None).unwrap().is_empty());
        let draft = MetadataDraft {
            case_enabled: true,
            ..Default::default()
        };
        assert_eq!(
            patch(&draft, None).unwrap().case,
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
        let edit = patch(&draft, None).unwrap();
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
        let edit = patch(&draft, None).unwrap();
        assert_eq!((edit.unix_mask, edit.unix_value), (0o777, 0o640));
        draft.unix_mode = "2750".into();
        let edit = patch(&draft, None).unwrap();
        assert_eq!((edit.unix_mask, edit.unix_value), (0o7777, 0o2750));
        for invalid in ["", "75", "888", "10000", "-755", "0x755"] {
            draft.unix_mode = invalid.into();
            assert!(patch(&draft, None).is_err(), "{invalid}");
        }
    }

    #[test]
    fn unix_matrix_changes_only_requested_bits_and_rejects_ambiguous_edits() {
        let mut draft = MetadataDraft {
            unix_mask: 0o220,
            unix_value: 0o020,
            ..Default::default()
        };
        let edit = patch(&draft, None).unwrap();
        assert_eq!((edit.unix_mask, edit.unix_value), (0o220, 0o020));
        draft.unix_edit = true;
        draft.unix_mode = "755".into();
        assert!(patch(&draft, None).is_err());
        draft.unix_edit = false;
        for (mask, value) in [(-1, 0), (0o10000, 0), (0, 1), (0o7777, -1)] {
            draft.unix_mask = mask;
            draft.unix_value = value;
            assert!(patch(&draft, None).is_err());
        }
    }

    #[test]
    fn timestamps_require_explicit_offsets_and_current_is_captured_by_backend() {
        let mut draft = MetadataDraft {
            modified_mode: 1,
            modified_text: "1970-01-01T01:00:01.125+01:00".into(),
            accessed_mode: 2,
            ..Default::default()
        };
        let edit = patch(&draft, None).unwrap();
        assert_eq!(
            edit.modified,
            TimeEdit::Set(UNIX_EPOCH + Duration::from_millis(1125))
        );
        assert_eq!(edit.accessed, TimeEdit::Now);
        for invalid in [
            "2026-10-25 02:30:00",
            "2026-02-30T12:00:00Z",
            "2016-12-31T23:59:60Z",
        ] {
            draft.modified_text = invalid.into();
            assert!(patch(&draft, None).is_err(), "{invalid}");
        }
    }

    #[test]
    fn captured_locale_is_used_by_the_typed_patch_without_relaxing_time_validation() {
        let format = DateTimeInputFormat {
            pattern: "%d. %m. %Y %H:%M:%S".into(),
            am_pm: ["".into(), "".into()],
        };
        let mut draft = MetadataDraft {
            modified_mode: 1,
            modified_text: "01. 01. 1970 01:00:01 +01:00".into(),
            ..Default::default()
        };
        assert_eq!(
            patch(&draft, Some(&format)).unwrap().modified,
            TimeEdit::Set(UNIX_EPOCH + Duration::from_secs(1))
        );
        assert!(
            patch(&draft, None).is_err(),
            "there is no guessed locale without a captured format"
        );
        draft.modified_text = "31. 12. 2016 23:59:60 Z".into();
        assert!(
            patch(&draft, Some(&format))
                .unwrap_err()
                .contains("Leap seconds")
        );
        draft.modified_text = "01. 01. 1970 01:00:01".into();
        assert!(patch(&draft, Some(&format)).is_err());
    }

    #[test]
    fn repeated_daylight_saving_hour_is_disambiguated_by_explicit_offset() {
        let TimeEdit::Set(earlier) = timestamp(1, "2026-10-25T02:30:00+02:00", None).unwrap()
        else {
            panic!("explicit timestamp")
        };
        let TimeEdit::Set(later) = timestamp(1, "2026-10-25T02:30:00+01:00", None).unwrap() else {
            panic!("explicit timestamp")
        };
        assert_eq!(
            later.duration_since(earlier).unwrap(),
            Duration::from_secs(3600)
        );
        assert!(timestamp(1, "2026-10-25T02:30:00", None).is_err());
    }

    #[test]
    fn out_of_range_modes_and_oversized_fields_are_rejected() {
        for draft in [
            MetadataDraft {
                read_only: 3,
                ..Default::default()
            },
            MetadataDraft {
                modified_mode: -1,
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
                modified_mode: 1,
                modified_text: "x".repeat(4097).into(),
                ..Default::default()
            },
        ] {
            assert!(patch(&draft, None).is_err());
        }
    }
}

//! Bounded form validation only; native inspection, time capture and edits are backend work.
use crate::MetadataDraft;
use chrono::DateTime;
use midnight_core::*;
use std::time::{Duration, UNIX_EPOCH};

pub(crate) fn patch(draft: &MetadataDraft) -> Result<MetadataPatch, String> {
    let mut edit = MetadataPatch {
        compressed: state(draft.compressed)?,
        created: timestamp(draft.created_mode, &draft.created_text)?,
        modified: timestamp(draft.modified_mode, &draft.modified_text)?,
        accessed: timestamp(draft.accessed_mode, &draft.accessed_text)?,
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
    if draft.unix_edit {
        let mode = draft.unix_mode.trim();
        if !(3..=4).contains(&mode.len()) || !mode.bytes().all(|byte| (b'0'..=b'7').contains(&byte))
        {
            return Err(
                "Permissions require three or four octal digits (for example 755 or 2755).".into(),
            );
        }
        edit.unix_mask = if mode.len() == 3 { 0o777 } else { 0o7777 };
        edit.unix_value = u32::from_str_radix(mode, 8).map_err(|_| "Invalid permissions.")?;
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

fn timestamp(mode: i32, text: &str) -> Result<TimeEdit, String> {
    match mode {
        0 => return Ok(TimeEdit::Keep),
        2 => return Ok(TimeEdit::Now),
        1 => {}
        _ => return Err("Choose a timestamp action.".into()),
    }
    if text.len() > 64 {
        return Err("Timestamp is too long.".into());
    }
    let value = DateTime::parse_from_rfc3339(text.trim()).map_err(|_| {
        "Use an ISO timestamp with an explicit timezone, e.g. 2026-11-06T12:30:00+01:00."
            .to_string()
    })?;
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
        assert!(patch(&MetadataDraft::default()).unwrap().is_empty());
        let draft = MetadataDraft {
            case_enabled: true,
            ..Default::default()
        };
        assert_eq!(patch(&draft).unwrap().case, Some(CaseChange::default()));
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
        let edit = patch(&draft).unwrap();
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
        let edit = patch(&draft).unwrap();
        assert_eq!((edit.unix_mask, edit.unix_value), (0o777, 0o640));
        draft.unix_mode = "2750".into();
        let edit = patch(&draft).unwrap();
        assert_eq!((edit.unix_mask, edit.unix_value), (0o7777, 0o2750));
        for invalid in ["", "75", "888", "10000", "-755", "0x755"] {
            draft.unix_mode = invalid.into();
            assert!(patch(&draft).is_err(), "{invalid}");
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
        let edit = patch(&draft).unwrap();
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
            assert!(patch(&draft).is_err(), "{invalid}");
        }
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
            assert!(patch(&draft).is_err());
        }
    }
}

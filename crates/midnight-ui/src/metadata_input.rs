//! Bounded form validation only; native inspection, time capture and edits are backend work.
use crate::{MetadataDraft, MetadataTimeDraft, metadata_time};
use midnight_core::*;

pub(crate) fn patch(draft: &MetadataDraft) -> Result<MetadataPatch, String> {
    let mut edit = MetadataPatch {
        compressed: state(draft.compressed)?,
        created: timestamp("Created", &draft.created)?,
        modified: timestamp("Modified", &draft.modified)?,
        accessed: timestamp("Accessed", &draft.accessed)?,
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

fn timestamp(label: &str, time: &MetadataTimeDraft) -> Result<TimeEdit, String> {
    match time.mode {
        0 => Ok(TimeEdit::Keep),
        2 => Ok(TimeEdit::Now),
        1 => metadata_time::instant(label, &time.date, &time.time, &time.offset).map(TimeEdit::Set),
        _ => Err("Choose a timestamp action.".into()),
    }
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
    fn unix_matrix_changes_only_requested_bits_and_rejects_ambiguous_edits() {
        let mut draft = MetadataDraft {
            unix_mask: 0o220,
            unix_value: 0o020,
            ..Default::default()
        };
        let edit = patch(&draft).unwrap();
        assert_eq!((edit.unix_mask, edit.unix_value), (0o220, 0o020));
        draft.unix_edit = true;
        draft.unix_mode = "755".into();
        assert!(patch(&draft).is_err());
        draft.unix_edit = false;
        for (mask, value) in [(-1, 0), (0o10000, 0), (0, 1), (0o7777, -1)] {
            draft.unix_mask = mask;
            draft.unix_value = value;
            assert!(patch(&draft).is_err());
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
        let edit = patch(&draft).unwrap();
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
        assert_eq!(patch(&draft).unwrap().modified, TimeEdit::Keep);
        draft.modified.mode = 1;
        assert!(patch(&draft).unwrap_err().starts_with("Modified date:"));
        draft.modified = time(1, "2026-10-25", "02:30:00", "");
        assert!(
            patch(&draft).unwrap_err().contains("UTC offset"),
            "there is no guessed local offset"
        );
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
            assert!(patch(&draft).is_err());
        }
    }
}

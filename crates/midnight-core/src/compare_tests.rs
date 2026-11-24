//! The comparison marks' details: packed losslessly beside the marks, merged when an
//! entry is told more, and charged to the same memory budget.
use super::*;
use std::ffi::OsStr;

fn marks() -> CompareMarks {
    CompareMarks::new([PathBuf::from("/l"), PathBuf::from("/r")], true)
}

fn every_number() -> CompareDetail {
    CompareDetail {
        flags: CompareFlags::TIME | CompareFlags::DST_SHIFT | CompareFlags::MODE,
        sizes: Some([u64::MAX, 0]),
        times: Some([-62_135_596_800, 253_402_300_799]),
        attributes: Some([0o4755, 0o644]),
        first_difference: Some(4_096),
        inside: Some(CompareInside {
            files: 3,
            folders: u32::MAX,
            only: [2, 0],
            unverified: 1,
        }),
    }
}

#[test]
fn every_flag_and_number_round_trips_through_the_packed_words() {
    let mut marks = marks();
    let key = OsStr::new("a/b");
    assert!(marks.record(key, None, CompareMark::Differs, &every_number()));
    assert_eq!(marks.detail(key), every_number());
    assert_eq!(
        marks.outcome(0, Path::new("/l/a/b")),
        Some((CompareMark::Differs, every_number()))
    );
    // Flags alone take no words.
    let flags = CompareDetail {
        flags: CompareFlags::KIND | CompareFlags::CASE | CompareFlags::LIMIT,
        ..CompareDetail::default()
    };
    marks.record(OsStr::new("c"), Some(1), CompareMark::Unverified, &flags);
    assert_eq!(marks.detail(OsStr::new("c")), flags);
    assert_eq!(
        marks.outcome(0, Path::new("/l/c")),
        None,
        "the left side is not stored"
    );
    assert_eq!(
        marks.outcome(1, Path::new("/r/c")),
        Some((CompareMark::Unverified, flags))
    );
}

#[test]
fn a_later_detail_is_merged_and_a_better_mark_never_replaces_a_worse_one() {
    let mut marks = marks();
    let key = OsStr::new("f");
    let newer = CompareDetail {
        flags: CompareFlags::TIME,
        times: Some([20, 10]),
        ..CompareDetail::default()
    };
    marks.record(key, None, CompareMark::Differs, &newer);
    // The contents of the newer file match: the mark stays, the detail keeps the time.
    marks.record(key, None, CompareMark::Same, &CompareDetail::default());
    let content = CompareDetail {
        first_difference: Some(7),
        ..CompareDetail::default()
    };
    marks.record(key, None, CompareMark::Differs, &content);
    assert_eq!(marks.stored(key, 0), Some(CompareMark::Differs));
    assert_eq!(
        marks.detail(key),
        CompareDetail {
            first_difference: Some(7),
            ..newer
        }
    );
    // Same numbers again, other values: rewritten in place, nothing more charged.
    let bytes = marks.bytes();
    marks.record(
        key,
        None,
        CompareMark::Differs,
        &CompareDetail {
            times: Some([30, 10]),
            first_difference: Some(9),
            ..CompareDetail::default()
        },
    );
    assert_eq!(marks.bytes(), bytes);
    assert_eq!(marks.detail(key).times, Some([30, 10]));
    assert_eq!(marks.detail(key).first_difference, Some(9));
}

/// The owner's question: what do the details cost? A map entry with its key is as
/// large as before (the flags and the arena index fill the marks' padding); only
/// an entry with numbers pays for them, eight bytes each.
#[test]
fn details_keep_the_entry_size_and_charge_only_their_numbers() {
    use std::mem::size_of;
    assert_eq!(size_of::<Stored>(), 8);
    assert_eq!(
        size_of::<(OsString, Stored)>(),
        size_of::<(OsString, [u8; 2])>(),
        "the map's bucket does not grow"
    );
    let mut marks = marks();
    marks.set(OsStr::new("same"), 0, CompareMark::Same);
    assert_eq!(marks.bytes(), 4 + CompareMarks::ENTRY_BYTES);
    let before = marks.bytes();
    let size_and_time = CompareDetail {
        flags: CompareFlags::TIME,
        sizes: Some([2, 1]),
        times: Some([20, 10]),
        ..CompareDetail::default()
    };
    marks.record(
        OsStr::new("diff"),
        None,
        CompareMark::Differs,
        &size_and_time,
    );
    assert_eq!(
        marks.bytes() - before,
        4 + CompareMarks::ENTRY_BYTES + 4 * 8
    );
    let before = marks.bytes();
    let folder = CompareDetail {
        inside: Some(CompareInside {
            files: 1,
            ..CompareInside::default()
        }),
        ..CompareDetail::default()
    };
    marks.record(OsStr::new("dir"), None, CompareMark::Differs, &folder);
    assert_eq!(
        marks.bytes() - before,
        3 + CompareMarks::ENTRY_BYTES + 3 * 8
    );
}

#[test]
fn numbers_that_do_not_fit_leave_the_mark_and_its_flags() {
    let mut marks = marks();
    // Spend the budget but for one entry without numbers.
    marks.bytes = CompareMarks::MAX_BYTES - (1 + CompareMarks::ENTRY_BYTES);
    let detail = CompareDetail {
        flags: CompareFlags::TIME,
        times: Some([2, 1]),
        ..CompareDetail::default()
    };
    assert!(marks.record(OsStr::new("x"), None, CompareMark::Differs, &detail));
    assert!(marks.complete(), "the mark itself was stored");
    assert_eq!(
        marks.detail(OsStr::new("x")),
        CompareDetail {
            flags: CompareFlags::TIME,
            ..CompareDetail::default()
        }
    );
    assert!(!marks.record(OsStr::new("y"), None, CompareMark::Differs, &detail));
    assert!(!marks.complete());
}

#[test]
fn unexamined_folder_children_are_not_inferred_as_matching() {
    let mut marks = marks();
    marks.record(
        OsStr::new("dir"),
        None,
        CompareMark::Differs,
        &CompareDetail {
            flags: CompareFlags::CASE,
            ..CompareDetail::default()
        },
    );
    assert_eq!(
        marks.mark(0, Path::new("/l/dir")),
        Some(CompareMark::Differs)
    );
    assert_eq!(marks.outcome(0, Path::new("/l/dir/child")), None);
}

#[test]
fn inferred_outcomes_have_no_detail() {
    let mut marks = marks();
    marks.record(
        OsStr::new("dir"),
        None,
        CompareMark::Differs,
        &CompareDetail {
            inside: Some(CompareInside {
                only: [1, 0],
                ..CompareInside::default()
            }),
            ..CompareDetail::default()
        },
    );
    marks.set(OsStr::new("dir/gone"), 0, CompareMark::OnlyHere);
    assert_eq!(
        marks.outcome(0, Path::new("/l/dir/match.txt")),
        Some((CompareMark::Same, CompareDetail::default()))
    );
    assert_eq!(
        marks.outcome(0, Path::new("/l/dir/gone/inner")),
        Some((CompareMark::OnlyHere, CompareDetail::default()))
    );
}

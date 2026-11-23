//! Salamander's per-pair rules: size, time with tolerance and daylight saving
//! shifts, attributes, and when contents must be read.
use super::*;
use std::time::Duration;

fn criteria(change: impl FnOnce(&mut CompareOptions)) -> Criteria {
    let mut options = CompareOptions::default();
    change(&mut options);
    Criteria::new(&options).unwrap()
}

fn file(size: u64, seconds: u64) -> EntryMetadata {
    EntryMetadata {
        size: Some(size),
        modified: Some(UNIX_EPOCH + Duration::from_secs(seconds)),
        ..EntryMetadata::default()
    }
}

#[test]
fn size_selects_the_larger_file_and_time_the_newer_one() {
    let rules = criteria(|_| ());
    assert_eq!(
        rules.files([&file(20, 10), &file(10, 10)]).select,
        [true, false]
    );
    assert_eq!(
        rules.files([&file(10, 10), &file(10, 20)]).select,
        [false, true]
    );
    // Larger on the left, newer on the right: both are selected, each for its reason.
    assert_eq!(
        rules.files([&file(20, 10), &file(10, 20)]).select,
        [true, true]
    );
    assert!(!rules.files([&file(10, 10), &file(10, 10)]).differs());
    let time_only = criteria(|options| options.by_size = false);
    assert!(!time_only.files([&file(20, 10), &file(10, 10)]).differs());
}

#[test]
fn time_tolerance_counts_a_difference_up_to_it_as_equal() {
    let rules = criteria(|_| ());
    assert!(
        !rules.files([&file(1, 100), &file(1, 102)]).differs(),
        "2 s is the default"
    );
    assert_eq!(
        rules.files([&file(1, 100), &file(1, 103)]).select,
        [false, true]
    );
    let exact = criteria(|options| options.time_tolerance_seconds = 0);
    assert_eq!(
        exact.files([&file(1, 101), &file(1, 100)]).select,
        [true, false]
    );
    // Sub-second parts are ignored: whole seconds are compared.
    let a = EntryMetadata {
        modified: Some(UNIX_EPOCH + Duration::from_millis(100_900)),
        ..file(1, 0)
    };
    assert!(!exact.files([&a, &file(1, 100)]).differs());
    // Before the epoch, whole seconds round down too.
    let early = |millis: u64| EntryMetadata {
        modified: Some(UNIX_EPOCH - Duration::from_millis(millis)),
        ..file(1, 0)
    };
    assert!(!exact.files([&early(1_100), &early(1_900)]).differs());
    assert!(exact.files([&early(900), &early(1_100)]).differs());
}

#[test]
fn daylight_saving_shifts_differ_unless_ignored_and_are_counted() {
    let rules = criteria(|_| ());
    for shift in [3600, 7200, 3601, 7198] {
        let verdict = rules.files([&file(1, 10_000 + shift), &file(1, 10_000)]);
        assert!(verdict.dst_shift, "{shift}");
        assert_eq!(verdict.select, [true, false], "{shift}");
    }
    let verdict = rules.files([&file(1, 10_000 + 3603), &file(1, 10_000)]);
    assert!(!verdict.dst_shift);
    let ignoring = criteria(|options| options.ignore_dst_shifts = true);
    let verdict = ignoring.files([&file(1, 10_000), &file(1, 10_000 + 7200)]);
    assert!(verdict.dst_shift && !verdict.differs());
    // Another criterion still selects the pair.
    let verdict = ignoring.files([&file(2, 10_000), &file(1, 10_000 + 7200)]);
    assert_eq!(verdict.select, [true, false]);
}

#[test]
fn attributes_select_both_files_and_compare_only_known_attributes() {
    let rules = criteria(|options| options.by_attributes = true);
    let with = |attributes: Option<u32>| EntryMetadata {
        native_attributes: attributes,
        ..file(1, 1)
    };
    assert_eq!(
        rules.files([&with(Some(0x21)), &with(Some(0x20))]).select,
        [true, true]
    );
    // Not compared: the directory bit, and flags Salamander does not show.
    assert!(
        !rules
            .files([&with(Some(0x20 | 0x10)), &with(Some(0x20))])
            .differs()
    );
    assert!(
        !rules
            .files([&with(Some(0x20 | 0x80)), &with(Some(0x20))])
            .differs()
    );
    assert!(!rules.files([&with(Some(0x1)), &with(None)]).differs());
    let mode = |mode: u32| EntryMetadata {
        unix_mode: Some(mode),
        ..file(1, 1)
    };
    assert!(rules.files([&mode(0o100644), &mode(0o100755)]).differs());
    let off = criteria(|_| ());
    assert!(!off.files([&with(Some(0x21)), &with(Some(0x20))]).differs());
}

#[test]
fn contents_are_read_unless_sizes_or_both_selections_decide() {
    let rules = criteria(|options| options.by_content = true);
    let verdict = rules.files([&file(5, 1), &file(5, 1)]);
    assert!(verdict.read_contents && !verdict.differs());
    // Different sizes select both, even without the size criterion.
    let no_size = criteria(|options| {
        options.by_content = true;
        options.by_size = false;
    });
    let verdict = no_size.files([&file(5, 1), &file(6, 1)]);
    assert_eq!(verdict.select, [true, true]);
    assert!(!verdict.read_contents);
    // A newer file is still read: different bytes would select the other one too.
    let verdict = rules.files([&file(5, 1), &file(5, 100)]);
    assert!(verdict.read_contents);
    assert_eq!(verdict.select, [false, true]);
    // Both already selected (attributes): nothing to read. Empty files are equal.
    let both = criteria(|options| {
        options.by_content = true;
        options.by_attributes = true;
    });
    let attributes = |value: u32| EntryMetadata {
        native_attributes: Some(value),
        ..file(5, 1)
    };
    assert!(!both.files([&attributes(1), &attributes(0)]).read_contents);
    assert!(!rules.files([&file(0, 1), &file(0, 1)]).read_contents);
}

#[test]
fn unknown_metadata_is_never_equal() {
    let rules = criteria(|_| ());
    let unknown = EntryMetadata {
        size: None,
        ..file(1, 1)
    };
    assert!(rules.files([&unknown, &file(1, 1)]).unknown);
}

#[test]
fn ignore_masks_apply_to_files_or_folders_and_names_fold_case() {
    let rules = criteria(|options| {
        options.ignore_files = Some("*.TMP; ~*".into());
        options.ignore_folders = Some(".git".into());
    });
    assert!(rules.ignored(OsStr::new("a.tmp"), false));
    assert!(rules.ignored(OsStr::new("~lock"), false));
    assert!(!rules.ignored(OsStr::new("a.tmp"), true));
    assert!(rules.ignored(OsStr::new(".GIT"), true));
    assert!(!rules.ignored(OsStr::new(".git"), false));
    let empty = criteria(|options| options.ignore_files = Some("  ".into()));
    assert!(
        !empty.ignored(OsStr::new("a"), false),
        "an empty mask ignores nothing"
    );
    let masks = CompareOptions {
        ignore_files: Some("a|b|c".into()),
        ..CompareOptions::default()
    };
    assert!(Criteria::new(&masks).is_err());
    let tolerance = CompareOptions {
        time_tolerance_seconds: 86_401,
        ..CompareOptions::default()
    };
    assert!(Criteria::new(&tolerance).is_err());
    let folded = criteria(|options| options.case_sensitive = false);
    assert_eq!(
        folded.name(OsStr::new("Readme.TXT")),
        folded.name(OsStr::new("README.txt"))
    );
    let exact = criteria(|options| options.case_sensitive = true);
    assert_ne!(
        exact.name(OsStr::new("Readme.TXT")),
        exact.name(OsStr::new("README.txt"))
    );
}

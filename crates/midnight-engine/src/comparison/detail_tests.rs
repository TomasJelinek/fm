//! Why each entry got its mark, as the panels' badge tooltips tell it: the numbers of
//! the criteria that differ, the first differing byte, a folder's children that did
//! not match, and why an entry could not be verified.
use super::*;

fn detail(result: &CompareResult, side: usize, path: &str) -> (CompareMark, CompareDetail) {
    result
        .marks
        .as_ref()
        .unwrap()
        .outcome(side, Path::new(path))
        .unwrap_or_else(|| panic!("{path} has no mark"))
}

fn walked(fake: &Fake, options: CompareOptions) -> CompareResult {
    let cancel = Cancellation::default();
    deep(fake, request(fake, options), &cancel).0.unwrap()
}

#[test]
fn size_time_and_shift_details_carry_both_sides_values() {
    let fake = twins();
    fake.file("/left/bigger", b"12345", 50)
        .file("/right/bigger", b"123", 50)
        .file("/left/older", b"o", 1_000)
        .file("/right/older", b"o", 1_000 + 2 * 86_400)
        .file("/left/shifted", b"s", 10_000)
        .file("/right/shifted", b"s", 10_000 + 3_600)
        .dir("/left/d")
        .dir("/right/d")
        .file("/left/d/inner", b"i", 7)
        .file("/right/d/inner", b"i", 10);
    let result = walked(&fake, options(false, true));
    let (mark, bigger) = detail(&result, 0, "/left/bigger");
    assert_eq!(mark, CompareMark::Differs);
    assert_eq!(bigger.sizes, Some([5, 3]));
    assert_eq!(bigger.times, None, "the times match");
    let (_, older) = detail(&result, 1, "/right/older");
    assert_eq!(older.times, Some([1_000, 1_000 + 2 * 86_400]));
    assert!(older.flags.contains(CompareFlags::TIME));
    assert!(!older.flags.contains(CompareFlags::DST_SHIFT));
    let (mark, shifted) = detail(&result, 0, "/left/shifted");
    assert_eq!(mark, CompareMark::Differs);
    assert!(
        shifted
            .flags
            .contains(CompareFlags::TIME | CompareFlags::DST_SHIFT)
    );
    assert_eq!(shifted.times, Some([10_000, 13_600]));
    // Inside a folder: the differing file and the folder's count of it.
    let (_, inner) = detail(&result, 0, "/left/d/inner");
    assert!(inner.flags.contains(CompareFlags::TIME), "{inner:?}");
    assert_eq!(
        inner.times,
        Some([7, 10]),
        "3 s is beyond the 2 s tolerance"
    );
    let (mark, folder) = detail(&result, 1, "/right/d");
    assert_eq!(mark, CompareMark::Differs);
    assert_eq!(
        folder.inside,
        Some(CompareInside {
            files: 1,
            ..CompareInside::default()
        })
    );

    // An ignored shift still tells why a displayed pair matches.
    let ignoring = CompareOptions {
        ignore_dst_shifts: true,
        ..options(false, true)
    };
    let result = walked(&fake, ignoring);
    let (mark, shifted) = detail(&result, 1, "/right/shifted");
    assert_eq!(mark, CompareMark::Same);
    assert!(shifted.flags.contains(CompareFlags::DST_SHIFT));
    assert!(!shifted.flags.contains(CompareFlags::TIME));
    assert_eq!(shifted.times, Some([10_000, 13_600]));
}

#[test]
fn contents_record_the_first_differing_byte_at_the_top_and_inside_folders() {
    let fake = twins();
    // Short provider reads (five bytes) must not move the offset.
    fake.file("/left/top", b"0123456789abcdef", 5)
        .file("/right/top", b"0123456789abXdef", 5)
        .file("/left/newer", b"same", 5)
        .file("/right/newer", b"same", 50)
        .dir("/left/d")
        .dir("/right/d")
        .file("/left/d/first", b"Xbc", 5)
        .file("/right/d/first", b"abc", 5);
    let result = walked(&fake, options(true, true));
    let (mark, top) = detail(&result, 0, "/left/top");
    assert_eq!(mark, CompareMark::Differs);
    assert_eq!(top.first_difference, Some(12));
    let (_, first) = detail(&result, 1, "/right/d/first");
    assert_eq!(first.first_difference, Some(0));
    // A newer file whose bytes match is still a difference (it is selected), for its
    // time; its contents add nothing.
    let (mark, newer) = detail(&result, 1, "/right/newer");
    assert_eq!(mark, CompareMark::Differs);
    assert!(newer.flags.contains(CompareFlags::TIME));
    assert_eq!(newer.first_difference, None);
}

#[test]
fn a_folder_counts_its_children_that_differ_are_one_sided_or_unverified() {
    let fake = twins();
    fake.dir("/left/d")
        .dir("/right/d")
        .file("/left/d/size1", b"12", 1)
        .file("/right/d/size1", b"1", 1)
        .file("/left/d/size2", b"12", 1)
        .file("/right/d/size2", b"1", 1)
        .dir("/left/d/sub")
        .dir("/right/d/sub")
        .file("/left/d/sub/x", b"x", 1)
        .dir("/left/d/same")
        .dir("/right/d/same")
        .file("/left/d/only-left-1", b"1", 1)
        .file("/left/d/only-left-2", b"2", 1)
        .dir("/right/d/only-right")
        .link("/left/d/kind")
        .file("/right/d/kind", b"k", 1);
    let result = walked(&fake, options(false, true));
    let (mark, folder) = detail(&result, 0, "/left/d");
    assert_eq!(mark, CompareMark::Differs);
    assert_eq!(
        folder.inside,
        Some(CompareInside {
            files: 3,
            folders: 1,
            only: [2, 1],
            unverified: 0,
        }),
        "size1, size2 and kind; sub; two files left, one folder right"
    );
    let (mark, sub) = detail(&result, 1, "/right/d/sub");
    assert_eq!(mark, CompareMark::Differs);
    assert_eq!(sub.inside.unwrap().only, [1, 0]);
    let (mark, same) = detail(&result, 0, "/left/d/same");
    assert_eq!((mark, same), (CompareMark::Same, CompareDetail::default()));
    let (mark, kind) = detail(&result, 0, "/left/d/kind");
    assert_eq!(mark, CompareMark::Differs);
    assert_eq!(kind.flags, CompareFlags::KIND);
}

#[test]
fn unverified_entries_tell_why() {
    let fake = twins();
    fake.file("/left/busy.log", b"same bytes", 3)
        .file("/right/busy.log", b"same bytes", 3)
        .dir("/left/locked")
        .dir("/right/locked")
        .dir("/left/partial")
        .dir("/right/partial")
        .file("/left/partial/hidden", b"h", 1)
        .file("/right/partial/hidden", b"h", 1);
    fake.unreadable("/right/locked");
    fake.unreadable("/left/partial/hidden");
    *fake.0.written_on_read.lock().unwrap() = Some("/right/busy.log".into());
    let result = walked(&fake, options(true, true));
    let (mark, busy) = detail(&result, 0, "/left/busy.log");
    assert_eq!(mark, CompareMark::Unverified);
    assert_eq!(busy.flags, CompareFlags::CHANGED);
    let (mark, locked) = detail(&result, 0, "/left/locked");
    assert_eq!(mark, CompareMark::Unverified);
    assert_eq!(locked.flags, CompareFlags::UNREADABLE);
    // A child that could not be described leaves its folder unproven, and its
    // counterpart is not verified either (not one-sided).
    let (mark, partial) = detail(&result, 1, "/right/partial");
    assert_eq!(mark, CompareMark::Unverified);
    assert_eq!(partial.flags, CompareFlags::UNREADABLE);
    let inside = partial.inside.unwrap();
    assert_eq!(inside.only, [0, 0]);
    assert_eq!(inside.unverified, 1);
    let (mark, hidden) = detail(&result, 1, "/right/partial/hidden");
    assert_eq!(mark, CompareMark::Unverified);
    assert_eq!(hidden.flags, CompareFlags::UNREADABLE);

    // A folder below the depth limit, and the folders above it.
    let fake = twins();
    let mut left = String::from("/left");
    let mut right = String::from("/right");
    for level in 0..=walk_depth_limit() {
        left.push_str(&format!("/d{level}"));
        right.push_str(&format!("/d{level}"));
        fake.dir(&left).dir(&right);
    }
    let result = walked(&fake, options(false, true));
    let deepest = left.clone();
    let (mark, limit) = detail(&result, 0, &deepest);
    assert_eq!(mark, CompareMark::Unverified);
    assert_eq!(limit.flags, CompareFlags::LIMIT);
    let parent = deepest.rsplit_once('/').unwrap().0;
    let (mark, above) = detail(&result, 0, parent);
    assert_eq!(mark, CompareMark::Differs);
    assert_eq!(above.inside.unwrap().unverified, 1);

    // Requested metadata missing: a link-free pair without times cannot be judged.
    let missing = CompareDetail {
        flags: CompareFlags::METADATA,
        ..CompareDetail::default()
    };
    let rules = rules::Criteria::new(&CompareOptions::default()).unwrap();
    let unknown = EntryMetadata::default();
    assert_eq!(rules.files([&unknown, &unknown]).detail, missing);
}

#[test]
fn names_that_differ_only_in_case_cannot_be_paired() {
    let fake = twins();
    fake.dir("/left/d")
        .dir("/right/d")
        .file("/left/d/Name", b"1", 1)
        .file("/left/d/name", b"1", 1)
        .file("/right/d/name", b"1", 1);
    let folded = CompareOptions {
        case_sensitive: false,
        ..options(false, true)
    };
    let result = walked(&fake, folded);
    let (mark, name) = detail(&result, 1, "/right/d/name");
    assert_eq!(mark, CompareMark::Differs);
    assert_eq!(name.flags, CompareFlags::CASE);
    assert_eq!(detail(&result, 0, "/left/d").1.inside.unwrap().files, 1);
}

//! The comparison badges' words (the tooltip and the accessible description) for
//! every verdict, and Clear comparison.
use super::describe::{Row, describe, span_for_tests};
use super::*;
use midnight_core::{CompareDetail, CompareFlags, CompareInside, LocalizedDateTime};
use std::{
    cell::RefCell,
    ffi::OsStr,
    time::{Duration, UNIX_EPOCH},
};

fn locale() -> NumberLocale {
    NumberLocale {
        decimal_separator: ".".into(),
        group_separator: ",".into(),
        grouping: vec![3],
        repeat_last_group: true,
    }
}

fn row<'a>(
    side: usize,
    directory: bool,
    options: &'a CompareOptions,
    locale: &'a NumberLocale,
) -> Row<'a> {
    Row {
        side,
        directory,
        options,
        here: None,
        there: None,
        locale,
    }
}

fn text(mark: CompareMark, detail: CompareDetail, row: &Row) -> String {
    describe(mark, &detail, row)
}

#[test]
fn file_verdicts_say_what_differs_from_this_panel() {
    let options = CompareOptions::default();
    let locale = locale();
    let left = row(0, false, &options, &locale);
    let right = row(1, false, &options, &locale);
    let sizes = CompareDetail {
        sizes: Some([64_293, 63_880]),
        ..CompareDetail::default()
    };
    assert_eq!(
        text(CompareMark::Differs, sizes, &left),
        "Larger here: 64,293 vs 63,880 bytes"
    );
    assert_eq!(
        text(CompareMark::Differs, sizes, &right),
        "Smaller here: 63,880 vs 64,293 bytes"
    );
    // Times: both panels' own texts when both rows still show the compared times.
    let base = 1_795_000_000;
    let times = CompareDetail {
        flags: CompareFlags::TIME,
        times: Some([base + 2 * 86_400, base]),
        ..CompareDetail::default()
    };
    let shown = Row {
        here: Some((base + 2 * 86_400, "23.11.2026 18:49:05".into())),
        there: Some((base, "21.11.2026 18:49:05".into())),
        ..row(0, false, &options, &locale)
    };
    assert_eq!(
        text(CompareMark::Differs, times, &shown),
        "Newer here: 23.11.2026 18:49:05 vs 21.11.2026 18:49:05 (2 d)"
    );
    let other = Row {
        here: Some((base, "21.11.2026 18:49:05".into())),
        there: None,
        ..row(1, false, &options, &locale)
    };
    assert_eq!(
        text(CompareMark::Differs, times, &other),
        "Older here by 2 d: 21.11.2026 18:49:05",
        "the other panel does not list the counterpart"
    );
    // A row whose time no longer is the compared one shows the difference only.
    let stale = Row {
        here: Some((base + 5, "elsewhen".into())),
        ..row(0, false, &options, &locale)
    };
    assert_eq!(
        text(CompareMark::Differs, times, &stale),
        "Newer here by 2 d"
    );
    // Daylight saving shifts, counted and ignored.
    let shift = CompareDetail {
        flags: CompareFlags::TIME | CompareFlags::DST_SHIFT,
        times: Some([base, base + 3_600]),
        ..CompareDetail::default()
    };
    assert_eq!(
        text(CompareMark::Differs, shift, &left),
        "Daylight saving shift of 1 h: older here (counted as a difference; the Compare directories dialog can ignore it)"
    );
    let ignored = CompareDetail {
        flags: CompareFlags::DST_SHIFT,
        ..shift
    };
    assert_eq!(
        text(CompareMark::Same, ignored, &left),
        "Matches: same size and time (within 2 s)\nDaylight saving shift of 1 h (ignored)"
    );
    // Attributes, Windows letters and POSIX permissions.
    let attributes = CompareDetail {
        attributes: Some([0x21, 0x0]),
        ..CompareDetail::default()
    };
    assert_eq!(
        text(CompareMark::Differs, attributes, &left),
        "Attributes differ: here RA, there —"
    );
    let mode = CompareDetail {
        flags: CompareFlags::MODE,
        attributes: Some([0o644, 0o4755]),
        ..CompareDetail::default()
    };
    assert_eq!(
        text(CompareMark::Differs, mode, &right),
        "Permissions differ: here 4755, there 0644"
    );
    // Contents, with what else differs first.
    let content = CompareDetail {
        flags: CompareFlags::TIME,
        times: Some([base + 90, base]),
        first_difference: Some(4_096),
        ..CompareDetail::default()
    };
    assert_eq!(
        text(CompareMark::Differs, content, &left),
        "Newer here by 1 min 30 s\nContent differs (first difference at byte 4,096)"
    );
    // Kinds and names that cannot be paired.
    let kind = CompareDetail {
        flags: CompareFlags::KIND,
        ..CompareDetail::default()
    };
    assert_eq!(
        text(CompareMark::Differs, kind, &left),
        "A file, a folder or a link opposite another kind: not compared"
    );
    let case = CompareDetail {
        flags: CompareFlags::CASE,
        ..CompareDetail::default()
    };
    assert!(text(CompareMark::Differs, case, &left).contains("differ only in case"));
    assert_eq!(
        text(CompareMark::OnlyHere, CompareDetail::default(), &left),
        "Only in this panel"
    );
}

#[test]
fn unverified_rows_say_why_and_matches_name_the_criteria() {
    let options = CompareOptions {
        by_attributes: true,
        by_content: true,
        time_tolerance_seconds: 0,
        ..CompareOptions::default()
    };
    let locale = locale();
    let file = row(0, false, &options, &locale);
    let folder = row(0, true, &options, &locale);
    let why = |flags| CompareDetail {
        flags,
        ..CompareDetail::default()
    };
    for (flags, wanted) in [
        (
            CompareFlags::METADATA,
            "Not verified: the size or modification time is unavailable",
        ),
        (
            CompareFlags::UNREADABLE,
            "Not verified: it could not be read",
        ),
        (
            CompareFlags::CHANGED,
            "Not verified: the file changed while it was compared; compare again",
        ),
        (
            CompareFlags::LIMIT,
            "Not verified: the comparison reached its depth or memory limit here",
        ),
        (
            CompareFlags::empty(),
            "Not verified: it could not be compared completely",
        ),
    ] {
        assert_eq!(text(CompareMark::Unverified, why(flags), &file), wanted);
    }
    assert_eq!(
        text(
            CompareMark::Unverified,
            why(CompareFlags::UNREADABLE),
            &folder
        ),
        "Not verified: the folder could not be listed completely"
    );
    assert_eq!(
        text(CompareMark::Same, CompareDetail::default(), &file),
        "Matches: same size, time, attributes and contents"
    );
    assert_eq!(
        text(CompareMark::Same, CompareDetail::default(), &folder),
        "Matches: everything inside has the same names, size, time, attributes and contents"
    );
    let none = CompareOptions {
        by_size: false,
        by_modified: false,
        ..CompareOptions::default()
    };
    assert_eq!(
        text(
            CompareMark::Same,
            CompareDetail::default(),
            &row(0, false, &none, &locale)
        ),
        "Matches: the same name in both panels"
    );
}

#[test]
fn folder_verdicts_count_the_children_that_did_not_match() {
    let options = CompareOptions::default();
    let locale = locale();
    let inside = CompareDetail {
        inside: Some(CompareInside {
            files: 3,
            folders: 1,
            only: [2, 0],
            unverified: 0,
        }),
        ..CompareDetail::default()
    };
    assert_eq!(
        text(
            CompareMark::Differs,
            inside,
            &row(0, true, &options, &locale)
        ),
        "Differs inside: 3 files and 1 folder differ, 2 only here"
    );
    assert_eq!(
        text(
            CompareMark::Differs,
            inside,
            &row(1, true, &options, &locale)
        ),
        "Differs inside: 3 files and 1 folder differ, 2 only in the other panel"
    );
    let one = CompareDetail {
        inside: Some(CompareInside {
            files: 1,
            unverified: 1_200,
            ..CompareInside::default()
        }),
        ..CompareDetail::default()
    };
    assert_eq!(
        text(CompareMark::Differs, one, &row(0, true, &options, &locale)),
        "Differs inside: 1 file differs, 1,200 not verified"
    );
    let attributes = CompareDetail {
        attributes: Some([0x2, 0x0]),
        ..CompareDetail::default()
    };
    assert_eq!(
        text(
            CompareMark::Differs,
            attributes,
            &row(0, true, &options, &locale)
        ),
        "Folder attributes differ: here H, there —"
    );
    assert_eq!(
        text(
            CompareMark::Differs,
            CompareDetail::default(),
            &row(0, true, &options, &locale)
        ),
        "Something inside differs"
    );
    assert_eq!(
        text(
            CompareMark::OnlyHere,
            CompareDetail::default(),
            &row(0, true, &options, &locale)
        ),
        "Only in this panel, with everything inside"
    );
    for (seconds, wanted) in [
        (0, "0 s"),
        (45, "45 s"),
        (3_600, "1 h"),
        (3_900, "1 h 5 min"),
        (2 * 86_400, "2 d"),
        (2 * 86_400 + 3 * 3_600 + 59, "2 d 3 h"),
    ] {
        assert_eq!(span_for_tests(seconds), wanted);
    }
}

/// Both panels list `names` (file "f" and folder "d" as given) under left and right.
fn listed(names: &[(&str, EntryKind, u64)]) -> Presenter {
    let mut p = Presenter::new("left".into(), "right".into());
    for pane in p.panes.iter_mut() {
        pane.generation = 1;
        pane.successful_generation = Some(NavigationId(1));
        pane.selection_context = NavigationId(1);
        pane.selection_committed_context = NavigationId(1);
        let entries: Vec<Entry> = names
            .iter()
            .enumerate()
            .map(|(index, (name, kind, seconds))| {
                let mut entry = Entry::new(EntryId(index as u64 + 1), pane.path.join(name), *kind);
                entry.metadata.modified = Some(UNIX_EPOCH + Duration::from_secs(*seconds));
                entry.metadata.localized_modified = Some(LocalizedDateTime {
                    date: format!("day {seconds}"),
                    time: "12:00:00".into(),
                });
                entry
            })
            .collect();
        pane.entries = entries.into();
        pane.index = Arc::new(DirectoryIndex::from_entries(&pane.entries));
    }
    p
}

/// A comparison with subfolders whose result selects `bits` of both panels and
/// stores `marks`.
fn compared(p: &mut Presenter, bits: u64, marks: CompareMarks) {
    let commands = RefCell::new(Vec::new());
    p.open_comparison();
    let options = CompareOptions {
        recursive: true,
        by_content: true,
        ..CompareOptions::default()
    };
    p.submit_comparison(options, IgnoreChoices::default(), &|command| {
        commands.borrow_mut().push(command);
        true
    });
    let request = commands
        .into_inner()
        .into_iter()
        .find_map(|command| match command {
            Command::CompareDirectories(request) => Some(Arc::new(request)),
            _ => None,
        })
        .unwrap();
    let selections = request
        .sides
        .each_ref()
        .map(|side| Arc::new(SelectionSnapshot::from_bits(side.view.clone(), vec![bits]).unwrap()));
    let result = CompareResult {
        request: request.clone(),
        selections,
        tree_replacements: [None, None],
        marks: Some(Arc::new(marks)),
        unknown: 0,
        ambiguous: 0,
        differing_folders: 0,
        compared_folders: 1,
        content_differences: 0,
        content_files: 0,
        content_bytes: 0,
        walked_folders: 0,
        walked_files: 0,
        dst_shifts: 0,
        skipped: 0,
        errors: 0,
        limited: false,
        diagnostics: Vec::new(),
    };
    p.apply(Event::DirectoriesCompared {
        id: request.id,
        result: Ok(Arc::new(result)),
    });
}

fn marks_with_newer_left() -> CompareMarks {
    let mut marks = CompareMarks::new([PathBuf::from("left"), PathBuf::from("right")], false);
    let key = marks.child_key(OsStr::new(""), OsStr::new("f"));
    marks.record(
        &key,
        None,
        CompareMark::Differs,
        &CompareDetail {
            flags: CompareFlags::TIME,
            times: Some([200_000, 27_200]),
            ..CompareDetail::default()
        },
    );
    marks
}

#[test]
fn a_row_badge_uses_both_panels_shown_times_and_follows_the_listing() {
    let mut p = listed(&[("f", EntryKind::File, 200_000)]);
    // The right panel's f is the older one.
    let mut right: Vec<Entry> = p.panes[1].entries.to_vec();
    right[0].metadata.modified = Some(UNIX_EPOCH + Duration::from_secs(27_200));
    right[0].metadata.localized_modified = Some(LocalizedDateTime {
        date: "day 27200".into(),
        time: "12:00:00".into(),
    });
    p.panes[1].entries = right.into();
    p.panes[1].index = Arc::new(DirectoryIndex::from_entries(&p.panes[1].entries));
    compared(&mut p, 1, marks_with_newer_left());
    let left = p.panes[0].entries[0].clone();
    let (mark, detail) = p.comparison_badge(PaneId::Left, &left, &left.metadata);
    assert_eq!(mark, 2);
    assert_eq!(
        detail,
        "Newer here: day 200000 12:00:00 vs day 27200 12:00:00 (2 d)"
    );
    let right = p.panes[1].entries[0].clone();
    assert_eq!(
        p.comparison_badge(PaneId::Right, &right, &right.metadata).1,
        "Older here: day 27200 12:00:00 vs day 200000 12:00:00 (2 d)"
    );
    // Another listing of a panel: no badge, no words.
    p.panes[0].generation += 1;
    assert_eq!(
        p.comparison_badge(PaneId::Left, &left, &left.metadata),
        (0, String::new())
    );
}

#[test]
fn silent_rereads_keep_both_badges_until_a_changed_listing_commits() {
    let mut p = listed(&[("f", EntryKind::File, 200_000)]);
    compared(&mut p, 1, marks_with_newer_left());
    let assert_badges = |p: &Presenter, expected| {
        for pane in [PaneId::Left, PaneId::Right] {
            assert_eq!(
                p.comparison_mark(pane, &p.panes[pane.index()].entries[0]),
                expected
            );
        }
    };
    for (pane, changed) in [
        (PaneId::Right, false),
        (PaneId::Left, false),
        (PaneId::Right, true),
    ] {
        let shown = &p.panes[pane.index()];
        let previous = shown.successful_generation.unwrap();
        let mut entries = shown.entries.clone();
        assert!(p.refresh_if_changed(pane, previous, &|_| true));
        p.flush_comparison(&|_| true);
        assert_badges(&p, 2);
        if changed {
            Arc::make_mut(&mut entries)[0].metadata.size = Some(17);
        }
        let navigation = NavigationId(p.panes[pane.index()].generation);
        p.apply(Event::ListingFinished {
            pane,
            navigation,
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        });
        p.flush_comparison(&|_| true);
        assert_badges(&p, if changed { 0 } else { 2 });
    }
}

#[test]
fn clear_comparison_removes_both_panels_marks_and_the_line_but_keeps_the_selection() {
    let mut p = listed(&[("f", EntryKind::File, 200_000)]);
    compared(&mut p, 1, marks_with_newer_left());
    let (line, details) = p.comparison_note(PaneId::Left).unwrap();
    assert!(
        line.ends_with(" · × clears the marks") || line.contains(" or × clears the marks"),
        "{line}"
    );
    assert!(
        details.contains("closes it and clears the marks"),
        "{details}"
    );
    assert!(p.comparison_shown());
    let selected = [0, 1].map(|slot| p.panes[slot].selected.clone());
    let revisions = [0, 1].map(|slot| p.panes[slot].selection_revision);
    assert!(p.panes[0].selected.len() == 1 && p.panes[1].selected.len() == 1);
    let entries = [0, 1].map(|slot| p.panes[slot].entries[0].clone());
    assert_eq!(p.comparison_mark(PaneId::Left, &entries[0]), 2);
    assert_eq!(p.comparison_mark(PaneId::Right, &entries[1]), 2);

    assert!(p.clear_comparison());
    assert_eq!(p.comparison_mark(PaneId::Left, &entries[0]), 0);
    assert_eq!(p.comparison_mark(PaneId::Right, &entries[1]), 0);
    assert!(p.comparison_note(PaneId::Left).is_none());
    assert!(!p.comparison_shown());
    for slot in 0..2 {
        assert!(
            Arc::ptr_eq(&p.panes[slot].selected, &selected[slot]),
            "the selection stays"
        );
        assert_eq!(p.panes[slot].selection_revision, revisions[slot]);
    }
    assert!(!p.clear_comparison(), "nothing more to clear");
}

#[test]
fn marks_outlive_the_result_line_with_a_line_of_their_own() {
    let mut p = listed(&[("f", EntryKind::File, 200_000)]);
    compared(&mut p, 1, marks_with_newer_left());
    // The user selects something else: the counts no longer apply, the marks do.
    p.panes[0].selection_revision.0 += 1;
    let (line, details) = p.comparison_note(PaneId::Left).unwrap();
    assert!(
        line.starts_with("Comparison marks: = matches, ≠ differs"),
        "{line}"
    );
    assert!(line.contains("× clears them"), "{line}");
    assert!(details.contains("Point at a mark"), "{details}");
    assert!(p.comparison_note(PaneId::Right).is_none());
    assert!(p.clear_comparison());
    assert!(p.comparison_note(PaneId::Left).is_none());
}

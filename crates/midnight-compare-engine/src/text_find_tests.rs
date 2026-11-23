use super::*;
use crate::Palette;
use midnight_core::{ViewerAppearance, ViewerGeometry};

fn query(text: &str) -> FindQuery {
    FindQuery {
        text: text.into(),
        ..FindQuery::default()
    }
}

fn finish(comparison: &mut TextComparison) {
    let cancel = AtomicBool::new(false);
    while comparison.find.pending() {
        comparison.find_turn(&cancel).unwrap();
    }
}

#[test]
fn paints_reuse_immutable_find_snapshots_and_state_changes_preserve_unread_frames() {
    let mut comparison = TextComparison::new(
        "needle\nother\n",
        "needle\nother\n",
        &AtomicBool::new(false),
    )
    .unwrap();
    let geometry = ViewerGeometry {
        width: 480,
        height: 160,
        scale: 1.0,
    };
    let appearance = crate::appearance_tests::changed();
    let palette = Palette::from(&appearance);
    let cancel = AtomicBool::new(false);
    let initial = comparison
        .paint(geometry, &appearance, palette, &cancel)
        .unwrap();
    let unchanged = comparison
        .paint(geometry, &appearance, palette, &cancel)
        .unwrap();
    assert!(Arc::ptr_eq(&initial.find, &unchanged.find));
    comparison.begin_find(query("needle"), false, 1);
    let pending = comparison
        .paint(geometry, &appearance, palette, &cancel)
        .unwrap();
    assert!(pending.find.searching);
    assert!(!Arc::ptr_eq(&initial.find, &pending.find));
    finish(&mut comparison);
    let complete = comparison
        .paint(geometry, &appearance, palette, &cancel)
        .unwrap();
    assert_eq!(
        (
            complete.find.revision,
            complete.find.matches,
            complete.find.active
        ),
        (1, 2, 1)
    );
    assert!(!complete.find.searching);
    assert!(pending.find.searching);
    assert_eq!(
        pending.find.matches, 0,
        "unread progress frame remains immutable after completion"
    );
    assert_eq!(pending.find.message, "Searching…");
    assert_eq!(initial.find.revision, 0);
    comparison.scroll(1);
    let scrolled = comparison
        .paint(geometry, &appearance, palette, &cancel)
        .unwrap();
    assert!(
        Arc::ptr_eq(&complete.find, &scrolled.find),
        "ordinary repaint/scroll must not clone the Find string or allocate a new snapshot"
    );
    comparison.begin_find(FindQuery::default(), false, 2);
    let cleared = comparison
        .paint(geometry, &appearance, palette, &cancel)
        .unwrap();
    assert_eq!((cleared.find.revision, cleared.find.matches), (2, 0));
    assert!(cleared.find.message.is_empty());
    assert_eq!(
        (
            complete.find.revision,
            complete.find.matches,
            complete.find.active
        ),
        (1, 2, 1)
    );
    assert!(!Arc::ptr_eq(&complete.find, &cleared.find));
}

#[test]
fn searches_both_panes_and_wraps_in_both_directions_with_an_explicit_count() {
    let mut comparison = TextComparison::new(
        "needle one\nneedle two\n",
        "needle one\nneedle two\n",
        &AtomicBool::new(false),
    )
    .unwrap();
    for (revision, backwards, ordinal, side, row) in [
        (1, false, 1, 0, 0),
        (2, false, 2, 1, 0),
        (3, false, 3, 0, 1),
        (4, true, 2, 1, 0),
        (5, true, 1, 0, 0),
        (6, true, 4, 1, 1),
        (7, false, 1, 0, 0),
    ] {
        comparison.begin_find(query("needle"), backwards, revision);
        finish(&mut comparison);
        let state = &comparison.find.state;
        let hit = comparison.find.selected.as_ref().unwrap();
        assert_eq!(
            (state.revision, state.matches, state.active),
            (revision, 4, ordinal)
        );
        assert_eq!((hit.side, hit.row, hit.range.clone()), (side, row, 0..6));
    }
    let mut sensitive = query("NEEDLE");
    sensitive.case_sensitive = true;
    comparison.begin_find(sensitive, false, 8);
    finish(&mut comparison);
    assert_eq!(comparison.find.state.matches, 0);
    assert_eq!(comparison.find.state.message, "No matches");
    assert!(comparison.find.selected.is_none());
}

#[test]
fn unicode_expansions_map_to_original_scalars_and_whole_words_include_marks_and_connectors() {
    let line = "Žluťoučký Straße ß S a a\u{301} a_b a\u{203f}b a\u{200c}b\n";
    let mut comparison = TextComparison::new(line, "", &AtomicBool::new(false)).unwrap();
    comparison.begin_find(query("STRASSE"), false, 1);
    finish(&mut comparison);
    let hit = comparison.find.selected.as_ref().unwrap();
    assert_eq!(&line[hit.range.clone()], "Straße");
    assert_eq!(comparison.find.state.matches, 1);

    comparison.begin_find(query("S"), false, 2);
    finish(&mut comparison);
    assert_eq!(comparison.find.state.matches, 2, "no half-ß matches");

    let mut whole = query("a");
    whole.whole_words = true;
    comparison.begin_find(whole, false, 3);
    finish(&mut comparison);
    assert_eq!(
        comparison.find.state.matches, 1,
        "marks/connectors/formats extend words"
    );
}

#[test]
fn rejected_partial_fold_candidates_do_not_hide_overlapping_complete_scalars() {
    for (text, pattern, expected) in [("sß", "SS", 1..3), ("fﬀ", "FF", 1..4)] {
        let mut comparison = TextComparison::new(text, "", &AtomicBool::new(false)).unwrap();
        comparison.begin_find(query(pattern), false, 1);
        finish(&mut comparison);
        assert_eq!(comparison.find.state.matches, 1, "{text} / {pattern}");
        assert_eq!(comparison.find.selected.as_ref().unwrap().range, expected);
    }
    let mut comparison = TextComparison::new("banana", "", &AtomicBool::new(false)).unwrap();
    comparison.begin_find(query("ana"), false, 1);
    finish(&mut comparison);
    assert_eq!(comparison.find.state.matches, 2);
    assert_eq!(comparison.find.selected.as_ref().unwrap().range, 1..4);
    comparison.advance_find(1, 2);
    finish(&mut comparison);
    assert_eq!(comparison.find.selected.as_ref().unwrap().range, 3..6);
}

#[test]
fn relative_bursts_accumulate_during_discovery_and_use_a_bounded_sparse_navigation_index() {
    let text = "needle\n".repeat(5000);
    let mut comparison = TextComparison::new(&text, &text, &AtomicBool::new(false)).unwrap();
    comparison.begin_find(query("needle"), false, 1);
    assert!(!comparison.find_turn(&AtomicBool::new(false)).unwrap());
    comparison.advance_find(5, 2);
    comparison.advance_find(-2, 3);
    finish(&mut comparison);
    assert_eq!(
        (comparison.find.state.active, comparison.find.state.matches),
        (4, 10_000)
    );
    assert_eq!(comparison.find.state.revision, 3);
    assert!(comparison.find.checkpoints.len() <= 1 + comparison.rows.len() / TURN_ROWS);
    comparison.advance_find(2000, 4);
    comparison.advance_find(-1000, 5);
    assert!(
        comparison.find.scan.as_ref().unwrap().row >= 448,
        "navigation uses its nearest checkpoint rather than rescanning from row zero"
    );
    let mut turns = 0;
    while comparison.find.pending() {
        comparison.find_turn(&AtomicBool::new(false)).unwrap();
        turns += 1;
    }
    assert_eq!(
        turns, 1,
        "short rows need at most one 64-row navigation turn"
    );
    assert_eq!(
        (comparison.find.state.active, comparison.find.state.revision),
        (1004, 5)
    );
    comparison.advance_find(-1004, 6);
    finish(&mut comparison);
    assert_eq!(comparison.find.state.active, 10_000);
    comparison.begin_find(query("NEEDLE"), true, 7);
    comparison.advance_find(2, 8);
    finish(&mut comparison);
    assert_eq!(
        comparison.find.state.active, 2,
        "first backwards query starts at the end, then accumulates relative steps"
    );
}

#[test]
fn large_search_yields_and_new_query_clear_and_cancellation_discard_pending_work() {
    let text = "ordinary line\n".repeat(4000) + "last needle\n";
    let mut comparison = TextComparison::new(&text, &text, &AtomicBool::new(false)).unwrap();
    comparison.begin_find(query("needle"), false, 1);
    assert!(!comparison.find_turn(&AtomicBool::new(false)).unwrap());
    assert!(comparison.find.pending());
    assert!(comparison.find.state.searching);
    assert!(comparison.find.selected.is_none());

    comparison.begin_find(query("absent"), false, 2);
    finish(&mut comparison);
    assert_eq!(comparison.find.state.revision, 2);
    assert_eq!(comparison.find.state.matches, 0);

    comparison.begin_find(query("needle"), false, 3);
    assert!(comparison.find_turn(&AtomicBool::new(true)).is_err());
    comparison.begin_find(FindQuery::default(), false, 4);
    assert!(!comparison.find.pending());
    assert!(comparison.find.selected.is_none());
    assert!(comparison.find.state.message.is_empty());
    assert!(!comparison.find.reveal);
    comparison.begin_find(query(&"a".repeat(MAX_QUERY_BYTES + 1)), false, 5);
    assert!(!comparison.find.pending());
    assert!(comparison.find.state.message.contains("4096"));
}

#[test]
fn maximum_admitted_lines_yield_without_retaining_every_match_and_can_be_cancelled() {
    let line = "a".repeat(super::super::MAX_LINE_BYTES - 1) + "\n";
    let text = line.repeat(40);
    assert!(super::super::decode(text.as_bytes()).is_ok());
    let mut comparison = TextComparison::new(&text, &text, &AtomicBool::new(false)).unwrap();
    comparison.begin_find(query("a"), false, 1);
    assert!(!comparison.find_turn(&AtomicBool::new(false)).unwrap());
    assert!(
        comparison.find.pending(),
        "a maximum-line scan yields between bounded turns"
    );
    assert!(comparison.find_turn(&AtomicBool::new(true)).is_err());
    comparison.begin_find(FindQuery::default(), false, 2);
    assert!(!comparison.find.pending());
    let mut folded = FoldedLine::default();
    assert!(
        folded
            .set(&"Ž".repeat(8192), &AtomicBool::new(true))
            .is_err()
    );
    assert!(super::super::decode(&vec![b'a'; super::super::MAX_LINE_BYTES + 1]).is_err());
}

#[test]
fn resize_reveals_a_visible_hit_but_preserves_intentional_vertical_and_horizontal_scroll_away() {
    let text: String = (0..100)
        .map(|line| {
            if line == 75 {
                format!("\t{} needle\n", "Ž".repeat(70))
            } else {
                format!("ordinary {line}\n")
            }
        })
        .collect();
    let mut comparison = TextComparison::new(&text, &text, &AtomicBool::new(false)).unwrap();
    let wide = ViewerGeometry {
        width: 800,
        height: 480,
        scale: 1.0,
    };
    let narrow = ViewerGeometry {
        width: 360,
        height: 160,
        scale: 1.0,
    };
    let appearance = ViewerAppearance {
        family: "monospace".into(),
        font_size: 14.0,
        foreground: [240, 240, 240, 255],
        background: [20, 20, 20, 255],
        selection: [210, 170, 60, 255],
        selection_text: [0; 4],
        accent: [60, 140, 220, 255],
    };
    let palette = Palette::from(&appearance);
    let cancel = AtomicBool::new(false);
    comparison
        .paint(wide, &appearance, palette, &cancel)
        .unwrap();
    comparison.begin_find(query("needle"), false, 1);
    finish(&mut comparison);
    comparison
        .paint(wide, &appearance, palette, &cancel)
        .unwrap();
    let old_top = comparison.top;
    comparison.reveal_visible_find_on_resize();
    let frame = comparison
        .paint(narrow, &appearance, palette, &cancel)
        .unwrap();
    assert!(
        comparison.top > old_top,
        "narrow viewport must keep its existing hit in view"
    );
    assert_eq!(comparison.top, 75 - comparison.page_rows / 2);
    assert!(frame.view_pixels().2.contains(&appearance.selection));
    let [left, right] = comparison.find.horizontal_bounds.unwrap();
    assert!(
        ((left + right) / 2.0 - f64::from(comparison.horizontal) - 90.0).abs() < 1.0,
        "horizontal reveal uses the exact shaped hit and the new pane width"
    );

    comparison.scroll(-1000);
    let old_horizontal = comparison.horizontal;
    comparison.reveal_visible_find_on_resize();
    let frame = comparison
        .paint(wide, &appearance, palette, &cancel)
        .unwrap();
    assert_eq!(
        comparison.top, 0,
        "resizing must preserve manually scrolled-away rows"
    );
    assert_eq!(comparison.horizontal, old_horizontal);
    assert!(!frame.view_pixels().2.contains(&appearance.selection));

    comparison.scroll(75);
    comparison.horizontal = 100_000.0;
    comparison
        .paint(wide, &appearance, palette, &cancel)
        .unwrap();
    comparison.reveal_visible_find_on_resize();
    let frame = comparison
        .paint(narrow, &appearance, palette, &cancel)
        .unwrap();
    assert_eq!(
        (comparison.top, comparison.horizontal),
        (75, 100_000.0),
        "a horizontally scrolled-away hit must stay away after resize"
    );
    assert!(!frame.view_pixels().2.contains(&appearance.selection));
}

#[test]
fn selected_hit_is_highlighted_and_centered_using_painted_gutter_tabs_and_unicode() {
    let mut lines: Vec<_> = (0..100).map(|line| format!("ordinary {line}\n")).collect();
    lines[70] = format!("\t{} Straße\n", "Ž".repeat(90));
    let text = lines.concat();
    let mut comparison = TextComparison::new(&text, &text, &AtomicBool::new(false)).unwrap();
    let geometry = ViewerGeometry {
        width: 800,
        height: 320,
        scale: 1.0,
    };
    let appearance = ViewerAppearance {
        family: "monospace".into(),
        font_size: 14.0,
        foreground: [240, 240, 240, 255],
        background: [20, 20, 20, 255],
        selection: [210, 170, 60, 255],
        selection_text: [0; 4],
        accent: [60, 140, 220, 255],
    };
    let palette = Palette::from(&appearance);
    comparison
        .paint(geometry, &appearance, palette, &AtomicBool::new(false))
        .unwrap();
    for (revision, expected_side) in [(1, 0), (2, 1)] {
        comparison.begin_find(query("STRASSE"), false, revision);
        finish(&mut comparison);
        let frame = comparison
            .paint(geometry, &appearance, palette, &AtomicBool::new(false))
            .unwrap();
        let hit = comparison.find.selected.as_ref().unwrap();
        assert_eq!((hit.row, hit.side), (70, expected_side));
        assert_eq!(comparison.top, 70 - comparison.page_rows / 2);
        assert!(
            comparison.horizontal > 100.0,
            "long prefix must remain visible through a true horizontal reveal"
        );
        let (width, _, pixels) = frame.view_pixels();
        assert_eq!(width, 800);
        let colored: Vec<_> = pixels
            .iter()
            .enumerate()
            .filter(|(_, rgba)| **rgba == appearance.selection)
            .map(|(index, _)| ((index % 800) as f64, (index / 800) as f64))
            .collect();
        assert!(
            !colored.is_empty(),
            "the actual match must be visibly highlighted"
        );
        let x0 = colored
            .iter()
            .map(|point| point.0)
            .fold(f64::INFINITY, f64::min);
        let x1 = colored.iter().map(|point| point.0).fold(0.0, f64::max);
        let y0 = colored
            .iter()
            .map(|point| point.1)
            .fold(f64::INFINITY, f64::min);
        assert!(((x0 + x1) / 2.0 - (200.0 + expected_side as f64 * 400.0)).abs() < 12.0);
        assert!(
            (120.0..190.0).contains(&y0),
            "Find must center vertically, actual y {y0}"
        );
    }
    comparison.begin_find(FindQuery::default(), false, 3);
    let frame = comparison
        .paint(geometry, &appearance, palette, &AtomicBool::new(false))
        .unwrap();
    assert!(!frame.view_pixels().2.contains(&appearance.selection));
}

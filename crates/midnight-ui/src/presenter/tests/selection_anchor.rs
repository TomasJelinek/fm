//! The owner's request: a Shift range starts at the current row, not at the anchor of a
//! selection that is gone. The anchor (the row last clicked or Ctrl+clicked) starts the
//! next Shift range only while the cursor stays where the anchor left it; after the
//! cursor moved without Shift (the cursor keys in every view, a jump) or a bulk change
//! of the selection (Deselect all, Select all, Invert), the range starts at the cursor.
use super::*;
use crate::panel_view_navigation::destination;

fn loaded() -> Presenter {
    let mut p = Presenter::new("left".into(), "right".into());
    let navigation = begin(&mut p, PaneId::Left, "left");
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        (0..10)
            .map(|row| Entry::new(EntryId(row), format!("left/{row}").into(), EntryKind::File))
            .collect(),
    );
    p
}

fn path(row: usize) -> PathBuf {
    format!("left/{row}").into()
}

/// Shift+click (with Ctrl when `add`) the entry shown `row` rows below the parent row;
/// returns the requested range's anchor and target.
fn shift_click(p: &mut Presenter, row: usize, add: bool) -> (PathBuf, PathBuf) {
    let sent = RefCell::new(None);
    let offset = p.panes[0].parent_offset();
    p.select_row_modified(PaneId::Left, offset + row, add, true, &|command| {
        *sent.borrow_mut() = Some(command);
        true
    });
    match sent.into_inner() {
        Some(Command::Selection(SelectionRequest {
            action: SelectionAction::Range { anchor, target, .. },
            ..
        })) => (anchor, target),
        other => panic!("Shift+click sent no range: {other:?}"),
    }
}

fn click(p: &mut Presenter, row: usize, control: bool) {
    let offset = p.panes[0].parent_offset();
    p.select_row_modified(PaneId::Left, offset + row, control, false, &|_| true);
}

fn key(p: &mut Presenter, key: i32) {
    p.move_cursor(PaneId::Left, key, &|_| true);
}

/// The cursor's entry row; None on the parent row.
fn cursor(p: &Presenter) -> Option<usize> {
    p.panes[0]
        .cursor_index()
        .and_then(|row| row.checked_sub(p.panes[0].parent_offset()))
        .filter(|_| !p.panes[0].parent_cursor)
}

#[test]
fn a_shift_range_after_the_cursor_keys_starts_at_the_cursor() {
    // Down, Up, Page Down, End, Space and Insert move the cursor without Shift; Home and
    // Page Up reach the parent row, which no range includes: the range then starts at
    // the clicked row.
    for (keys, from) in [
        (&[3, 3][..], Some(5)),
        (&[2][..], Some(2)),
        (&[5][..], Some(9)),
        (&[7][..], Some(9)),
        (&[9][..], Some(4)),
        (&[6][..], None),
        (&[4][..], None),
    ] {
        let mut p = loaded();
        click(&mut p, 1, false);
        assert_eq!(shift_click(&mut p, 3, false), (path(1), path(3)));
        for code in keys {
            key(&mut p, *code);
        }
        if p.panes[0].parent_offset() == 0 && from.is_none() {
            continue;
        }
        assert_eq!(cursor(&p), from, "keys {keys:?}");
        let target = 7;
        assert_eq!(
            shift_click(&mut p, target, false),
            (path(from.unwrap_or(target)), path(target)),
            "keys {keys:?}: the range starts at the cursor, not at the cleared anchor"
        );
        // The range keeps its start for the next Shift+click.
        assert_eq!(
            shift_click(&mut p, 8, false),
            (path(from.unwrap_or(target)), path(8))
        );
    }
}

#[test]
fn a_shift_range_after_deselect_all_or_another_bulk_change_starts_at_the_cursor() {
    for operation in [
        SelectionOperation::Clear,
        SelectionOperation::All,
        SelectionOperation::Invert,
    ] {
        let mut p = loaded();
        click(&mut p, 1, false);
        assert_eq!(shift_click(&mut p, 3, false), (path(1), path(3)));
        assert!(p.bulk_selection(PaneId::Left, operation, &|_| true));
        assert_eq!(
            shift_click(&mut p, 6, false),
            (path(3), path(6)),
            "{operation:?}: the range starts at the cursor"
        );
    }
}

#[test]
fn ctrl_click_and_a_plain_click_set_the_anchor_as_explorer_does() {
    let mut p = loaded();
    click(&mut p, 1, false);
    click(&mut p, 4, true);
    assert_eq!(shift_click(&mut p, 6, false), (path(4), path(6)));
    assert_eq!(
        shift_click(&mut p, 2, true),
        (path(4), path(2)),
        "Ctrl+Shift+click adds a range from the same anchor"
    );
    click(&mut p, 7, false);
    assert_eq!(shift_click(&mut p, 9, false), (path(7), path(9)));
    // Without any anchor, a Shift+click starts at the cursor.
    let mut p = loaded();
    p.focus_display_row(PaneId::Left, p.panes[0].parent_offset() + 2);
    assert_eq!(shift_click(&mut p, 5, false), (path(2), path(5)));
}

#[test]
fn a_jump_of_the_cursor_starts_the_next_range_at_its_row() {
    let mut p = loaded();
    click(&mut p, 1, false);
    // Filename search, a right click or Alt+Down put the cursor on another row.
    p.focus_display_row(PaneId::Left, p.panes[0].parent_offset() + 6);
    assert_eq!(shift_click(&mut p, 8, false), (path(6), path(8)));
}

/// Brief and Thumbnails move the cursor through their columns and grid rows
/// (`panel_view_navigation`) and then focus the row; the next range starts there too.
#[test]
fn a_shift_range_after_grid_and_column_moves_starts_at_the_cursor() {
    for (mode, key, from) in [(2, 3, 5), (2, 12, 1), (1, 12, 4), (2, 5, 9)] {
        let mut p = loaded();
        click(&mut p, 1, false);
        assert_eq!(shift_click(&mut p, 0, false), (path(1), path(0)));
        let offset = p.panes[0].parent_offset();
        let count = p.panes[0].display_len() + offset;
        let current = p.panes[0].cursor_index().unwrap();
        // Thumbnails: five cells a grid row; Brief: four rows a column.
        let row = destination(mode, key, current, count, 5, 4).unwrap();
        p.focus_display_row(PaneId::Left, row);
        assert_eq!(cursor(&p), Some(from), "mode {mode}, key {key}");
        let target = if from == 7 { 8 } else { 7 };
        assert_eq!(
            shift_click(&mut p, target, false),
            (path(from), path(target)),
            "mode {mode}, key {key}"
        );
    }
}

/// One cursor move costs O(1) whatever the folder's size: nothing scans the rows.
#[test]
fn a_million_rows_keep_the_anchor_bookkeeping_constant() {
    let mut p = Presenter::new("left".into(), "right".into());
    let navigation = begin(&mut p, PaneId::Left, "left");
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        (0..1_000_000)
            .map(|row| {
                Entry::new(
                    EntryId(row),
                    format!("left/{row:07}").into(),
                    EntryKind::File,
                )
            })
            .collect(),
    );
    click(&mut p, 10, false);
    let started = std::time::Instant::now();
    for _ in 0..10_000 {
        key(&mut p, 3);
    }
    let offset = p.panes[0].parent_offset();
    let sent = RefCell::new(None);
    p.select_row_modified(PaneId::Left, offset + 999_999, false, true, &|command| {
        *sent.borrow_mut() = Some(command);
        true
    });
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "10,000 cursor moves and a range took {:?}",
        started.elapsed()
    );
    let Some(Command::Selection(SelectionRequest {
        action: SelectionAction::Range { anchor, .. },
        ..
    })) = sent.into_inner()
    else {
        panic!("Shift+click sent no range")
    };
    assert_eq!(anchor, PathBuf::from("left/0010010"));
}

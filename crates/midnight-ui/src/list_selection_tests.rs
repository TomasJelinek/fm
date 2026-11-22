use super::*;

/// Rows shown in this order, each by its slot.
struct Shown(Vec<usize>);
impl ListOrder for Shown {
    fn len(&self) -> usize {
        self.0.len()
    }
    fn slot_at(&self, index: usize) -> Option<usize> {
        self.0.get(index).copied()
    }
    fn index_of(&self, slot: usize) -> Option<usize> {
        self.0.iter().position(|shown| *shown == slot)
    }
}

/// Shown rows `0..count`, slot = index × 10 (identities are not places).
fn shown(count: usize) -> Shown {
    Shown((0..count).map(|index| index * 10).collect())
}

/// The selected rows by display index.
fn marked(selection: &ListSelection, order: &Shown) -> Vec<usize> {
    (0..order.len())
        .filter(|index| selection.is_selected(order.0[*index]))
        .collect()
}

#[test]
fn clicks_select_one_toggle_and_mark_ranges_from_the_anchor() {
    let order = shown(8);
    let mut selection = ListSelection::default();
    selection.click(&order, 2, false, false).unwrap();
    assert_eq!(marked(&selection, &order), [2]);
    assert_eq!(
        (selection.cursor(), selection.anchor()),
        (Some(20), Some(20))
    );
    // Ctrl toggles a row and moves the anchor to it.
    selection.click(&order, 5, true, false).unwrap();
    assert_eq!(marked(&selection, &order), [2, 5]);
    selection.click(&order, 2, true, false).unwrap();
    assert_eq!(marked(&selection, &order), [5]);
    assert_eq!(selection.anchor(), Some(20));
    // Shift marks the range from the anchor (row 2) and replaces the selection.
    selection.click(&order, 4, false, true).unwrap();
    assert_eq!(marked(&selection, &order), [2, 3, 4]);
    assert_eq!(
        (selection.cursor(), selection.anchor()),
        (Some(40), Some(20))
    );
    // A range upwards from the same anchor.
    selection.click(&order, 0, false, true).unwrap();
    assert_eq!(marked(&selection, &order), [0, 1, 2]);
    // Ctrl+Shift adds the range to the selection.
    selection.click(&order, 6, true, false).unwrap();
    selection.click(&order, 7, true, true).unwrap();
    assert_eq!(marked(&selection, &order), [0, 1, 2, 6, 7]);
    // A plain click selects that row alone.
    selection.click(&order, 3, false, false).unwrap();
    assert_eq!(marked(&selection, &order), [3]);
    assert_eq!(selection.len(), 1);
    assert!(
        selection.click(&order, 8, false, false).is_none(),
        "no such row"
    );
}

#[test]
fn cursor_keys_select_extend_from_the_anchor_or_move_the_cursor_alone() {
    let order = shown(6);
    let mut selection = ListSelection::default();
    selection.move_to(&order, 1, false, false).unwrap();
    assert_eq!(marked(&selection, &order), [1]);
    // Down selects the next row alone, as a click.
    selection.move_to(&order, 2, false, false).unwrap();
    assert_eq!(marked(&selection, &order), [2]);
    // Shift+Down twice extends from the anchor.
    selection.move_to(&order, 3, true, false).unwrap();
    selection.move_to(&order, 4, true, false).unwrap();
    assert_eq!(marked(&selection, &order), [2, 3, 4]);
    // Shift+Up shrinks the same range.
    selection.move_to(&order, 3, true, false).unwrap();
    assert_eq!(marked(&selection, &order), [2, 3]);
    // Ctrl moves the cursor alone; Space then adds its row.
    assert_eq!(
        selection.move_to(&order, 5, false, true).unwrap(),
        Vec::new()
    );
    assert_eq!(marked(&selection, &order), [2, 3]);
    assert_eq!(selection.cursor_index(&order), Some(5));
    selection.toggle_cursor(&order, false).unwrap();
    assert_eq!(marked(&selection, &order), [2, 3, 5]);
    // Without an anchor a Shift move starts from the cursor.
    let mut fresh = ListSelection::default();
    fresh.move_to(&order, 1, false, true).unwrap();
    fresh.move_to(&order, 3, true, false).unwrap();
    assert_eq!(marked(&fresh, &order), [1, 2, 3]);
    assert_eq!(fresh.anchor(), Some(10));
}

#[test]
fn space_toggles_the_cursor_row_and_insert_also_moves_down() {
    let order = shown(3);
    let mut selection = ListSelection::default();
    // No cursor yet: the first row.
    selection.toggle_cursor(&order, false).unwrap();
    assert_eq!(marked(&selection, &order), [0]);
    assert_eq!(selection.cursor_index(&order), Some(0));
    selection.toggle_cursor(&order, false).unwrap();
    assert!(selection.is_empty(), "Space toggles back");
    for _ in 0..4 {
        selection.toggle_cursor(&order, true).unwrap();
    }
    // Insert marks 0, 1 and 2, then stays on the last row and unmarks it.
    assert_eq!(marked(&selection, &order), [0, 1]);
    assert_eq!(selection.cursor_index(&order), Some(2));
    assert!(
        ListSelection::default()
            .toggle_cursor(&shown(0), false)
            .is_none()
    );
}

#[test]
fn select_all_and_deselect_all_keep_the_cursor_and_report_their_rows() {
    let order = shown(5);
    let mut selection = ListSelection::default();
    selection.click(&order, 1, false, false).unwrap();
    assert_eq!(selection.select_all(&order), one(0..5));
    assert_eq!(selection.len(), 5);
    assert_eq!(marked(&selection, &order), [0, 1, 2, 3, 4]);
    let changed = selection.clear(&order);
    assert_eq!(changed, one(0..5), "consecutive rows merge into one range");
    assert!(selection.is_empty());
    assert_eq!(selection.cursor_index(&order), Some(1));
    assert_eq!(selection.clear(&order), Vec::new(), "nothing to deselect");
    // A click reports the rows it unmarks and the row it marks.
    selection.click(&order, 0, true, false).unwrap();
    selection.click(&order, 3, true, false).unwrap();
    let changed = selection.click(&order, 2, false, false).unwrap();
    assert_eq!(changed, vec![0..1, 3..4, 2..3]);
}

#[test]
fn many_unmarked_rows_are_reported_as_every_shown_row() {
    let order = shown(LISTED_CHANGES + 10);
    let mut selection = ListSelection::default();
    selection.select_all(&order);
    // A gap: the marks are not one range, yet they are reported as every shown row.
    selection.click(&order, 3, true, false).unwrap();
    let changed = selection.click(&order, 7, false, false).unwrap();
    assert!(changed.contains(&(0..order.len())), "{changed:?}");
    assert_eq!(marked(&selection, &order), [7]);
}

#[test]
fn marks_cursor_and_anchor_follow_their_rows_when_rows_move() {
    let mut order = shown(5);
    let mut selection = ListSelection::default();
    selection.click(&order, 1, false, false).unwrap();
    selection.click(&order, 3, true, false).unwrap();
    // The rows are sorted the other way round: the same rows stay selected.
    order.0.reverse();
    assert_eq!(marked(&selection, &order), [1, 3]);
    assert_eq!(
        selection.cursor_index(&order),
        Some(1),
        "row 3 is shown at 1 now"
    );
    // A Shift range starts at the anchor's new place.
    selection.click(&order, 4, false, true).unwrap();
    assert_eq!(marked(&selection, &order), [1, 2, 3, 4]);
    // A row gets a new identity (a listed file gets its result row).
    selection.rename(30, 99);
    order.0[1] = 99;
    assert!(selection.is_selected(99) && !selection.is_selected(30));
    assert_eq!(selection.anchor(), Some(99));
    assert_eq!(marked(&selection, &order), [1, 2, 3, 4]);
    assert_eq!(selection.slots().collect::<Vec<_>>(), [0, 10, 20, 99]);
    selection.reset();
    assert_eq!(selection, ListSelection::default());
}

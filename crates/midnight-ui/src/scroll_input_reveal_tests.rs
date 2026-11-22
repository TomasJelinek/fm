//! Every cursor-navigation key shows the cursor row, even where the cursor stays: Home
//! and Up on the first row, End and Down on the last, Page Up and Page Down at either
//! end, and Left and Right in Brief and Thumbnails. A refresh, Insert and Space that
//! keep the row, and the wheel do not.
use super::*;

const UP: i32 = 2;
const DOWN: i32 = 3;
const PAGE_UP: i32 = 4;
const PAGE_DOWN: i32 = 5;
const HOME: i32 = 6;
const END: i32 = 7;
/// Space: toggle the row (with its folder size) and move down.
const SPACE: i32 = 8;
/// Insert: toggle the row and move down.
const INSERT: i32 = 9;
const LEFT: i32 = 11;
const RIGHT: i32 = 12;
const DETAILS: i32 = 0;
const BRIEF: i32 = 1;
const THUMBNAILS: i32 = 2;

fn files() -> Arc<[Entry]> {
    (0..1000)
        .map(|id| {
            Entry::new(
                EntryId(id),
                PathBuf::from(format!("file-{id:04}.txt")),
                EntryKind::File,
            )
        })
        .collect()
}

fn panels() -> Harness {
    let h = harness_entries(false, false, files);
    h.point(
        h.window.get_left_list_left() + 100.0,
        h.window.get_left_list_top() + 100.0,
    );
    h
}

fn cursor(h: &Harness) -> Option<usize> {
    h.presenter.borrow().panes[0].cursor_index()
}

/// In `view`, put the cursor on the first row (or the last) through Home (End), which
/// shows it, then turn the wheel three notches away from it. Returns the offset that
/// shows the cursor row.
fn scrolled_away_from_cursor(h: &Harness, view: i32, last: bool) -> f32 {
    h.window.invoke_view_mode(0, view);
    h.settle(60);
    h.window.invoke_key(0, if last { END } else { HOME });
    h.settle(60);
    let shown = h.offset(0);
    for _ in 0..3 {
        assert!(h.wheel(if last { 1.0 } else { -1.0 }));
        h.advance(2);
    }
    h.settle(90);
    assert!(
        (h.offset(0) - shown).abs() > 2.0 * h.window.get_left_view_row_height(),
        "view {view}: the wheel scrolled the list away from the cursor ({shown} -> {})",
        h.offset(0)
    );
    shown
}

/// In each view, `first` on the first row and `last` on the last row leave the cursor
/// where it is and bring its row back into view. Every case is checked, and the
/// failing ones are listed together.
#[track_caller]
fn assert_keys_reveal(views: &[i32], first: i32, last: i32) {
    let h = panels();
    let mut failures = Vec::new();
    for &view in views {
        for (key, on_last) in [(first, false), (last, true)] {
            let shown = scrolled_away_from_cursor(&h, view, on_last);
            let before = cursor(&h);
            h.window.invoke_key(0, key);
            h.settle(60);
            let (after, offset) = (cursor(&h), h.offset(0));
            if after != before || offset != shown {
                failures.push(format!(
                    "view {view}, key {key}: cursor {before:?} -> {after:?}, offset {offset} \
                     instead of {shown}"
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "keys that left the cursor row out of view:\n{}",
        failures.join("\n")
    );
}

#[test]
fn home_and_end_show_the_cursor_row_where_the_cursor_stays() {
    assert_keys_reveal(&[DETAILS, BRIEF, THUMBNAILS], HOME, END);
}

#[test]
fn up_and_down_show_the_cursor_row_at_the_first_and_last_rows() {
    assert_keys_reveal(&[DETAILS, BRIEF, THUMBNAILS], UP, DOWN);
}

#[test]
fn page_up_and_page_down_show_the_cursor_row_at_the_first_and_last_rows() {
    assert_keys_reveal(&[DETAILS, BRIEF, THUMBNAILS], PAGE_UP, PAGE_DOWN);
}

#[test]
fn left_and_right_show_the_cursor_row_in_brief_and_thumbnails() {
    assert_keys_reveal(&[BRIEF, THUMBNAILS], LEFT, RIGHT);
}

/// What is not navigation leaves a list the wheel scrolled where it is: a refresh, and
/// Insert or Space on the last row (they toggle it and the cursor stays).
#[test]
fn refreshes_and_toggles_that_keep_the_row_leave_a_scrolled_list_alone() {
    let h = panels();
    for view in [DETAILS, BRIEF, THUMBNAILS] {
        scrolled_away_from_cursor(&h, view, true);
        let away = h.offset(0);
        let last = cursor(&h);
        crate::refresh(&h.window, &h.presenter.borrow());
        h.settle(30);
        assert_eq!(h.offset(0), away, "view {view}: a refresh");
        for key in [INSERT, SPACE] {
            h.window.invoke_key(0, key);
            h.settle(30);
            assert_eq!(cursor(&h), last, "view {view}, key {key}: the cursor stays");
            assert_eq!(h.offset(0), away, "view {view}, key {key}");
        }
    }
}

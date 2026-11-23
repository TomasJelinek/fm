//! How far the scroll keys move text: the same in every viewer mode that scrolls by
//! pixels and in the file comparison, so a key feels the same everywhere. Pure policy.

/// Logical pixels Up and Down move.
pub const KEY_LINE_PIXELS: f32 = 28.0;
/// Logical pixels Left and Right move.
pub const KEY_COLUMN_PIXELS: f32 = 40.0;

/// The logical distance (x, y; positive moves the view right or down) of the scroll key
/// `action` over content whose view is `page` logical pixels tall: a line, a column, or
/// the whole view for Page Up and Page Down. `None` for any other action.
pub fn key_scroll(action: &str, page: f32) -> Option<(f32, f32)> {
    Some(match action {
        "viewer.up" => (0.0, -KEY_LINE_PIXELS),
        "viewer.down" => (0.0, KEY_LINE_PIXELS),
        "viewer.left" => (-KEY_COLUMN_PIXELS, 0.0),
        "viewer.right" => (KEY_COLUMN_PIXELS, 0.0),
        "viewer.page-up" => (0.0, -page),
        "viewer.page-down" => (0.0, page),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_move_a_line_a_column_or_a_page_and_nothing_else() {
        assert_eq!(key_scroll("viewer.down", 600.0), Some((0.0, 28.0)));
        assert_eq!(key_scroll("viewer.up", 600.0), Some((0.0, -28.0)));
        assert_eq!(key_scroll("viewer.right", 600.0), Some((40.0, 0.0)));
        assert_eq!(key_scroll("viewer.left", 600.0), Some((-40.0, 0.0)));
        assert_eq!(key_scroll("viewer.page-down", 600.0), Some((0.0, 600.0)));
        assert_eq!(key_scroll("viewer.page-up", 489.0), Some((0.0, -489.0)));
        assert_eq!(key_scroll("viewer.first", 600.0), None);
    }
}

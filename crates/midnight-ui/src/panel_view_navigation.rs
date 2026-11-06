//! Source-row movement for the two virtualized multi-column panel layouts.

pub(crate) fn destination(
    mode: i32,
    key: i32,
    current: usize,
    count: usize,
    columns: usize,
    visible_rows: usize,
) -> Option<usize> {
    if !matches!(mode, 1 | 2) || count == 0 {
        return None;
    }
    let columns = columns.max(1);
    let visible_rows = visible_rows.max(1);
    let distance = match key {
        2 | 3 => {
            if mode == 1 {
                1
            } else {
                columns
            }
        }
        11 | 12 => {
            if mode == 1 {
                visible_rows
            } else {
                1
            }
        }
        4 | 5 => columns.saturating_mul(visible_rows),
        _ => return None,
    };
    Some(if matches!(key, 2 | 4 | 11) {
        current.saturating_sub(distance)
    } else {
        current.saturating_add(distance).min(count - 1)
    })
}

#[cfg(test)]
mod tests {
    use super::destination;

    #[test]
    fn brief_moves_down_columns_and_pages_in_source_order() {
        assert_eq!(destination(1, 3, 2, 1000, 3, 20), Some(3));
        assert_eq!(destination(1, 12, 2, 1000, 3, 20), Some(22));
        assert_eq!(destination(1, 11, 22, 1000, 3, 20), Some(2));
        assert_eq!(destination(1, 5, 22, 1000, 3, 20), Some(82));
        assert_eq!(destination(1, 4, 82, 1000, 3, 20), Some(22));
    }

    #[test]
    fn thumbnails_move_across_rows_and_keep_column_on_vertical_moves() {
        assert_eq!(destination(2, 12, 2, 1000, 4, 3), Some(3));
        assert_eq!(destination(2, 3, 2, 1000, 4, 3), Some(6));
        assert_eq!(destination(2, 2, 6, 1000, 4, 3), Some(2));
        assert_eq!(destination(2, 5, 6, 1000, 4, 3), Some(18));
        assert_eq!(destination(2, 4, 18, 1000, 4, 3), Some(6));
    }

    #[test]
    fn boundaries_and_non_navigation_keys_preserve_existing_policy() {
        assert_eq!(destination(2, 2, 1, 10, 4, 3), Some(0));
        assert_eq!(destination(2, 3, 8, 10, 4, 3), Some(9));
        assert_eq!(destination(1, 11, 0, 10, 4, 3), Some(0));
        assert_eq!(destination(1, 5, 8, 10, 4, 3), Some(9));
        assert_eq!(destination(0, 3, 2, 10, 1, 20), None);
        assert_eq!(destination(2, 8, 2, 10, 1, 20), None);
        assert_eq!(destination(1, 3, 0, 0, 0, 0), None);
        assert_eq!(destination(2, 3, 0, 10, 0, 0), Some(1));
    }
}

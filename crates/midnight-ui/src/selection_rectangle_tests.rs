use super::*;

fn grid(mode: i32, columns: usize, count: usize) -> Grid {
    Grid {
        mode,
        columns,
        page_rows: 4,
        count,
        parent: 1,
        width: 300.0,
        row_height: 100.0,
    }
}
fn spans(grid: Grid, from: (f32, f32), to: (f32, f32)) -> Vec<(usize, usize)> {
    grid.ranges(from, to)
        .unwrap()
        .spans()
        .iter()
        .map(|s| (s.first, s.last))
        .collect()
}

#[test]
fn details_and_thumbnail_rectangles_skip_parent_and_keep_grid_holes() {
    assert_eq!(
        spans(grid(0, 1, 7), (290.0, 700.0), (10.0, 150.0)),
        vec![(0, 5)]
    );
    assert_eq!(
        spans(grid(2, 3, 11), (105.0, 105.0), (190.0, 295.0)),
        vec![(3, 3), (6, 6)]
    );
    assert_eq!(
        spans(grid(2, 3, 11), (0.0, 0.0), (300.0, 100.0)),
        vec![(0, 1)]
    );
}

#[test]
fn brief_rectangle_uses_column_major_pages_and_partial_last_page() {
    assert_eq!(
        spans(grid(1, 3, 28), (105.0, 150.0), (190.0, 650.0)),
        vec![(4, 6), (15, 17)]
    );
    assert_eq!(
        spans(grid(1, 3, 28), (0.0, 400.0), (300.0, 800.0)),
        vec![(11, 22)]
    );
    assert_eq!(
        spans(grid(1, 3, 28), (0.0, 800.0), (300.0, 1200.0)),
        vec![(23, 26)]
    );
}

#[test]
fn empty_cell_hit_testing_preserves_row_gestures_and_rejects_bad_geometry() {
    let g = grid(2, 3, 5);
    assert!(!g.empty_at((150.0, 50.0)));
    assert!(g.empty_at((250.0, 150.0)));
    assert!(g.empty_at((50.0, 250.0)));
    assert!(!g.empty_at((301.0, 150.0)));
    assert!(!g.empty_at((f32::NAN, 0.0)));
    assert!(g.ranges((0.0, 0.0), (f32::INFINITY, 100.0)).is_err());
}

#[test]
fn huge_full_width_drag_compacts_and_sparse_drag_stops_at_explicit_budget() {
    let g = grid(2, 3, 30_000_000);
    assert_eq!(
        spans(g, (0.0, 0.0), (300.0, 1_000_000_000.0)),
        vec![(0, 29_999_998)]
    );
    let g = grid(2, 3, 30_000_000);
    assert!(g.ranges((105.0, 0.0), (190.0, 1_000_000_000.0)).is_err());
    let g = grid(1, 3, 30_000_000);
    assert_eq!(
        spans(g, (0.0, 0.0), (300.0, 1_000_000_000.0)),
        vec![(0, 29_999_998)]
    );
}
#[test]
fn point_hit_uses_same_column_major_and_row_major_layout_and_rejects_gutters() {
    let grid = super::Grid {
        mode: 0,
        columns: 1,
        page_rows: 10,
        count: 42,
        parent: 1,
        width: 400.0,
        row_height: 20.0,
    };
    assert_eq!(grid.row_at((25.0, 65.0)), Some(3));
    assert_eq!(grid.row_at((0.0, 0.0)), Some(0));
    assert_eq!(grid.row_at((400.0, 10.0)), None);
    assert_eq!(grid.row_at((25.0, -1.0)), None);
    assert_eq!(grid.row_at((25.0, f32::NAN)), None);
    assert_eq!(grid.row_at((25.0, 1000.0)), None);
    let brief = super::Grid {
        mode: 1,
        columns: 2,
        ..grid
    };
    assert_eq!(brief.row_at((250.0, 65.0)), Some(13));
    assert_eq!(brief.row_at((250.0, 205.0)), Some(30));
    let thumbs = super::Grid {
        mode: 2,
        columns: 2,
        ..grid
    };
    assert_eq!(thumbs.row_at((250.0, 65.0)), Some(7));
}

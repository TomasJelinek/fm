use super::*;

fn map() -> ViewerScrollMap {
    ViewerScrollMap::new(
        [(0.0, 0.0), (100.0, 0.1), (200.0, 0.7), (300.0, 1.0)]
            .into_iter()
            .map(|(y, fraction)| ViewerScrollAnchor { y, fraction })
            .collect(),
    )
    .unwrap()
}

#[test]
fn source_interval_couples_position_extent_and_reaches_real_endpoints() {
    let map = map();
    assert_eq!(map.metrics(0.0, 100.0), Some((0.0, 0.1)));
    let (position, extent) = map.metrics(100.0, 100.0).unwrap();
    assert!((position - 0.25).abs() < 1e-12);
    assert!((extent - 0.6).abs() < 1e-12);
    assert_eq!(map.metrics(200.0, 100.0).unwrap().0, 1.0);
    assert_eq!(map.metrics(0.0, 400.0), Some((0.0, 1.0)));
}

#[test]
fn inverse_round_trips_in_both_directions_and_rejects_uncovered_targets() {
    let map = map();
    for y in [0.0, 7.25, 95.75, 100.0, 132.375, 199.9, 200.0, 85.5] {
        let position = map.metrics(y, 100.0).unwrap().0;
        assert!((map.inverse(position, 100.0, 0.0, 200.0).unwrap() - y).abs() < 1e-8);
    }
    assert!(map.inverse(0.9, 100.0, 0.0, 50.0).is_none());
    assert!(map.inverse(f64::NAN, 100.0, 0.0, 200.0).is_none());
    assert!(map.inverse(0.5, 100.0, 200.0, 0.0).is_none());
}

#[test]
fn rebase_shares_rows_and_preserves_fractional_metrics_and_inverse() {
    let map = map();
    let shifted = map.translated(-837.375);
    assert!(Arc::ptr_eq(&map.anchors, &shifted.anchors));
    for y in [12.75, 98.625, 142.0] {
        let expected = map.metrics(y, 100.0).unwrap();
        assert_eq!(shifted.metrics(y - 837.375, 100.0), Some(expected));
        let actual = shifted
            .inverse(expected.0, 100.0, -837.375, -637.375)
            .unwrap();
        assert!((actual - (y - 837.375)).abs() < 1e-8);
    }
}

#[test]
fn both_normal_and_minimum_sized_thumbs_move_monotonically_with_variable_density() {
    let map = map();
    for track in [40.0_f64, 600.0] {
        let mut last = -1.0;
        for step in 0..=800 {
            let (position, extent) = map.metrics(step as f64 / 4.0, 100.0).unwrap();
            let thumb = (track * extent).max(28.0).min(track);
            let top = position * (track - thumb);
            assert!(
                top >= last - 1e-10,
                "thumb reversed at {step}: {last} -> {top}"
            );
            last = top;
        }
    }
}

#[test]
fn malformed_and_unloaded_coordinates_are_not_silently_extrapolated() {
    for rows in [
        vec![(0.0, 0.0)],
        vec![(0.0, 0.2), (0.0, 0.3)],
        vec![(0.0, 0.6), (1.0, 0.5)],
        vec![(0.0, 0.0), (f64::NAN, 1.0)],
    ] {
        assert!(
            ViewerScrollMap::new(
                rows.into_iter()
                    .map(|(y, fraction)| ViewerScrollAnchor { y, fraction })
                    .collect()
            )
            .is_none()
        );
    }
    let map = ViewerScrollMap::new(vec![
        ViewerScrollAnchor {
            y: 0.0,
            fraction: 0.2,
        },
        ViewerScrollAnchor {
            y: 100.0,
            fraction: 0.4,
        },
    ])
    .unwrap();
    assert!(map.metrics(-1.0, 50.0).is_none());
    assert!(map.metrics(75.0, 50.0).is_none());
}

#[test]
fn unchanged_plateau_seek_keeps_the_displayed_occurrence_but_endpoints_remain_absolute() {
    let map = ViewerScrollMap::new(
        [(0.0, 0.2), (1000.0, 0.2), (1100.0, 0.3)]
            .into_iter()
            .map(|(y, fraction)| ViewerScrollAnchor { y, fraction })
            .collect(),
    )
    .unwrap();
    for position in [0.2, f64::from(0.2_f32)] {
        assert_eq!(
            map.inverse_near(position, 100.0, 0.0, 1000.0, 500.0),
            Some(500.0)
        );
    }
    assert!(map.inverse_near(0.21, 100.0, 0.0, 1000.0, 500.0).unwrap() > 900.0);
    assert!(
        map.inverse_near(0.2, 100.0, 1001.0, 1100.0, 500.0)
            .is_none()
    );
    let edges = ViewerScrollMap::new(
        [(0.0, 0.0), (100.0, 0.0), (200.0, 1.0), (300.0, 1.0)]
            .into_iter()
            .map(|(y, fraction)| ViewerScrollAnchor { y, fraction })
            .collect(),
    )
    .unwrap();
    assert_eq!(edges.inverse_near(0.0, 50.0, 0.0, 250.0, 25.0), Some(0.0));
    assert_eq!(
        edges.inverse_near(1.0, 50.0, 0.0, 250.0, 225.0),
        Some(250.0)
    );
}

#[test]
fn cached_blank_plateaus_do_not_masquerade_as_uncovered_real_endpoints() {
    let map = ViewerScrollMap::new(
        [(0.0, 0.0), (200.0, 0.0), (300.0, 1.0), (500.0, 1.0)]
            .into_iter()
            .map(|(y, fraction)| ViewerScrollAnchor { y, fraction })
            .collect(),
    )
    .unwrap();
    assert_eq!(map.metrics(100.0, 50.0).unwrap().0, 0.0);
    assert!(map.inverse_near(0.0, 50.0, 50.0, 150.0, 100.0).is_none());
    assert_eq!(map.metrics(400.0, 50.0).unwrap().0, 1.0);
    assert!(map.inverse_near(1.0, 50.0, 350.0, 425.0, 400.0).is_none());
    assert_eq!(map.inverse_near(0.0, 50.0, 0.0, 450.0, 100.0), Some(0.0));
    assert_eq!(map.inverse_near(1.0, 50.0, 0.0, 450.0, 400.0), Some(450.0));
    assert_eq!(map.inverse_near(1.0, 600.0, 0.0, 0.0, 0.0), Some(0.0));
    for viewport in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(map.inverse_near(1.0, viewport, 0.0, 450.0, 100.0).is_none());
    }
}

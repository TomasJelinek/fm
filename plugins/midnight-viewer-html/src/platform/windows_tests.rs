use super::*;

/// What Chromium scrolls for `units` of a CEF wheel delta when a notch scrolls `count`
/// lines of 100/3 DIPs (CEF's `TranslateWebWheelEvent`).
fn scrolled(units: f64, count: u32) -> f64 {
    units / 120.0 * f64::from(count) * 100.0 / 3.0
}

#[test]
fn a_dip_of_host_travel_scrolls_one_dip_at_any_line_count() {
    for count in 1..=12 {
        let per = units_per_dip(count);
        assert!(
            (scrolled(60.0 * per, count) - 60.0).abs() < 1e-9,
            "{count} lines"
        );
    }
    // Three lines, Windows' default: 1.2 units per DIP; one unit per DIP scrolled 5/6.
    assert!((units_per_dip(3) - 1.2).abs() < 1e-12);
    assert!((scrolled(120.0, 3) - 100.0).abs() < 1e-9);
}

#[test]
fn a_quantum_is_the_fewest_units_that_scroll_whole_dips() {
    for count in 1..=12 {
        let quantum = quantum(count);
        let dips = scrolled(f64::from(quantum), count);
        assert!((dips - dips.round()).abs() < 1e-9, "{count} lines: {dips}");
        for smaller in 1..quantum {
            let dips = scrolled(f64::from(smaller), count);
            assert!(
                (dips - dips.round()).abs() > 1e-9,
                "{count} lines, {smaller} units"
            );
        }
    }
    // At three lines, six units scroll five DIPs.
    assert_eq!(quantum(3), 6);
}

#[test]
fn settings_that_cannot_scroll_by_pixels_keep_the_default() {
    for count in [None, Some(0), Some(WHEEL_PAGESCROLL)] {
        assert_eq!(scroll_count(count), 3, "{count:?}");
    }
    assert_eq!(scroll_count(Some(7)), 7);
}

#[test]
fn the_system_setting_is_read() {
    let units = wheel_units();
    for axis in 0..2 {
        assert!(units.per_dip[axis].is_finite() && units.per_dip[axis] > 0.0);
        assert!(units.quantum[axis] > 0);
    }
}

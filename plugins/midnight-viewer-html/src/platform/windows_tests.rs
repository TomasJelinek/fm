use super::*;
use crate::platform::{MAX_WHEEL_DIPS, WheelStep};

/// The DIPs `units` are at `lines` before CEF's `f32` arithmetic: 5 lines / 18 a unit.
fn exact(units: u32, lines: u32) -> f64 {
    f64::from(units) * 5.0 * f64::from(lines) / 18.0
}

#[test]
fn a_unit_scrolls_what_cef_computes_in_f32_without_the_fraction() {
    // Three lines, Windows' default: a notch of 120 units scrolls 100 DIPs, so the
    // host's 120 DIPs are 144 units and six units scroll 5 DIPs.
    let three = WheelScale::with_lines(3);
    assert_eq!(three.dips(120), 100);
    assert_eq!(three.dips(144), 120);
    assert_eq!(three.dips(6), 5);
    // Chromium drops each event's fraction: three units are 2.5 DIPs and scroll 2.
    assert_eq!(three.dips(3), 2);
    // At one line, 216 units are 60 DIPs exactly, but CEF's f32 arithmetic gives
    // 59.999996 and Chromium scrolls 59.
    let one = WheelScale::with_lines(1);
    assert_eq!(216 * 5 % 18, 0);
    assert_eq!(one.dips(216), 59);
    assert_eq!(one.dips(217), 60);
    for lines in 1..=100 {
        let scale = WheelScale::with_lines(lines);
        for units in [1, 7, 100, 1000, 100_000] {
            let (dips, exact) = (f64::from(scale.dips(units)), exact(units, lines));
            let rounding = exact * 1e-6;
            assert!(
                dips <= exact + rounding && dips > exact - 1.0 - rounding,
                "{lines} lines, {units} units: {dips} of {exact}"
            );
        }
    }
}

#[test]
fn a_step_is_the_fewest_units_that_scroll_the_most_dips_not_past_its_own() {
    for lines in [1, 2, 3, 4, 5, 6, 7, 8, 10, 15, 20, 100] {
        let scale = WheelScale::with_lines(lines);
        for wanted in (0..=400).chain([5_000, MAX_WHEEL_DIPS]) {
            for sign in [1, -1] {
                let step = scale.step(sign * wanted);
                let units = step.units.unsigned_abs();
                let scrolled = scale.dips(units);
                let context = format!("{lines} lines, {} DIPs: {step:?}", sign * wanted);
                assert_eq!(step.dips, sign * i64::from(scrolled), "{context}");
                assert!(step.dips.abs() <= wanted, "{context}");
                // The next DIPs any number of units scrolls pass `wanted`.
                let more = (units..)
                    .map(|units| scale.dips(units))
                    .find(|&dips| dips > scrolled);
                assert!(
                    more.is_some_and(|dips| i64::from(dips) > wanted),
                    "{context}"
                );
                // Fewer units scroll fewer DIPs, so nothing fits means no units at all.
                assert!(units == 0 || scale.dips(units - 1) < scrolled, "{context}");
                assert!(
                    step.units == 0 || step.units.signum() as i64 == sign,
                    "{context}"
                );
            }
        }
    }
    // Not one DIP: three units at one line are 0.83 DIP, which Chromium drops, so a
    // remainder's event would scroll nothing (and, for a negative one, point upward).
    assert_eq!(
        WheelScale::with_lines(1).step(0),
        WheelStep { units: 0, dips: 0 }
    );
    // At three lines or fewer a unit is at most 5/6 DIP: every whole DIP is reachable.
    for lines in 1..=3 {
        let scale = WheelScale::with_lines(lines);
        for wanted in 0..=400 {
            assert_eq!(scale.step(wanted).dips, wanted, "{lines} lines");
        }
    }
    // Above three lines some are not: at five lines (25/18 DIP a unit) three units
    // scroll 4, so 3 DIPs are two units scrolling 2.
    assert_eq!(
        WheelScale::with_lines(5).step(3),
        WheelStep { units: 2, dips: 2 }
    );
    assert_eq!(
        WheelScale::with_lines(1).step(60),
        WheelStep {
            units: 217,
            dips: 60
        }
    );
    assert_eq!(
        WheelScale::with_lines(3).step(-60),
        WheelStep {
            units: -72,
            dips: -60
        }
    );
    // Beyond any page an event is bounded.
    assert_eq!(
        WheelScale::with_lines(3).step(i64::MAX),
        WheelScale::with_lines(3).step(MAX_WHEEL_DIPS)
    );
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
    for scale in wheel_scales() {
        assert!(scale.lines > 0);
        let step = scale.step(60);
        assert!(
            step.units >= 0 && (0..=60).contains(&step.dips),
            "{scale:?}: {step:?}"
        );
    }
}

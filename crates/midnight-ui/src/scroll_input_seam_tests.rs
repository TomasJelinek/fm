//! The acceptance seam (feature `acceptance-seams`) reaches the panels through the
//! observer closure that `on_winit_window_event` calls, and moves them as native input
//! does.
use super::*;
use crate::acceptance_seams::{
    NativeWheel, SystemSettings, WheelSetting, focus, override_settings, serial, wheel,
};

fn files() -> Arc<[Entry]> {
    (0..5000)
        .map(|id| {
            Entry::new(
                EntryId(id),
                PathBuf::from(format!("file-{id:05}.txt")),
                EntryKind::File,
            )
        })
        .collect()
}

/// Both panels list 5,000 files; frames reach only the window's stored observer.
fn seam_harness() -> Harness {
    let h = harness_entries(false, false, files);
    h.fixture_frames.set(false);
    h
}

fn over_left_list(h: &Harness) -> slint::LogicalPosition {
    slint::LogicalPosition::new(
        h.window.get_left_list_left() + 100.0,
        h.window.get_left_list_top() + 100.0,
    )
}

const NOTCH_DOWN: NativeWheel = NativeWheel::Lines { x: 0.0, y: -1.0 };

fn defaults(reduce_motion: bool) -> SystemSettings {
    SystemSettings {
        wheel: Some(WheelSetting::default()),
        reduce_motion: Some(reduce_motion),
    }
}

fn to_top(h: &Harness) {
    h.window.invoke_scroll_selection(0, -h.offset(0));
    h.settle(60);
    assert_eq!(h.offset(0), 0.0);
}

/// The same notch through the test fixture's observer (the production order of the
/// native hook on the test's own state) and through the seam (the window's stored
/// observer closure, which `on_winit_window_event` calls): the panel moves alike,
/// frame for frame.
#[test]
fn a_seam_notch_moves_a_panel_frame_for_frame_like_the_native_observer_fixture() {
    let _serial = serial();
    let _settings = override_settings(defaults(false));
    let h = harness_entries(false, false, files);
    let at = over_left_list(&h);
    h.point(at.x, at.y);
    assert!(h.wheel(-1.0));
    let fixture: Vec<f32> = (0..20)
        .map(|_| {
            h.advance(1);
            h.offset(0)
        })
        .collect();
    h.settle(60);
    to_top(&h);
    h.fixture_frames.set(false);
    assert_eq!(wheel(&h.window, at, NOTCH_DOWN), Some(true));
    let seam: Vec<f32> = (0..20)
        .map(|_| {
            h.advance(1);
            h.offset(0)
        })
        .collect();
    assert_eq!(seam, fixture);
    let row = h.window.get_left_view_row_height();
    assert!(
        fixture.windows(2).filter(|pair| pair[1] < pair[0]).count() >= 4,
        "the notch glides over several frames: {fixture:?}"
    );
    let rest = fixture[fixture.len() - 1];
    assert!((rest + notch(row)).abs() < 1e-3, "{rest}");
}

/// One notch moves the rows nearest to the viewers' notch distance for the system's
/// lines per notch, or one screen, which the panels read on a focus gain.
#[test]
fn one_seam_notch_moves_the_rows_nearest_the_notch_of_each_wheel_setting() {
    let _serial = serial();
    let h = seam_harness();
    let at = over_left_list(&h);
    let row = h.window.get_left_view_row_height();
    let page = h.window.get_left_list_height();
    for setting in [
        WheelSetting::Lines(1),
        WheelSetting::Lines(3),
        WheelSetting::Lines(6),
        WheelSetting::Page,
    ] {
        let _settings = override_settings(SystemSettings {
            wheel: Some(setting),
            reduce_motion: Some(false),
        });
        assert_eq!(focus(&h.window, true), Some(()));
        to_top(&h);
        assert_eq!(wheel(&h.window, at, NOTCH_DOWN), Some(true));
        h.settle(60);
        let rows = (notch_pixels_for(setting, page) / row).round().max(1.0);
        let offset = h.offset(0);
        assert!((offset + rows * row).abs() < 1e-3, "{setting:?}: {offset}");
    }
}

/// With animation effects off, the panels read less motion and a notch moves at once.
#[test]
fn with_reduced_motion_a_seam_notch_moves_at_once() {
    let _serial = serial();
    let h = seam_harness();
    let _settings = override_settings(defaults(true));
    assert_eq!(focus(&h.window, true), Some(()));
    assert!(h.window.get_reduce_motion());
    assert_eq!(wheel(&h.window, over_left_list(&h), NOTCH_DOWN), Some(true));
    h.advance(1);
    let offset = h.offset(0);
    assert!(
        (offset + notch(h.window.get_left_view_row_height())).abs() < 1e-3,
        "{offset}"
    );
}

/// A focus loss reaches the observer (a gliding notch stops where it is) and Slint
/// (the window is no longer active); a focus gain re-reads the system settings.
#[test]
fn seam_focus_changes_reach_the_observer_and_slint() {
    let _serial = serial();
    let h = seam_harness();
    let _settings = override_settings(defaults(false));
    let active = || WindowInner::from_pub(h.window.window()).active();
    assert_eq!(focus(&h.window, true), Some(()));
    assert!(active());
    assert!(!h.window.get_reduce_motion());
    assert_eq!(wheel(&h.window, over_left_list(&h), NOTCH_DOWN), Some(true));
    h.advance(3);
    let stopped = h.offset(0);
    let row = h.window.get_left_view_row_height();
    assert!(stopped < 0.0 && stopped > -notch(row), "{stopped}");
    assert_eq!(focus(&h.window, false), Some(()));
    assert!(!active());
    // Shorter than the 100 ms rest before the rows align.
    h.advance(3);
    assert_eq!(h.offset(0), stopped);
    let _less_motion = override_settings(defaults(true));
    assert!(!h.window.get_reduce_motion());
    assert_eq!(focus(&h.window, true), Some(()));
    assert!(active());
    assert!(h.window.get_reduce_motion());
}

/// A wheel outside the lists is left to Slint; software windows keep Slint's own
/// keys; a hidden window takes no native input.
#[test]
fn the_seam_leaves_other_input_to_slint_and_needs_an_open_window() {
    let _serial = serial();
    let h = seam_harness();
    let _settings = override_settings(defaults(false));
    let header = slint::LogicalPosition::new(
        h.window.get_left_list_left() + 100.0,
        h.window.get_left_list_top() - 2.0,
    );
    assert_eq!(wheel(&h.window, header, NOTCH_DOWN), Some(false));
    h.settle(30);
    assert_eq!(h.offset(0), 0.0);
    assert!(!h.window.get_native_shortcuts_authoritative());
    h.window.hide().unwrap();
    assert_eq!(wheel(&h.window, over_left_list(&h), NOTCH_DOWN), None);
    assert_eq!(focus(&h.window, true), None);
}

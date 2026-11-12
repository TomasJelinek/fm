//! Actual overflow controls retain panel ownership, reveal every drive and clamp on resize.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;
use i_slint_core::{accessibility::AccessibleStringProperty, items::AccessibleRole};
use midnight_ui::{LocationRole, LocationRow};
use slint::{ModelRc, VecModel};
use std::{cell::RefCell, rc::Rc};
fn enabled(test: &ui::TestUi, label: &str) -> bool {
    ui::root_item(&test.window, label, AccessibleRole::Button)
        .accessible_string_property(AccessibleStringProperty::Enabled)
        .is_some_and(|v| v == "true")
}
fn origin(test: &ui::TestUi, label: &str) -> f32 {
    let item = ui::root_item(&test.window, label, AccessibleRole::Button);
    item.map_to_native_window(item.geometry().origin).x
}
#[test]
fn overflowing_drive_toolbar_scrolls_to_both_ends_and_keeps_the_other_pane_still() {
    let mut test = ui::TestUi::new();
    let rows: Vec<_> = (0..24)
        .map(|i| LocationRow {
            identity: format!("drive-{i}").into(),
            kind: LocationRole::Drive,
            is_drive: true,
            compact_label: format!("{}:", char::from(b'A' + i)).into(),
            tooltip: format!("Test drive {i}").into(),
            ..Default::default()
        })
        .collect();
    test.window
        .set_left_locations(ModelRc::new(VecModel::from(rows)));
    test.window
        .set_right_locations(ModelRc::new(VecModel::from(vec![LocationRow {
            tooltip: "Other panel drive".into(),
            compact_label: "Z:".into(),
            is_drive: true,
            kind: LocationRole::Drive,
            ..Default::default()
        }])));
    let selected = Rc::new(RefCell::new(Vec::new()));
    let capture = selected.clone();
    test.window
        .on_navigation_choice(move |pane, id| capture.borrow_mut().push((pane, id.to_string())));
    test.draw();
    let left = "Scroll left panel drives left";
    let right = "Scroll left panel drives right";
    assert!(ui::root_item(&test.window, right, AccessibleRole::Button).is_visible());
    assert!(!enabled(&test, left));
    let initial = origin(&test, "Test drive 0");
    let other = origin(&test, "Other panel drive");
    let mut steps = 0;
    while enabled(&test, right) {
        ui::click_root(&test.window, right, AccessibleRole::Button);
        test.draw();
        steps += 1;
        assert!(steps < 80);
    }
    assert!(steps > 0);
    assert!(origin(&test, "Test drive 0") < initial);
    assert_eq!(origin(&test, "Other panel drive"), other);
    let final_drive = ui::root_item(&test.window, "Test drive 23", AccessibleRole::Button);
    assert!(
        origin(&test, "Test drive 23") + final_drive.geometry().width()
            <= origin(&test, right) + 1.0
    );
    ui::click_root(&test.window, "Test drive 23", AccessibleRole::Button);
    assert_eq!(*selected.borrow(), [(0, "drive-23".into())]);
    while enabled(&test, left) {
        ui::click_root(&test.window, left, AccessibleRole::Button);
        test.draw();
        steps += 1;
        assert!(steps < 160);
    }
    assert!((origin(&test, "Test drive 0") - initial).abs() < 1.0);
    test.resize(800, 600);
    assert!(ui::root_item(&test.window, right, AccessibleRole::Button).is_visible());
    ui::click_root(&test.window, right, AccessibleRole::Button);
    test.draw();
    // At the absolute minimum, the established compact layout hides the drive bar.
    test.resize(320, 200);
    test.resize(4000, 820);
    assert!(!ui::root_item(&test.window, right, AccessibleRole::Button).is_visible());
    assert!(!ui::root_item(&test.window, left, AccessibleRole::Button).is_visible());
    assert!(origin(&test, "Test drive 0") >= 0.0);
    test.resize(1340, 820);
    render::snapshot(&test, "drive-toolbar-overflow");
}

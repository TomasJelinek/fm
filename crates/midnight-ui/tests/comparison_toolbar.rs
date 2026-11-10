//! Shared comparison buttons use the current active panel's existing commands.
#[path = "support/render.rs"]
mod render;
#[path = "support/toolbar.rs"]
mod toolbar;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{accessibility::AccessibleStringProperty, items::AccessibleRole};
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc};

fn label(files: bool) -> &'static str {
    if files {
        "Compare files"
    } else {
        "Compare directories"
    }
}

#[test]
fn comparison_buttons_follow_the_active_panel_even_when_panels_are_swapped() {
    let mut test = ui::TestUi::new();
    test.window.set_left_can_target_selection(true);
    test.window.set_right_can_target_selection(true);
    let actions = Rc::new(RefCell::new(Vec::new()));
    let captured = actions.clone();
    let weak = test.window.as_weak();
    test.window.on_activate(move |pane| {
        weak.upgrade().unwrap().set_active_pane(pane);
        captured.borrow_mut().push(format!("activate:{pane}"));
    });
    let captured = actions.clone();
    test.window.on_compare_files(move |pane| {
        captured.borrow_mut().push(format!("files:{pane}"));
    });
    let captured = actions.clone();
    let weak = test.window.as_weak();
    test.window.on_open_comparison(move || {
        captured.borrow_mut().push(format!(
            "directories:{}",
            weak.upgrade().unwrap().get_active_pane()
        ));
    });
    for density in [0, 1] {
        test.window.invoke_apply_preferences(0, density);
        for swapped in [false, true] {
            test.window.set_panels_swapped(swapped);
            for width in [1340, 900, 660] {
                test.resize(width, 820);
                for pane in [0, 1] {
                    for files in [true, false] {
                        test.window.set_active_pane(pane);
                        test.draw();
                        actions.borrow_mut().clear();
                        let label = label(files);
                        toolbar::reveal(&mut test, label, AccessibleRole::Button);
                        let position =
                            ui::root_position(&test.window, label, AccessibleRole::Button);
                        assert!(position.y < test.window.get_left_pane_top());
                        ui::click_root(&test.window, label, AccessibleRole::Button);
                        let kind = if files { "files" } else { "directories" };
                        assert_eq!(
                            *actions.borrow(),
                            [format!("{kind}:{pane}")],
                            "density {density}, swapped {swapped}, width {width}, {label}"
                        );
                    }
                }
            }
        }
    }
    render::snapshot(&test, "comparison-toolbar-compact-touch");
    test.window.invoke_apply_preferences(0, 0);
    test.resize(1340, 820);
    render::snapshot(&test, "comparison-toolbar");
}

#[test]
fn file_pair_picker_stays_available_while_directory_comparison_requires_native_views() {
    let mut test = ui::TestUi::new();
    let files_label = label(true);
    let directories_label = label(false);
    let enabled = |label: &str| {
        ui::root_item(&test.window, label, AccessibleRole::Button)
            .accessible_string_property(AccessibleStringProperty::Enabled)
            .is_some_and(|value| value == "true")
    };
    assert!(
        enabled(files_label),
        "a missing file pair opens the explicit picker"
    );
    assert!(enabled(directories_label), "comparison needs no selection");
    test.window.set_left_can_target_selection(true);
    assert!(enabled(files_label));
    test.window.set_right_native_filesystem(false);
    assert!(
        !enabled(directories_label),
        "both directories must be native"
    );
    // Two selected files in the native active panel may still be compared.
    assert!(enabled(files_label));
    test.window.set_left_native_filesystem(false);
    assert!(
        enabled(files_label),
        "virtual views can still open the explicit file picker"
    );
    test.draw();
    let invocations = Rc::new(RefCell::new(0));
    let captured = invocations.clone();
    test.window
        .on_compare_files(move |_| *captured.borrow_mut() += 1);
    ui::click_root(&test.window, files_label, AccessibleRole::Button);
    assert_eq!(*invocations.borrow(), 1);
}

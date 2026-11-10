//! Compare/filter dialogs retain drafts on validation and keep actions in the viewport.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{accessibility::AccessibleStringProperty, items::AccessibleRole};
use midnight_ui::{ComparisonView, PanelFilterView};
use slint::{ComponentHandle, platform::Key};
use std::{cell::RefCell, rc::Rc};

fn settle(test: &mut ui::TestUi) {
    for _ in 0..2 {
        slint::platform::update_timers_and_animations();
        test.draw();
    }
}

fn action_in_viewport(test: &ui::TestUi, label: &str) {
    let item = ui::popup_item(&test.window, label, AccessibleRole::Button);
    let bounds = item.geometry();
    let origin = item.map_to_native_window(bounds.origin);
    assert!(
        origin.x >= 0.0
            && origin.y >= 0.0
            && origin.x + bounds.width() <= test.width as f32
            && origin.y + bounds.height() <= test.height as f32,
        "{label} outside viewport: {origin:?} {bounds:?}"
    );
}

fn footer_actions_aligned(test: &ui::TestUi, action: &str, dismissal: &str) {
    let action = ui::popup_item(&test.window, action, AccessibleRole::Button);
    let dismissal = ui::popup_item(&test.window, dismissal, AccessibleRole::Button);
    let action_bounds = action.geometry();
    let dismiss_bounds = dismissal.geometry();
    let action_top = action.map_to_native_window(action_bounds.origin).y;
    let dismiss_top = dismissal.map_to_native_window(dismiss_bounds.origin).y;
    assert!(
        (action_top - dismiss_top).abs() < 1.0,
        "footer actions must share a top edge: {action_top} vs {dismiss_top}"
    );
    assert!((action_bounds.height() - dismiss_bounds.height()).abs() < 1.0);
}

// Tab through the real native focus chain. At the minimum size some body
// controls can be clipped; only send Space when its checkbox is in view.
fn toggle_reachable_checkbox_by_keyboard(test: &mut ui::TestUi, label: &str) -> bool {
    use i_slint_core::{item_tree::ParentItemTraversalMode, items::Flickable, window::WindowInner};
    for _ in 0..24 {
        ui::key(&test.window, Key::Tab);
        settle(test);
        let checkbox = ui::popup_item(&test.window, label, AccessibleRole::Checkbox);
        let inner = WindowInner::from_pub(test.window.window());
        let mut focused = inner.focus_item.borrow().upgrade();
        let mut matches = false;
        while let Some(item) = focused {
            if item == checkbox {
                matches = true;
                break;
            }
            focused = item.parent_item(ParentItemTraversalMode::StopAtPopups);
        }
        if !matches {
            continue;
        }
        let bounds = checkbox.geometry();
        let origin = checkbox.map_to_native_window(bounds.origin);
        let mut visible = checkbox.is_visible()
            && origin.x >= 0.0
            && origin.y >= 0.0
            && origin.x + bounds.width() <= test.width as f32 + 0.5
            && origin.y + bounds.height() <= test.height as f32 + 0.5;
        let mut parent = checkbox.parent_item(ParentItemTraversalMode::StopAtPopups);
        while let Some(item) = parent {
            if item.downcast::<Flickable>().is_some() {
                let clip = item.map_to_native_window(item.geometry().origin);
                visible &= origin.x >= clip.x
                    && origin.y >= clip.y
                    && origin.x + bounds.width() <= clip.x + item.geometry().width() + 0.5
                    && origin.y + bounds.height() <= clip.y + item.geometry().height() + 0.5;
            }
            parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
        }
        if visible {
            ui::key(&test.window, Key::Space);
            settle(test);
            return true;
        }
    }
    false
}

#[test]
fn comparison_opened_at_minimum_size_keeps_keyboard_draft_and_escape() {
    let mut test = ui::TestUi::new();
    test.resize(320, 200);
    let submissions = Rc::new(RefCell::new(Vec::new()));
    let captured = submissions.clone();
    test.window
        .on_submit_comparison(move |size, time, case, tolerance, directories| {
            captured
                .borrow_mut()
                .push((size, time, case, tolerance, directories));
        });
    let weak = test.window.as_weak();
    test.window.on_cancel_comparison(move || {
        let window = weak.upgrade().unwrap();
        let mut model = window.get_comparison_view();
        model.open = false;
        window.set_comparison_view(model);
    });
    test.window.set_comparison_view(ComparisonView {
        open: true,
        left_path: "C:\\文档\\left".into(),
        right_path: "C:\\文档\\right".into(),
        by_size: false,
        by_modified: true,
        case_sensitive: true,
        tolerance_seconds: "17".into(),
        unmatched_directories: true,
        ..Default::default()
    });
    settle(&mut test);
    let toggled = toggle_reachable_checkbox_by_keyboard(&mut test, "Compare file size");
    assert!(
        submissions.borrow().is_empty(),
        "Tab and Space never submit a comparison"
    );
    assert_eq!(
        ui::popup_item(
            &test.window,
            "Time tolerance in seconds",
            AccessibleRole::TextInput
        )
        .accessible_string_property(AccessibleStringProperty::Value)
        .unwrap(),
        "17"
    );
    assert_eq!(
        ui::popup_item(&test.window, "Compare file size", AccessibleRole::Checkbox)
            .accessible_string_property(AccessibleStringProperty::Checked)
            .unwrap()
            .as_str(),
        if toggled { "true" } else { "false" }
    );
    // No pointer event repairs focus before this Escape.
    ui::key(&test.window, Key::Escape);
    settle(&mut test);
    assert!(!test.window.get_comparison_view().open);
}

#[test]
fn filter_opened_at_minimum_size_keeps_keyboard_draft_and_escape() {
    let mut test = ui::TestUi::new();
    test.resize(320, 200);
    let submissions = Rc::new(RefCell::new(Vec::new()));
    let captured = submissions.clone();
    test.window
        .on_apply_panel_filter(move |pane, enabled, mode, expression, case| {
            captured
                .borrow_mut()
                .push((pane, enabled, mode, expression, case));
        });
    let weak = test.window.as_weak();
    test.window.on_cancel_panel_filter(move || {
        let window = weak.upgrade().unwrap();
        let mut model = window.get_panel_filter_view();
        model.open = false;
        window.set_panel_filter_view(model);
    });
    test.window.set_panel_filter_view(PanelFilterView {
        open: true,
        pane: 1,
        mode: 0,
        expression: "*.český;*.世界".into(),
        case_sensitive: true,
        ..Default::default()
    });
    settle(&mut test);
    let toggled = toggle_reachable_checkbox_by_keyboard(&mut test, "Case sensitive");
    assert!(
        submissions.borrow().is_empty(),
        "Tab and Space never apply a filter"
    );
    assert_eq!(
        ui::popup_item(
            &test.window,
            "File filter expression",
            AccessibleRole::TextInput
        )
        .accessible_string_property(AccessibleStringProperty::Value)
        .unwrap(),
        "*.český;*.世界"
    );
    assert_eq!(
        ui::popup_item(&test.window, "Case sensitive", AccessibleRole::Checkbox)
            .accessible_string_property(AccessibleStringProperty::Checked)
            .unwrap()
            .as_str(),
        if toggled { "false" } else { "true" }
    );
    // No pointer event repairs focus before this Escape.
    ui::key(&test.window, Key::Escape);
    settle(&mut test);
    assert!(!test.window.get_panel_filter_view().open);
}

#[test]
fn filter_ok_enables_a_draft_and_show_all_is_a_separate_direct_action() {
    let mut test = ui::TestUi::new();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let output = calls.clone();
    test.window
        .on_apply_panel_filter(move |pane, enabled, _, expression, _| {
            output
                .borrow_mut()
                .push((pane, enabled, expression.to_string()));
        });
    let cleared = Rc::new(RefCell::new(Vec::new()));
    let output = cleared.clone();
    test.window
        .on_clear_panel_filter(move |pane| output.borrow_mut().push(pane));
    test.window.set_panel_filter_view(PanelFilterView {
        open: true,
        pane: 1,
        enabled: false,
        expression: "*.rs".into(),
        ..Default::default()
    });
    settle(&mut test);
    ui::click_popup(&test.window, "OK", AccessibleRole::Button);
    assert_eq!(*calls.borrow(), [(1, true, "*.rs".into())]);
    let mut view = test.window.get_panel_filter_view();
    view.enabled = true;
    test.window.set_panel_filter_view(view);
    settle(&mut test);
    ui::click_popup(&test.window, "Show all", AccessibleRole::Button);
    assert_eq!(*cleared.borrow(), [1]);
    assert_eq!(
        calls.borrow().len(),
        1,
        "clearing needs no validation submission"
    );
}

#[test]
fn comparison_dialog_keeps_draft_and_actions_visible_at_large_fonts() {
    let mut test = ui::TestUi::new();
    test.resize(1040, 820);
    let submissions = Rc::new(RefCell::new(Vec::new()));
    let captured = submissions.clone();
    test.window
        .on_submit_comparison(move |size, time, case, tolerance, directories| {
            captured
                .borrow_mut()
                .push((size, time, case, tolerance, directories));
        });
    let weak = test.window.as_weak();
    test.window.on_cancel_comparison(move || {
        let window = weak.upgrade().unwrap();
        let mut model = window.get_comparison_view();
        model.open = false;
        window.set_comparison_view(model);
    });
    for density in [0, 1] {
        for font in [11, 32] {
            test.window.invoke_apply_fonts("".into(), font, font);
            test.window.invoke_apply_preferences(0, density);
            test.window.set_comparison_view(ComparisonView {
                open: true,
                by_size: true,
                by_modified: true,
                left_path: "C:\\文档\\left".into(),
                right_path: "C:\\文档\\right".into(),
                tolerance_seconds: "2".into(),
                ..Default::default()
            });
            settle(&mut test);
            action_in_viewport(&test, "Compare");
            action_in_viewport(&test, "Close");
            footer_actions_aligned(&test, "Compare", "Close");
            ui::click_popup(
                &test.window,
                "Time tolerance in seconds",
                AccessibleRole::TextInput,
            );
            ui::chord(&test.window, &[Key::Control], "a");
            ui::key(&test.window, "12.5");
            let mut model = test.window.get_comparison_view();
            model.error = "Validation feedback preserves the typed draft.".into();
            test.window.set_comparison_view(model);
            settle(&mut test);
            render::snapshot(&test, &format!("comparison-{density}-{font}"));
            let mut model = test.window.get_comparison_view();
            model.busy = true;
            test.window.set_comparison_view(model);
            settle(&mut test);
            action_in_viewport(&test, "Cancel");
            let mut model = test.window.get_comparison_view();
            model.busy = false;
            test.window.set_comparison_view(model);
            settle(&mut test);
            ui::click_popup(&test.window, "Compare", AccessibleRole::Button);
            assert_eq!(submissions.borrow().last().unwrap().3, "12.5");
            ui::key(&test.window, Key::Escape);
            settle(&mut test);
            assert!(!test.window.get_comparison_view().open);
        }
    }
}

#[test]
fn filter_dialog_preserves_expression_and_captured_pane_after_validation_feedback() {
    let mut test = ui::TestUi::new();
    test.resize(1040, 820);
    let submissions = Rc::new(RefCell::new(Vec::new()));
    let captured = submissions.clone();
    test.window
        .on_apply_panel_filter(move |pane, enabled, regex, expression, case| {
            captured
                .borrow_mut()
                .push((pane, enabled, regex, expression, case));
        });
    let weak = test.window.as_weak();
    test.window.on_cancel_panel_filter(move || {
        let window = weak.upgrade().unwrap();
        let mut model = window.get_panel_filter_view();
        model.open = false;
        window.set_panel_filter_view(model);
    });
    for density in [0, 1] {
        for font in [11, 32] {
            test.window.invoke_apply_fonts("".into(), font, font);
            test.window.invoke_apply_preferences(0, density);
            test.window.set_panel_filter_view(PanelFilterView {
                open: true,
                pane: 1,
                enabled: true,
                expression: "*.txt".into(),
                ..Default::default()
            });
            settle(&mut test);
            action_in_viewport(&test, "OK");
            action_in_viewport(&test, "Cancel");
            footer_actions_aligned(&test, "OK", "Cancel");
            footer_actions_aligned(&test, "Show all", "Cancel");
            ui::click_popup(
                &test.window,
                "File filter expression",
                AccessibleRole::TextInput,
            );
            ui::chord(&test.window, &[Key::Control], "a");
            ui::key(&test.window, "*.rs;*.md");
            let mut model = test.window.get_panel_filter_view();
            model.error = "Validation feedback preserves the typed draft.".into();
            test.window.set_panel_filter_view(model);
            test.window.set_active_pane(0);
            settle(&mut test);
            render::snapshot(&test, &format!("filter-{density}-{font}"));
            ui::click_popup(&test.window, "OK", AccessibleRole::Button);
            let submissions = submissions.borrow();
            let latest = submissions.last().unwrap();
            assert_eq!(latest.0, 1);
            assert!(latest.1);
            assert_eq!(latest.3, "*.rs;*.md");
            drop(submissions);
            ui::key(&test.window, Key::Escape);
            settle(&mut test);
            assert!(!test.window.get_panel_filter_view().open);
        }
    }
}

#[test]
fn expression_mode_uses_the_explicit_picker_and_keeps_draft_on_feedback_at_minimum_size() {
    let mut test = ui::TestUi::new();
    test.resize(1040, 820);
    let submissions = Rc::new(RefCell::new(Vec::new()));
    let captured = submissions.clone();
    test.window
        .on_apply_panel_filter(move |pane, enabled, mode, expression, case| {
            captured
                .borrow_mut()
                .push((pane, enabled, mode.to_string(), expression, case));
        });
    let weak = test.window.as_weak();
    test.window.on_cancel_panel_filter(move || {
        let window = weak.upgrade().unwrap();
        let mut model = window.get_panel_filter_view();
        model.open = false;
        window.set_panel_filter_view(model);
    });
    test.window.set_panel_filter_view(PanelFilterView {
        open: true,
        pane: 1,
        enabled: true,
        expression: "kind = file and size >= 10MiB".into(),
        ..Default::default()
    });
    settle(&mut test);
    ui::click_popup(&test.window, "Filter mode", AccessibleRole::Combobox);
    ui::key(&test.window, Key::DownArrow);
    ui::key(&test.window, Key::DownArrow);
    ui::key(&test.window, Key::Return);
    settle(&mut test);
    assert!(
        submissions.borrow().is_empty(),
        "Choosing a mode does not apply a draft"
    );
    let mut model = test.window.get_panel_filter_view();
    model.error = "Line 1, column 24: Use an unsigned byte size.".into();
    test.window.set_panel_filter_view(model);
    test.window.set_active_pane(0);
    test.resize(320, 200);
    settle(&mut test);
    action_in_viewport(&test, "OK");
    action_in_viewport(&test, "Cancel");
    ui::click_popup(&test.window, "OK", AccessibleRole::Button);
    let calls = submissions.borrow();
    assert_eq!(calls.last().unwrap().0, 1);
    assert_eq!(calls.last().unwrap().2, "2");
    assert_eq!(calls.last().unwrap().3, "kind = file and size >= 10MiB");
    drop(calls);
    ui::key(&test.window, Key::Escape);
    settle(&mut test);
    assert!(!test.window.get_panel_filter_view().open);
}

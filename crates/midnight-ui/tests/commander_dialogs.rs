//! Compare/filter dialogs retain drafts on validation and keep actions in the viewport.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::items::AccessibleRole;
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
            action_in_viewport(&test, "Apply");
            action_in_viewport(&test, "Cancel");
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
            ui::click_popup(&test.window, "Apply", AccessibleRole::Button);
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
    action_in_viewport(&test, "Apply");
    action_in_viewport(&test, "Cancel");
    ui::click_popup(&test.window, "Apply", AccessibleRole::Button);
    let calls = submissions.borrow();
    assert_eq!(calls.last().unwrap().0, 1);
    assert_eq!(calls.last().unwrap().2, "2");
    assert_eq!(calls.last().unwrap().3, "kind = file and size >= 10MiB");
    drop(calls);
    ui::key(&test.window, Key::Escape);
    settle(&mut test);
    assert!(!test.window.get_panel_filter_view().open);
}

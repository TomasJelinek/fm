//! Options footer alignment and diagnostics at actual rendered sizes.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    item_tree::ItemRc,
    items::AccessibleRole,
    window::{WindowInner, WindowKind},
};
use slint::ComponentHandle;

fn popup(test: &ui::TestUi) -> ItemRc {
    let popups = WindowInner::from_pub(test.window.window()).active_popups();
    let popup = popups
        .iter()
        .rev()
        .find(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
        .unwrap();
    ItemRc::new_root(popup.component.clone())
}

fn bounds(test: &ui::TestUi, label: &str) -> (f32, f32, f32, f32) {
    let item = ui::popup_item(&test.window, label, AccessibleRole::Button);
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    (origin.x, origin.y, rect.width(), rect.height())
}

#[test]
fn reset_uses_left_margin_while_ok_and_cancel_stay_right_with_matching_heights() {
    let mut test = ui::TestUi::new();
    for (width, height, density, font) in [(1340, 820, 0, 11), (800, 600, 1, 24), (520, 420, 0, 18)]
    {
        test.window.invoke_apply_fonts("".into(), font, font);
        test.window.invoke_apply_preferences(0, density);
        test.resize(width, height);
        test.window.invoke_show_options();
        test.draw();
        let reset = bounds(&test, "Reset to defaults");
        let ok = bounds(&test, "OK");
        let cancel = bounds(&test, "Cancel");
        let left = test.window.get_options_left();
        let right = left + test.window.get_options_width();
        assert!(
            (reset.0 - left - 18.0).abs() < 2.0,
            "Reset belongs at the left content margin: {reset:?}, left={left}"
        );
        assert!(
            (right - cancel.0 - cancel.2 - 18.0).abs() < 2.0,
            "Cancel belongs at the right content margin: {cancel:?}, right={right}"
        );
        assert!(ok.0 >= reset.0 + reset.2 + 8.0);
        assert!((cancel.0 - ok.0 - ok.2 - 8.0).abs() < 2.0);
        assert!((reset.1 - ok.1).abs() < 1.0 && (ok.1 - cancel.1).abs() < 1.0);
        assert!((reset.3 - ok.3).abs() < 1.0 && (ok.3 - cancel.3).abs() < 1.0);
        for button in [reset, ok, cancel] {
            assert!(button.0 >= 0.0 && button.1 >= 0.0);
            assert!(button.0 + button.2 <= width as f32 && button.1 + button.3 <= height as f32);
        }
        render::snapshot(&test, &format!("options-footer-{width}-{font}"));
        ui::click_popup(&test.window, "Cancel", AccessibleRole::Button);
        test.draw();
    }
}

#[test]
fn footer_does_not_spend_visible_space_on_preview_instructions() {
    let mut test = ui::TestUi::new();
    let instructions = "Preview only. OK saves; Cancel discards changes.";
    test.window.set_preferences_save_status(instructions.into());
    test.window.invoke_show_options();
    test.draw();
    assert!(
        !ui::visible_text_values(popup(&test))
            .iter()
            .any(|text| text == instructions)
    );
}

#[test]
fn footer_keeps_actual_save_failure_visible_above_its_actions() {
    let mut test = ui::TestUi::new();
    let message = "Settings could not be saved. Access denied.";
    test.window.set_preferences_save_status(message.into());
    test.window.set_preferences_save_failed(true);
    test.window.invoke_show_options();
    test.draw();
    let error = ui::popup_item(&test.window, message, AccessibleRole::Text);
    let rect = error.geometry();
    let origin = error.map_to_native_window(rect.origin);
    let reset = bounds(&test, "Reset to defaults");
    assert!(
        origin.y + rect.height() <= reset.1,
        "save errors need their own visible line"
    );
    assert!(error.is_visible());
    render::snapshot(&test, "options-footer-save-error");
}

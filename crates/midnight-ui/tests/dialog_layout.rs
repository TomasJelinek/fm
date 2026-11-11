//! Render actual dialog controls with long Unicode paths and every theme/density.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;
use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::ItemRc,
    items::AccessibleRole,
    window::{WindowInner, WindowKind},
};
use midnight_ui::{KeyboardEditorView, ShortcutCommandRow};
use render::snapshot;
use slint::ComponentHandle;

fn find(item: ItemRc, label: &str) -> Option<ItemRc> {
    if matches!(
        item.accessible_role(),
        AccessibleRole::Button | AccessibleRole::Combobox
    ) && item
        .accessible_string_property(AccessibleStringProperty::Label)
        .is_some_and(|text| text == label)
    {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = find(item, label) {
            return Some(found);
        }
    }
    None
}
fn popup_button(test: &ui::TestUi, label: &str) -> ItemRc {
    let inner = WindowInner::from_pub(test.window.window());
    let popups = inner.active_popups();
    let popup = popups
        .iter()
        .rev()
        .find(|p| !matches!(p.window_kind, WindowKind::ToolTip))
        .unwrap();
    find(ItemRc::new_root(popup.component.clone()), label)
        .unwrap_or_else(|| panic!("missing {label}"))
}
fn inside(test: &ui::TestUi, item: ItemRc, label: &str) {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    assert!(
        rect.size.width > 0.0
            && rect.size.height > 0.0
            && origin.x >= 0.0
            && origin.y >= 0.0
            && origin.x + rect.size.width <= test.width as f32 + 1.0
            && origin.y + rect.size.height <= test.height as f32 + 1.0,
        "{label} clipped: {origin:?}, {rect:?}"
    );
}

// Native Jobs, decisions and Hot Paths retain the former geometry/icon checks
// in native_tool_layout; adapter identity and close semantics have dedicated tests.

#[test]
fn dialogs_keep_actions_visible_in_all_themes_and_densities() {
    let mut test = ui::TestUi::new();
    // The synthetic dialog fixture represents an actionable file caret.
    test.window.set_left_can_target_cursor(true);
    test.window.set_left_can_target_selection(true);
    test.resize(1040, 720);
    test.window.set_network_available(true);
    test.window
        .set_left_path("/very/long/文档/👩🏽‍💻/folder".into());
    let editor = test.window.global::<KeyboardEditorView>();
    editor.set_commands(slint::ModelRc::new(slint::VecModel::from(vec![
        ShortcutCommandRow {
            id: "files.rename".into(),
            label: "Rename focused item".into(),
            aliases: "F2".into(),
            selected: true,
        },
    ])));
    editor.set_selected_id("files.rename".into());
    editor.set_selected_label("Rename focused item".into());
    editor.set_aliases(slint::ModelRc::new(slint::VecModel::from(vec![
        "F2".into(),
    ])));
    editor.set_scope("File list and filename search".into());
    for scheme in midnight_core::PreferenceTheme::ALL.map(|theme| theme.index()) {
        for (density, font) in [(0, 11), (1, 15), (0, 32), (1, 32)] {
            test.window.invoke_apply_preferences(scheme, density);
            test.window.invoke_apply_fonts("".into(), font, font);
            test.draw();
            for label in ["F1 Help", "F2 Rename", "F12 Disconnect", "Options", "Jobs"] {
                inside(
                    &test,
                    ui::root_item(&test.window, label, AccessibleRole::Button),
                    label,
                );
            }
            let label = format!("theme-{scheme}-density-{density}-font-{font}");
            // Native operation and tool form layout have dedicated fixtures.
            test.window.invoke_show_options();
            for topic in [0, 1, 3, 4, 5, 6, 7, 8, 9] {
                test.window.set_options_topic_index(topic);
                test.draw();
                inside(&test, popup_button(&test, "Cancel"), "Options Done");
                if topic == 3 {
                    inside(
                        &test,
                        ui::popup_item(&test.window, "Search commands", AccessibleRole::TextInput),
                        "Keyboard search",
                    );
                    if font != 32 {
                        inside(&test, popup_button(&test, "Assign"), "Assign");
                    }
                }
                if topic == 6 {
                    inside(
                        &test,
                        popup_button(&test, "Clear thumbnail cache"),
                        "Clear cache",
                    );
                }
                if font != 32 || (scheme == 2 && density == 1) {
                    snapshot(&test, &format!("{label}-options-{topic}"));
                }
            }
            ui::click_popup(&test.window, "Cancel", AccessibleRole::Button);
        }
    }
    test.window.hide().unwrap();
}

//! Native operation content keeps all lines and actions at every theme/font/density.
#[path = "support/operation_form.rs"]
mod ui;
use i_slint_core::{
    item_tree::ItemRc,
    items::{AccessibleRole, ComplexText, SimpleText},
    window::WindowInner,
};
use slint::ComponentHandle;
fn find_text(item: ItemRc, value: &str) -> Option<ItemRc> {
    let text = item
        .downcast::<SimpleText>()
        .map(|t| t.as_pin_ref().text())
        .or_else(|| {
            item.downcast::<ComplexText>()
                .map(|t| t.as_pin_ref().text())
        });
    if item.is_visible() && text.is_some_and(|text| text == value) {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = find_text(item, value) {
            return Some(found);
        }
    }
    None
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

#[test]
fn native_operation_layout_preserves_long_unicode_content_and_action_bounds() {
    let mut test = ui::TestUi::new();
    test.resize(1040, 720);
    for scheme in midnight_core::PreferenceTheme::ALL.map(|theme| theme.index()) {
        for (density, font) in [(0, 11), (1, 15), (0, 32), (1, 32)] {
            test.window
                .invoke_apply_preferences(scheme, density, "".into(), font, font);
            let label = format!("native-theme-{scheme}-density-{density}-font-{font}");
            for (kind, title, has_name, warning) in [
                (0, "Rename", true, ""),
                (
                    1,
                    "Move to Trash",
                    false,
                    "Selected items will be removed from this folder and moved to Trash.",
                ),
                (
                    2,
                    "Permanently delete",
                    false,
                    "Permanent deletion cannot be undone.",
                ),
                (3, "Create directory", true, ""),
                (4, "New empty file", true, ""),
            ] {
                test.window.set_kind(kind);
                test.window.set_title_text(title.into());
                test.window.set_confirm_text(title.into());
                test.window.set_has_name(has_name);
                test.window.set_dangerous(kind == 2);
                test.window.set_warning(warning.into());
                test.window.set_initial_name("文档 👩🏽‍💻.txt".into());
                test.window.set_body(format!("Location: /very/long/文档/e\u{301}/👩🏽‍💻/folder\n\n{}\n123 456 789 bytes · 06.11.2026 10:20:30", "source-file-with-long-Unicode-name-文档.txt\n".repeat(20)).into());
                test.window.invoke_begin();
                test.draw();
                let body = test.window.get_body();
                let text = find_text(
                    ItemRc::new_root(WindowInner::from_pub(test.window.window()).component()),
                    &body,
                )
                .unwrap();
                assert_eq!(
                    text.geometry().origin,
                    Default::default(),
                    "Dialog text must start at its scrollable content origin, not be centered/clipped"
                );
                assert!(
                    text.geometry().height() >= body.lines().count() as f32 * font as f32,
                    "Every body line needs scrollable space; a fixed-height text clips its beginning/end: {:?}",
                    text.geometry()
                );
                for label in [title, "Cancel"] {
                    inside(
                        &test,
                        ui::root_item(&test.window, label, AccessibleRole::Button),
                        label,
                    );
                }
                if font != 32 {
                    ui::snapshot(&test, &format!("{label}-operation-{kind}"));
                }
            }
        }
    }
}

#[test]
fn rename_initial_height_fits_its_summary_without_a_large_empty_body() {
    let mut test = ui::TestUi::new();
    test.window
        .invoke_apply_preferences(3, 0, "Segoe UI".into(), 12, 15);
    test.window.set_kind(0);
    test.window.set_title_text("Rename".into());
    test.window.set_confirm_text("Rename".into());
    test.window.set_has_name(true);
    test.window.set_input_label("New name".into());
    test.window.set_initial_name("Picture 文档.jpg".into());
    test.window
        .set_body("Current item\nC:\\Users\\user\\Pictures\\Backgrounds\\Picture 文档.jpg".into());
    test.resize(test.window.get_initial_width().round() as usize, 300);
    let height = test.window.get_initial_height().ceil() as usize;
    assert!(
        (220..=360).contains(&height),
        "Unexpected initial rename height {height}"
    );
    test.resize(test.width, height);
    test.window.invoke_begin();
    test.draw();
    for (label, role) in [
        ("Rename", AccessibleRole::Button),
        ("Cancel", AccessibleRole::Button),
        ("New name", AccessibleRole::TextInput),
    ] {
        inside(&test, ui::root_item(&test.window, label, role), label);
    }
    let root = ItemRc::new_root(WindowInner::from_pub(test.window.window()).component());
    inside(
        &test,
        find_text(root, "Rename").expect("The form needs an internal title"),
        "title",
    );
    ui::snapshot(&test, "rename-content-sized");
}

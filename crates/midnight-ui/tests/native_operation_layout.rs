//! Embedded operation content keeps all lines and actions at every theme/font/density.
#[path = "support/operation_form.rs"]
mod ui;
use i_slint_core::{
    item_tree::ItemRc,
    items::{AccessibleRole, ComplexText, SimpleText},
};
use midnight_ui::DetailRow;
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

#[test]
fn native_operation_layout_preserves_long_unicode_content_and_action_bounds() {
    let mut test = ui::TestUi::new();
    test.resize(1040, 720);
    // Themes select colours only (ui/theme.slint), so every theme is laid out and
    // checked, but only the first one is software-rendered per density, font and kind.
    let themes = midnight_core::PreferenceTheme::ALL.map(|theme| theme.index());
    for scheme in themes {
        let render = scheme == themes[0]
            || std::env::var_os("MIDNIGHT_UI_RENDER_DIR").is_some_and(|dir| !dir.is_empty());
        for (density, font) in [(0, 11), (1, 15), (0, 32), (1, 32)] {
            test.apply_preferences(scheme, density, "", font, font);
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
                let form = test.form();
                form.set_kind(kind);
                form.set_title_text(title.into());
                form.set_confirm_text(title.into());
                form.set_has_name(has_name);
                form.set_dangerous(kind == 2);
                form.set_warning(warning.into());
                form.set_initial_name("文档 👩🏽‍💻.txt".into());
                form.set_body(format!("Location: /very/long/文档/e\u{301}/👩🏽‍💻/folder\n\n{}\n123 456 789 bytes · 06.11.2026 10:20:30", "source-file-with-long-Unicode-name-文档.txt\n".repeat(20)).into());
                test.begin();
                if render {
                    test.draw();
                } else {
                    test.layout();
                }
                let body = test.form().get_body();
                let text = find_text(test.popup(), &body).unwrap();
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
                    ui::assert_inside(&ui::root_item(&test, label, AccessibleRole::Button), label);
                }
                if font != 32 {
                    ui::snapshot(&test, &format!("{label}-operation-{kind}"));
                }
            }
        }
    }
}

/// One target is named in the question and the details; a long name without spaces
/// wraps inside the dialog, which keeps its width and its actions.
#[test]
fn a_long_single_delete_name_wraps_inside_the_dialog() {
    let mut test = ui::TestUi::new();
    test.apply_preferences(3, 0, "Segoe UI", 12, 15);
    let warning = "Permanent deletion cannot be undone.";
    let question =
        |name: &str| format!("Permanently delete the folder “{name}” and everything in it?");
    let details = |name: &str| {
        let row = |label: &str, value: &str| DetailRow {
            label: label.into(),
            value: value.into(),
            ..DetailRow::default()
        };
        slint::ModelRc::new(slint::VecModel::from(vec![
            row("Name", name),
            row("Type", "Folder"),
            row("Modified", "11/20/2026 01:17:41 PM"),
        ]))
    };
    let long = format!("20261120-052223-{}end", "folder-name-文档-👩🏽‍💻-".repeat(12));
    for (width, height) in [(1040, 720), (420, 520)] {
        test.resize(width, height);
        let mut drawn = Vec::new();
        for name in ["20261120-052223", long.as_str()] {
            let form = test.form();
            form.set_kind(2);
            form.set_title_text("Permanently delete".into());
            form.set_confirm_text("Permanently delete".into());
            form.set_has_name(false);
            form.set_dangerous(true);
            form.set_warning(warning.into());
            let question = question(name);
            form.set_body(question.clone().into());
            form.set_question(question.clone().into());
            form.set_details(details(name));
            form.set_detail_labels("Name\nType\nModified".into());
            test.begin();
            test.draw();
            let text = find_text(test.popup(), &question).expect("the question is drawn");
            assert!(
                find_text(test.popup(), warning).is_some(),
                "the warning is drawn"
            );
            let (origin, size) = ui::body(&ui::root_item(&test, "Cancel", AccessibleRole::Button));
            let at = text.map_to_native_window(text.geometry().origin);
            assert!(
                at.x >= origin.x - 0.5
                    && at.x + text.geometry().width() <= origin.x + size.width + 0.5,
                "the text stays inside the {width} px dialog: {at:?} {:?}, {origin:?} {size:?}",
                text.geometry()
            );
            for label in ["Permanently delete", "Cancel"] {
                ui::assert_inside(&ui::root_item(&test, label, AccessibleRole::Button), label);
            }
            drawn.push(text.geometry().height());
        }
        assert!(
            drawn[1] >= drawn[0] + 12.0,
            "the long name wraps onto more rows instead of widening the dialog: {drawn:?}"
        );
        ui::snapshot(&test, &format!("delete-long-name-{width}"));
    }
}

#[test]
fn rename_initial_height_fits_its_summary_without_a_large_empty_body() {
    let mut test = ui::TestUi::new();
    test.apply_preferences(3, 0, "Segoe UI", 12, 15);
    let form = test.form();
    form.set_kind(0);
    form.set_title_text("Rename".into());
    form.set_confirm_text("Rename".into());
    form.set_has_name(true);
    form.set_input_label("New name".into());
    form.set_initial_name("Picture 文档.jpg".into());
    form.set_body("Current item\nC:\\Users\\user\\Pictures\\Backgrounds\\Picture 文档.jpg".into());
    // In a larger main window the dialog body takes the form's initial size.
    test.resize(1340, 900);
    test.begin();
    test.draw();
    let (_, size) = ui::body(&ui::root_item(&test, "Cancel", AccessibleRole::Button));
    assert_eq!(size.width, 540.0, "the initial width at a 12 px font");
    let height = size.height.ceil() as usize;
    assert!(
        (220..=360).contains(&height),
        "Unexpected initial rename height {height}"
    );
    for (label, role) in [
        ("Rename", AccessibleRole::Button),
        ("Cancel", AccessibleRole::Button),
        ("New name", AccessibleRole::TextInput),
    ] {
        ui::assert_inside(&ui::root_item(&test, label, role), label);
    }
    ui::assert_inside(
        &find_text(test.popup(), "Rename").expect("The form needs an internal title"),
        "title",
    );
    ui::snapshot(&test, "rename-content-sized");
}

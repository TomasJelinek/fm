//! Render actual dialog controls with long Unicode paths and every theme/density.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;
use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::ItemRc,
    items::{AccessibleRole, ComplexText, SimpleText},
    window::{WindowInner, WindowKind},
};
use midnight_ui::{
    BookmarkRow, KeyboardEditorView, NavigationChoiceRow, OperationRow, ShortcutCommandRow,
};
use render::snapshot;
use slint::ComponentHandle;

fn find_text(item: ItemRc, value: &str) -> Option<ItemRc> {
    let text = item
        .downcast::<SimpleText>()
        .map(|t| t.as_pin_ref().text())
        .or_else(|| {
            item.downcast::<ComplexText>()
                .map(|t| t.as_pin_ref().text())
        });
    if text.is_some_and(|text| text == value) {
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

#[test]
fn dialogs_keep_actions_visible_in_all_themes_and_densities() {
    let mut test = ui::TestUi::new();
    test.resize(1040, 720);
    test.window.set_network_available(true);
    test.window.set_operations(slint::ModelRc::new(slint::VecModel::from(vec![OperationRow {
        identity: "copy-job:1".into(), title: "Copy · 文档 · waiting for a decision".into(),
        detail: "123 456 bytes copied · 123 456 bytes verified with SHA-256\n/very/long/文档/source-file.txt".into(),
        cancellable: true, attention: true, ..Default::default()
    }])));
    test.window
        .set_left_path("/very/long/文档/👩🏽‍💻/folder".into());
    test.window.set_left_bookmark_name("文档 👩🏽‍💻".into());
    test.window
        .set_bookmarks(slint::ModelRc::new(slint::VecModel::from(vec![
            BookmarkRow {
                identity: "1".into(),
                label: "文档 · Saved project".into(),
                path: "/very/long/文档/👩🏽‍💻/project".into(),
                slot: 1,
            },
        ])));
    test.window
        .set_hot_path_choices(slint::ModelRc::new(slint::VecModel::from(vec![
            NavigationChoiceRow {
                identity: "manage-hot-paths".into(),
                label: "Manage Hot Paths…".into(),
                ..Default::default()
            },
        ])));
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
    for scheme in 0..8 {
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
                test.window.set_operation_token("1".into());
                test.window.set_operation_title(title.into());
                test.window.set_operation_kind(kind);
                test.window.set_operation_has_name(has_name);
                test.window.set_operation_dangerous(kind == 2);
                test.window.set_operation_warning(warning.into());
                test.window.set_operation_name("文档 👩🏽‍💻.txt".into());
                test.window.set_operation_body(format!("Location: /very/long/文档/e\u{301}/👩🏽‍💻/folder\n\n{}\n123 456 789 bytes · 06.11.2026 10:20:30", "source-file-with-long-Unicode-name-文档.txt\n".repeat(20)).into());
                test.window.invoke_begin_operation();
                test.draw();
                let body = test.window.get_operation_body();
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
                    snapshot(&test, &format!("{label}-operation-{kind}"));
                }
                test.window.set_operation_token("".into());
            }
            test.window.set_copy_token("copy:1:1".into());
            test.window.set_copy_name("文档 👩🏽‍💻.txt".into());
            test.window.set_copy_body(format!("Source\n/very/long/文档/source.txt\n123 456 bytes · 06.11.2026 10:20:30\n\nExisting destination\n/very/long/文档/destination.txt\n234 567 bytes · 05.11.2026 08:00:00\n\n{}", "Choose another name or skip. Replacement is unavailable.").into());
            test.window.invoke_show_copy_conflict();
            test.draw();
            {
                let popups = WindowInner::from_pub(test.window.window()).active_popups();
                let popup = popups
                    .iter()
                    .rev()
                    .find(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
                    .unwrap();
                let title = find_text(
                    ItemRc::new_root(popup.component.clone()),
                    "Item already exists",
                )
                .unwrap();
                inside(&test, title, "Copy conflict title");
            }
            for label in ["Skip", "Skip all", "Copy as", "Cancel job", "Decide later"] {
                inside(&test, popup_button(&test, label), label);
            }
            if font != 32 {
                snapshot(&test, &format!("{label}-copy"));
            }
            test.window.invoke_close_copy_conflict();
            test.window.set_copy_token("".into());
            test.window.invoke_show_operations();
            test.draw();
            for label in ["Cancel", "Resolve", "Close"] {
                inside(&test, popup_button(&test, label), label);
                assert!(
                    popup_button(&test, label).geometry().height() <= 100.0,
                    "Job actions must keep normal button height instead of filling the viewport"
                );
            }
            if font != 32 {
                snapshot(&test, &format!("{label}-jobs"));
            }
            ui::click_popup(&test.window, "Close", AccessibleRole::Button);
            ui::click_root(&test.window, "Hot Paths", AccessibleRole::Combobox);
            test.draw();
            ui::click_popup(&test.window, "Manage Hot Paths…", AccessibleRole::Button);
            test.draw();
            for label in ["Add current", "Close"] {
                inside(&test, popup_button(&test, label), label);
            }
            if font != 32 {
                snapshot(&test, &format!("{label}-bookmarks"));
            }
            ui::click_popup(&test.window, "Close", AccessibleRole::Button);
            test.window.invoke_show_options();
            for topic in 0..7 {
                test.window.set_options_topic_index(topic);
                test.draw();
                inside(&test, popup_button(&test, "Done"), "Options Done");
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
            ui::click_popup(&test.window, "Done", AccessibleRole::Button);
        }
    }
    test.window.hide().unwrap();
}

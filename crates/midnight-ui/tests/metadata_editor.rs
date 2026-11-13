//! Exercise the actual merged editor, including deliberate preview before Apply.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;
use i_slint_core::{
    accessibility::AccessibleStringProperty, item_tree::ItemRc, items::AccessibleRole,
    window::WindowInner,
};
use midnight_ui::{MetadataDraft, MetadataFileRow};
use slint::{ComponentHandle, ModelRc, VecModel, platform::Key};
use std::{cell::RefCell, rc::Rc};

#[test]
fn unix_permissions_offer_owner_group_other_and_special_bits() {
    let mut test = ui::TestUi::new();
    test.window.set_metadata_can_unix(true);
    test.window.set_metadata_token("unix-grid:1".into());
    settle(&mut test);
    for label in [
        "Owner read",
        "Owner write",
        "Owner execute",
        "Group read",
        "Group write",
        "Group execute",
        "Other read",
        "Other write",
        "Other execute",
        "SUID",
        "SGID",
        "Sticky",
    ] {
        ui::popup_item(&test.window, label, AccessibleRole::Checkbox);
    }
}

fn settle(test: &mut ui::TestUi) {
    // Slint repositions embedded popups through a deferred property-tracker timer.
    for _ in 0..2 {
        slint::platform::update_timers_and_animations();
        test.draw();
    }
}

fn choose_attribute(test: &ui::TestUi, field: &str, value: &str) {
    for _ in 0..3 {
        let item = ui::popup_item(&test.window, field, AccessibleRole::Checkbox);
        if item
            .accessible_string_property(AccessibleStringProperty::Value)
            .unwrap()
            == value
        {
            return;
        }
        ui::click_popup(&test.window, field, AccessibleRole::Checkbox);
    }
    panic!("attribute {field} cannot reach {value}");
}

fn rendered_file_rows(item: ItemRc) -> usize {
    let mut count = usize::from(
        item.accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|label| label.starts_with("Captured file ")),
    );
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        count += rendered_file_rows(item);
    }
    count
}

fn action_is_not_vertically_clipped(test: &ui::TestUi, label: &str) {
    use i_slint_core::{item_tree::ParentItemTraversalMode, items::Flickable};
    let action = ui::popup_item(&test.window, label, AccessibleRole::Button);
    let origin = action.map_to_native_window(action.geometry().origin);
    let mut parent = action.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(item) = parent {
        if item.downcast::<Flickable>().is_some() {
            let clip = item.map_to_native_window(item.geometry().origin);
            assert!(
                origin.y >= clip.y
                    && origin.y + action.geometry().height()
                        <= clip.y + item.geometry().height() + 0.5,
                "{label} is clipped vertically: action={origin:?} {:?}, viewport={clip:?} {:?}",
                action.geometry(),
                item.geometry()
            );
        }
        parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
}

fn has_label(item: ItemRc, label: &str) -> bool {
    if item
        .accessible_string_property(AccessibleStringProperty::Label)
        .is_some_and(|value| value == label)
    {
        return true;
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if has_label(item, label) {
            return true;
        }
    }
    false
}

#[test]
fn attributes_use_one_compact_three_state_control_each() {
    let mut test = ui::TestUi::new();
    test.window.set_metadata_can_windows(true);
    test.window.set_metadata_windows_mask(39);
    test.window.set_metadata_can_compress(true);
    test.window.set_metadata_token("compact:1".into());
    settle(&mut test);
    let drafts = Rc::new(RefCell::new(Vec::<MetadataDraft>::new()));
    let captured = drafts.clone();
    test.window
        .on_preview_metadata(move |_, draft| captured.borrow_mut().push(draft));
    for label in ["Read-only", "Hidden", "System", "Archive", "Compression"] {
        let control = ui::popup_item(&test.window, label, AccessibleRole::Checkbox);
        assert!(
            control.geometry().height() <= 30.0,
            "attribute row should be compact"
        );
        assert_eq!(
            control
                .accessible_string_property(AccessibleStringProperty::Value)
                .unwrap()
                .as_str(),
            "Unchanged"
        );
    }
    for (expected, value) in [(1, "Set"), (2, "Clear"), (0, "Unchanged")] {
        ui::click_popup(&test.window, "Read-only", AccessibleRole::Checkbox);
        let control = ui::popup_item(&test.window, "Read-only", AccessibleRole::Checkbox);
        assert_eq!(
            control
                .accessible_string_property(AccessibleStringProperty::Value)
                .unwrap()
                .as_str(),
            value
        );
        ui::click_popup(&test.window, "Review changes", AccessibleRole::Button);
        assert_eq!(drafts.borrow().last().unwrap().read_only, expected);
    }
    ui::key(&test.window, Key::Escape);
}

#[test]
fn compact_attribute_keyboard_and_capabilities_preserve_leave_semantics() {
    let mut test = ui::TestUi::new();
    test.window.set_metadata_can_windows(true);
    test.window.set_metadata_windows_mask(1);
    test.window.set_metadata_token("keyboard:1".into());
    settle(&mut test);
    let hidden = ui::popup_item(&test.window, "Hidden", AccessibleRole::Checkbox);
    assert_eq!(
        hidden
            .accessible_string_property(AccessibleStringProperty::Enabled)
            .unwrap(),
        "false"
    );
    assert_eq!(
        hidden
            .accessible_string_property(AccessibleStringProperty::Value)
            .unwrap(),
        "Unavailable"
    );
    drop(hidden);
    ui::click_popup(&test.window, "Read-only", AccessibleRole::Checkbox);
    ui::key(&test.window, Key::Space);
    let value = || {
        ui::popup_item(&test.window, "Read-only", AccessibleRole::Checkbox)
            .accessible_string_property(AccessibleStringProperty::Value)
            .unwrap()
    };
    assert_eq!(value(), "Clear");
    ui::chord(&test.window, &[Key::Shift], Key::Space);
    assert_eq!(value(), "Set");
    ui::key(&test.window, Key::Delete);
    assert_eq!(value(), "Unchanged");
}

#[test]
fn metadata_edit_sections_exclude_obsolete_preview_and_require_prepared_confirmation() {
    let mut test = ui::TestUi::new();
    test.window.set_metadata_can_windows(true);
    test.window.set_metadata_can_modified(true);
    test.window.set_metadata_can_rename(true);
    test.window.set_metadata_token("review:1".into());
    settle(&mut test);
    let root = {
        let popups = WindowInner::from_pub(test.window.window()).active_popups();
        ItemRc::new_root(popups.last().unwrap().component.clone())
    };
    assert!(
        !has_label(root.clone(), "Preview"),
        "obsolete Preview tab remains"
    );
    for tab in ["Attributes", "Times", "Case"] {
        assert!(has_label(root.clone(), tab));
    }
    let previews = Rc::new(RefCell::new(Vec::new()));
    let captured = previews.clone();
    test.window
        .on_preview_metadata(move |_, draft| captured.borrow_mut().push(draft));
    let applied = Rc::new(RefCell::new(false));
    let captured = applied.clone();
    test.window
        .on_confirm_metadata(move |_, _, _| *captured.borrow_mut() = true);
    ui::click_popup(&test.window, "Review changes", AccessibleRole::Button);
    assert_eq!(previews.borrow().len(), 1);
    assert!(
        !*applied.borrow(),
        "review must not apply an unprepared draft"
    );
    test.window.set_metadata_revision("4".into());
    test.window.set_metadata_ready(true);
    test.window.set_metadata_preview_current(true);
    settle(&mut test);
    ui::click_popup(&test.window, "Apply", AccessibleRole::Button);
    assert!(*applied.borrow());
}

#[test]
fn metadata_editor_requires_review_and_keeps_sections_readable() {
    let mut test = ui::TestUi::new();
    test.window.set_metadata_can_windows(true);
    test.window.set_metadata_windows_mask(39);
    test.window.set_metadata_can_compress(true);
    test.window.set_metadata_can_created(true);
    test.window.set_metadata_can_modified(true);
    test.window.set_metadata_can_accessed(true);
    test.window.set_metadata_can_rename(true);
    test.window.set_metadata_rows(ModelRc::new(VecModel::from((0..256).map(|index| MetadataFileRow {
        current_name: format!("ŽLUŤOUČKÝ-{index}.TXT").into(),
        proposed_name: format!("žluťoučký-{index}.txt").into(),
        attributes: "Current: H\nProposed: R".into(),
        times: "Modified: 2026-11-06 12:30:00 UTC\nCreated: Unavailable\nAccessed: 2026-11-06 12:30:00 UTC".into(),
        detail: format!("Captured file {index}").into(), issue: false,
    }).collect::<Vec<_>>())));
    let drafts = Rc::new(RefCell::new(Vec::<MetadataDraft>::new()));
    let captured = drafts.clone();
    test.window
        .on_preview_metadata(move |_, draft| captured.borrow_mut().push(draft));
    let applies = Rc::new(RefCell::new(Vec::new()));
    let captured = applies.clone();
    test.window
        .on_confirm_metadata(move |token, revision, choice| {
            captured.borrow_mut().push((token, revision, choice))
        });
    let weak = test.window.as_weak();
    test.window
        .on_cancel_metadata(move |_| weak.upgrade().unwrap().set_metadata_token("".into()));

    test.window.set_metadata_start_case(true);
    test.window.set_metadata_summary("2 selected items".into());
    test.window.set_metadata_token("1".into());
    slint::platform::update_timers_and_animations();
    test.draw();
    let inner = WindowInner::from_pub(test.window.window());
    let popups = inner.active_popups();
    let visible_rows =
        rendered_file_rows(ItemRc::new_root(popups.last().unwrap().component.clone()));
    assert_eq!(
        visible_rows, 0,
        "captured-file preview rows are no longer rendered"
    );
    drop(popups);
    let popups = WindowInner::from_pub(test.window.window()).active_popups();
    let text = ui::text_values(ItemRc::new_root(popups.last().unwrap().component.clone()));
    assert!(
        !text.iter().any(|value| value.as_str() == "Current name"),
        "the redundant captured-file preview list is no longer part of the editor"
    );
    drop(popups);
    let popups = WindowInner::from_pub(test.window.window()).active_popups();
    assert!(
        !has_label(
            ItemRc::new_root(popups.last().unwrap().component.clone()),
            "Apply"
        ),
        "Apply must only be offered after an explicit prepared review"
    );
    drop(popups);
    assert!(applies.borrow().is_empty(), "unreviewed draft cannot apply");
    ui::click_popup(&test.window, "Attributes", AccessibleRole::Button);
    choose_attribute(&test, "Read-only", "Set");
    choose_attribute(&test, "Hidden", "Clear");
    choose_attribute(&test, "Read-only", "Clear");
    settle(&mut test);
    render::snapshot(&test, "metadata-before-review");
    action_is_not_vertically_clipped(&test, "Review changes");
    ui::click_popup(&test.window, "Review changes", AccessibleRole::Button);
    assert_eq!(
        drafts
            .borrow()
            .last()
            .expect("Review validates the current draft")
            .read_only,
        2
    );
    assert_eq!(drafts.borrow().last().unwrap().hidden, 2);
    choose_attribute(&test, "Read-only", "Unchanged");
    choose_attribute(&test, "Compression", "Compress");
    choose_attribute(&test, "Compression", "Uncompress");
    ui::click_popup(&test.window, "Review changes", AccessibleRole::Button);
    assert_eq!(drafts.borrow().last().unwrap().read_only, 0);
    assert_eq!(drafts.borrow().last().unwrap().hidden, 2);
    assert_eq!(drafts.borrow().last().unwrap().compressed, 2);
    ui::click_popup(&test.window, "Times", AccessibleRole::Button);
    ui::click_popup(&test.window, "Change Modified", AccessibleRole::Checkbox);
    ui::click_popup(&test.window, "Modified: Now", AccessibleRole::Button);
    ui::click_popup(&test.window, "Review changes", AccessibleRole::Button);
    assert_eq!(drafts.borrow().last().unwrap().modified_mode, 2);
    assert_eq!(drafts.borrow().last().unwrap().created_mode, 0);
    ui::click_popup(&test.window, "Times", AccessibleRole::Button);
    ui::click_popup(&test.window, "Change Modified", AccessibleRole::Checkbox);
    ui::click_popup(&test.window, "Review changes", AccessibleRole::Button);
    assert_eq!(drafts.borrow().last().unwrap().modified_mode, 0);
    ui::click_popup(&test.window, "Review changes", AccessibleRole::Button);
    assert!(
        drafts
            .borrow()
            .last()
            .expect("preview callback")
            .case_enabled
    );
    ui::click_popup(&test.window, "Case", AccessibleRole::Button);
    ui::click_popup(&test.window, "Conversion", AccessibleRole::Combobox);
    ui::key(&test.window, Key::Home);
    ui::key(&test.window, Key::DownArrow);
    ui::key(&test.window, Key::Return);
    ui::click_popup(&test.window, "Review changes", AccessibleRole::Button);
    assert_eq!(drafts.borrow().last().unwrap().case_mode, 1);
    test.window.set_metadata_revision("7".into());
    test.window.set_metadata_ready(true);
    test.window.set_metadata_preview_current(true);
    settle(&mut test);
    ui::click_popup(&test.window, "Apply", AccessibleRole::Button);
    assert_eq!(applies.borrow()[0].0.as_str(), "1");
    assert_eq!(applies.borrow()[0].1.as_str(), "7");
    ui::click_popup(&test.window, "Back to editing", AccessibleRole::Button);

    // Recursion belongs to the whole prepared operation, including edits on other topics.
    ui::click_popup(&test.window, "Include subfolders", AccessibleRole::Checkbox);
    for tab in ["Times", "Case", "Attributes"] {
        ui::click_popup(&test.window, tab, AccessibleRole::Button);
        ui::click_popup(&test.window, "Review changes", AccessibleRole::Button);
        assert!(drafts.borrow().last().unwrap().recursive);
    }
    ui::click_popup(&test.window, "Include subfolders", AccessibleRole::Checkbox);
    ui::click_popup(&test.window, "Review changes", AccessibleRole::Button);
    assert!(!drafts.borrow().last().unwrap().recursive);

    test.resize(1040, 720);
    settle(&mut test);
    for density in [0, 1] {
        for font in [11, 32] {
            test.window.invoke_apply_preferences(7, density);
            test.window.invoke_apply_fonts("".into(), font, font);
            std::thread::sleep(std::time::Duration::from_millis(250));
            settle(&mut test);
            for tab in ["Attributes", "Times", "Case"] {
                ui::click_popup(&test.window, tab, AccessibleRole::Button);
                settle(&mut test);
                render::snapshot(&test, &format!("metadata-{tab}-{density}-{font}"));
                if tab == "Times" {
                    let explanation = ui::popup_item(
                        &test.window,
                        "Now is captured once when reviewing, for all chosen fields and files. An explicit offset distinguishes repeated daylight-saving hours.",
                        AccessibleRole::Text,
                    );
                    let status = ui::popup_item(
                        &test.window,
                        "Draft changed. Review it again before applying.",
                        AccessibleRole::Text,
                    );
                    let body = explanation.geometry();
                    let bottom = explanation.map_to_native_window(body.origin).y + body.height();
                    let status_top = status.map_to_native_window(status.geometry().origin).y;
                    assert!(
                        bottom <= status_top,
                        "wrapped time help overlaps status: density={density}, font={font}, help_bottom={bottom}, status_top={status_top}"
                    );
                }
                let apply = ui::popup_item(&test.window, "Review changes", AccessibleRole::Button);
                action_is_not_vertically_clipped(&test, "Review changes");
                let area = apply.geometry();
                let origin = apply.map_to_native_window(area.origin);
                assert!(area.size.width > 0.0 && area.size.height > 0.0);
                assert!(
                    origin.y >= 0.0 && origin.y + area.size.height <= test.height as f32,
                    "{tab}, density={density}, font={font}: origin={origin:?}, bounds={area:?}"
                );
                assert!(
                    origin.x >= 0.0 && origin.x + area.size.width <= test.width as f32,
                    "{tab}, density={density}, font={font}: origin={origin:?}, bounds={area:?}"
                );
            }
            ui::click_popup(&test.window, "Review changes", AccessibleRole::Button);
            assert_eq!(
                drafts.borrow().last().unwrap().case_mode,
                1,
                "resize/font changes preserve the edited draft"
            );
            test.window.set_metadata_error(
                format!(
                    "Access denied\n\nPath\n資料/{}/žluťoučký.txt\n\nOperation: inspect metadata",
                    "long folder/".repeat(30)
                )
                .into(),
            );
            settle(&mut test);
            render::snapshot(&test, &format!("metadata-error-{density}-{font}"));
            let error = ui::popup_item(
                &test.window,
                &format!(
                    "Access denied\n\nPath\n資料/{}/žluťoučký.txt\n\nOperation: inspect metadata",
                    "long folder/".repeat(30)
                ),
                AccessibleRole::Text,
            );
            let mut ancestor =
                error.parent_item(i_slint_core::item_tree::ParentItemTraversalMode::StopAtPopups);
            let viewport = loop {
                let item = ancestor.expect("error text has a scroll viewport");
                if item.downcast::<i_slint_core::items::Flickable>().is_some() {
                    break item;
                }
                ancestor = item
                    .parent_item(i_slint_core::item_tree::ParentItemTraversalMode::StopAtPopups);
            };
            assert!(
                error.geometry().width() >= viewport.geometry().width() - 24.0,
                "error text must fill the available compact viewport"
            );
            let apply = ui::popup_item(&test.window, "Review changes", AccessibleRole::Button);
            let area = apply.geometry();
            let origin = apply.map_to_native_window(area.origin);
            assert!(
                origin.y >= 0.0 && origin.y + area.size.height <= test.height as f32,
                "error density={density}, font={font}: origin={origin:?}, bounds={area:?}"
            );
            test.window.set_metadata_error("".into());
            settle(&mut test);
        }
    }
    for (density, font) in [(0, 11), (1, 32)] {
        test.window.invoke_apply_preferences(0, density);
        test.window.invoke_apply_fonts("".into(), font, font);
        test.resize(320, 200);
        settle(&mut test);
        for action in ["Review changes", "Cancel"] {
            action_is_not_vertically_clipped(&test, action);
        }
        render::snapshot(&test, &format!("metadata-minimum-{density}-{font}"));
    }
    ui::key(&test.window, Key::Escape);
    slint::platform::update_timers_and_animations();
    assert!(test.window.get_metadata_token().is_empty());
    test.window.hide().unwrap();
}

#[test]
fn timestamp_fields_are_single_line_and_refresh_has_no_duplicate_action() {
    let mut test = ui::TestUi::new();
    test.window.set_metadata_can_modified(true);
    test.window.set_metadata_can_created(true);
    test.window.set_metadata_can_accessed(true);
    test.window
        .set_metadata_timestamp_example("06. 11. 2026 12:30:00 +01:00".into());
    test.window.set_metadata_token("times:1".into());
    settle(&mut test);
    ui::click_popup(&test.window, "Times", AccessibleRole::Button);
    for (density, font, width, height) in
        [(0, 11, 1340, 820), (1, 32, 1340, 960), (0, 13, 600, 700)]
    {
        test.window.invoke_apply_preferences(0, density);
        test.window.invoke_apply_fonts("".into(), font, font);
        test.resize(width, height);
        settle(&mut test);
        for label in ["Modified", "Created", "Accessed"] {
            let field = ui::popup_item(
                &test.window,
                &format!("{label} timestamp"),
                AccessibleRole::TextInput,
            );
            let now = ui::popup_item(
                &test.window,
                &format!("{label}: Now"),
                AccessibleRole::Button,
            );
            // The accessible text input is inset in its single-line edit frame.
            assert!(
                field.geometry().height() <= now.geometry().height() + 1.0,
                "{label} input must not stretch into a tall editor: {:?} vs {:?}",
                field.geometry(),
                now.geometry()
            );
        }
        let inner = WindowInner::from_pub(test.window.window());
        for popup in inner.active_popups().iter() {
            assert!(!has_label(
                ItemRc::new_root(popup.component.clone()),
                "Refresh"
            ));
        }
        action_is_not_vertically_clipped(&test, "Review changes");
    }
    ui::key(&test.window, Key::Escape);
}

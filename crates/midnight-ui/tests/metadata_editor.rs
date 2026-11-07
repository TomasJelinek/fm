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

fn settle(test: &mut ui::TestUi) {
    // Slint repositions embedded popups through a deferred property-tracker timer.
    for _ in 0..2 {
        slint::platform::update_timers_and_animations();
        test.draw();
    }
}

fn assert_choice(test: &ui::TestUi, field: &str, labels: &[&str], chosen: &str) {
    for label in labels {
        let item = ui::popup_item(
            &test.window,
            &format!("{field}: {label}"),
            AccessibleRole::Button,
        );
        let checked = item
            .accessible_string_property(AccessibleStringProperty::Checked)
            .unwrap();
        assert_eq!(
            checked.as_str(),
            if *label == chosen { "true" } else { "false" }
        );
    }
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

#[test]
fn metadata_editor_requires_preview_and_keeps_sections_readable() {
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
    assert!(
        visible_rows > 0 && visible_rows < 20,
        "file list must virtualize 256 rows, rendered {visible_rows}"
    );
    drop(popups);
    let header = ui::popup_item(&test.window, "Current name", AccessibleRole::Text);
    let first = ui::popup_item(&test.window, "Captured file 0", AccessibleRole::Text);
    let header_area = header.geometry();
    let first_area = first.geometry();
    let header_origin = header.map_to_native_window(header_area.origin);
    let first_origin = first.map_to_native_window(first_area.origin);
    assert!(
        header_origin.y + header_area.height() <= first_origin.y,
        "column headings must remain above the virtualized rows"
    );
    ui::click_popup(&test.window, "Apply", AccessibleRole::Button);
    assert!(applies.borrow().is_empty(), "unreviewed draft cannot apply");
    ui::click_popup(&test.window, "Attributes", AccessibleRole::Button);
    ui::click_popup(&test.window, "Read-only: Set", AccessibleRole::Button);
    ui::click_popup(&test.window, "Read-only: Set", AccessibleRole::Button);
    assert_choice(&test, "Read-only", &["Leave", "Set", "Clear"], "Set");
    ui::click_popup(&test.window, "Hidden: Clear", AccessibleRole::Button);
    ui::click_popup(&test.window, "Read-only: Clear", AccessibleRole::Button);
    assert_choice(&test, "Read-only", &["Leave", "Set", "Clear"], "Clear");
    ui::click_popup(&test.window, "Refresh", AccessibleRole::Button);
    assert_eq!(drafts.borrow().last().unwrap().read_only, 2);
    assert_eq!(drafts.borrow().last().unwrap().hidden, 2);
    ui::click_popup(&test.window, "Read-only: Leave", AccessibleRole::Button);
    assert_choice(&test, "Read-only", &["Leave", "Set", "Clear"], "Leave");
    ui::click_popup(
        &test.window,
        "Compression: Compress",
        AccessibleRole::Button,
    );
    ui::click_popup(
        &test.window,
        "Compression: Uncompress",
        AccessibleRole::Button,
    );
    ui::click_popup(&test.window, "Preview changes", AccessibleRole::Button);
    assert_eq!(drafts.borrow().last().unwrap().read_only, 0);
    assert_eq!(drafts.borrow().last().unwrap().hidden, 2);
    assert_eq!(drafts.borrow().last().unwrap().compressed, 2);
    ui::click_popup(&test.window, "Times", AccessibleRole::Button);
    ui::click_popup(&test.window, "Change Modified", AccessibleRole::Checkbox);
    ui::click_popup(&test.window, "Modified: Now", AccessibleRole::Button);
    ui::click_popup(&test.window, "Preview changes", AccessibleRole::Button);
    assert_eq!(drafts.borrow().last().unwrap().modified_mode, 2);
    assert_eq!(drafts.borrow().last().unwrap().created_mode, 0);
    ui::click_popup(&test.window, "Times", AccessibleRole::Button);
    ui::click_popup(&test.window, "Change Modified", AccessibleRole::Checkbox);
    ui::click_popup(&test.window, "Preview changes", AccessibleRole::Button);
    assert_eq!(drafts.borrow().last().unwrap().modified_mode, 0);
    ui::click_popup(&test.window, "Preview changes", AccessibleRole::Button);
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
    ui::click_popup(&test.window, "Preview changes", AccessibleRole::Button);
    assert_eq!(drafts.borrow().last().unwrap().case_mode, 1);
    test.window.set_metadata_revision("7".into());
    test.window.set_metadata_ready(true);
    slint::platform::update_timers_and_animations();
    ui::click_popup(&test.window, "Apply", AccessibleRole::Button);
    assert_eq!(applies.borrow()[0].0.as_str(), "1");
    assert_eq!(applies.borrow()[0].1.as_str(), "7");

    test.resize(1040, 720);
    settle(&mut test);
    for density in [0, 1] {
        for font in [11, 32] {
            test.window.invoke_apply_preferences(7, density);
            test.window.invoke_apply_fonts("".into(), font, font);
            std::thread::sleep(std::time::Duration::from_millis(250));
            settle(&mut test);
            for tab in ["Attributes", "Times", "Case", "Preview"] {
                ui::click_popup(&test.window, tab, AccessibleRole::Button);
                settle(&mut test);
                render::snapshot(&test, &format!("metadata-{tab}-{density}-{font}"));
                let apply = ui::popup_item(&test.window, "Apply", AccessibleRole::Button);
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
            ui::click_popup(&test.window, "Preview changes", AccessibleRole::Button);
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
            assert!(
                error.geometry().width() > 500.0,
                "error text must use the available card width, got {:?}",
                error.geometry()
            );
            let apply = ui::popup_item(&test.window, "Apply", AccessibleRole::Button);
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
    ui::key(&test.window, Key::Escape);
    slint::platform::update_timers_and_animations();
    assert!(test.window.get_metadata_token().is_empty());
    test.window.hide().unwrap();
}

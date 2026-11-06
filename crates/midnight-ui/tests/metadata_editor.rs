//! Exercise the actual merged editor, including deliberate preview before Apply.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;
use i_slint_core::items::AccessibleRole;
use midnight_ui::MetadataDraft;
use slint::{ComponentHandle, platform::Key};
use std::{cell::RefCell, rc::Rc};

fn settle(test: &mut ui::TestUi) {
    // Slint repositions embedded popups through a deferred property-tracker timer.
    for _ in 0..2 {
        slint::platform::update_timers_and_animations();
        test.draw();
    }
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
    ui::click_popup(&test.window, "Apply", AccessibleRole::Button);
    assert!(applies.borrow().is_empty(), "unreviewed draft cannot apply");
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
        }
    }
    ui::key(&test.window, Key::Escape);
    slint::platform::update_timers_and_animations();
    assert!(test.window.get_metadata_token().is_empty());
    test.window.hide().unwrap();
}

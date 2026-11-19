//! The About dialog lives inside the main window, can be dragged by its sky and
//! lists the third-party libraries.
#[path = "support/render.rs"]
mod render;
use crate::support::ui;
use i_slint_core::items::AccessibleRole;
use midnight_ui::LicenseRow;
use slint::{ComponentHandle, LogicalPosition, ModelRc, VecModel, platform::PointerEventButton};
use std::rc::Rc;

fn open() -> ui::TestUi {
    let mut test = ui::TestUi::new();
    test.resize(1000, 800);
    test.window.set_application_version("1.2.3".into());
    let rows: Vec<LicenseRow> = (0..40)
        .map(|index| LicenseRow {
            name: format!("library-{index}").into(),
            version: "0.1.0".into(),
            license: "MIT OR Apache-2.0".into(),
        })
        .collect();
    test.window
        .set_license_rows(ModelRc::from(Rc::new(VecModel::from(rows))));
    test.window.set_license_summary("40 libraries".into());
    test.draw();
    test.window.invoke_show_about();
    test.draw();
    test
}

#[test]
fn about_opens_in_the_main_window_and_both_tabs_render() {
    let mut test = open();
    assert!(test.window.get_about_open());
    assert!(ui::has_popup(&test.window));
    render::snapshot(&test, "about-tab");
    ui::click_popup(&test.window, "Licenses", AccessibleRole::Text);
    test.draw();
    assert!(ui::try_popup_item(&test.window, "library-3", AccessibleRole::Text).is_some());
    render::snapshot(&test, "about-licenses");
}

#[test]
fn dragging_the_sky_moves_the_dialog_and_close_dismisses_it() {
    let mut test = open();
    let origin = |test: &ui::TestUi| {
        let item = ui::popup_item(&test.window, "Close", AccessibleRole::Button);
        item.map_to_native_window(item.geometry().origin)
    };
    let before = origin(&test);
    let title = ui::popup_item(&test.window, "Midnight Explorer", AccessibleRole::Text);
    let rect = title.geometry();
    let at = title.map_to_native_window(rect.origin);
    let start = LogicalPosition::new(at.x + rect.size.width / 2.0, at.y + rect.size.height / 2.0);
    let left = PointerEventButton::Left;
    ui::pointer_button(&test.window, start, left, true);
    for step in 1..=4 {
        let position = LogicalPosition::new(start.x - 10.0 * step as f32, start.y + 8.0 * step as f32);
        test.window
            .window()
            .dispatch_event(slint::platform::WindowEvent::PointerMoved { position });
    }
    ui::pointer_button(
        &test.window,
        LogicalPosition::new(start.x - 40.0, start.y + 32.0),
        left,
        false,
    );
    test.draw();
    let after = origin(&test);
    assert!(
        (after.x - before.x + 40.0).abs() < 2.0 && (after.y - before.y - 32.0).abs() < 2.0,
        "moved from {before:?} to {after:?}"
    );
    ui::click_popup(&test.window, "Close", AccessibleRole::Button);
    test.draw();
    assert!(!test.window.get_about_open());
}

#[test]
fn double_clicking_a_library_opens_its_license_and_back_returns_to_the_list() {
    let mut test = open();
    let weak = test.window.as_weak();
    test.window.on_license_open(move |name, version| {
        let window = weak.upgrade().unwrap();
        window.set_license_detail_title(format!("{name} {version}").into());
        window.set_license_detail_license("MIT OR Apache-2.0".into());
        window.set_license_detail_authors("Ada Lovelace".into());
        window.set_license_detail_text("Permission is hereby granted".into());
        window.set_license_detail_open(true);
    });
    ui::click_popup(&test.window, "Licenses", AccessibleRole::Text);
    test.draw();
    let row = ui::popup_item(&test.window, "library-3 0.1.0", AccessibleRole::Button);
    let rect = row.geometry();
    let at = row.map_to_native_window(rect.origin);
    let position = LogicalPosition::new(at.x + rect.size.width / 2.0, at.y + rect.size.height / 2.0);
    let left = PointerEventButton::Left;
    for _ in 0..2 {
        ui::pointer_button(&test.window, position, left, true);
        ui::pointer_button(&test.window, position, left, false);
    }
    test.draw();
    assert!(test.window.get_license_detail_open());
    assert!(ui::try_popup_item(&test.window, "Permission is hereby granted", AccessibleRole::Text).is_some());
    render::snapshot(&test, "about-license-detail");
    ui::click_popup(&test.window, "Back", AccessibleRole::Button);
    test.draw();
    assert!(!test.window.get_license_detail_open());
    assert!(ui::try_popup_item(&test.window, "library-3", AccessibleRole::Text).is_some());
}

#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;
use i_slint_core::items::AccessibleRole;
use midnight_ui::OperationRow;
use slint::{ModelRc, VecModel, platform::Key};
use std::{cell::RefCell, rc::Rc};

fn job(id: &str, title: &str) -> OperationRow {
    OperationRow {
        identity: id.into(),
        title: title.into(),
        detail: format!("Source\n資料/{id}").into(),
        cancellable: true,
        ..Default::default()
    }
}

#[test]
fn job_selection_survives_updates_and_cancel_does_not_close_the_window() {
    let mut test = ui::TestUi::new();
    test.window.set_operations(ModelRc::new(VecModel::from(vec![
        job("first", "First job"),
        job("second", "Second job"),
    ])));
    let cancelled = Rc::new(RefCell::new(Vec::new()));
    let sink = cancelled.clone();
    test.window
        .on_cancel_job(move |id| sink.borrow_mut().push(id));
    let copied = Rc::new(RefCell::new(Vec::new()));
    let sink = copied.clone();
    test.window
        .on_copy_job_details(move |id| sink.borrow_mut().push(id));
    test.window.invoke_show_operations();
    slint::platform::update_timers_and_animations();
    test.draw();
    ui::click_popup(&test.window, "Second job", AccessibleRole::ListItem);
    test.window.set_operations(ModelRc::new(VecModel::from(vec![
        job("new", "New job"),
        job("first", "First job"),
        job("second", "Second job"),
    ])));
    slint::platform::update_timers_and_animations();
    test.draw();
    ui::click_popup(&test.window, "Cancel", AccessibleRole::Button);
    assert_eq!(
        cancelled.borrow().as_slice(),
        &[slint::SharedString::from("second")]
    );
    assert!(ui::has_popup(&test.window));
    ui::click_popup(&test.window, "Copy details", AccessibleRole::Button);
    assert_eq!(
        copied.borrow().as_slice(),
        &[slint::SharedString::from("second")]
    );
    assert!(ui::has_popup(&test.window));
    render::snapshot(&test, "jobs-selected");
    ui::key(&test.window, Key::Escape);
    assert!(!ui::has_popup(&test.window));
    assert_eq!(
        cancelled.borrow().len(),
        1,
        "closing does not cancel another job"
    );
    test.window.invoke_show_operations();
    slint::platform::update_timers_and_animations();
    test.draw();
    ui::key(&test.window, Key::Escape);
    assert!(
        !ui::has_popup(&test.window),
        "reopened Jobs also owns keyboard focus"
    );
}

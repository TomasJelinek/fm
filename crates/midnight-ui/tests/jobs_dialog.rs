#[allow(dead_code)]
#[path = "support/modeless.rs"]
mod modeless;
#[path = "support/jobs_form.rs"]
mod ui;
use i_slint_core::{
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, Flickable, Path},
};
use midnight_ui::{OperationRow, SpeedChartData};
use slint::{ComponentHandle, ModelRc, VecModel, platform::Key};
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
    test.window.set_rows(ModelRc::new(VecModel::from(vec![
        job("first", "First job"),
        job("second", "Second job"),
    ])));
    let cancelled = Rc::new(RefCell::new(Vec::new()));
    let sink = cancelled.clone();
    test.window.on_cancel(move |id| sink.borrow_mut().push(id));
    let copied = Rc::new(RefCell::new(Vec::new()));
    let sink = copied.clone();
    test.window
        .on_copy_details(move |id| sink.borrow_mut().push(id));
    test.window.show().unwrap();
    slint::platform::update_timers_and_animations();
    test.draw();
    ui::click_popup(&test.window, "Second job", AccessibleRole::ListItem);
    test.window.set_rows(ModelRc::new(VecModel::from(vec![
        job("new", "New job"),
        job("first", "First job"),
        job("second", "Second job"),
    ])));
    slint::platform::update_timers_and_animations();
    test.draw();
    ui::click_popup(&test.window, "Cancel job", AccessibleRole::Button);
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
    test.snapshot("jobs-selected");
    ui::key(&test.window, Key::Escape);
    assert!(!ui::has_popup(&test.window));
    assert_eq!(
        cancelled.borrow().len(),
        1,
        "closing does not cancel another job"
    );
    test.window.show().unwrap();
    slint::platform::update_timers_and_animations();
    test.draw();
    ui::key(&test.window, Key::Escape);
    assert!(
        !ui::has_popup(&test.window),
        "reopened Jobs also owns keyboard focus"
    );
}

#[test]
fn transfer_and_delete_jobs_show_their_own_speed_chart() {
    let mut test = ui::TestUi::new();
    let mut deletion = job("delete", "Delete files");
    deletion.kind = 1;
    test.window.set_rows(ModelRc::new(VecModel::from(vec![
        job("copy", "Copy files"),
        deletion,
    ])));
    test.window.show().unwrap();
    test.draw();
    ui::popup_item(&test.window, "Transfer speed", AccessibleRole::Groupbox);
    ui::click_popup(&test.window, "Delete files", AccessibleRole::ListItem);
    test.draw();
    ui::popup_item(&test.window, "Deletion speed", AccessibleRole::Groupbox);
    ui::popup_item(&test.window, "Cancel job", AccessibleRole::Button);
    ui::key(&test.window, Key::Escape);
    assert!(!ui::has_popup(&test.window));
}

#[test]
fn speed_chart_resizes_with_jobs_and_keeps_actions_visible_in_all_themes() {
    let mut test = ui::TestUi::new();
    let mut copy = job("copy", "Copy files");
    copy.state = "Copying".into();
    copy.speed_chart = SpeedChartData {
        area: "M 0 100 L 0 80 L 150 40 L 300 65 L 450 20 L 600 55 L 750 35 L 1000 30 L 1000 100 Z"
            .into(),
        primary: "M 0 80 L 150 40 L 300 65 L 450 20 L 600 55 L 750 35 L 1000 30".into(),
        verification: "M 0 100 L 250 100 L 400 70 L 550 40 L 700 55 L 1000 15".into(),
        speed: "72.4 MiB/s".into(),
        verification_speed: "88.2 MiB/s".into(),
        scale: "100 MiB/s".into(),
        history: "Elapsed 1 min 00 s".into(),
        description: "Write: 72.4 MiB/s. Verify: 88.2 MiB/s. Elapsed 1 min 00 s.".into(),
        has_samples: true,
        ..Default::default()
    };
    test.window
        .set_rows(ModelRc::new(VecModel::from(vec![copy])));
    for theme in [0, 1, 2, 21, 22] {
        test.window
            .invoke_apply_preferences(theme, 0, "".into(), 11, 15);
        for (width, height) in [(1000, 700), (480, 360), (320, 200)] {
            test.resize(width, height);
            test.window.show().unwrap();
            test.draw();
            let chart = ui::popup_item(&test.window, "Transfer speed", AccessibleRole::Groupbox);
            assert_eq!(
                check_history_width(&chart),
                2,
                "write and verification curves"
            );
            let close = ui::popup_item(&test.window, "Close", AccessibleRole::Button);
            let cancel = ui::popup_item(&test.window, "Cancel job", AccessibleRole::Button);
            // The full chart and diagnostics scroll at small sizes; dismissal
            // and cancellation remain anchored outside that content viewport.
            let chart_viewport = scroll_parent(&chart);
            for item in [chart_viewport, close, cancel] {
                let rect = item.geometry();
                assert!(rect.size.width > 0.0 && rect.size.height > 0.0);
                let location = item.map_to_native_window(rect.origin);
                assert!(location.x >= 0.0 && location.y >= 0.0);
                assert!(location.x + rect.size.width <= width as f32 + 1.0);
                assert!(location.y + rect.size.height <= height as f32 + 1.0);
            }
            test.snapshot(&format!("job-speed-{theme}-{width}x{height}"));
            ui::key(&test.window, Key::Escape);
        }
    }
}

fn scroll_parent(item: &ItemRc) -> ItemRc {
    let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    loop {
        let item = parent.expect("job content must have a scrolling viewport");
        if item.downcast::<Flickable>().is_some() {
            return item;
        }
        parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
}

// Check the actual renderer's fitted endpoints, not just the item's rectangle:
// a contain-fit curve can occupy only the middle of a wide, short chart.
fn check_history_width(item: &ItemRc) -> usize {
    let mut curves = 0;
    if let Some(path) = item.downcast::<Path>() {
        let path = path.as_pin_ref();
        if path.stroke_width().get() > 0.0 {
            let first = path.point_at(item, 0.0);
            let last = path.point_at(item, 1.0);
            assert!(
                first.x <= 2.0,
                "history must begin at the left edge: {first:?}"
            );
            assert!(
                last.x >= item.geometry().size.width - 2.0,
                "history must reach the right edge: {last:?}"
            );
            curves += 1;
        }
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        curves += check_history_width(&item);
    }
    curves
}

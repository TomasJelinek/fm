#[allow(dead_code)]
#[path = "support/modeless.rs"]
mod modeless;
#[path = "support/progress_form.rs"]
mod ui;
use i_slint_core::{
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, Flickable, Path},
};
use midnight_ui::{OperationRow, SpeedChartData};
use slint::{ComponentHandle, platform::Key};
fn job(id: &str, title: &str) -> OperationRow {
    OperationRow {
        identity: id.into(),
        title: title.into(),
        cancellable: true,
        detail: format!("Source\n資料/{id}").into(),
        ..Default::default()
    }
}
#[test]
fn progress_meter_is_visible_even_with_unknown_totals_and_current_file_has_its_own_meter() {
    let mut test = ui::TestUi::new();
    let mut row = job("copy-job:7", "Copy");
    row.active = true;
    row.file_determinate = true;
    row.file_progress = 0.5;
    row.file_progress_label = "Current file written: 500 / 1000 bytes".into();
    test.window.set_job(row.clone());
    test.draw();
    ui::popup_item(
        &test.window,
        "Overall progress",
        AccessibleRole::ProgressIndicator,
    );
    ui::popup_item(
        &test.window,
        "Current file written",
        AccessibleRole::ProgressIndicator,
    );
    row.kind = 1;
    row.file_determinate = false;
    test.window.set_job(row);
    test.draw();
    ui::popup_item(&test.window, "Deletion speed", AccessibleRole::Groupbox);
    ui::key(&test.window, Key::Escape);
    assert!(!ui::has_popup(&test.window));
}

#[test]
fn speed_chart_resizes_with_jobs_and_keeps_actions_visible_in_all_themes() {
    let mut test = ui::TestUi::new();
    let mut copy = job("copy", "Copy files");
    copy.state = "Copying".into();
    copy.active = true;
    copy.pausable = true;
    copy.determinate = true;
    copy.progress = 0.5;
    copy.progress_label = "50% complete".into();
    copy.current_path = "D:\\Source\\資料\\example.zip".into();
    copy.eta = "About 3 min".into();
    copy.remaining = "Items remaining: 12 (620 MiB)".into();
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
    test.window.set_job(copy);
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
            let meter = ui::popup_item(
                &test.window,
                "Overall progress",
                AccessibleRole::ProgressIndicator,
            );
            let fill = meter
                .first_child()
                .expect("overall meter has a completion fill");
            let origin = meter.map_to_native_window(meter.geometry().origin);
            let fill_origin = fill.map_to_native_window(fill.geometry().origin);
            assert!(
                (fill_origin.x - origin.x).abs() <= 1.0,
                "completion fill must start at the left edge"
            );
            assert!((fill.geometry().width() - meter.geometry().width() / 2.0).abs() <= 1.0);
            let close = ui::popup_item(&test.window, "Cancel operation", AccessibleRole::Button);
            // The full chart and diagnostics scroll at small sizes; dismissal
            // and cancellation remain anchored outside that content viewport.
            let chart_viewport = scroll_parent(&chart);
            for item in [chart_viewport, close] {
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
                "history must reach the filled completion edge: {last:?}"
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

use crate::support::modeless;
#[path = "support/progress_form.rs"]
mod ui;
use i_slint_core::{
    accessibility::AccessibleStringProperty,
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
    assert!(
        ui::find_popup_item(
            &test.window,
            "Current file written",
            AccessibleRole::ProgressIndicator
        )
        .is_none()
    );
    ui::click_popup(&test.window, "Details", AccessibleRole::Button);
    test.draw();
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
fn verification_keeps_the_aggregate_chart_and_exposes_readback_in_details() {
    let mut test = ui::TestUi::new();
    let mut row = job("copy-job:22", "Copy");
    row.active = true;
    row.determinate = true;
    row.progress = 0.6;
    row.progress_label = "60% complete".into();
    row.verification_view = true;
    row.file_determinate = true;
    row.file_progress = 0.25;
    row.file_progress_title = "Current file verification".into();
    row.file_progress_label = "Verifying samples: 49152 / 196608 bytes".into();
    row.activity_label = "Verifying current file...".into();
    row.speed_chart = SpeedChartData {
        area: "M 0 100 L 0 80 L 1000 40 L 1000 100 Z".into(),
        primary: "M 0 80 L 1000 40".into(),
        speed: "128 KiB/s".into(),
        has_samples: true,
        ..Default::default()
    };
    row.transfer_speed_chart = SpeedChartData {
        primary: "M 0 20 L 1000 40".into(),
        verification: "M 0 100 L 1000 50".into(),
        has_samples: true,
        ..Default::default()
    };
    test.window.set_job(row);
    test.draw();
    assert!(
        ui::find_popup_item(&test.window, "Verification speed", AccessibleRole::Groupbox).is_none()
    );
    ui::click_popup(&test.window, "Details", AccessibleRole::Button);
    test.draw();
    for (density, font) in [(0, 13), (1, 32)] {
        test.window
            .invoke_apply_preferences(3, density, "".into(), font, font);
        for (width, height) in [(1000, 700), (480, 360), (320, 200)] {
            test.resize(width, height);
            let chart =
                ui::popup_item(&test.window, "Verification speed", AccessibleRole::Groupbox);
            assert_eq!(check_history_width(&chart), 1, "readback has one curve");
            let aggregate =
                ui::popup_item(&test.window, "Transfer speed", AccessibleRole::Groupbox);
            assert_eq!(
                check_history_width(&aggregate),
                2,
                "the whole-transfer history remains visible"
            );
            let overall = ui::popup_item(
                &test.window,
                "Overall progress",
                AccessibleRole::ProgressIndicator,
            );
            assert_eq!(
                overall
                    .accessible_string_property(AccessibleStringProperty::Value)
                    .unwrap(),
                "60% complete"
            );
            let file = ui::popup_item(
                &test.window,
                "Current file verification progress",
                AccessibleRole::ProgressIndicator,
            );
            assert_eq!(
                file.accessible_string_property(AccessibleStringProperty::Value)
                    .unwrap(),
                "Verifying samples: 49152 / 196608 bytes"
            );
            for endpoint in ["0%", "100%"] {
                ui::popup_item(&test.window, endpoint, AccessibleRole::Text);
            }
            let close = ui::popup_item(&test.window, "Cancel operation", AccessibleRole::Button);
            let rect = close.geometry();
            let location = close.map_to_native_window(rect.origin);
            assert!(location.x >= 0.0 && location.y >= 0.0);
            assert!(location.x + rect.size.width <= width as f32 + 1.0);
            assert!(location.y + rect.size.height <= height as f32 + 1.0);
            test.snapshot(&format!("verify-phase-{density}-{font}-{width}x{height}"));
        }
    }
}

#[test]
fn waiting_status_replaces_an_empty_eta_without_claiming_calculation() {
    let mut test = ui::TestUi::new();
    let mut row = job("copy-job:23", "Move");
    row.active = true;
    row.waiting = true;
    row.progress_label = "Queued".into();
    row.activity_label = "Queued — waiting for earlier transfers".into();
    test.window.set_job(row);
    test.draw();
    ui::popup_item(
        &test.window,
        "Queued — waiting for earlier transfers",
        AccessibleRole::Text,
    );
    ui::popup_item(
        &test.window,
        "Queued — waiting for earlier transfers",
        AccessibleRole::ProgressIndicator,
    );
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
    copy.transfer_directory = "D:\\Source\\資料".into();
    copy.transfer_name = "example.zip".into();
    copy.transfer_totals = "Files: 6 of 18 · Bytes: 900 B of 1.8 KiB".into();
    copy.eta = "About 3 min".into();
    copy.remaining = "Items remaining: 12 (620 MiB)".into();
    copy.transfer_speed_chart = SpeedChartData {
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
    test.draw();
    for label in [
        "D:\\Source\\資料\\example.zip",
        "Files: 6 of 18 · Bytes: 900 B of 1.8 KiB",
        "Time remaining: About 3 min",
    ] {
        ui::popup_item(&test.window, label, AccessibleRole::Text);
    }
    assert!(
        ui::find_popup_item(
            &test.window,
            "Items remaining: 12 (620 MiB)",
            AccessibleRole::Text
        )
        .is_none()
    );
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

#[test]
fn cancel_question_keeps_both_actions_visible_and_continue_focused_at_compact_sizes() {
    use std::{cell::Cell, rc::Rc};
    let mut test = ui::TestUi::new();
    let mut row = job("copy-job:1", "Move files");
    row.active = true;
    row.pausable = true;
    row.pause_requested = true;
    row.paused = true;
    row.state = "Paused".into();
    row.current_path = "D:\\資料\\selected folder\\file.txt".into();
    test.window.set_job(row);
    let weak = test.window.as_weak();
    test.window.on_request_dismiss(move || {
        weak.unwrap().set_cancel_confirmation(false);
    });
    let confirmed = Rc::new(Cell::new(0));
    let count = confirmed.clone();
    test.window
        .on_confirm_cancel(move |_| count.set(count.get() + 1));
    for (theme, density) in [(0, 0), (1, 0), (21, 1), (22, 1)] {
        test.window
            .invoke_apply_preferences(theme, density, "".into(), 11, 15);
        for (width, height) in [(640, 420), (320, 200)] {
            test.resize(width, height);
            test.window.set_cancel_confirmation(true);
            test.draw();
            for label in ["Continue", "Cancel operation"] {
                let item = ui::popup_item(&test.window, label, AccessibleRole::Button);
                let rect = item.geometry();
                let origin = item.map_to_native_window(rect.origin);
                assert!(rect.width() > 0.0 && rect.height() > 0.0);
                assert!(origin.x >= 0.0 && origin.y >= 0.0);
                assert!(
                    origin.x + rect.width() <= width as f32 + 1.0,
                    "{label} horizontal fit"
                );
                assert!(
                    origin.y + rect.height() <= height as f32 + 1.0,
                    "{label} vertical fit"
                );
            }
            test.snapshot(&format!(
                "cancel-question-{theme}-{density}-{width}x{height}"
            ));
            ui::key(&test.window, Key::Return);
            test.draw();
            assert!(
                !test.window.get_cancel_confirmation(),
                "Enter must choose Continue"
            );
            assert_eq!(confirmed.get(), 0);
            assert!(test.window.get_job().paused && test.window.get_job().pause_requested);
        }
    }
    test.window.set_cancel_confirmation(true);
    test.draw();
    ui::click_popup(&test.window, "Cancel operation", AccessibleRole::Button);
    assert_eq!(
        confirmed.get(),
        1,
        "explicit pointer confirmation is reachable"
    );
}

fn native_rect(item: &ItemRc) -> (f32, f32, f32, f32) {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    (origin.x, origin.y, rect.size.width, rect.size.height)
}

#[test]
fn every_file_shows_its_path_and_its_own_bar_under_the_chart_without_moving() {
    let mut test = ui::TestUi::new();
    let mut copy = job("copy-job:31", "Copy");
    copy.state = "Copying".into();
    copy.active = true;
    copy.pausable = true;
    copy.determinate = true;
    copy.progress = 0.25;
    copy.progress_label = "25% complete".into();
    copy.transfer_totals = "Files: 3 of 12 · Bytes: 1.0 GiB of 4.0 GiB".into();
    copy.eta = "About 3 min".into();
    copy.file_determinate = true;
    copy.file_progress_title = "Current file written".into();
    let bar = |test: &ui::TestUi| {
        ui::find_popup_item(
            &test.window,
            "Current file progress",
            AccessibleRole::ProgressIndicator,
        )
        .expect("the current file's bar is always shown")
    };
    // Each file in turn: a large one part written, a small one already through, none
    // between two files, and a long archive member path whose middle is left out.
    let long = format!(
        "/source/資料/backup.zip/{}/deep/large.bin",
        "a-folder-with-a-long-name/".repeat(8)
    );
    let files: [(&str, &str, f32, &str); 4] = [
        ("/source/資料/large.bin", "large.bin", 0.45, "45%"),
        ("/source/資料/small.txt", "small.txt", 1.0, "100%"),
        ("", "", 0.0, ""),
        (long.as_str(), "large.bin", 0.1, "Verifying 10%"),
    ];
    for (width, height) in [(1000, 700), (480, 360), (320, 200)] {
        test.resize(width, height);
        let mut place = None;
        for (index, (path, name, fraction, percent)) in files.iter().enumerate() {
            copy.current_path = (*path).into();
            copy.transfer_name = (*name).into();
            copy.file_progress = *fraction;
            copy.file_percent = (*percent).into();
            test.window.set_job(copy.clone());
            test.draw();
            let meter = bar(&test);
            assert_eq!(
                meter
                    .accessible_string_property(AccessibleStringProperty::Value)
                    .unwrap(),
                *percent
            );
            let rect = native_rect(&meter);
            // The same place for every file: nothing appears, disappears or shifts.
            assert_eq!(
                *place.get_or_insert(rect),
                rect,
                "file {index} at {width}x{height}"
            );
            let fill = meter.first_child().expect("the bar has a fill");
            assert!((fill.geometry().width() - rect.2 * fraction).abs() <= 1.0);
            // Right under the job's chart.
            let chart = ui::popup_item(
                &test.window,
                "Overall progress",
                AccessibleRole::ProgressIndicator,
            );
            let (_, chart_y, _, chart_height) = native_rect(&chart);
            assert!(rect.1 >= chart_y + chart_height, "below the chart");
            if !path.is_empty() {
                let label = ui::popup_item(&test.window, path, AccessibleRole::Text);
                let (label_x, label_y, label_width, _) = native_rect(&label);
                assert!(label_y < rect.1, "the path is above its bar");
                assert!(
                    label_x + label_width <= width as f32 + 1.0,
                    "the path stays inside the window"
                );
            }
            test.snapshot(&format!("current-file-{index}-{width}x{height}"));
        }
    }
    // A pause keeps the estimate, marked as paused.
    copy.paused = true;
    copy.pause_requested = true;
    copy.eta = "About 3 min (paused)".into();
    test.window.set_job(copy);
    test.draw();
    ui::popup_item(
        &test.window,
        "Time remaining: About 3 min (paused)",
        AccessibleRole::Text,
    );
}

#[test]
fn delete_shows_its_item_totals_and_a_held_estimate_while_paused() {
    let mut test = ui::TestUi::new();
    let mut delete = job("41", "Delete");
    delete.kind = 1;
    delete.active = true;
    delete.pausable = true;
    delete.determinate = true;
    delete.progress = 0.4;
    delete.progress_label = "40% complete".into();
    delete.transfer_totals = "Items deleted: 400 of 1000".into();
    delete.remaining = "Items remaining: 600".into();
    delete.eta = "About 30 s".into();
    test.window.set_job(delete.clone());
    test.draw();
    for label in [
        "Items deleted: 400 of 1000",
        "Items remaining: 600",
        "Time remaining: About 30 s",
    ] {
        ui::popup_item(&test.window, label, AccessibleRole::Text);
    }
    delete.paused = true;
    delete.pause_requested = true;
    delete.activity_label = "Paused".into();
    delete.eta = "About 30 s (paused)".into();
    test.window.set_job(delete);
    test.draw();
    ui::popup_item(
        &test.window,
        "Time remaining: About 30 s (paused)",
        AccessibleRole::Text,
    );
}

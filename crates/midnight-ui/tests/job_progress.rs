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
        "D:\\Source\\資料",
        "example.zip",
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

/// Show `folder` and `name` as the current file, with the ends that the presenter
/// keeps of them (`current_file_tails`).
fn show_file(copy: &mut OperationRow, folder: &str, name: &str) {
    copy.current_path = if name.is_empty() {
        "".into()
    } else {
        format!("{folder}\\{name}").into()
    };
    copy.transfer_directory = folder.into();
    copy.transfer_name = name.into();
    copy.transfer_directory_tail = folder
        .rfind(['\\', '/'])
        .map_or("", |at| &folder[at..])
        .into();
    let kept: Vec<char> = name.chars().collect();
    copy.transfer_name_tail = if kept.len() > 12 {
        kept[kept.len() - 12..].iter().collect::<String>().into()
    } else {
        "".into()
    };
}

/// Under the job's chart: the current file's folder on one line, its name with its
/// percentage on the next, then its own bar. Nothing there moves or changes size
/// from file to file, whatever the lengths of their folders and names, or between
/// files; a long folder or name leaves out its middle and stays inside the window.
#[test]
fn every_file_shows_its_folder_and_name_and_its_own_bar_under_the_chart_without_moving() {
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
    // between two files, a long archive member path, and a very long name.
    let long = format!(
        "D:\\source\\資料\\backup.zip\\{}deep",
        "a-folder-with-a-long-name\\".repeat(8)
    );
    let long_name = format!("{}.MP4", "harbour sunrise wide shot ".repeat(6));
    let files: [(&str, &str, f32, &str); 5] = [
        ("D:\\source\\資料", "large.bin", 0.45, "45%"),
        ("D:\\source\\資料", "small.txt", 1.0, "100%"),
        ("", "", 0.0, ""),
        (long.as_str(), "large.bin", 0.1, "Verifying 10%"),
        ("D:\\s", long_name.as_str(), 0.7, "70%"),
    ];
    for (scheme, density, font) in APPEARANCES {
        test.window
            .invoke_apply_preferences(scheme, density, "".into(), font, font);
        for (width, height) in SIZES {
            let size = format!("theme {scheme}, density {density}, font {font}, {width}x{height}");
            test.resize(width, height);
            let (mut place, mut bar_place) = (None, None);
            for (index, (folder, name, fraction, percent)) in files.iter().enumerate() {
                show_file(&mut copy, folder, name);
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
                // The same place and size for every file, and between files.
                assert_eq!(*bar_place.get_or_insert(rect), rect, "file {index}: {size}");
                if name.is_empty() {
                    continue;
                }
                let folder_line = ui::popup_item(&test.window, folder, AccessibleRole::Text);
                let name_line = ui::popup_item(&test.window, name, AccessibleRole::Text);
                let lines = (native_rect(&folder_line), native_rect(&name_line), rect);
                assert_eq!(*place.get_or_insert(lines), lines, "file {index}: {size}");
                let fill = meter.first_child().expect("the bar has a fill");
                assert!((fill.geometry().width() - rect.2 * fraction).abs() <= 1.0);
                // Right under the job's chart: the folder, the name, then the bar.
                let chart = ui::popup_item(
                    &test.window,
                    "Overall progress",
                    AccessibleRole::ProgressIndicator,
                );
                let (_, chart_y, _, chart_height) = native_rect(&chart);
                let (folder_rect, name_rect, _) = lines;
                assert!(
                    folder_rect.1 >= chart_y + chart_height,
                    "below the chart: {size}"
                );
                assert!(
                    folder_rect.1 + folder_rect.3 <= name_rect.1 + 0.5,
                    "the folder above the name: {size}"
                );
                assert!(
                    name_rect.1 + name_rect.3 <= rect.1 + 0.5,
                    "the name above its bar: {size}"
                );
                for (what, line) in [("folder", folder_rect), ("name", name_rect)] {
                    assert!(
                        line.0 >= 0.0 && line.0 + line.2 <= width as f32 + 1.0,
                        "the {what} stays inside the window: {size}"
                    );
                }
                test.snapshot(&format!("current-file-{index}-{scheme}-{density}-{font}"));
            }
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

/// The text of a shown popup: a hover tooltip (`tooltip`) or another popup.
fn popup_text(window: &midnight_ui::JobProgressWindow, tooltip: bool) -> Option<String> {
    use i_slint_core::{
        items::{ComplexText, SimpleText},
        window::{WindowInner, WindowKind},
    };
    fn text(item: ItemRc) -> Option<String> {
        if let Some(text) = item.downcast::<SimpleText>() {
            return Some(text.as_pin_ref().text().to_string());
        }
        if let Some(text) = item.downcast::<ComplexText>() {
            return Some(text.as_pin_ref().text().to_string());
        }
        let mut child = item.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            if let Some(found) = text(item) {
                return Some(found);
            }
        }
        None
    }
    let inner = WindowInner::from_pub(window.window());
    let popups = inner.active_popups();
    popups
        .iter()
        .filter(|popup| matches!(popup.window_kind, WindowKind::ToolTip) == tooltip)
        .find_map(|popup| text(ItemRc::new_root(popup.component.clone())))
}

/// Where the opened (non-tooltip) popup lies in the window: left, top, right, bottom.
fn opened_bounds(window: &midnight_ui::JobProgressWindow) -> (f32, f32, f32, f32) {
    use i_slint_core::{
        items::WindowItem,
        window::{PopupWindowLocation, WindowInner, WindowKind},
    };
    let inner = WindowInner::from_pub(window.window());
    let popups = inner.active_popups();
    let popup = popups
        .iter()
        .find(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
        .expect("the opened explanation");
    let PopupWindowLocation::ChildWindow(origin) = popup.location else {
        panic!("embedded popup");
    };
    let item = ItemRc::new_root(popup.component.clone());
    let frame = item.downcast::<WindowItem>().expect("popup Window item");
    let (width, height) = (
        frame.as_pin_ref().width().get(),
        frame.as_pin_ref().height().get(),
    );
    (origin.x, origin.y, origin.x + width, origin.y + height)
}

/// Rest the pointer at `position` for longer than the tooltip delay.
fn rest_pointer(test: &mut ui::TestUi, position: slint::LogicalPosition) {
    use slint::platform::WindowEvent;
    test.window
        .window()
        .dispatch_event(WindowEvent::PointerExited);
    test.draw();
    test.window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    test.draw();
    test.wait(std::time::Duration::from_millis(800));
    test.draw();
}

fn centre(item: &ItemRc) -> slint::LogicalPosition {
    let (x, y, width, height) = native_rect(item);
    slint::LogicalPosition::new(x + width / 2.0, y + height / 2.0)
}

fn charted_copy() -> OperationRow {
    let mut copy = job("copy-job:52", "Copy");
    copy.state = "Copying".into();
    copy.active = true;
    copy.pausable = true;
    copy.determinate = true;
    copy.progress = 0.4;
    copy.progress_label = "40% complete".into();
    copy.current_path = "/source/large.bin".into();
    copy.transfer_name = "large.bin".into();
    copy.transfer_speed_chart = SpeedChartData {
        area: "M 0 100 L 0 60 L 500 40 L 1000 50 L 1000 100 Z".into(),
        primary: "M 0 60 L 500 40 L 1000 50".into(),
        speed: "40 MiB/s".into(),
        has_samples: true,
        ..Default::default()
    };
    copy
}

/// Screenshots of the chart hovered and of its explanation, before and after the
/// change (`MIDNIGHT_UI_RENDER_DIR`); no assertions, so that it runs on either layout.
#[test]
fn chart_explanation_snapshots() {
    let mut test = ui::TestUi::with_virtual_clock();
    test.window.set_job(charted_copy());
    test.resize(640, 420);
    let chart = ui::popup_item(
        &test.window,
        "Overall progress",
        AccessibleRole::ProgressIndicator,
    );
    rest_pointer(&mut test, centre(&chart));
    test.snapshot("chart-hovered");
    if let Some(button) = ui::find_popup_item(&test.window, "Chart details", AccessibleRole::Button)
    {
        rest_pointer(&mut test, centre(&button));
        test.snapshot("info-hovered");
    }
}

/// Only the "i" button above the chart's top right corner explains the chart: the
/// chart shows no tooltip; hovering the button shows the explanation as a tooltip,
/// and clicking it (or Enter once focused) opens the same text without moving anything.
#[test]
fn only_the_info_button_above_the_chart_shows_its_explanation() {
    let mut test = ui::TestUi::with_virtual_clock();
    let explanation = "Write: bytes written per second.";
    // Fonts 8, 12 and 32, Desktop (0) and Touch (1) density, light and dark themes.
    for (scheme, density, font) in [(0, 0, 8), (0, 0, 12), (1, 0, 32), (1, 1, 12), (0, 1, 32)] {
        for (width, height) in [(1000, 700), (640, 420), (320, 260)] {
            test.window
                .invoke_apply_preferences(scheme, density, "".into(), font, font);
            test.window.set_job(charted_copy());
            test.resize(width, height);
            let size = format!("theme {scheme}, density {density}, font {font}, {width}x{height}");
            let chart = ui::popup_item(
                &test.window,
                "Overall progress",
                AccessibleRole::ProgressIndicator,
            );
            let button = ui::popup_item(&test.window, "Chart details", AccessibleRole::Button);
            let (chart_x, chart_y, chart_width, _) = native_rect(&chart);
            let (button_x, button_y, button_width, button_height) = native_rect(&button);
            assert!(
                button_y + button_height <= chart_y,
                "above the chart: {size}"
            );
            assert!(
                (button_x + button_width - (chart_x + chart_width)).abs() <= 2.0,
                "at the chart's right edge: {size}"
            );
            assert!(
                button_x + button_width <= width as f32,
                "inside the window: {size}"
            );
            // The legend beside it keeps clear of it.
            if let Some(legend) =
                ui::find_popup_item(&test.window, "Write: 40 MiB/s", AccessibleRole::Text)
            {
                let (legend_x, _, legend_width, _) = native_rect(&legend);
                assert!(
                    legend_x + legend_width <= button_x,
                    "beside the legend: {size}"
                );
            }
            let before = native_rect(&chart);
            rest_pointer(&mut test, centre(&chart));
            assert_eq!(
                popup_text(&test.window, true),
                None,
                "the chart has no tooltip: {size}"
            );
            rest_pointer(&mut test, centre(&button));
            let tip = popup_text(&test.window, true).expect("the button's tooltip");
            assert!(tip.starts_with(explanation), "{tip}");
            // Clicked (or focused): the same text, opened; nothing moves.
            ui::click_popup(&test.window, "Chart details", AccessibleRole::Button);
            test.draw();
            let opened = popup_text(&test.window, false).expect("the opened explanation");
            assert_eq!(opened, tip);
            // Below the button, right-aligned with the chart, inside the window.
            let (left, top, right, bottom) = opened_bounds(&test.window);
            // Where it fits below the button it opens there; a window too short for it
            // keeps it inside.
            if button_y + button_height + 4.0 + (bottom - top) <= height as f32 {
                assert!(top >= button_y + button_height, "below the button: {size}");
            } else {
                // Taller than the window only at the largest font in the smallest window.
                assert!(top >= -0.5, "inside: {size}");
                if bottom - top <= height as f32 {
                    assert!(bottom <= height as f32 + 0.5, "inside: {size}");
                }
            }
            assert!(
                (right - (button_x + button_width)).abs() <= 2.0,
                "right-aligned with the chart: {size}"
            );
            assert!(
                left >= -0.5 && right <= width as f32 + 0.5,
                "inside the window: {size}"
            );
            assert!(bottom > top);
            assert_eq!(native_rect(&chart), before, "no layout shift: {size}");
            test.snapshot(&format!("info-open-{scheme}-{density}-{font}"));
            // The next click, here on the button again, closes it.
            ui::click_popup(&test.window, "Chart details", AccessibleRole::Button);
            test.draw();
            assert_eq!(
                popup_text(&test.window, false),
                None,
                "the next click closes it"
            );
        }
    }
    // Keyboard: Enter on the focused button opens it too.
    test.window
        .invoke_apply_preferences(0, 0, "".into(), 12, 12);
    test.draw();
    let button = ui::popup_item(&test.window, "Chart details", AccessibleRole::Button);
    rest_pointer(&mut test, slint::LogicalPosition::new(1.0, 1.0));
    // Focused by a click that opened it, closed by the next one: Enter opens it again.
    ui::click_popup(&test.window, "Chart details", AccessibleRole::Button);
    test.draw();
    ui::click_popup(&test.window, "Chart details", AccessibleRole::Button);
    test.draw();
    assert_eq!(popup_text(&test.window, false), None);
    ui::key(&test.window, slint::platform::Key::Return);
    test.draw();
    assert!(
        popup_text(&test.window, false).is_some_and(|text| text.starts_with(explanation)),
        "Enter opens the explanation"
    );
    drop(button);
}

/// A Copy at 42% through a long folder path, as the owner sees it: written as
/// artifacts only (`MIDNIGHT_UI_RENDER_DIR`), so that it runs on either layout.
fn owner_copy() -> OperationRow {
    let mut copy = job("copy-job:61", "Copy");
    copy.state = "Copying".into();
    copy.active = true;
    copy.pausable = true;
    copy.determinate = true;
    copy.progress = 0.42;
    copy.progress_label = "42% complete".into();
    copy.transfer_totals = "Files: 1,204 of 2,861 · Bytes: 18.4 GiB of 43.7 GiB".into();
    copy.eta = "About 3 min".into();
    copy.remaining = "Items remaining: 1,657".into();
    let folder = "D:\\Projects\\Customer archive 2026\\Raw footage\\Day 03 - harbour\\Camera B";
    copy.transfer_directory = folder.into();
    copy.transfer_name = "C0042 harbour sunrise wide shot.MP4".into();
    copy.transfer_directory_tail = "\\Camera B".into();
    copy.transfer_name_tail = "ide shot.MP4".into();
    copy.current_path = format!("{folder}\\C0042 harbour sunrise wide shot.MP4").into();
    copy.file_determinate = true;
    copy.file_progress = 0.63;
    copy.file_percent = "63%".into();
    copy.file_progress_title = "Current file written".into();
    copy.file_progress_label = "Current file written: 1,353,711,616 / 2,148,532,224 bytes".into();
    copy.write_speed = "write 187.0 MiB/s".into();
    copy.verify_speed = "verify 160.4 MiB/s".into();
    copy.transfer_speed_chart = SpeedChartData {
        area: "M 0 100 L 0 62 L 100 30 L 200 55 L 300 41 L 400 48 L 500 22 L 600 44 L 700 39 L 800 47 L 900 40 L 1000 43 L 1000 100 Z".into(),
        primary: "M 0 62 L 100 30 L 200 55 L 300 41 L 400 48 L 500 22 L 600 44 L 700 39 L 800 47 L 900 40 L 1000 43".into(),
        verification: "M 0 100 L 100 100 L 200 80 L 300 60 L 400 52 L 500 57 L 600 49 L 700 55 L 800 50 L 900 53 L 1000 51".into(),
        speed: "187.0 MiB/s".into(),
        verification_speed: "160.4 MiB/s".into(),
        scale: "330.0 MiB/s".into(),
        history: "Elapsed 1 min 41 s".into(),
        has_samples: true,
        ..Default::default()
    };
    copy
}

#[test]
fn owner_progress_snapshots() {
    let mut test = ui::TestUi::new();
    for (scheme, density, font, width, height) in [
        (0, 0, 12, 640, 420),
        (1, 0, 12, 640, 420),
        (0, 0, 8, 480, 360),
        (1, 1, 32, 1100, 820),
    ] {
        test.window
            .invoke_apply_preferences(scheme, density, "".into(), font, font);
        test.window.set_job(owner_copy());
        test.resize(width, height);
        test.snapshot(&format!("owner-{scheme}-{density}-{font}"));
    }
}

/// Fonts 8, 12 and 32, Desktop and Touch, dark and light, narrow and wide windows.
const APPEARANCES: [(i32, i32, i32); 6] = [
    (0, 0, 8),
    (0, 0, 12),
    (1, 0, 12),
    (1, 1, 12),
    (0, 1, 32),
    (1, 0, 32),
];
const SIZES: [(usize, usize); 3] = [(1100, 820), (640, 420), (420, 360)];

/// The speeds sit beside the headline on its line, and neither moves while the
/// percentage and the rates change, nor when a rate is not measured ("—").
#[test]
fn the_speeds_beside_the_headline_hold_still_while_their_values_change() {
    let mut test = ui::TestUi::new();
    let states = [
        ("9% complete", "write —", "verify —"),
        ("42% complete", "write 187.0 MiB/s", "verify 160.4 MiB/s"),
        ("99% complete", "write 1,023.9 KiB/s", "verify 1.2 GiB/s"),
        ("100% complete", "write 8.5 B/s", "verify —"),
    ];
    for (scheme, density, font) in APPEARANCES {
        test.window
            .invoke_apply_preferences(scheme, density, "".into(), font, font);
        for (width, height) in SIZES {
            let size = format!("theme {scheme}, density {density}, font {font}, {width}x{height}");
            let mut places = None;
            for (headline, write, verify) in states {
                let mut copy = owner_copy();
                copy.progress_label = headline.into();
                copy.write_speed = write.into();
                copy.verify_speed = verify.into();
                test.window.set_job(copy);
                test.resize(width, height);
                let label = ui::popup_item(&test.window, headline, AccessibleRole::Text);
                let write =
                    ui::popup_item(&test.window, &format!(" · {write}"), AccessibleRole::Text);
                let verify =
                    ui::popup_item(&test.window, &format!(" · {verify}"), AccessibleRole::Text);
                let (label_rect, write_rect, verify_rect) = (
                    native_rect(&label),
                    native_rect(&write),
                    native_rect(&verify),
                );
                // One line, in reading order, inside the window.
                assert!(
                    (label_rect.1 + label_rect.3 - (write_rect.1 + write_rect.3)).abs() <= 1.0,
                    "same line: {size}"
                );
                assert!(label_rect.0 + label_rect.2 <= write_rect.0 + 0.5, "{size}");
                assert!(write_rect.0 + write_rect.2 <= verify_rect.0 + 0.5, "{size}");
                assert!(
                    verify_rect.0 + verify_rect.2 <= width as f32,
                    "inside: {size}"
                );
                let place = (write_rect.0, verify_rect.0, verify_rect.2);
                assert_eq!(*places.get_or_insert(place), place, "no shift: {size}");
            }
            test.snapshot(&format!("speeds-{scheme}-{density}-{font}"));
        }
    }
    // Without a sampled or full readback only the write speed shows; paused, none.
    let mut copy = owner_copy();
    copy.verify_speed = "".into();
    test.window.set_job(copy.clone());
    test.draw();
    ui::popup_item(&test.window, " · write 187.0 MiB/s", AccessibleRole::Text);
    assert!(
        ui::find_popup_item(&test.window, " · verify 160.4 MiB/s", AccessibleRole::Text).is_none()
    );
    copy.write_speed = "".into();
    copy.progress_label = "Paused".into();
    test.window.set_job(copy);
    test.draw();
    ui::popup_item(&test.window, "Paused", AccessibleRole::Text);
    assert!(
        ui::find_popup_item(&test.window, " · write 187.0 MiB/s", AccessibleRole::Text).is_none()
    );
}

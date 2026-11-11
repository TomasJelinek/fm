//! Native wheel frames on real lists: row geometry, nested ownership and popup routing.
#[path = "support/scroll_frames.rs"]
mod frames;

use frames::{Frames, bounds, near, offset, scroll_parent};
use i_slint_core::{
    accessibility::AccessibilityAction,
    items::{AccessibleRole, Flickable},
    lengths::LogicalPoint,
};
use midnight_ui::{
    ColorOptionsView, FindRow, FindWindow, MainWindow, NavigationChoiceRow, OperationRow,
};
use midnight_ui_input as native_scroll;
use slint::{ComponentHandle, ModelRc, VecModel};

#[test]
fn find_results_render_native_wheel_frames_without_dragging_selection() {
    let mut h = Frames::new(FindWindow::new, 860, 620);
    h.window.set_rows(ModelRc::new(VecModel::from(
        (0..100)
            .map(|index| FindRow {
                token: format!("result-{index}").into(),
                name: format!("Result {index:02}").into(),
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    )));
    h.window.set_selected_row(3);
    h.settle();
    h.check_motion(AccessibleRole::ListItem, "Result 03 ·  ·  ·  · ");
    assert_eq!(
        h.window.get_selected_row(),
        3,
        "scrolling must retain result identity"
    );
}

#[test]
fn settings_inner_list_owns_wheel_motion_until_the_pointer_moves_outside_it() {
    let mut h = Frames::new(MainWindow::new, 720, 520);
    h.window.set_preferences_ready(true);
    let colors = h.window.global::<ColorOptionsView>();
    colors.set_labels(ModelRc::new(VecModel::from(
        (0..32)
            .map(|index| format!("Rule {index:02}").into())
            .collect::<Vec<_>>(),
    )));
    colors.set_selected(3);
    h.window.invoke_show_options();
    h.window.set_options_topic_index(8);
    h.settle();
    let row = h.item(AccessibleRole::ListItem, "Rule 03");
    let inner = scroll_parent(&row);
    let outer = scroll_parent(&inner);
    let initial_outer = offset(&outer);
    h.check_motion(AccessibleRole::ListItem, "Rule 03");
    near(
        offset(&outer),
        initial_outer,
        "wheel over nested list must not move settings page",
    );
    let inner_offset = offset(&inner);
    let area = bounds(&outer);
    let extent = outer
        .downcast::<Flickable>()
        .unwrap()
        .as_pin_ref()
        .content_height()
        .get();
    assert!(
        extent > area.3 - area.1 + 60.0,
        "fixture needs a scrollable settings page"
    );
    let inner_area = bounds(&inner);
    let point = LogicalPoint::new(area.0 + 20.0, area.1 + 3.0);
    assert!(
        point.y < inner_area.1,
        "outer target must be over the heading above the inner list"
    );
    h.native_wheel(point, -37.0);
    let mut previous = initial_outer;
    for _ in 0..15 {
        h.advance(16);
        let current = offset(&outer);
        assert!(current <= previous + 0.01 && (current - previous).abs() < 12.0);
        previous = current;
        near(
            offset(&inner),
            inner_offset,
            "outer wheel must not move the nested list",
        );
    }
    near(
        offset(&outer),
        initial_outer - 37.0,
        "outer settings page receives its own wheel",
    );
}

#[test]
fn jobs_list_renders_native_wheel_frames_without_scrolling_its_details() {
    let mut h = Frames::new(MainWindow::new, 1000, 760);
    h.window.set_operations(ModelRc::new(VecModel::from(
        (0..30)
            .map(|index| OperationRow {
                identity: format!("job-{index}").into(),
                title: format!("Job {index:02}").into(),
                detail: "Retained job details".into(),
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    )));
    h.window.invoke_show_operations();
    h.settle();
    let row = h.item(AccessibleRole::ListItem, "Job 02");
    let inner = scroll_parent(&row);
    let outer = scroll_parent(&inner);
    let original = offset(&outer);
    h.check_motion(AccessibleRole::ListItem, "Job 02");
    near(
        offset(&outer),
        original,
        "Jobs wheel must stay within the operation list",
    );
}

#[test]
fn command_history_dropdown_renders_native_wheel_frames_without_committing_a_choice() {
    let mut h = Frames::new(MainWindow::new, 1000, 760);
    h.window.set_command_history(ModelRc::new(VecModel::from(
        (0..60)
            .map(|index| NavigationChoiceRow {
                identity: format!("command-{index}").into(),
                label: format!("echo command {index:02}").into(),
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    )));
    h.window.invoke_set_command("unchanged draft".into(), 0);
    h.settle();
    h.item(AccessibleRole::Combobox, "Executed command history")
        .accessible_action(&AccessibilityAction::Expand);
    h.settle();
    h.check_motion(AccessibleRole::Button, "echo command 03");
    assert_eq!(h.window.get_command_text(), "unchanged draft");
}

#[test]
fn find_scrollbar_wheel_uses_the_same_animation_as_result_content() {
    let mut h = Frames::new(FindWindow::new, 860, 620);
    h.window.set_rows(ModelRc::new(VecModel::from(
        (0..100)
            .map(|index| FindRow {
                token: format!("result-{index}").into(),
                name: format!("Result {index:02}").into(),
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    )));
    h.settle();
    let viewport = scroll_parent(&h.item(AccessibleRole::ListItem, "Result 03 ·  ·  ·  · "));
    let area = bounds(&viewport);
    let point = LogicalPoint::new(area.2 - 3.0, (area.1 + area.3) / 2.0);
    let initial = offset(&viewport);
    h.install_native_scroll();
    assert!(
        h.driver_wheel(point, 0.0, -1.0),
        "native scrollbar hook must own line input"
    );
    near(
        offset(&viewport),
        initial,
        "wheel over scrollbar must begin an animation",
    );
    h.advance(16);
    let first = offset(&viewport);
    assert!(
        first < initial && first > initial - 12.0,
        "first scrollbar wheel frame must move only part of the notch: {initial} -> {first}"
    );
    for _ in 0..14 {
        h.advance(16);
    }
    near(
        offset(&viewport),
        initial - 60.0,
        "scrollbar wheel retains exact distance",
    );
}

#[test]
fn native_scrollbar_hook_preserves_pixels_and_cancels_when_a_popup_takes_ownership() {
    use slint::winit_030::winit::{
        dpi::PhysicalPosition,
        event::{DeviceId, MouseScrollDelta, TouchPhase, WindowEvent},
    };
    let mut h = Frames::new(MainWindow::new, 1000, 760);
    h.window.set_operations(ModelRc::new(VecModel::from(
        (0..30)
            .map(|index| OperationRow {
                identity: format!("job-{index}").into(),
                title: format!("Job {index:02}").into(),
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    )));
    h.window.invoke_show_operations();
    h.settle();
    let viewport = scroll_parent(&h.item(AccessibleRole::ListItem, "Job 02"));
    let area = bounds(&viewport);
    let point = LogicalPoint::new(area.2 - 3.0, (area.1 + area.3) / 2.0);
    h.install_native_scroll();
    assert!(h.driver_wheel(point, 0.0, -1.0));
    h.advance(16);
    let before = offset(&viewport);
    assert!(
        !h.driver_event(WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -3.25)),
            phase: TouchPhase::Moved,
        }),
        "native pixels stay on the original SDK route"
    );
    h.pixels(point, -3.25);
    near(
        offset(&viewport),
        before - 3.25,
        "native scrollbar pixels retain their exact delta",
    );
    for _ in 0..20 {
        h.advance(16);
    }
    near(
        offset(&viewport),
        before - 3.25,
        "native pixel input cancels the synthetic wheel tail",
    );
    assert!(h.driver_wheel(point, 0.0, -1.0));
    h.advance(16);
    let paused = offset(&viewport);
    h.window.invoke_show_options();
    h.settle();
    for _ in 0..20 {
        h.advance(16);
    }
    near(
        offset(&viewport),
        paused,
        "a new popup must not inherit old scrollbar movement",
    );
}

#[test]
fn find_horizontal_scrollbar_is_detected_at_nonzero_window_coordinates() {
    let mut h = Frames::new(FindWindow::new, 520, 620);
    h.window.set_rows(ModelRc::new(VecModel::from(
        (0..100)
            .map(|index| FindRow {
                name: format!("Result {index:02}").into(),
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    )));
    h.settle();
    let viewport = scroll_parent(&h.item(AccessibleRole::ListItem, "Result 03 ·  ·  ·  · "));
    let area = bounds(&viewport);
    let point = LogicalPoint::new((area.0 + area.2) / 2.0, area.3 - 3.0);
    assert!(
        area.0 > 0.0 && area.1 > 0.0,
        "fixture must exercise an offset viewport"
    );
    let x = || {
        viewport
            .downcast::<Flickable>()
            .unwrap()
            .as_pin_ref()
            .content_x()
            .get()
    };
    h.install_native_scroll();
    let initial = x();
    assert!(
        h.driver_wheel(point, -1.0, 0.0),
        "horizontal overlay must be found"
    );
    near(x(), initial, "horizontal scrollbar starts an animation");
    for _ in 0..15 {
        h.advance(16);
    }
    near(
        x(),
        initial - 60.0,
        "horizontal scrollbar receives the complete notch",
    );
    near(
        offset(&viewport),
        0.0,
        "horizontal movement must not move the vertical axis",
    );
}

#[test]
fn command_strip_smooths_line_remapping_without_treating_hidden_bars_as_visible() {
    use i_slint_core::lengths::{LogicalRect, LogicalSize};
    let mut h = Frames::new(MainWindow::new, 1000, 760);
    h.window
        .set_left_path(format!("D:/{}", "long-directory/".repeat(25)).into());
    h.window.set_active_pane(0);
    h.settle();
    let area = LogicalRect::new(
        LogicalPoint::new(
            h.window.get_command_path_left(),
            h.window.get_command_path_top(),
        ),
        LogicalSize::new(
            h.window.get_command_path_width(),
            h.window.get_command_path_height(),
        ),
    );
    assert!(h.window.get_command_path_content_width() > area.width() + 60.0);
    let point = LogicalPoint::new(area.center().x, area.max_y() - 3.0);
    h.install_native_scroll();
    assert!(
        !h.driver_wheel(point, 1.0, 0.0),
        "always-off horizontal bar must not be invented from overflow"
    );
    h.command = Some(native_scroll::CommandStrip {
        bounds: area,
        path: h.window.get_command_directory(),
    });
    let initial = h.window.get_command_path_text_left();
    assert!(
        h.driver_wheel(point, 0.0, 1.0),
        "command strip owns the vertical-to-horizontal remap"
    );
    near(
        h.window.get_command_path_text_left(),
        initial,
        "command wheel begins an animation",
    );
    let mut previous = initial;
    for _ in 0..15 {
        h.advance(16);
        let current = h.window.get_command_path_text_left();
        assert!(current >= previous - 0.03 && current - previous < 12.0);
        previous = current;
    }
    near(
        previous,
        initial + 60.0,
        "command strip retains remapped distance",
    );
    h.pixels(point, 3.25);
    near(
        h.window.get_command_path_text_left(),
        previous + 3.25,
        "direct pixels preserve the remap exactly",
    );
    assert!(h.driver_wheel(point, 0.0, 1.0));
    h.advance(16);
    let paused = h.window.get_command_path_text_left();
    h.command.as_mut().unwrap().path = "different-context".into();
    for _ in 0..20 {
        h.advance(16);
    }
    near(
        h.window.get_command_path_text_left(),
        paused,
        "directory identity change cancels remaining motion",
    );
}

#[test]
fn native_scrollbar_hit_testing_uses_logical_coordinates_at_high_dpi() {
    let mut h = Frames::new(FindWindow::new, 1040, 1240);
    h.window
        .window()
        .dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor: 2.0 });
    h.window
        .window()
        .dispatch_event(slint::platform::WindowEvent::Resized {
            size: slint::LogicalSize::new(520.0, 620.0),
        });
    h.window.set_rows(ModelRc::new(VecModel::from(
        (0..100)
            .map(|index| FindRow {
                name: format!("Result {index:02}").into(),
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    )));
    h.settle();
    let viewport = scroll_parent(&h.item(AccessibleRole::ListItem, "Result 03 ·  ·  ·  · "));
    let area = bounds(&viewport);
    h.install_native_scroll();
    assert!(h.driver_wheel(
        LogicalPoint::new(area.2 - 3.0, (area.1 + area.3) / 2.0),
        0.0,
        -1.0
    ));
    for _ in 0..15 {
        h.advance(16);
    }
    near(
        offset(&viewport),
        -60.0,
        "physical input must retain the same logical wheel distance",
    );
}

#[test]
fn auxiliary_hook_does_not_take_wheel_events_from_a_closed_combo_box() {
    let mut h = Frames::new(FindWindow::new, 860, 620);
    h.settle();
    let combo = h.item(AccessibleRole::Combobox, "Find match mode");
    let area = bounds(&combo);
    h.install_native_scroll();
    assert!(
        !h.driver_wheel(
            LogicalPoint::new((area.0 + area.2) / 2.0, (area.1 + area.3) / 2.0),
            0.0,
            -1.0
        ),
        "closed ComboBox wheel selection must stay on its existing discrete handler"
    );
}

#[test]
fn standard_options_combo_popup_uses_its_own_scrollbar_hook() {
    let mut h = Frames::new(MainWindow::new, 620, 620);
    h.window.set_preferences_ready(true);
    h.window.invoke_show_options();
    h.settle();
    h.item(AccessibleRole::Combobox, "Options topic")
        .accessible_action(&AccessibilityAction::Expand);
    h.settle();
    let viewport = scroll_parent(&h.item(AccessibleRole::ListItem, "Editors"));
    let area = bounds(&viewport);
    let initial = offset(&viewport);
    let topic = h.window.get_options_topic_index();
    h.install_native_scroll();
    assert!(h.driver_wheel(
        LogicalPoint::new(area.2 - 3.0, (area.1 + area.3) / 2.0),
        0.0,
        -1.0
    ));
    for _ in 0..15 {
        h.advance(16);
    }
    near(
        offset(&viewport),
        initial - 60.0,
        "SDK popup scrollbar receives full animated distance",
    );
    assert_eq!(
        h.window.get_options_topic_index(),
        topic,
        "scrolling the popup must not select its rows"
    );
}

#[test]
fn scrollbar_wheel_releases_find_cursor_following_without_retargeting_selection() {
    let mut h = Frames::new(FindWindow::new, 860, 620);
    h.window.set_rows(ModelRc::new(VecModel::from(
        (0..100)
            .map(|index| FindRow {
                name: format!("Result {index:02}").into(),
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    )));
    h.window.set_selected_row(99);
    h.settle();
    let viewport = scroll_parent(&h.item(AccessibleRole::ListItem, "Result 99 ·  ·  ·  · "));
    let area = bounds(&viewport);
    let initial = offset(&viewport);
    h.install_native_scroll();
    assert!(h.driver_wheel(
        LogicalPoint::new(area.2 - 3.0, (area.1 + area.3) / 2.0),
        0.0,
        1.0
    ));
    for _ in 0..15 {
        h.advance(16);
    }
    near(
        offset(&viewport),
        initial + 60.0,
        "scrollbar motion must release Find cursor-following",
    );
    assert_eq!(
        h.window.get_selected_row(),
        99,
        "scrolling preserves selected result identity"
    );
}

#[test]
fn find_scrollbar_precise_pixels_release_cursor_following_without_an_extra_tail() {
    use slint::winit_030::winit::{
        dpi::PhysicalPosition,
        event::{DeviceId, MouseScrollDelta, TouchPhase, WindowEvent},
    };
    let mut h = Frames::new(FindWindow::new, 860, 620);
    h.window.set_rows(ModelRc::new(VecModel::from(
        (0..100)
            .map(|index| FindRow {
                name: format!("Result {index:02}").into(),
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    )));
    h.window.set_selected_row(99);
    h.settle();
    let viewport = scroll_parent(&h.item(AccessibleRole::ListItem, "Result 99 ·  ·  ·  · "));
    let area = bounds(&viewport);
    let point = LogicalPoint::new(area.2 - 3.0, (area.1 + area.3) / 2.0);
    let initial = offset(&viewport);
    h.install_native_scroll();
    h.driver_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(f64::from(point.x), f64::from(point.y)),
    });
    assert!(
        !h.driver_event(WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, 7.25)),
            phase: TouchPhase::Moved,
        }),
        "precise input must keep its native dispatch"
    );
    h.pixels(point, 7.25);
    near(
        offset(&viewport),
        initial + 7.25,
        "precise pixels release Find cursor-following before changing the offset",
    );
    h.advance(400);
    near(
        offset(&viewport),
        initial + 7.25,
        "precise input has no added tail",
    );
    assert_eq!(h.window.get_selected_row(), 99);
}

fn scrollbar_observer_does_not_retain_closed_window(settle: bool) {
    let mut h = Frames::new(FindWindow::new, 860, 620);
    h.window.set_rows(ModelRc::new(VecModel::from(
        (0..100)
            .map(|index| FindRow {
                name: format!("Result {index:02}").into(),
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    )));
    h.settle();
    let point = {
        let viewport = scroll_parent(&h.item(AccessibleRole::ListItem, "Result 03 ·  ·  ·  · "));
        let area = bounds(&viewport);
        LogicalPoint::new(area.2 - 3.0, (area.1 + area.3) / 2.0)
    };
    h.install_native_scroll();
    assert!(h.driver_wheel(point, 0.0, -1.0));
    for _ in 0..if settle { 15 } else { 1 } {
        h.advance(16);
    }
    let weak = h.window.as_weak();
    // A native adapter owns its hook even as the application releases the
    // component. Keep that observer alive while dropping our last component.
    let observer = h.take_native_scroll();
    h.window.hide().unwrap();
    drop(h);
    assert!(
        weak.upgrade().is_none(),
        "scrollbar observer retained the closed component"
    );
    drop(observer);
}

#[test]
fn active_scrollbar_motion_does_not_retain_closed_component() {
    scrollbar_observer_does_not_retain_closed_window(false);
}

#[test]
fn settled_scrollbar_motion_does_not_retain_closed_component() {
    scrollbar_observer_does_not_retain_closed_window(true);
}

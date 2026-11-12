//! The application toolbar follows one active panel and cancels stale popups.
#[path = "support/render.rs"]
mod render;
#[path = "support/toolbar.rs"]
mod toolbar;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    accessibility::AccessibleStringProperty, input::TouchPhase, item_tree::ItemRc,
    items::AccessibleRole, lengths::LogicalPoint, platform::InternalEvent, window::WindowInner,
};
use midnight_ui::{FileRow, MainWindow, NavigationChoiceRow, Theme, UserCommandRow};
use slint::{ComponentHandle, platform::WindowEvent};
use std::{cell::RefCell, rc::Rc};

fn count(item: ItemRc, label: &str, role: AccessibleRole) -> usize {
    let mut total = usize::from(
        item.accessible_role() == role
            && item
                .accessible_string_property(AccessibleStringProperty::Label)
                .is_some_and(|text| text.as_str() == label),
    );
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        total += count(item, label, role);
    }
    total
}

fn native_touch(window: &MainWindow, point: slint::LogicalPosition, phase: TouchPhase) {
    // Raw capture precedes the runtime's touch translation. A fresh public mouse
    // press would correctly reset cancellation and does not model this sequence.
    match phase {
        TouchPhase::Started => window.invoke_capture_function_press(point.x, point.y),
        TouchPhase::Cancelled => window.invoke_cancel_function_press(),
        _ => {}
    }
    window
        .window()
        .dispatch_event(WindowEvent::internal(InternalEvent::Touch {
            id: 1,
            position: LogicalPoint::new(point.x, point.y),
            phase,
        }));
    if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
        window.invoke_release_function_press();
    }
}

#[test]
fn options_and_about_stay_at_the_top_right_when_toolbar_scrolls() {
    let mut test = ui::TestUi::new();
    test.window.set_application_version("0.1".into());
    let about_requests = Rc::new(RefCell::new(0));
    let requests = about_requests.clone();
    test.window
        .on_open_about_window(move || *requests.borrow_mut() += 1);
    let mut expected_requests = 0;
    for (width, height) in [(1340, 820), (320, 200)] {
        test.resize(width, height);
        test.draw();
        let options = ui::root_item(&test.window, "Options", AccessibleRole::Button);
        let rect = options.geometry();
        let origin = options.map_to_native_window(rect.origin);
        assert!(
            origin.y < 50.0,
            "Options must be on the top toolbar, got {origin:?}"
        );
        assert!(
            !ui::visible_text_values(options.clone())
                .iter()
                .any(|s| s == "Options"),
            "Options uses a gear and tooltip, not a text caption"
        );
        let about = ui::root_item(
            &test.window,
            "About Midnight Explorer",
            AccessibleRole::Button,
        );
        let about_rect = about.geometry();
        let about_origin = about.map_to_native_window(about_rect.origin);
        assert!((origin.y - about_origin.y).abs() < 1.0);
        assert!(origin.x + rect.width() <= about_origin.x);
        assert!(about_origin.x + about_rect.width() <= width as f32);
        assert!(about_origin.x + about_rect.width() > width as f32 - 25.0);
        let root = ItemRc::new_root(WindowInner::from_pub(test.window.window()).component());
        assert_eq!(count(root, "Options", AccessibleRole::Button), 1);
        render::snapshot(&test, &format!("top-application-buttons-{width}"));
        ui::click_root(&test.window, "Options", AccessibleRole::Button);
        test.draw();
        assert!(ui::has_popup(&test.window));
        ui::click_popup(&test.window, "Cancel", AccessibleRole::Button);
        test.draw();
        ui::click_root(
            &test.window,
            "About Midnight Explorer",
            AccessibleRole::Button,
        );
        test.draw();
        expected_requests += 1;
        assert_eq!(*about_requests.borrow(), expected_requests);
        assert!(!ui::has_popup(&test.window));
    }
}

#[test]
fn filter_indicator_has_a_direct_clear_action_bound_to_its_panel() {
    let mut test = ui::TestUi::new();
    test.window.set_right_filter_label("Filter: *.rs".into());
    test.window.set_active_pane(0);
    let cleared = Rc::new(RefCell::new(Vec::new()));
    let output = cleared.clone();
    test.window
        .on_clear_panel_filter(move |pane| output.borrow_mut().push(pane));
    test.draw();
    ui::click_root(&test.window, "Clear panel filter", AccessibleRole::Button);
    assert_eq!(*cleared.borrow(), [1]);
}

#[test]
fn filter_clear_rejects_delayed_and_cancelled_native_presses() {
    let mut test = ui::TestUi::new();
    test.window.set_left_filter_label("Filter: *.rs".into());
    let cleared = Rc::new(RefCell::new(Vec::new()));
    let output = cleared.clone();
    test.window
        .on_clear_panel_filter(move |pane| output.borrow_mut().push(pane));
    test.draw();
    let label = "Clear panel filter";
    let point = ui::root_position(&test.window, label, AccessibleRole::Button);

    native_touch(&test.window, point, TouchPhase::Started);
    test.window.set_left_filter_label("Filter: *.txt".into());
    native_touch(&test.window, point, TouchPhase::Ended);
    assert!(
        cleared.borrow().is_empty(),
        "a changed filter invalidates the press"
    );

    native_touch(&test.window, point, TouchPhase::Started);
    test.window.set_left_search_context("new directory".into());
    native_touch(&test.window, point, TouchPhase::Ended);
    assert!(
        cleared.borrow().is_empty(),
        "navigation invalidates the press"
    );

    native_touch(&test.window, point, TouchPhase::Started);
    native_touch(&test.window, point, TouchPhase::Cancelled);
    assert!(
        cleared.borrow().is_empty(),
        "cancelled input must not clear a filter"
    );

    native_touch(&test.window, point, TouchPhase::Started);
    native_touch(&test.window, point, TouchPhase::Ended);
    assert_eq!(
        *cleared.borrow(),
        [0],
        "a new valid press still clears the filter"
    );
}

#[test]
fn panel_picker_rejects_a_delayed_or_cancelled_press_after_navigation() {
    let mut test = ui::TestUi::new();
    let label = "Left panel sort";
    let point = ui::root_position(&test.window, label, AccessibleRole::Combobox);
    native_touch(&test.window, point, TouchPhase::Started);
    test.window.set_left_search_context("new directory".into());
    native_touch(&test.window, point, TouchPhase::Ended);
    test.draw();
    assert!(!ui::has_popup(&test.window));

    native_touch(&test.window, point, TouchPhase::Started);
    native_touch(&test.window, point, TouchPhase::Cancelled);
    test.draw();
    assert!(!ui::has_popup(&test.window));

    native_touch(&test.window, point, TouchPhase::Started);
    native_touch(&test.window, point, TouchPhase::Ended);
    test.draw();
    assert!(
        ui::has_popup(&test.window),
        "the next valid press still works"
    );
}

#[test]
fn one_shared_checksum_and_comparison_action_follow_the_active_panel() {
    let mut test = ui::TestUi::new();
    test.window.set_left_can_target_selection(true);
    test.window.set_right_can_target_selection(true);
    let actions = Rc::new(RefCell::new(Vec::new()));
    let output = actions.clone();
    test.window
        .on_calculate_checksums(move |pane| output.borrow_mut().push(("checksum", pane)));
    let output = actions.clone();
    test.window
        .on_compare_files(move |pane| output.borrow_mut().push(("compare", pane)));
    for pane in [0, 1] {
        test.window.set_active_pane(pane);
        test.draw();
        let root = ItemRc::new_root(WindowInner::from_pub(test.window.window()).component());
        assert_eq!(count(root.clone(), "Checksums", AccessibleRole::Button), 1);
        assert_eq!(
            count(root.clone(), "Compare files", AccessibleRole::Button),
            1
        );
        assert_eq!(count(root, "Hot Paths", AccessibleRole::Combobox), 1);
        ui::click_root(&test.window, "Checksums", AccessibleRole::Button);
        ui::click_root(&test.window, "Compare files", AccessibleRole::Button);
        assert_eq!(
            &actions.borrow()[actions.borrow().len() - 2..],
            &[("checksum", pane), ("compare", pane)]
        );
    }
}

#[test]
fn comparison_can_choose_files_without_any_panel_target() {
    let mut test = ui::TestUi::new();
    test.window.set_left_can_target_selection(false);
    test.window.set_right_can_target_selection(false);
    let actions = Rc::new(RefCell::new(Vec::new()));
    let output = actions.clone();
    test.window
        .on_compare_files(move |pane| output.borrow_mut().push(pane));
    for pane in [0, 1] {
        test.window.set_active_pane(pane);
        test.draw();
        ui::click_root(&test.window, "Compare files", AccessibleRole::Button);
    }
    assert_eq!(*actions.borrow(), [0, 1]);
}

#[test]
fn panel_sort_keeps_its_origin_and_navigation_invalidates_its_popup() {
    let mut test = ui::TestUi::new();
    let sorts = Rc::new(RefCell::new(Vec::new()));
    let output = sorts.clone();
    test.window
        .on_sort(move |pane, column| output.borrow_mut().push((pane, column)));
    test.window.set_history_revision(12);
    test.window.set_left_sort(0);
    test.window.set_right_sort(0);
    ui::click_root(&test.window, "Left panel sort", AccessibleRole::Combobox);
    test.draw();
    assert!(ui::has_popup(&test.window));
    test.window.set_left_search_context("new directory".into());
    test.draw();
    assert!(
        !ui::has_popup(&test.window),
        "an open sort popup must not adopt a new directory"
    );
    assert!(sorts.borrow().is_empty());
    ui::click_root(&test.window, "Right panel sort", AccessibleRole::Combobox);
    test.draw();
    ui::click_popup(&test.window, "Size", AccessibleRole::Button);
    assert_eq!(*sorts.borrow(), [(1, 1)]);
}

#[test]
fn hot_path_choice_cannot_switch_panels_before_deferred_change_handlers_run() {
    let mut test = ui::TestUi::new();
    test.window.set_hot_path_choices(
        Rc::new(slint::VecModel::from(vec![NavigationChoiceRow {
            identity: "owned-hot-path".into(),
            label: "Saved folder".into(),
            ..Default::default()
        }]))
        .into(),
    );
    test.window
        .set_left_search_context("same numeric generation".into());
    test.window
        .set_right_search_context("same numeric generation".into());
    ui::click_root(&test.window, "Hot Paths", AccessibleRole::Combobox);
    test.draw();
    assert_eq!(test.window.get_hot_path_popup_pane(), 0);
    test.window.set_active_pane(1);
    assert!(
        test.window.invoke_take_location_choice(1, 0).is_empty(),
        "opened choices cannot be transplanted to the new pane"
    );
    test.draw();
    assert!(!ui::has_popup(&test.window));
}

#[test]
fn checksum_properties_and_folder_sizes_keep_provider_and_selection_gates() {
    let mut test = ui::TestUi::new();
    let enabled = |label: &str| {
        ui::root_item(&test.window, label, AccessibleRole::Button)
            .accessible_string_property(AccessibleStringProperty::Enabled)
            .is_some_and(|text| text == "true")
    };
    assert!(
        enabled("Checksums"),
        "empty native panels can open manifest verification"
    );
    assert!(!enabled("Properties"));
    assert!(enabled("Folder sizes"), "no selection measures all folders");
    test.window.set_left_can_target_selection(true);
    assert!(enabled("Checksums") && enabled("Properties"));
    test.window.set_left_native_filesystem(false);
    assert!(!enabled("Checksums") && !enabled("Properties") && !enabled("Folder sizes"));
    test.window.set_left_can_measure_folders(true);
    assert!(
        enabled("Folder sizes"),
        "archive metadata sums are available"
    );
    assert!(!enabled("Checksums") && !enabled("Properties"));
    test.window.set_active_pane(1);
    assert!(enabled("Folder sizes"));
    assert!(enabled("Checksums"));
    test.draw();
}

#[test]
fn archive_folder_size_button_targets_active_panel_without_enabling_mutation() {
    let mut test = ui::TestUi::new();
    test.window.set_active_pane(1);
    test.window.set_right_native_filesystem(false);
    test.window.set_right_can_measure_folders(true);
    test.window.set_right_can_target_selection(true);
    let actions = Rc::new(RefCell::new(Vec::new()));
    let captured = actions.clone();
    test.window
        .on_folder_sizes(move |pane| captured.borrow_mut().push(pane));
    toolbar::reveal(&mut test, "Folder sizes", AccessibleRole::Button);
    ui::click_root(&test.window, "Folder sizes", AccessibleRole::Button);
    assert_eq!(*actions.borrow(), [1]);
    test.window.invoke_show_app_menu(2);
    test.draw();
    for label in ["Delete permanently…", "Move to Trash…", "Rename…"] {
        let enabled = ui::popup_item(&test.window, label, AccessibleRole::Button)
            .accessible_string_property(AccessibleStringProperty::Enabled)
            .is_some_and(|text| text == "true");
        assert!(!enabled, "archive size support cannot enable {label}");
    }
    let sizes = ui::popup_item(
        &test.window,
        "Calculate / cancel folder sizes",
        AccessibleRole::Button,
    );
    assert!(
        sizes
            .accessible_string_property(AccessibleStringProperty::Enabled)
            .is_some_and(|text| text == "true")
    );
}

#[test]
fn tiny_window_keeps_one_file_row_and_custom_commands_reachable_with_large_fonts() {
    let mut test = ui::TestUi::new();
    let rows = Rc::new(slint::VecModel::from(vec![FileRow {
        name: "owned fixture.txt".into(),
        identity: "fixture".into(),
        ..Default::default()
    }]));
    test.window.set_left_rows(rows.clone().into());
    test.window.set_right_rows(rows.into());
    test.window.set_left_can_target_selection(true);
    test.window.set_right_can_target_selection(true);
    test.window.set_left_filter_label("Filter: *.txt".into());
    test.window.set_user_commands(
        Rc::new(slint::VecModel::from(vec![UserCommandRow {
            identity: "fixture-command".into(),
            revision: "fixture-revision".into(),
            name: "Fixture command".into(),
            label: "Fixture".into(),
            enabled: true,
            ..Default::default()
        }]))
        .into(),
    );
    let executions = Rc::new(RefCell::new(Vec::new()));
    let output = executions.clone();
    test.window
        .on_execute_user_command(move |id, revision| output.borrow_mut().push((id, revision)));
    for density in [0, 1] {
        test.window.invoke_apply_preferences(0, density);
        test.window.invoke_apply_fonts("".into(), 32, 32);
        test.resize(320, 200);
        let root = WindowInner::from_pub(test.window.window()).component();
        let tree = i_slint_core::item_tree::ItemTreeRc::borrow_pin(&root);
        assert_eq!(
            tree.as_ref()
                .layout_info(i_slint_core::layout::Orientation::Horizontal)
                .min,
            320.0
        );
        assert_eq!(test.window.get_effective_minimum_height(), 200.0);
        assert!(
            test.window.get_left_list_visible_height()
                >= test.window.global::<Theme>().get_row_height(),
            "density {density}: {}",
            test.window.get_left_list_visible_height()
        );
        toolbar::reveal(&mut test, "Checksums", AccessibleRole::Button);
        toolbar::reveal(&mut test, "Custom commands", AccessibleRole::Button);
        ui::click_root(&test.window, "Custom commands", AccessibleRole::Button);
        test.draw();
        ui::click_popup(&test.window, "Fixture command", AccessibleRole::Button);
        assert_eq!(executions.borrow().len(), density as usize + 1);
        assert!(!ui::has_popup(&test.window));
        ui::click_root(&test.window, "Custom commands", AccessibleRole::Button);
        test.draw();
        test.window.set_active_pane(1);
        test.draw();
        assert!(
            !ui::has_popup(&test.window),
            "custom commands retain their originating pane"
        );
        test.window.set_active_pane(0);
        test.draw();
        render::snapshot(&test, &format!("shared-toolbar-tiny-{density}"));
    }
    test.window.invoke_apply_preferences(0, 0);
    test.window.invoke_apply_fonts("".into(), 11, 15);
    test.resize(1340, 820);
    let command = ui::root_position(&test.window, "Fixture command", AccessibleRole::Button);
    assert!(
        command.y > test.window.get_left_pane_top() + test.window.get_left_pane_height(),
        "normal custom-command row stays below the panels"
    );
    render::snapshot(&test, "shared-toolbar-wide");
}

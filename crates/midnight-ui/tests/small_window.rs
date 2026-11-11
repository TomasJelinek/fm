//! Actual control geometry and actions at the compact window minimum.
#[path = "support/render.rs"]
mod render;
#[path = "support/toolbar.rs"]
mod toolbar;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    accessibility::AccessibleStringProperty, item_tree::ItemRc, items::AccessibleRole,
    window::WindowInner,
};
use midnight_core::{
    Command, DirectoryIndex, Entry, EntryId, EntryKind, Event, PaneId, SelectionSnapshot,
    ShortcutBinding, UserPreferences,
};
use midnight_ui::{FileRow, OperationRow, Presenter, ShortcutHints, Theme, install, refresh};
use slint::{
    ComponentHandle, VecModel,
    platform::{Key, WindowEvent},
};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
};

fn label(item: &ItemRc) -> String {
    item.accessible_string_property(AccessibleStringProperty::Label)
        .map_or_else(String::new, |value| value.to_string())
}
fn controls(item: ItemRc, result: &mut Vec<ItemRc>) {
    if matches!(
        item.accessible_role(),
        AccessibleRole::Button | AccessibleRole::Combobox
    ) {
        result.push(item.clone());
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        controls(item, result);
    }
}
fn inside(test: &ui::TestUi, item: &ItemRc) {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    assert!(
        rect.width() > 0.0
            && rect.height() > 0.0
            && origin.x >= -0.5
            && origin.y >= -0.5
            && origin.x + rect.width() <= test.width as f32 + 0.5
            && origin.y + rect.height() <= test.height as f32 + 0.5,
        "{} outside {}x{}: {origin:?} {rect:?}",
        label(item),
        test.width,
        test.height
    );
}
fn main_geometry(test: &mut ui::TestUi) {
    for name in ["Left panel view mode", "Left panel sort", "Hot Paths"] {
        if name == "Hot Paths" {
            toolbar::reveal(test, name, AccessibleRole::Combobox);
        }
        inside(
            test,
            &ui::root_item(&test.window, name, AccessibleRole::Combobox),
        );
    }
    for name in [
        "Add current path to Hot Paths",
        "Options",
        "Jobs",
        "Panel splitter",
    ] {
        if name == "Add current path to Hot Paths" {
            toolbar::reveal(test, name, AccessibleRole::Button);
        }
        inside(
            test,
            &ui::root_item(&test.window, name, AccessibleRole::Button),
        );
    }
    let mut items = Vec::new();
    controls(
        ItemRc::new_root(WindowInner::from_pub(test.window.window()).component()),
        &mut items,
    );
    let toolbar: Vec<_> = items
        .iter()
        .filter(|item| {
            let rect = item.geometry();
            let origin = item.map_to_native_window(rect.origin);
            item.is_visible()
                && origin.y >= test.window.get_left_pane_top()
                && origin.y < test.window.get_left_path_top()
                && rect.height() <= test.window.global::<Theme>().get_control_height() + 1.0
        })
        .collect();
    for (index, item) in toolbar.iter().enumerate() {
        inside(test, item);
        let rect = item.geometry();
        let origin = item.map_to_native_window(rect.origin);
        for other in toolbar.iter().skip(index + 1) {
            let other_rect = other.geometry();
            let other_origin = other.map_to_native_window(other_rect.origin);
            let overlap_x = (origin.x + rect.width()).min(other_origin.x + other_rect.width())
                - origin.x.max(other_origin.x);
            let overlap_y = (origin.y + rect.height()).min(other_origin.y + other_rect.height())
                - origin.y.max(other_origin.y);
            assert!(
                overlap_x < 0.5 || overlap_y < 0.5,
                "Toolbar overlap: {} and {}",
                label(item),
                label(other)
            );
        }
    }
    assert!(
        test.window.get_left_list_visible_height()
            >= test.window.global::<Theme>().get_row_height()
    );
    assert!(
        test.window.get_right_list_visible_height()
            >= test.window.global::<Theme>().get_row_height()
    );
    assert!(test.window.get_command_input_width() >= 160.0);
}

#[test]
fn compact_mouse_window_keeps_toolbars_splitter_command_and_dialog_actions_reachable() {
    let mut test = ui::TestUi::new();
    test.window.set_network_available(true);
    let path = format!("C:\\文档\\{}\\tail", "e\u{301}🐈".repeat(60));
    test.window.set_left_path(path.clone().into());
    let rows = Rc::new(VecModel::from(vec![FileRow {
        name: "Very long filename 世界.txt".into(),
        identity: "one".into(),
        size: "1 234 567".into(),
        modified_date: "2026-11-07".into(),
        modified_time: "12:00".into(),
        ..Default::default()
    }]));
    test.window.set_left_rows(rows.clone().into());
    test.window.set_right_rows(rows.into());
    let sorts = Rc::new(RefCell::new(Vec::new()));
    let received = sorts.clone();
    test.window
        .on_sort(move |pane, column| received.borrow_mut().push((pane, column)));
    for width in [640, 800] {
        test.resize(width, 360);
        assert_eq!(test.window.get_effective_minimum_height(), 200.0);
        main_geometry(&mut test);
        assert_eq!(test.window.get_command_prompt(), format!("{path}>"));
        assert!(
            test.window.get_command_path_content_width() > test.window.get_command_path_width()
        );
        let split_before = test.window.get_left_pane_width();
        ui::click_root(&test.window, "Panel splitter", AccessibleRole::Button);
        ui::key(&test.window, Key::LeftArrow);
        test.draw();
        assert!(
            test.window.get_left_pane_width() < split_before,
            "Splitter must still adjust at {width}px"
        );
        ui::key(&test.window, Key::Return);
        test.draw();
        ui::click_root(&test.window, "Left panel sort", AccessibleRole::Combobox);
        test.draw();
        ui::click_popup(&test.window, "Size", AccessibleRole::Button);
        test.draw();
        assert_eq!(sorts.borrow().last(), Some(&(0, 1)));
        ui::click_root(&test.window, "Options", AccessibleRole::Button);
        test.draw();
        inside(
            &test,
            &ui::popup_item(&test.window, "Cancel", AccessibleRole::Button),
        );
        render::snapshot(&test, &format!("small-options-{width}"));
        ui::click_popup(&test.window, "Cancel", AccessibleRole::Button);
        test.draw();
        test.window.set_operations(
            Rc::new(VecModel::from(vec![OperationRow {
                identity: "one".into(),
                title: "Copy files".into(),
                detail: path.clone().into(),
                cancellable: true,
                attention: true,
                ..Default::default()
            }]))
            .into(),
        );
        ui::click_root(&test.window, "Jobs", AccessibleRole::Button);
        test.draw();
        for action in ["Copy details", "Cancel job", "Resolve", "Close"] {
            inside(
                &test,
                &ui::popup_item(&test.window, action, AccessibleRole::Button),
            );
        }
        ui::click_popup(&test.window, "Close", AccessibleRole::Button);
        test.draw();
        render::snapshot(&test, &format!("small-main-{width}"));
    }
}

#[test]
fn exact_320_by_200_minimum_survives_density_fonts_and_recovers_both_panels() {
    let mut test = ui::TestUi::new();
    test.window.set_operations(
        Rc::new(VecModel::from(vec![OperationRow {
            identity: "active".into(),
            title: "Copy files".into(),
            detail: "C:\\long Unicode 文档\\file.txt → D:\\destination\\file.txt".into(),
            cancellable: true,
            attention: true,
            active: true,
            determinate: true,
            progress: 0.6,
            ..Default::default()
        }]))
        .into(),
    );
    test.window.set_left_path("C:\\left".into());
    test.window.set_right_path("D:\\right".into());
    test.window.set_left_rows(
        Rc::new(VecModel::from(vec![FileRow {
            name: "left file.txt".into(),
            identity: "left".into(),
            ..Default::default()
        }]))
        .into(),
    );
    test.window.set_right_rows(
        Rc::new(VecModel::from(vec![FileRow {
            name: "right file.txt".into(),
            identity: "right".into(),
            ..Default::default()
        }]))
        .into(),
    );
    let window = test.window.as_weak();
    test.window
        .on_activate(move |pane| window.upgrade().unwrap().set_active_pane(pane));
    for (density, font) in [(0, 11), (1, 15), (0, 24), (1, 24), (0, 32), (1, 32)] {
        test.window.invoke_apply_fonts("".into(), font, font);
        test.window.invoke_apply_preferences(0, density);
        test.resize(320, 200);
        assert_eq!(
            test.window.window().size(),
            slint::PhysicalSize::new(320, 200)
        );
        assert_eq!(test.window.get_effective_minimum_height(), 200.0);
        assert!(test.window.get_compact_window());
        let component = WindowInner::from_pub(test.window.window()).component();
        let tree = i_slint_core::item_tree::ItemTreeRc::borrow_pin(&component);
        assert_eq!(
            tree.as_ref()
                .layout_info(i_slint_core::layout::Orientation::Horizontal)
                .min,
            320.0
        );
        assert_eq!(
            tree.as_ref()
                .layout_info(i_slint_core::layout::Orientation::Vertical)
                .min,
            200.0
        );
        assert_eq!(
            tree.as_ref()
                .layout_info(i_slint_core::layout::Orientation::Horizontal)
                .preferred,
            1340.0
        );
        assert_eq!(
            tree.as_ref()
                .layout_info(i_slint_core::layout::Orientation::Vertical)
                .preferred,
            820.0
        );
        for name in ["Options", "Jobs", "Activate right panel"] {
            inside(
                &test,
                &ui::root_item(&test.window, name, AccessibleRole::Button),
            );
        }
        assert!(
            test.window.get_left_list_visible_height()
                >= test.window.global::<Theme>().get_row_height()
        );
        assert!(test.window.get_left_pane_width() > 240.0);
        assert!(test.window.get_command_input_width() >= 50.0);
        inside(
            &test,
            &ui::root_item(&test.window, "left file.txt", AccessibleRole::ListItem),
        );
        assert!(test.window.get_left_list_top() < 100.0);
        assert!(
            test.window.get_left_list_top() + test.window.get_left_list_visible_height()
                <= test.window.get_left_pane_top() + test.window.get_left_pane_height() + 0.5
        );
        ui::click_root(&test.window, "Activate right panel", AccessibleRole::Button);
        test.draw();
        assert_eq!(test.window.get_active_pane(), 1);
        assert!(test.window.get_right_pane_width() > 240.0);
        inside(
            &test,
            &ui::root_item(&test.window, "right file.txt", AccessibleRole::ListItem),
        );
        ui::click_root(&test.window, "Activate left panel", AccessibleRole::Button);
        test.draw();
        assert_eq!(
            test.window.global::<Theme>().get_location_icon_size(),
            if density == 1 { 26.0 } else { 20.0 }
        );
        render::snapshot(&test, &format!("small-density-{density}-font-{font}"));
        ui::click_root(&test.window, "Options", AccessibleRole::Button);
        test.draw();
        inside(
            &test,
            &ui::popup_item(&test.window, "Cancel", AccessibleRole::Button),
        );
        render::snapshot(
            &test,
            &format!("small-options-density-{density}-font-{font}"),
        );
        ui::click_popup(&test.window, "Cancel", AccessibleRole::Button);
        test.draw();
        ui::click_root(&test.window, "Jobs", AccessibleRole::Button);
        test.draw();
        inside(
            &test,
            &ui::popup_item(&test.window, "Close", AccessibleRole::Button),
        );
        render::snapshot(&test, &format!("small-jobs-density-{density}-font-{font}"));
        ui::click_popup(&test.window, "Close", AccessibleRole::Button);
        test.draw();
        test.resize(1340, 820);
        assert!(!test.window.get_compact_window());
        main_geometry(&mut test);
    }
}

#[test]
fn all_function_slots_keep_captions_in_both_layers_after_remapping() {
    let mut test = ui::TestUi::new();
    let presenter = Rc::new(RefCell::new(Presenter::new("left".into(), "right".into())));
    install(&test.window, presenter.clone(), Rc::new(|_| true));
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
            shortcuts: vec![
                ShortcutBinding {
                    action: "files.attributes".into(),
                    keys: vec![],
                },
                ShortcutBinding {
                    action: "files.rename".into(),
                    keys: vec!["Ctrl+F2".into()],
                },
                ShortcutBinding {
                    action: "panel.zoom".into(),
                    keys: vec![],
                },
            ],
            ..Default::default()
        }))));
    refresh(&test.window, &presenter.borrow());
    test.window.set_left_can_target_cursor(true);
    test.window.set_left_can_target_selection(true);
    let actions = Rc::new(RefCell::new(Vec::new()));
    let received = actions.clone();
    test.window
        .on_function_action(move |action, pane| received.borrow_mut().push((action, pane)));
    for (width, height) in [(1340, 820), (320, 200)] {
        test.resize(width, height);
        for control in [false, true] {
            test.window.set_function_modifiers(i32::from(control));
            test.draw();
            let mut items = Vec::new();
            controls(
                ItemRc::new_root(WindowInner::from_pub(test.window.window()).component()),
                &mut items,
            );
            let keys: Vec<_> = items
                .into_iter()
                .filter(|item| {
                    let origin = item.map_to_native_window(item.geometry().origin);
                    origin.y >= height as f32 - 36.0
                        && !matches!(label(item).as_str(), "Options" | "Jobs")
                })
                .collect();
            assert_eq!(keys.len(), 12, "Every function slot remains present");
            assert!(
                keys[0].map_to_native_window(keys[0].geometry().origin).x <= 12.0,
                "The function strip must start at the left edge without centering under Jobs"
            );
            for (index, item) in keys.iter().enumerate() {
                assert!(!label(item).is_empty());
                let origin = item.map_to_native_window(item.geometry().origin);
                let jobs = ui::root_item(&test.window, "Jobs", AccessibleRole::Button);
                let jobs_left = jobs.map_to_native_window(jobs.geometry().origin).x;
                // Narrow windows intentionally clip cells in the horizontal strip.
                // Only fully exposed cells have rendered text to inspect.
                if origin.x >= 0.0 && origin.x + item.geometry().width() <= jobs_left - 4.0 {
                    assert!(
                        ui::visible_text_values(item.clone())
                            .iter()
                            .any(|text| !text.is_empty()),
                        "Blank visible caption at slot{} in Ctrl={control}: {}",
                        index + 1,
                        label(item)
                    );
                }
                if control {
                    let slot = test.window.global::<ShortcutHints>().invoke_function_key(
                        index as i32,
                        1,
                        false,
                    );
                    if slot.action.is_empty() {
                        assert_eq!(label(item), format!("Ctrl+F{} — unavailable", index + 1));
                    } else {
                        assert_eq!(label(item), format!("{} {}", slot.key, slot.label));
                    }
                }
            }
            if control {
                ui::click_root(&test.window, "Ctrl+F2 Rename", AccessibleRole::Button);
                assert_eq!(actions.borrow().last().unwrap().0, "files.rename");
                let before = actions.borrow().len();
                // Pan the strip to the final slots at the smallest width.
                test.window
                    .window()
                    .dispatch_event(WindowEvent::PointerScrolled {
                        position: slint::LogicalPosition::new(100.0, height as f32 - 15.0),
                        delta_x: -1200.0,
                        delta_y: 0.0,
                    });
                test.draw();
                let last = &keys[11];
                let last_origin = last.map_to_native_window(last.geometry().origin);
                let jobs = ui::root_item(&test.window, "Jobs", AccessibleRole::Button);
                let jobs_origin = jobs.map_to_native_window(jobs.geometry().origin);
                assert!(
                    last_origin.x + last.geometry().width() <= jobs_origin.x,
                    "Scrolled final key must remain clear of the fixed Jobs button"
                );
                inside(
                    &test,
                    &ui::root_item(
                        &test.window,
                        "Ctrl+F11 — unavailable",
                        AccessibleRole::Button,
                    ),
                );
                ui::click_root(
                    &test.window,
                    "Ctrl+F11 — unavailable",
                    AccessibleRole::Button,
                );
                assert_eq!(
                    actions.borrow().len(),
                    before,
                    "Unassigned Ctrl key must not run its normal action"
                );
                render::snapshot(&test, &format!("small-control-footer-{width}"));
                test.window
                    .window()
                    .dispatch_event(WindowEvent::PointerScrolled {
                        position: slint::LogicalPosition::new(100.0, height as f32 - 15.0),
                        delta_x: 1200.0,
                        delta_y: 0.0,
                    });
                test.draw();
            }
        }
    }
}

#[test]
fn footer_disables_parent_targets_but_allows_a_real_selection() {
    let mut test = ui::TestUi::new();
    let path = PathBuf::from(std::path::MAIN_SEPARATOR_STR).join("footer-fixture");
    let presenter = Rc::new(RefCell::new(Presenter::new(path.clone(), path.clone())));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let sink = commands.clone();
    install(
        &test.window,
        presenter.clone(),
        Rc::new(move |command| {
            sink.borrow_mut().push(command);
            true
        }),
    );
    let navigation = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::Navigate {
                pane: PaneId::Left,
                navigation,
                ..
            } => Some(*navigation),
            _ => None,
        })
        .unwrap();
    let entries = vec![Entry::new(
        EntryId(1),
        path.join("real.txt"),
        EntryKind::File,
    )];
    let index = Arc::new(DirectoryIndex::from_entries(&entries));
    presenter.borrow_mut().apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation,
        index: index.clone(),
        entries: entries.into(),
    });
    refresh(&test.window, &presenter.borrow());
    test.window.invoke_row_click(0, 0, false, false);
    let actions = Rc::new(RefCell::new(Vec::new()));
    let received = actions.clone();
    test.window
        .on_function_action(move |action, _| received.borrow_mut().push(action));
    let metadata = Rc::new(Cell::new(0));
    let received = metadata.clone();
    test.window
        .on_open_metadata(move |_, _| received.set(received.get() + 1));
    assert!(!test.window.get_left_can_target_cursor());
    assert!(!test.window.get_left_can_target_selection());
    test.draw();
    for caption in ["F2 Rename", "F3 View", "F4 Edit", "F5 Copy", "F8 Delete"] {
        ui::click_root(
            &test.window,
            &format!("{caption} — unavailable"),
            AccessibleRole::Button,
        );
    }
    assert!(actions.borrow().is_empty());
    ui::click_root(
        &test.window,
        "Attributes, times and case (Ctrl+F2)",
        AccessibleRole::Button,
    );
    assert_eq!(metadata.get(), 0);
    ui::root_item(&test.window, "F7 Create dir", AccessibleRole::Button);
    test.window.set_function_modifiers(1);
    test.draw();
    ui::root_item(
        &test.window,
        "Ctrl+F2 Attributes — unavailable",
        AccessibleRole::Button,
    );
    ui::root_item(
        &test.window,
        "Ctrl+F7 Case — unavailable",
        AccessibleRole::Button,
    );
    test.window.set_function_modifiers(0);
    test.window.invoke_row_click(0, 1, true, false);
    let revision = commands
        .borrow()
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::Selection(request) => Some(request.revision),
            _ => None,
        })
        .unwrap();
    presenter.borrow_mut().apply(Event::SelectionChanged {
        pane: PaneId::Left,
        navigation,
        revision,
        selection: Arc::new(SelectionSnapshot::from_bits(index, vec![1]).unwrap()),
        deferred: Default::default(),
    });
    refresh(&test.window, &presenter.borrow());
    test.window.invoke_row_click(0, 0, false, false);
    assert!(!test.window.get_left_can_target_cursor());
    assert!(test.window.get_left_can_target_selection());
    test.draw();
    ui::root_item(
        &test.window,
        "F2 Rename — unavailable",
        AccessibleRole::Button,
    );
    ui::click_root(
        &test.window,
        "Attributes, times and case (Ctrl+F2)",
        AccessibleRole::Button,
    );
    assert_eq!(metadata.get(), 1);
    ui::click_root(&test.window, "F5 Copy", AccessibleRole::Button);
    ui::click_root(&test.window, "F8 Delete", AccessibleRole::Button);
    assert_eq!(
        actions
            .borrow()
            .iter()
            .map(|value| value.as_str())
            .collect::<Vec<_>>(),
        ["files.copy-to", "files.trash"]
    );
    test.window.set_function_modifiers(1);
    test.draw();
    ui::root_item(&test.window, "Ctrl+F2 Attributes", AccessibleRole::Button);
    ui::root_item(&test.window, "Ctrl+F7 Case", AccessibleRole::Button);
    test.window.set_function_modifiers(0);
    test.window.set_left_native_filesystem(false);
    test.draw();
    ui::click_root(&test.window, "F5 Copy", AccessibleRole::Button);
    assert_eq!(
        actions.borrow().last().unwrap(),
        "files.copy-to",
        "Archive copy-out remains available"
    );
    test.window.set_left_native_filesystem(true);
    test.window.invoke_row_click(0, 1, false, false);
    test.draw();
    ui::click_root(&test.window, "F2 Rename", AccessibleRole::Button);
    assert_eq!(actions.borrow().last().unwrap(), "files.rename");
}

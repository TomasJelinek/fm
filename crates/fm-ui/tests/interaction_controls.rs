//! Commander gestures and geometry use the real Slint component tree.
#[path = "support/ui.rs"]
mod ui;

use fm_ui::{FileRow, MainWindow, NavigationChoiceRow, PathPartRow, TabRow};
use i_slint_core::{
    accessibility::{AccessibilityAction, AccessibleStringProperty},
    item_tree::ItemRc,
    items::AccessibleRole,
    window::WindowInner,
};
use slint::{
    ComponentHandle, Model, ModelRc, SharedString, VecModel,
    platform::{Key, PointerEventButton, WindowEvent},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use ui::{TestUi, chord, click, key};

#[test]
fn drive_menu_copies_the_other_folder_with_period_or_mouse_after_panel_swap() {
    use fm_core::{Command, PaneId};
    use fm_ui::{Presenter, install, refresh};
    use std::path::PathBuf;

    let mut ui = TestUi::new();
    let root = PathBuf::from(std::path::MAIN_SEPARATOR_STR);
    let left = root.join("left 文档");
    let right = root.join("right e\u{301} 👩🏽‍💻");
    let presenter = Rc::new(RefCell::new(Presenter::new(left, right.clone())));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let sent = commands.clone();
    install(
        &ui.window,
        presenter.clone(),
        Rc::new(move |command| {
            sent.borrow_mut().push(command);
            true
        }),
    );
    refresh(&ui.window, &presenter.borrow());
    ui.draw();
    let choices = ui.window.get_navigation_choices();
    assert!(
        choices.row_count() > 0,
        "the menu action exists before drive discovery"
    );
    assert_eq!(
        choices.row_data(choices.row_count() - 1).unwrap().label,
        ". As Other Panel"
    );

    ui.window.invoke_focus_files();
    chord(&ui.window, &[Key::Alt], Key::F1);
    key(&ui.window, ".");
    assert!(commands.borrow().iter().any(|command| matches!(command,
        Command::Navigate { pane: PaneId::Left, path, .. } if path == &right)));
    assert_eq!(ui.window.get_focused_pane(), 0);

    // The operation uses raw current paths and logical pane identities even
    // after Ctrl+U exchanges their visual positions.
    let next = root.join("new opposite 文档");
    ui.window.invoke_navigate(1, next.to_str().unwrap().into());
    commands.borrow_mut().clear();
    chord(&ui.window, &[Key::Control], "u");
    chord(&ui.window, &[Key::Alt], Key::F2);
    ui::click_popup(&ui.window, ". As Other Panel", AccessibleRole::Button);
    assert!(commands.borrow().iter().any(|command| matches!(command,
        Command::Navigate { pane: PaneId::Left, path, .. } if path == &next)));
    assert_eq!(ui.window.get_focused_pane(), 0);
}

fn named_items(window: &MainWindow, label: &str, role: AccessibleRole) -> Vec<ItemRc> {
    fn collect(item: ItemRc, label: &str, role: AccessibleRole, found: &mut Vec<ItemRc>) {
        if item.accessible_role() == role
            && item
                .accessible_string_property(AccessibleStringProperty::Label)
                .is_some_and(|value| value.as_str() == label)
        {
            found.push(item.clone());
        }
        let mut child = item.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            collect(item, label, role, found);
        }
    }
    let mut result = Vec::new();
    collect(
        ItemRc::new_root(WindowInner::from_pub(window.window()).component()),
        label,
        role,
        &mut result,
    );
    result
}

fn bounds(item: &ItemRc) -> (f32, f32, f32, f32) {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    (origin.x, origin.y, rect.size.width, rect.size.height)
}

fn tabs(count: usize, active: usize) -> ModelRc<TabRow> {
    ModelRc::new(VecModel::from(
        (0..count)
            .map(|index| TabRow {
                identity: format!("tab-{index}").into(),
                label: format!("Tab {index}").into(),
                path: format!("D:\\tabs\\{index}").into(),
                active: index == active,
            })
            .collect::<Vec<_>>(),
    ))
}

fn tab_item(window: &MainWindow, index: usize) -> ItemRc {
    ui::root_item(
        window,
        &format!("Tab {index}\nD:\\tabs\\{index}"),
        AccessibleRole::Button,
    )
}

fn path_parts(folder_prefix: &str) -> ModelRc<PathPartRow> {
    ModelRc::new(VecModel::from(vec![
        PathPartRow {
            identity: "root-token".into(),
            label: "C:\\".into(),
            prefix: "C:\\".into(),
        },
        PathPartRow {
            identity: "folder-token".into(),
            label: "文档 👩🏽‍💻".into(),
            prefix: folder_prefix.into(),
        },
    ]))
}

fn pointer_button(
    window: &MainWindow,
    position: slint::LogicalPosition,
    button: PointerEventButton,
    pressed: bool,
) {
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    window.window().dispatch_event(if pressed {
        WindowEvent::PointerPressed { position, button }
    } else {
        WindowEvent::PointerReleased { position, button }
    });
}

#[test]
fn commander_focus_and_panel_controls_preserve_the_active_content() {
    let mut ui = TestUi::new();
    ui.window.set_command_text("keep this draft".into());
    ui.window.invoke_focus_files();
    ui.draw();
    assert_eq!(ui.window.get_focused_pane(), 0);

    chord(&ui.window, &[Key::Control], Key::Tab);
    ui.draw();
    assert!(
        ui.window.get_command_focused(),
        "Ctrl+Tab must focus the command editor"
    );
    assert_eq!(ui.window.get_command_text(), "keep this draft");

    chord(&ui.window, &[Key::Control], Key::Tab);
    ui.draw();
    assert_eq!(ui.window.get_focused_pane(), 0);
}

#[test]
fn visual_swap_preserves_logical_panels_and_routes_side_shortcuts() {
    let mut ui = TestUi::new();
    ui.window.set_left_path("C:\\left 文档".into());
    ui.window.set_right_path("D:\\right 👩🏽‍💻".into());
    ui.window
        .set_left_rows(ModelRc::new(VecModel::from(vec![FileRow {
            identity: "left-entry".into(),
            name: "left.txt".into(),
            selected: true,
            ..Default::default()
        }])));
    ui.window
        .set_right_rows(ModelRc::new(VecModel::from(vec![FileRow {
            identity: "right-entry".into(),
            name: "right.txt".into(),
            selected: false,
            ..Default::default()
        }])));
    ui.window.set_left_tabs(tabs(2, 1));
    ui.window.set_right_tabs(tabs(1, 0));
    ui.window
        .set_navigation_choices(ModelRc::new(VecModel::from(vec![NavigationChoiceRow {
            identity: "drive-token".into(),
            label: "Drive C:".into(),
            compact_label: "C:".into(),
            ..Default::default()
        }])));
    ui.window.set_left_navigation_index(0);
    ui.window.set_right_navigation_index(0);
    let drives = Rc::new(RefCell::new(Vec::new()));
    let captured = drives.clone();
    ui.window
        .on_navigation_choice(move |pane, identity| captured.borrow_mut().push((pane, identity)));
    let inserted = Rc::new(RefCell::new(Vec::new()));
    let captured = inserted.clone();
    ui.window
        .on_insert_path(move |pane| captured.borrow_mut().push(pane));
    ui.window.set_active_pane(0);
    ui.window.invoke_focus_files();
    ui.draw();
    let left_x = ui.window.get_left_list_left();
    let right_x = ui.window.get_right_list_left();
    assert!(left_x < right_x);
    chord(&ui.window, &[Key::Control], "[");
    chord(&ui.window, &[Key::Control], "]");
    chord(&ui.window, &[Key::Control], "u");
    ui.draw();
    assert!(ui.window.get_panels_swapped());
    assert_eq!(ui.window.get_active_pane(), 0);
    assert_eq!(ui.window.get_focused_pane(), 0);
    assert!((ui.window.get_left_list_left() - right_x).abs() < 1.0);
    assert!((ui.window.get_right_list_left() - left_x).abs() < 1.0);
    assert_eq!(ui.window.get_command_directory(), "C:\\left 文档");
    assert_eq!(
        ui.window.get_left_rows().row_data(0).unwrap().identity,
        "left-entry"
    );
    assert!(ui.window.get_left_rows().row_data(0).unwrap().selected);
    assert_eq!(
        ui.window.get_right_rows().row_data(0).unwrap().identity,
        "right-entry"
    );
    assert!(!ui.window.get_right_rows().row_data(0).unwrap().selected);
    assert!(ui.window.get_left_tabs().row_data(1).unwrap().active);
    chord(&ui.window, &[Key::Control], "[");
    chord(&ui.window, &[Key::Control], "]");
    assert_eq!(*inserted.borrow(), [0, 1, 1, 0]);
    for shortcut in [Key::F1, Key::F2] {
        chord(&ui.window, &[Key::Alt], shortcut);
        assert!(ui::has_popup(&ui.window));
        key(&ui.window, Key::Return);
        assert!(!ui::has_popup(&ui.window));
    }
    assert_eq!(
        *drives.borrow(),
        [
            (1, SharedString::from("drive-token")),
            (0, SharedString::from("drive-token"))
        ]
    );
    chord(&ui.window, &[Key::Control], "u");
    ui.draw();
    assert!(!ui.window.get_panels_swapped());
    assert_eq!(ui.window.get_focused_pane(), 0);
    assert!((ui.window.get_left_list_left() - left_x).abs() < 1.0);
}

#[test]
fn splitter_drag_resizes_panels_preserves_ratio_and_double_click_centers() {
    let mut ui = TestUi::new();
    let start = ui::root_position(&ui.window, "Panel splitter", AccessibleRole::Button);
    let original_width = ui.window.get_left_list_width();
    pointer_button(&ui.window, start, PointerEventButton::Left, true);
    for delta in [60.0, 120.0] {
        ui.window
            .window()
            .dispatch_event(WindowEvent::PointerMoved {
                position: slint::LogicalPosition::new(start.x + delta, start.y),
            });
        ui.draw();
    }
    pointer_button(
        &ui.window,
        slint::LogicalPosition::new(start.x + 120.0, start.y),
        PointerEventButton::Left,
        false,
    );
    ui.draw();
    assert!((ui.window.get_left_list_width() - original_width - 120.0).abs() < 2.0);
    let ratio = ui.window.get_panel_ratio();
    assert!(ratio > 0.55 && ratio < 0.7);
    ui.resize(1600, 900);
    assert!((ui.window.get_panel_ratio() - ratio).abs() < 0.001);
    assert!(ui.window.get_left_list_width() > ui.window.get_right_list_width());
    let position = ui::root_position(&ui.window, "Panel splitter", AccessibleRole::Button);
    click(&ui.window, position.x, position.y);
    click(&ui.window, position.x, position.y);
    ui.draw();
    assert!((ui.window.get_panel_ratio() - 0.5).abs() < 0.001);
    assert!((ui.window.get_left_list_width() - ui.window.get_right_list_width()).abs() < 1.0);
}

#[test]
fn extreme_splitter_drag_keeps_modified_column_inside_both_panes_at_large_fonts() {
    let mut ui = TestUi::new();
    ui.window.set_sizes_in_bytes(true);
    for density in [0, 1] {
        ui.window.invoke_apply_preferences(0, density);
        for (desktop_font, touch_font) in [(11, 15), (32, 32)] {
            ui.window
                .invoke_apply_fonts("".into(), desktop_font, touch_font);
            ui.resize(1340, 1040);
            for direction in [-1.0, 1.0] {
                ui.window.set_panel_ratio(0.5);
                ui.draw();
                let start = ui::root_position(&ui.window, "Panel splitter", AccessibleRole::Button);
                let end = slint::LogicalPosition::new(start.x + direction * 1000.0, start.y);
                pointer_button(&ui.window, start, PointerEventButton::Left, true);
                ui.window
                    .window()
                    .dispatch_event(WindowEvent::PointerMoved { position: end });
                pointer_button(&ui.window, end, PointerEventButton::Left, false);
                ui.draw();
                assert!(
                    (ui.window.get_panel_ratio() - 0.5).abs() > 0.15,
                    "the drag must reach a narrow pane before checking its columns"
                );
                let mut headers: Vec<_> =
                    named_items(&ui.window, "Sort by Modified", AccessibleRole::Button)
                        .iter()
                        .map(bounds)
                        .collect();
                headers.sort_by(|left, right| left.0.total_cmp(&right.0));
                assert_eq!(headers.len(), 2);
                for ((x, _, width, _), (pane_x, pane_width)) in headers.into_iter().zip([
                    (
                        ui.window.get_left_list_left(),
                        ui.window.get_left_list_width(),
                    ),
                    (
                        ui.window.get_right_list_left(),
                        ui.window.get_right_list_width(),
                    ),
                ]) {
                    assert!(x >= pane_x - 1.0 && width > 0.0);
                    assert!(
                        x + width <= pane_x + pane_width + 1.0,
                        "Modified extends beyond pane: density={density}, fonts={desktop_font}/{touch_font}, direction={direction}, header={x}+{width}, pane={pane_x}+{pane_width}"
                    );
                }
            }
        }
    }
}

#[test]
fn tabs_shrink_uniformly_then_scroll_at_minimum_and_reveal_the_active_tab() {
    let mut ui = TestUi::new();
    let selected = Rc::new(RefCell::new(Vec::new()));
    let captured = selected.clone();
    ui.window
        .on_tab_select(move |pane, identity| captured.borrow_mut().push((pane, identity)));
    for density in [0, 1] {
        ui.window.invoke_apply_preferences(0, density);
        ui.window.set_left_tabs(tabs(5, 0));
        ui.resize(1840, 900);
        let preferred = bounds(&tab_item(&ui.window, 0)).2;
        ui.resize(1040, 900);
        let narrow = bounds(&tab_item(&ui.window, 0)).2;
        assert!(
            narrow < preferred - 2.0,
            "tabs should shrink before overflowing"
        );
        for index in 1..5 {
            assert!((bounds(&tab_item(&ui.window, index)).2 - narrow).abs() < 0.05);
        }
        assert!(named_items(&ui.window, "Scroll tabs right", AccessibleRole::Button).is_empty());
        ui.window.set_left_tabs(tabs(12, 0));
        ui.draw();
        let minimum = bounds(&tab_item(&ui.window, 0)).2;
        assert!(minimum < narrow);
        assert_eq!(
            named_items(&ui.window, "Scroll tabs left", AccessibleRole::Button).len(),
            1
        );
        assert_eq!(
            named_items(&ui.window, "Scroll tabs right", AccessibleRole::Button).len(),
            1
        );
        let first_x = bounds(&tab_item(&ui.window, 0)).0;
        ui::click_root(&ui.window, "Scroll tabs right", AccessibleRole::Button);
        ui.draw();
        assert!(bounds(&tab_item(&ui.window, 0)).0 < first_x - 2.0);
        ui::click_root(&ui.window, "Scroll tabs left", AccessibleRole::Button);
        ui.draw();
        assert!((bounds(&tab_item(&ui.window, 0)).0 - first_x).abs() < 1.0);
        assert!(
            selected.borrow().is_empty(),
            "scrolling tabs cannot activate them"
        );
        ui.window.set_left_tabs(tabs(16, 15));
        ui.draw();
        assert!((bounds(&tab_item(&ui.window, 0)).2 - minimum).abs() < 0.05);
        for width in [1040, 1340] {
            ui.resize(width, 900);
            let (x, _, width, _) = bounds(&tab_item(&ui.window, 15));
            let arrow = bounds(&ui::root_item(
                &ui.window,
                "Scroll tabs left",
                AccessibleRole::Button,
            ));
            assert!(x >= ui.window.get_left_list_left() - 2.0);
            assert!(
                x + width <= arrow.0 + 1.0,
                "active tab must remain fully visible"
            );
        }
    }
}

#[test]
fn breadcrumb_right_click_dispatches_prefix_identity_and_rejects_stale_gestures() {
    let mut ui = TestUi::new();
    ui.window.set_left_path("C:\\文档 👩🏽‍💻".into());
    ui.window.set_left_path_parts(path_parts("C:\\文档 👩🏽‍💻"));
    let copied = Rc::new(RefCell::new(Vec::new()));
    let captured = copied.clone();
    ui.window
        .on_path_copy(move |pane, identity| captured.borrow_mut().push((pane, identity)));
    let navigated = Rc::new(RefCell::new(Vec::new()));
    let captured = navigated.clone();
    ui.window
        .on_path_part(move |pane, identity| captured.borrow_mut().push((pane, identity)));
    let captured = navigated.clone();
    ui.window
        .on_navigate(move |pane, text| captured.borrow_mut().push((pane, text)));
    ui.draw();
    let position = ui::root_position(&ui.window, "Open C:\\文档 👩🏽‍💻", AccessibleRole::Button);
    pointer_button(&ui.window, position, PointerEventButton::Right, true);
    ui.draw();
    assert_eq!(ui.window.get_left_highlighted_path(), "C:\\文档 👩🏽‍💻");
    pointer_button(&ui.window, position, PointerEventButton::Right, false);
    assert_eq!(*copied.borrow(), [(0, SharedString::from("folder-token"))]);
    assert!(navigated.borrow().is_empty());
    pointer_button(&ui.window, position, PointerEventButton::Right, true);
    // Reusing the display identity still changes the breadcrumb generation.
    ui.window.set_left_path_parts(path_parts("C:\\replacement"));
    ui.draw();
    pointer_button(&ui.window, position, PointerEventButton::Right, false);
    assert_eq!(
        copied.borrow().len(),
        1,
        "held right-click cannot copy a replacement path"
    );
    assert!(navigated.borrow().is_empty());
    let root = ui::root_position(&ui.window, "Open C:\\", AccessibleRole::Button);
    pointer_button(&ui.window, root, PointerEventButton::Right, true);
    pointer_button(&ui.window, root, PointerEventButton::Right, false);
    assert_eq!(copied.borrow()[1], (0, SharedString::from("root-token")));
    click(&ui.window, root.x, root.y);
    assert_eq!(*navigated.borrow(), [(0, SharedString::from("root-token"))]);
}

#[test]
fn terminal_split_control_selects_shell_separately_and_launch_keeps_draft() {
    let mut ui = TestUi::new();
    ui.window.set_left_path("C:\\文档".into());
    ui.window
        .set_terminal_choices(ModelRc::new(VecModel::from(vec![
            "PowerShell".into(),
            "cmd".into(),
        ])));
    ui.window.set_windows_shells(true);
    ui.window.set_terminal_available(true);
    ui.window.set_terminal_ready(true);
    let choices = Rc::new(RefCell::new(Vec::new()));
    let captured = choices.clone();
    let weak = ui.window.as_weak();
    ui.window.on_terminal_changed(move |index| {
        captured.borrow_mut().push(index);
        weak.upgrade().unwrap().set_terminal_index(index);
    });
    let launched = Rc::new(Cell::new(0));
    let captured = launched.clone();
    ui.window
        .on_open_terminal(move || captured.set(captured.get() + 1));
    let submitted = Rc::new(RefCell::new(Vec::new()));
    let captured = submitted.clone();
    ui.window.on_submit_command(move |text| {
        captured.borrow_mut().push(text);
        true
    });
    ui.draw();
    ui::click_root(&ui.window, "Command shell", AccessibleRole::Combobox);
    assert!(ui::has_popup(&ui.window));
    ui::click_popup(&ui.window, "cmd", AccessibleRole::Button);
    ui.draw();
    assert_eq!(*choices.borrow(), [1]);
    assert_eq!(launched.get(), 0);
    assert!(submitted.borrow().is_empty());
    assert_eq!(ui.window.get_terminal_index(), 1);
    let draft = SharedString::from("echo 文档 & echo !literal!");
    ui.window
        .invoke_set_command(draft.clone(), draft.len() as i32);
    ui.draw();
    ui::click_root(
        &ui.window,
        "Open cmd\nWorking folder: C:\\文档",
        AccessibleRole::Button,
    );
    ui.draw();
    assert_eq!(launched.get(), 1);
    assert!(
        submitted.borrow().is_empty(),
        "terminal main button cannot submit the draft"
    );
    assert_eq!(ui.window.get_command_text(), draft);
    assert!(ui.window.get_command_focused());
    ui::root_item(&ui.window, "Command shell", AccessibleRole::Combobox)
        .accessible_action(&AccessibilityAction::Expand);
    key(&ui.window, Key::Return);
    assert_eq!(*choices.borrow(), [1, 1]);
    assert_eq!(launched.get(), 1, "keyboard shell selection cannot launch");
    assert!(submitted.borrow().is_empty());
    let main_button = ui::root_item(
        &ui.window,
        "Open cmd\nWorking folder: C:\\文档",
        AccessibleRole::Button,
    );
    main_button.accessible_action(&AccessibilityAction::Default);
    assert_eq!(launched.get(), 2);
    assert_eq!(ui.window.get_command_text(), draft);
    ui.window.set_terminal_ready(false);
    main_button.accessible_action(&AccessibilityAction::Default);
    assert_eq!(
        launched.get(),
        2,
        "pending terminal launch disables the main action"
    );
    ui.window.set_terminal_ready(true);
    key(&ui.window, Key::Return);
    assert_eq!(*submitted.borrow(), [draft]);
    assert_eq!(ui.window.get_command_text(), "");
    assert_eq!(launched.get(), 2);
}

#[test]
fn drive_picker_fits_caption_under_action_tools_and_type_header_is_absent() {
    let mut ui = TestUi::new();
    for density in [0, 1] {
        ui.window.invoke_apply_preferences(0, density);
        ui.window
            .set_navigation_choices(ModelRc::new(VecModel::from(vec![NavigationChoiceRow {
                identity: "drive-token".into(),
                label: "Drive C: with free-space information".into(),
                compact_label: "C:".into(),
                ..Default::default()
            }])));
        ui.window.set_left_navigation_index(0);
        ui.window.set_right_navigation_index(0);
        ui.draw();
        let short = bounds(&ui::root_item(
            &ui.window,
            "Drive or volume",
            AccessibleRole::Combobox,
        ));
        let tool = bounds(&ui::root_item(
            &ui.window,
            "Back (Alt+Left)",
            AccessibleRole::Button,
        ));
        assert!(
            tool.1 + tool.3 <= short.1 + 1.0,
            "action tools precede the drive row"
        );
        assert!(
            short.2 < 120.0,
            "short drive captions should use compact controls"
        );
        ui.window
            .set_navigation_choices(ModelRc::new(VecModel::from(vec![NavigationChoiceRow {
                identity: "mount-token".into(),
                label: "Long mounted directory".into(),
                compact_label: "A very long mounted directory with 文档".into(),
                ..Default::default()
            }])));
        ui.draw();
        let long = bounds(&ui::root_item(
            &ui.window,
            "Drive or volume",
            AccessibleRole::Combobox,
        ));
        assert!(long.2 > short.2 + 20.0);
        assert!(long.2 <= 190.0 + 0.5, "long labels must stay bounded");
        assert!(named_items(&ui.window, "Sort by Type", AccessibleRole::Button).is_empty());
        for label in ["Sort by Name", "Sort by Size", "Sort by Modified"] {
            assert_eq!(
                named_items(&ui.window, label, AccessibleRole::Button).len(),
                2
            );
        }
    }
}

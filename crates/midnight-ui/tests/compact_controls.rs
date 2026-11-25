//! Exercise compact controls through actual pointer and keyboard dispatch.
use crate::support::ui;

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::ItemRc,
    items::AccessibleRole,
    window::{WindowInner, WindowKind},
};
use midnight_ui::{LocationRow, MainWindow, NavigationChoiceRow};
use slint::{
    ComponentHandle, ModelRc, VecModel,
    platform::{Key, PointerEventButton, WindowEvent},
};
use std::{cell::Cell, rc::Rc, time::Duration};
// The shared helpers send keys the way the platform does (on Windows, with the
// Winit-shaped key metadata).
use ui::{click, key};

fn popup_position(window: &MainWindow, label: &str) -> slint::LogicalPosition {
    fn find(item: ItemRc, label: &str) -> Option<ItemRc> {
        if item.accessible_role() == AccessibleRole::Button
            && item
                .accessible_string_property(AccessibleStringProperty::Label)
                .is_some_and(|value| value.as_str() == label)
        {
            return Some(item);
        }
        let mut child = item.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            if let Some(found) = find(item, label) {
                return Some(found);
            }
        }
        None
    }
    let inner = WindowInner::from_pub(window.window());
    let popups = inner.active_popups();
    let popup = popups
        .iter()
        .rev()
        .find(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
        .expect("a drive picker is open");
    let item = find(ItemRc::new_root(popup.component.clone()), label)
        .unwrap_or_else(|| panic!("no drive row: {label}"));
    drop(popups);
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    slint::LogicalPosition::new(
        origin.x + rect.size.width / 2.0,
        origin.y + rect.size.height / 2.0,
    )
}

#[test]
fn icon_shell_picker_commits_only_on_enter_and_restores_editor_focus() {
    let (test, time) = ui::TestUi::with_virtual_clock();
    let window = test.window.clone_strong();
    let native = test.native.clone();
    window.set_terminal_available(true);
    window.set_terminal_choices(ModelRc::new(VecModel::from(vec![
        "PowerShell — PowerShell syntax".into(),
        "Command Prompt — cmd.exe syntax".into(),
    ])));
    window.set_terminal_index(0);
    let picked = Rc::new(Cell::new(-1));
    let selected = picked.clone();
    let allow_pick = Rc::new(Cell::new(true));
    let allowed = allow_pick.clone();
    let weak = window.as_weak();
    window.on_terminal_changed(move |index| {
        selected.set(index);
        if allowed.get() {
            weak.upgrade().unwrap().set_terminal_index(index);
        }
    });
    // Partial repaints build on the shared host's last full frame.
    let mut pixels = test.pixels.clone();
    let draw = |pixels: &mut [slint::Rgb8Pixel]| {
        native.draw_if_needed(|renderer| {
            renderer.render(pixels, 1340);
        });
    };
    draw(&mut pixels);
    window.set_left_locations(ModelRc::new(VecModel::from(vec![
        LocationRow {
            label: "Home folder with a long descriptive caption".into(),
            path: "/home/example".into(),
            tooltip: "Home folder with a long descriptive caption\n/home/example".into(),
            ..Default::default()
        },
        LocationRow {
            label: "D:\\".into(),
            compact_label: "D:".into(),
            is_drive: true,
            path: "D:\\".into(),
            tooltip: "D:\\ — Work 文档 · 420 GiB free".into(),
            ..Default::default()
        },
    ])));
    let location_picked = Rc::new(Cell::new(-1));
    let location_selected = location_picked.clone();
    window.on_location(move |pane, index| {
        assert_eq!(pane, 0);
        location_selected.set(index);
    });
    draw(&mut pixels);
    ui::click_root(
        &window,
        "D:\\ — Work 文档 · 420 GiB free",
        AccessibleRole::Button,
    );
    assert_eq!(
        location_picked.get(),
        1,
        "the compact drive target must sit next to the volume dropdown"
    );
    for density in [0, 1] {
        window.invoke_apply_preferences(0, density);
        draw(&mut pixels);
        if density == 1 {
            ui::click_root(
                &window,
                "Home folder with a long descriptive caption\n/home/example",
                AccessibleRole::Button,
            );
            assert_eq!(
                location_picked.get(),
                0,
                "Touch keeps a compact Home target beside the volume dropdown"
            );
            draw(&mut pixels);
        }
        ui::click_root(&window, "Command shell", AccessibleRole::Combobox);
        draw(&mut pixels);
        key(&window, Key::DownArrow);
        assert_eq!(picked.get(), -1, "browsing the popup must not change shell");
        key(&window, Key::Return);
        assert_eq!(picked.get(), 1, "the side arrow must open the shell picker");
        assert_eq!(window.get_terminal_index(), 1);
        assert!(window.get_command_focused());
        key(&window, "echo 文档");
        assert_eq!(window.get_command_text().as_str(), "echo 文档");
        window.invoke_set_command("".into(), 0);
        draw(&mut pixels);
        ui::click_root(&window, "Command shell", AccessibleRole::Combobox);
        draw(&mut pixels);
        key(&window, Key::Home);
        key(&window, Key::Escape);
        assert_eq!(
            window.get_terminal_index(),
            1,
            "Escape cancels the pending choice"
        );
        // A rejected callback keeps the previous selection when the picker reopens.
        allow_pick.set(false);
        ui::click_root(&window, "Command shell", AccessibleRole::Combobox);
        draw(&mut pixels);
        key(&window, Key::Home);
        key(&window, Key::Return);
        assert_eq!(picked.get(), 0);
        assert_eq!(window.get_terminal_index(), 1);
        ui::click_root(&window, "Command shell", AccessibleRole::Combobox);
        draw(&mut pixels);
        key(&window, Key::Return);
        assert_eq!(picked.get(), 1, "reopening must follow the accepted shell");
        allow_pick.set(true);
        picked.set(-1);
        window.set_terminal_index(0);
        draw(&mut pixels);
    }
    window.invoke_apply_preferences(0, 0);
    window.set_volumes(ModelRc::new(VecModel::from(vec![
        "C:\\ — System · 121 GiB free".into(),
        "D:\\ — Work 文档 · 420 GiB free".into(),
    ])));
    window.set_volume_short_labels(ModelRc::new(VecModel::from(vec![
        "C:\\".into(),
        "D:\\".into(),
    ])));
    window.set_left_volume(0);
    let volume_picked = Rc::new(Cell::new(-1));
    let volume_selected = volume_picked.clone();
    window.on_volume(move |pane, index| {
        assert_eq!(pane, 0);
        volume_selected.set(index);
    });
    draw(&mut pixels);
    ui::click_root(&window, "Drive or volume", AccessibleRole::Combobox);
    draw(&mut pixels);
    key(&window, Key::DownArrow);
    assert_eq!(volume_picked.get(), -1);
    key(&window, Key::Return);
    assert_eq!(
        volume_picked.get(),
        1,
        "compact drive menu must support keyboard selection"
    );
    volume_picked.set(-1);
    draw(&mut pixels);
    ui::click_root(&window, "Drive or volume", AccessibleRole::Combobox);
    draw(&mut pixels);
    let position = popup_position(&window, "C:\\ — System · 121 GiB free");
    let button = PointerEventButton::Left;
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    window
        .window()
        .dispatch_event(WindowEvent::PointerPressed { position, button });
    window.set_volumes(ModelRc::new(VecModel::from(vec![
        "D:\\ — Replacement drive".into(),
        "C:\\ — System".into(),
    ])));
    slint::platform::update_timers_and_animations();
    draw(&mut pixels);
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased { position, button });
    assert_eq!(
        volume_picked.get(),
        -1,
        "replacing drives during a press must cancel selection"
    );
    // Display strings can remain equal when distinct raw mount identities are
    // reordered. The host revision must also invalidate the pressed row.
    ui::click_root(&window, "Drive or volume", AccessibleRole::Combobox);
    draw(&mut pixels);
    let position = popup_position(&window, "D:\\ — Replacement drive");
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    window
        .window()
        .dispatch_event(WindowEvent::PointerPressed { position, button });
    window.set_volumes_revision(1);
    slint::platform::update_timers_and_animations();
    draw(&mut pixels);
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased { position, button });
    assert_eq!(
        volume_picked.get(),
        -1,
        "an identity revision must cancel a press even when all display labels are equal"
    );
    ui::click_root(&window, "Drive or volume", AccessibleRole::Combobox);
    draw(&mut pixels);
    ui::click_popup(&window, "D:\\ — Replacement drive", AccessibleRole::Button);
    assert_eq!(
        volume_picked.get(),
        0,
        "a fresh drive-row click still commits"
    );
    draw(&mut pixels);
    let tooltips = || {
        WindowInner::from_pub(window.window())
            .active_popups()
            .iter()
            .filter(|popup| matches!(popup.window_kind, WindowKind::ToolTip))
            .count()
    };
    // Finish the animations of the picker steps above before the hover baseline,
    // so the frame difference below can only come from the tooltip.
    time.set(time.get() + Duration::from_millis(1000));
    slint::platform::update_timers_and_animations();
    draw(&mut pixels);
    let position = ui::root_position(&window, "F2 Rename — unavailable", AccessibleRole::Button);
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    draw(&mut pixels);
    assert_eq!(tooltips(), 0, "the tooltip waits for its hover delay");
    let before_tip = pixels.clone();
    time.set(time.get() + Duration::from_millis(1000));
    slint::platform::update_timers_and_animations();
    draw(&mut pixels);
    assert_eq!(
        tooltips(),
        1,
        "a disabled F2 control must show a real hover tooltip"
    );
    let changed = before_tip
        .iter()
        .zip(&pixels)
        .filter(|(a, b)| a != b)
        .count();
    assert!(changed > 100, "the hover tooltip must be painted");
    let with_tip = pixels.clone();
    click(&window, position.x, position.y);
    assert_eq!(
        picked.get(),
        -1,
        "a disabled function must never select a shell"
    );
    window.window().dispatch_event(WindowEvent::PointerMoved {
        position: slint::LogicalPosition::new(0.0, 0.0),
    });
    draw(&mut pixels);
    assert_eq!(
        tooltips(),
        0,
        "leaving the control must dismiss its tooltip"
    );
    assert!(
        pixels != with_tip,
        "the dismissed tooltip must no longer be painted"
    );
    // Many drives stay inside a flexible inline strip rather than pushing
    // essential tools outside a narrow pane, including in the Touch profile.
    window.set_left_locations(ModelRc::new(VecModel::from(
        (0..32)
            .map(|index| LocationRow {
                label: format!("Very long drive caption 文档 {index}").into(),
                compact_label: format!("{index}:").into(),
                is_drive: true,
                tooltip: format!("Native drive {index} · full name and capacity").into(),
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    )));
    let action = Rc::new(Cell::new((-1, -1)));
    let captured = action.clone();
    window.on_action(move |pane, selected| captured.set((pane, selected)));
    window.set_hot_path_choices(ModelRc::new(VecModel::from(vec![NavigationChoiceRow {
        identity: "manage-hot-paths".into(),
        label: "Manage Hot Paths…".into(),
        ..Default::default()
    }])));
    native.set_size(slint::PhysicalSize::new(1040, 820));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(1040.0, 820.0),
    });
    for density in [0, 1] {
        window.invoke_apply_preferences(0, density);
        window.invoke_focus_files();
        draw(&mut pixels);
        // The 32 drives share the existing navigation rows: without any drive the
        // file list starts at exactly the same place (no extra drive row).
        let list_top = window.get_left_list_top();
        let drives = window.get_left_locations();
        window.set_left_locations(ModelRc::new(VecModel::<LocationRow>::default()));
        draw(&mut pixels);
        assert!(
            (window.get_left_list_top() - list_top).abs() < 0.5,
            "no extra drive row remains: list top {list_top} with drives, {} without",
            window.get_left_list_top()
        );
        window.set_left_locations(drives);
        draw(&mut pixels);
        ui::click_root(
            &window,
            "Refresh (Ctrl+R / Ctrl+F9)",
            AccessibleRole::Button,
        );
        assert_eq!(action.get(), (0, 3), "drives cannot obscure Refresh");
        ui::click_root(&window, "Hot Paths", AccessibleRole::Combobox);
        draw(&mut pixels);
        assert_eq!(window.get_focused_pane(), -1, "Bookmarks stays reachable");
        key(&window, Key::Escape);
        action.set((-1, -1));
    }
    window.hide().unwrap();
}

//! Panel ownership follows real pointer input without taking control focus.
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::items::AccessibleRole;
use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::{
    ComponentHandle, Model,
    platform::{Key, PointerEventButton, WindowEvent},
};
use std::{cell::RefCell, path::PathBuf, rc::Rc};

fn installed_ui() -> (ui::TestUi, Rc<RefCell<Presenter>>) {
    let mut ui = ui::TestUi::new();
    let presenter = Rc::new(RefCell::new(Presenter::new(
        PathBuf::from("_build/panel-activation/left"),
        PathBuf::from("_build/panel-activation/right"),
    )));
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(UserPreferences::default())));
    install(&ui.window, presenter.clone(), Rc::new(|_| true));
    refresh(&ui.window, &presenter.borrow());
    ui.window.invoke_focus_files();
    ui.draw();
    (ui, presenter)
}

#[test]
fn clicking_an_inactive_toolbar_activates_its_panel_and_keeps_the_menu_open() {
    let (mut ui, _) = installed_ui();
    ui::click_root(&ui.window, "Right panel sort", AccessibleRole::Combobox);
    ui.draw();
    assert_eq!(ui.window.get_active_pane(), 1);
    assert!(ui::has_popup(&ui.window), "the clicked picker remains open");
    ui::click_popup(&ui.window, "Size", AccessibleRole::Button);
    assert_eq!(ui.window.get_right_sort(), 1);
    ui.window.hide().unwrap();
}

#[test]
fn clicking_blank_panel_space_targets_the_next_keyboard_action() {
    let (mut ui, _) = installed_ui();
    let x = ui.window.get_right_list_left() + 30.0;
    let y = ui.window.get_right_list_top() + 40.0;
    ui::click(&ui.window, x, y);
    assert_eq!(ui.window.get_active_pane(), 1);
    assert_eq!(ui.window.get_focused_pane(), 1);
    ui::click(&ui.window, x, y);
    assert_eq!(ui.window.get_focused_pane(), 1);
    ui::key(&ui.window, "query");
    assert_eq!(ui.window.get_right_search_query(), "query");
    assert_eq!(ui.window.get_left_search_query(), "");
    ui::chord(&ui.window, &[Key::Control], "t");
    ui.draw();
    assert_eq!(ui.window.get_left_tabs().row_count(), 1);
    assert_eq!(ui.window.get_right_tabs().row_count(), 2);
    ui.window.hide().unwrap();
}

#[test]
fn clicking_an_existing_path_editor_preserves_its_draft_and_text_focus() {
    let (mut ui, _) = installed_ui();
    ui::click_root(&ui.window, "Edit folder path", AccessibleRole::Button);
    ui::chord(&ui.window, &[Key::Control], "a");
    ui::key(&ui.window, "draft 文档");
    assert_eq!(ui.window.get_left_path_draft(), "draft 文档");
    ui.window.invoke_activate(1);
    ui.draw();
    ui::click(
        &ui.window,
        ui.window.get_left_path_left() + ui.window.get_left_path_width() / 2.0,
        ui.window.get_left_path_top() + ui.window.get_left_path_control_height() / 2.0,
    );
    assert_eq!(ui.window.get_active_pane(), 0);
    assert_eq!(ui.window.get_focused_pane(), -1);
    assert_eq!(ui.window.get_left_path_draft(), "draft 文档");
    ui::key(&ui.window, "X");
    assert_eq!(ui.window.get_left_path_draft(), "draft 文档X");
    ui.window.hide().unwrap();
}

#[test]
fn pointer_activation_is_once_per_change_and_includes_other_mouse_buttons() {
    let (mut ui, _) = installed_ui();
    let activations = Rc::new(RefCell::new(Vec::new()));
    let captured = activations.clone();
    let weak = ui.window.as_weak();
    ui.window.on_activate(move |pane| {
        captured.borrow_mut().push(pane);
        weak.upgrade().unwrap().set_active_pane(pane);
    });
    for button in [
        PointerEventButton::Left,
        PointerEventButton::Right,
        PointerEventButton::Middle,
    ] {
        ui.window.set_active_pane(1);
        ui.draw();
        activations.borrow_mut().clear();
        let position = slint::LogicalPosition::new(
            ui.window.get_left_details_left() + 12.0,
            ui.window.get_left_details_top() + 6.0,
        );
        ui.window
            .window()
            .dispatch_event(WindowEvent::PointerPressed { position, button });
        ui.window
            .window()
            .dispatch_event(WindowEvent::PointerReleased { position, button });
        assert_eq!(ui.window.get_active_pane(), 0, "{button:?}");
        assert_eq!(activations.borrow().as_slice(), &[0], "{button:?}");
    }
    ui.window.set_active_pane(1);
    activations.borrow_mut().clear();
    ui::click_root(&ui.window, "Edit folder path", AccessibleRole::Button);
    assert_eq!(activations.borrow().as_slice(), &[0]);
    ui.window.hide().unwrap();
}

#[test]
fn popup_splitter_and_command_clicks_do_not_activate_underlying_panels() {
    let (mut ui, _) = installed_ui();
    ui.window.invoke_show_options();
    ui.draw();
    ui.window.invoke_activate(1);
    ui.options_topic(1);
    assert_eq!(ui.window.get_active_pane(), 1);
    ui::key(&ui.window, Key::Escape);
    ui.draw();
    assert!(!ui::has_popup(&ui.window));
    let splitter_x = (ui.window.get_left_list_left()
        + ui.window.get_left_list_width()
        + ui.window.get_right_list_left())
        / 2.0;
    ui::click(&ui.window, splitter_x, ui.window.get_left_list_top() + 40.0);
    assert_eq!(ui.window.get_active_pane(), 1);
    ui::click(&ui.window, 30.0, ui.height as f32 - 3.0);
    assert_eq!(ui.window.get_active_pane(), 1);
    let command = ui::root_item(&ui.window, "Command line", AccessibleRole::TextInput);
    let rect = command.geometry();
    let origin = command.map_to_native_window(rect.origin);
    ui::click(&ui.window, origin.x + 8.0, origin.y + rect.height() / 2.0);
    assert_eq!(ui.window.get_active_pane(), 1);
    assert!(ui.window.get_command_focused());
    ui::key(&ui.window, "echo");
    assert_eq!(ui.window.get_command_text(), "echo");
    ui.window.hide().unwrap();
}

#[test]
fn inactive_drive_view_and_hot_path_pickers_keep_their_own_input() {
    let (mut ui, _) = installed_ui();
    for label in ["Drive or volume", "Left panel view mode", "Hot Paths"] {
        ui.window.invoke_activate(1);
        ui.window.invoke_focus_files();
        ui.draw();
        ui::click_root(&ui.window, label, AccessibleRole::Combobox);
        ui.draw();
        assert_eq!(ui.window.get_active_pane(), 0, "{label}");
        assert!(ui::has_popup(&ui.window), "{label}");
        ui::key(&ui.window, Key::DownArrow);
        assert!(ui::has_popup(&ui.window), "{label} retains keyboard input");
        ui::key(&ui.window, Key::Escape);
        ui.draw();
        assert!(!ui::has_popup(&ui.window), "{label} closes normally");
    }
    ui.window.hide().unwrap();
}

#[test]
fn panel_ownership_follows_swapped_resized_and_zoomed_layouts() {
    let (mut ui, _) = installed_ui();
    for (width, swapped) in [(1340, false), (1040, true), (1180, false)] {
        ui.resize(width, 820);
        ui.window.set_panels_swapped(swapped);
        ui.window.set_panel_ratio(0.6);
        let pane_at_left = i32::from(swapped);
        ui.window.invoke_activate(1 - pane_at_left);
        ui.window.invoke_focus_files();
        ui.draw();
        let y = ui.window.get_left_list_top() + 80.0;
        ui.window
            .window()
            .dispatch_event(WindowEvent::PointerMoved {
                position: slint::LogicalPosition::new(30.0, y),
            });
        assert_eq!(ui.window.get_active_pane(), 1 - pane_at_left);
        ui::click(&ui.window, 30.0, y);
        assert_eq!(ui.window.get_active_pane(), pane_at_left);
        ui::click(&ui.window, width as f32 - 30.0, y);
        assert_eq!(ui.window.get_active_pane(), 1 - pane_at_left);
    }
    ui.window.invoke_activate(0);
    ui.window.set_panel_zoomed(true);
    ui.draw();
    ui::click(&ui.window, 600.0, ui.window.get_left_list_top() + 80.0);
    assert_eq!(ui.window.get_active_pane(), 0);
    ui::click_root(&ui.window, "Activate right panel", AccessibleRole::Button);
    assert_eq!(ui.window.get_active_pane(), 1);
    assert!(ui.window.get_panel_zoomed());
    ui.window.hide().unwrap();
}

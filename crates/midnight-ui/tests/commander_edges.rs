//! Mouse capture, configurable Commander actions and zoom retain logical pane identity.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::items::AccessibleRole;
use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::{
    ComponentHandle, Model,
    platform::{Key, PointerEventButton, WindowEvent},
};
use std::{cell::RefCell, rc::Rc};

fn control(test: &ui::TestUi, pressed: bool) {
    test.window.window().dispatch_event(if pressed {
        WindowEvent::KeyPressed {
            text: Key::Control.into(),
        }
    } else {
        WindowEvent::KeyReleased {
            text: Key::Control.into(),
        }
    });
}

#[test]
fn control_footer_and_zoom_fit_touch_density_with_large_fonts() {
    let mut test = ui::TestUi::new();
    // This test isolates geometry; parent/selection capabilities have their own integration test.
    test.window.set_left_can_target_cursor(true);
    test.window.set_left_can_target_selection(true);
    test.resize(1040, 720);
    test.window.invoke_apply_preferences(3, 1);
    test.window.invoke_apply_fonts("".into(), 32, 32);
    test.window.invoke_focus_files();
    control(&test, true);
    test.draw();
    for label in [
        "Ctrl+F1 Drive info",
        "Ctrl+F2 Attributes",
        "Ctrl+F7 Case",
        "Ctrl+F10 Compare",
        "Ctrl+F11 Zoom",
        "Ctrl+F12 Filter",
        "Options",
        "Jobs",
    ] {
        let item = ui::root_item(&test.window, label, AccessibleRole::Button);
        let geometry = item.geometry();
        let origin = item.map_to_native_window(geometry.origin);
        assert!(
            origin.x >= 0.0 && origin.x + geometry.width() <= test.width as f32,
            "{label}: {origin:?} {geometry:?}"
        );
        assert!(origin.y >= 0.0 && origin.y + geometry.height() <= test.height as f32);
    }
    render::snapshot(&test, "commander-control-touch-font32");
    control(&test, false);
    test.window.invoke_toggle_panel_zoom(0);
    test.draw();
    assert!(test.window.get_left_list_width() > 940.0);
    ui::root_item(&test.window, "Activate right panel", AccessibleRole::Button);
    render::snapshot(&test, "commander-zoom-touch-font32");
}

#[test]
fn attributes_button_and_sort_stay_inside_narrow_panel_toolbars() {
    let mut test = ui::TestUi::new();
    for width in [1040, 1340] {
        test.resize(width, 820);
        for density in [0, 1] {
            for ratio in [0.3, 0.5, 0.7] {
                test.window.invoke_apply_preferences(0, density);
                test.window.set_panel_ratio(ratio);
                test.draw();
                let left = test.window.get_left_list_left();
                let right = left + test.window.get_left_list_width();
                for (label, role) in [
                    (
                        "Attributes, times and case (Ctrl+F2)",
                        AccessibleRole::Button,
                    ),
                    ("Left panel sort", AccessibleRole::Combobox),
                ] {
                    let item = ui::root_item(&test.window, label, role);
                    let geometry = item.geometry();
                    let origin = item.map_to_native_window(geometry.origin);
                    assert!(
                        origin.x >= left - 1.0 && origin.x + geometry.width() <= right + 1.0,
                        "{label}: width={width}, density={density}, ratio={ratio}, pane={left}..{right}, control={origin:?} {geometry:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn footer_mouse_release_keeps_the_pressed_action_and_logical_pane() {
    let mut test = ui::TestUi::new();
    test.window.set_left_can_target_cursor(true);
    test.window.set_left_can_target_selection(true);
    test.window.set_right_can_target_cursor(true);
    test.window.set_right_can_target_selection(true);
    let weak = test.window.as_weak();
    test.window
        .on_activate(move |pane| weak.upgrade().unwrap().set_active_pane(pane));
    let actions = Rc::new(RefCell::new(Vec::new()));
    let captured = actions.clone();
    test.window.on_open_metadata(move |pane, case| {
        captured.borrow_mut().push((pane, if case { 7 } else { 2 }))
    });
    let captured = actions.clone();
    test.window
        .on_sort(move |pane, _| captured.borrow_mut().push((pane, 3)));
    let captured = actions.clone();
    test.window
        .on_mutation(move |pane, action| captured.borrow_mut().push((pane, 100 + action)));
    for (width, height) in [(1340, 820), (320, 200)] {
        test.resize(width, height);
        for (label, expected) in [
            ("Ctrl+F2 Attributes", 2),
            ("Ctrl+F3 Name", 3),
            ("Ctrl+F6 Size", 3),
            ("Ctrl+F7 Case", 7),
        ] {
            for pane in [0, 1] {
                test.window.set_active_pane(pane);
                test.window.invoke_focus_files();
                control(&test, true);
                let position = reveal_footer(&mut test, label);
                let before_press = actions.borrow().len();
                test.window
                    .window()
                    .dispatch_event(WindowEvent::PointerPressed {
                        position,
                        button: PointerEventButton::Left,
                    });
                assert_eq!(
                    actions.borrow().len(),
                    before_press,
                    "Press must not dispatch {label}"
                );
                assert!(
                    test.window.get_control_layer(),
                    "Pointer press cleared Ctrl for {label}"
                );
                control(&test, false);
                test.window.set_active_pane(1 - pane);
                test.draw();
                test.window
                    .window()
                    .dispatch_event(WindowEvent::PointerReleased {
                        position,
                        button: PointerEventButton::Left,
                    });
                assert_eq!(actions.borrow().last(), Some(&(pane, expected)), "{label}");
                if expected == 3 {
                    assert_eq!(test.window.get_focused_pane(), pane);
                }
            }
        }
        // The normal layer must also survive switching to Ctrl before release.
        test.window.set_active_pane(0);
        control(&test, false);
        let position = reveal_footer(&mut test, "F2 Rename");
        test.window
            .window()
            .dispatch_event(WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            });
        control(&test, true);
        test.window.set_active_pane(1);
        test.draw();
        test.window
            .window()
            .dispatch_event(WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            });
        assert_eq!(actions.borrow().last(), Some(&(0, 100)));

        // Disabled Move cannot turn into the enabled Ctrl+F6 sort during a press.
        control(&test, false);
        let position = reveal_footer(&mut test, "Move — unavailable");
        let before = actions.borrow().len();
        test.window
            .window()
            .dispatch_event(WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            });
        control(&test, true);
        test.draw();
        test.window
            .window()
            .dispatch_event(WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            });
        assert_eq!(
            actions.borrow().len(),
            before,
            "disabled press at {width}px"
        );

        // A drag that scrolls the compact footer must not activate its starting cell.
        let position = reveal_footer(&mut test, "Ctrl+F2 Attributes");
        test.window
            .window()
            .dispatch_event(WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            });
        let outside = if width == 320 {
            slint::LogicalPosition::new(position.x - 50.0, position.y)
        } else {
            slint::LogicalPosition::new(position.x + 90.0, position.y - 60.0)
        };
        test.window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position: outside });
        test.window
            .window()
            .dispatch_event(WindowEvent::PointerReleased {
                position: outside,
                button: PointerEventButton::Left,
            });
        assert_eq!(
            actions.borrow().len(),
            before,
            "cancelled press at {width}px"
        );
    }
    assert_eq!(actions.borrow().len(), 18);
}

fn reveal_footer(test: &mut ui::TestUi, label: &str) -> slint::LogicalPosition {
    let viewport = slint::LogicalPosition::new(100.0, test.height as f32 - 15.0);
    test.window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: viewport,
            delta_x: 2000.0,
            delta_y: 0.0,
        });
    test.draw();
    let mut position = ui::root_position(&test.window, label, AccessibleRole::Button);
    if test.width < 640 && position.x > 190.0 {
        test.window
            .window()
            .dispatch_event(WindowEvent::PointerScrolled {
                position: viewport,
                delta_x: 150.0 - position.x,
                delta_y: 0.0,
            });
        test.draw();
        position = ui::root_position(&test.window, label, AccessibleRole::Button);
    }
    assert!(
        position.x >= 0.0 && position.x < test.width as f32 - 80.0,
        "{label}: {position:?}"
    );
    position
}

#[test]
fn control_layer_clears_on_release_and_excludes_alt_shift_meta_and_altgr() {
    let mut test = ui::TestUi::new();
    test.window.invoke_focus_files();
    control(&test, true);
    assert!(test.window.get_control_layer());
    for modifier in [Key::Alt, Key::Shift, Key::Meta, Key::AltGr] {
        test.window
            .window()
            .dispatch_event(WindowEvent::KeyPressed {
                text: modifier.into(),
            });
        assert!(!test.window.get_control_layer(), "{modifier:?}");
        test.window
            .window()
            .dispatch_event(WindowEvent::KeyReleased {
                text: modifier.into(),
            });
    }
    control(&test, false);
    test.draw();
    assert!(!test.window.get_control_layer());
    ui::root_item(&test.window, "F1 Help", AccessibleRole::Button);
}

#[test]
fn commander_dialog_keys_route_to_the_focused_logical_pane() {
    let mut test = ui::TestUi::new();
    let actions = Rc::new(RefCell::new(Vec::new()));
    let captured = actions.clone();
    test.window.on_open_metadata(move |pane, case| {
        captured.borrow_mut().push((pane, if case { 7 } else { 2 }))
    });
    let captured = actions.clone();
    test.window
        .on_open_panel_filter(move |pane| captured.borrow_mut().push((pane, 12)));
    let captured = actions.clone();
    test.window
        .on_open_comparison(move || captured.borrow_mut().push((-1, 10)));
    for swapped in [false, true] {
        test.window.set_panels_swapped(swapped);
        for pane in [0, 1] {
            test.window.set_active_pane(pane);
            test.window.invoke_focus_files();
            test.draw();
            for (key, action) in [(Key::F2, 2), (Key::F7, 7), (Key::F12, 12)] {
                ui::chord(&test.window, &[Key::Control], key);
                assert_eq!(actions.borrow().last(), Some(&(pane, action)));
            }
            ui::chord(&test.window, &[Key::Control], Key::F10);
            assert_eq!(actions.borrow().last(), Some(&(-1, 10)));
        }
    }
    assert_eq!(actions.borrow().len(), 16);
}

#[test]
fn zoom_follows_activation_and_swapping_without_changing_models_or_split() {
    let mut test = ui::TestUi::new();
    let weak = test.window.as_weak();
    test.window
        .on_activate(move |pane| weak.upgrade().unwrap().set_active_pane(pane));
    test.window.set_panel_ratio(0.6);
    test.window
        .set_left_rows(slint::ModelRc::new(slint::VecModel::from(vec![
            midnight_ui::FileRow {
                identity: "left-stable".into(),
                name: "left.txt".into(),
                ..Default::default()
            },
        ])));
    test.window
        .set_right_rows(slint::ModelRc::new(slint::VecModel::from(vec![
            midnight_ui::FileRow {
                identity: "right-stable".into(),
                name: "right.txt".into(),
                ..Default::default()
            },
        ])));
    let left_rows = test.window.get_left_rows();
    let right_rows = test.window.get_right_rows();
    test.window.invoke_focus_files();
    for swapped in [false, true] {
        test.window.set_panels_swapped(swapped);
        test.window.set_active_pane(0);
        test.window.invoke_focus_files();
        ui::chord(&test.window, &[Key::Control], Key::F11);
        test.draw();
        assert!(test.window.get_panel_zoomed());
        assert!(test.window.get_left_list_width() > 1200.0);
        let position = test.window.get_left_list_left();
        test.window.invoke_swap_panels();
        test.draw();
        assert_eq!(test.window.get_active_pane(), 0);
        assert!((test.window.get_left_list_left() - position).abs() > 10.0);
        assert!(test.window.get_left_list_width() > 1200.0);
        test.window.invoke_swap_panels();
        test.draw();
        ui::click_root(&test.window, "Activate right panel", AccessibleRole::Button);
        test.draw();
        assert_eq!(test.window.get_active_pane(), 1);
        assert!(test.window.get_right_list_width() > 1200.0);
        test.resize(1100, 780);
        assert!(test.window.get_right_list_width() > 1000.0);
        ui::chord(&test.window, &[Key::Control], Key::F11);
        test.draw();
        assert!(!test.window.get_panel_zoomed());
        assert!((test.window.get_panel_ratio() - 0.6).abs() < 0.001);
        assert_eq!(
            test.window.get_left_rows().row_count(),
            left_rows.row_count()
        );
        assert_eq!(
            test.window.get_right_rows().row_count(),
            right_rows.row_count()
        );
        assert_eq!(
            test.window.get_left_rows().row_data(0).unwrap().identity,
            "left-stable"
        );
        assert_eq!(
            test.window.get_right_rows().row_data(0).unwrap().identity,
            "right-stable"
        );
        test.resize(1340, 820);
    }
    test.window.invoke_toggle_panel_zoom(1);
    test.draw();
    ui::click_root(&test.window, "Panel splitter", AccessibleRole::Button);
    ui::key(&test.window, Key::Return);
    assert!(!test.window.get_panel_zoomed());
    assert!((test.window.get_panel_ratio() - 0.5).abs() < 0.001);
}

#[test]
fn resizing_the_zoomed_splitter_exits_zoom_and_tracks_the_pointer() {
    let mut test = ui::TestUi::new();
    test.window.set_panel_ratio(0.6);
    test.window.invoke_toggle_panel_zoom(0);
    test.draw();
    let start = ui::root_position(&test.window, "Panel splitter", AccessibleRole::Button);
    let destination = slint::LogicalPosition::new(530.0, start.y);
    test.window
        .window()
        .dispatch_event(WindowEvent::PointerPressed {
            position: start,
            button: PointerEventButton::Left,
        });
    test.window
        .window()
        .dispatch_event(WindowEvent::PointerMoved {
            position: destination,
        });
    test.draw();
    test.window
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position: destination,
            button: PointerEventButton::Left,
        });
    assert!(
        !test.window.get_panel_zoomed(),
        "resizing must leave the fixed zoom layout"
    );
    let final_position = ui::root_position(&test.window, "Panel splitter", AccessibleRole::Button);
    assert!(
        (final_position.x - destination.x).abs() <= 1.0,
        "splitter must track the pointer: {final_position:?}, wanted {destination:?}"
    );
}

fn presenter(test: &ui::TestUi, values: UserPreferences) -> Rc<RefCell<Presenter>> {
    let presenter = Rc::new(RefCell::new(Presenter::new("left".into(), "right".into())));
    install(&test.window, presenter.clone(), Rc::new(|_| true));
    presenter.borrow_mut().apply(Event::DesktopContext {
        shell: ShellFlavor::PowerShell,
        locations: vec![],
    });
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(values.into())));
    refresh(&test.window, &presenter.borrow());
    presenter
}

#[test]
fn reassigned_control_slot_dispatches_the_effective_action_and_disabled_slot_does_not_dispatch() {
    let mut test = ui::TestUi::new();
    let _presenter = presenter(
        &test,
        UserPreferences {
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
        },
    );
    test.window.set_left_can_target_cursor(true);
    test.window.set_left_can_target_selection(true);
    let actions = Rc::new(RefCell::new(Vec::new()));
    let captured = actions.clone();
    test.window
        .on_mutation(move |pane, action| captured.borrow_mut().push((pane, action)));
    test.window
        .on_open_metadata(|_, _| panic!("old binding is disabled"));
    test.window.invoke_focus_files();
    control(&test, true);
    test.draw();
    ui::click_root(&test.window, "Ctrl+F2 Rename", AccessibleRole::Button);
    assert_eq!(actions.borrow().as_slice(), &[(0, 0)]);
    control(&test, false);
    test.window.invoke_focus_files();
    ui::chord(&test.window, &[Key::Control], Key::F2);
    assert_eq!(actions.borrow().len(), 2);
    ui::chord(&test.window, &[Key::Control], Key::F11);
    assert!(!test.window.get_panel_zoomed());
}

#[test]
fn history_can_be_remapped_disabled_and_reset_by_replacing_the_draft() {
    let test = ui::TestUi::new();
    let _presenter = presenter(
        &test,
        UserPreferences {
            shortcuts: vec![
                ShortcutBinding {
                    action: "command.history-older".into(),
                    keys: vec!["Alt+PageUp".into()],
                },
                ShortcutBinding {
                    action: "command.history-newer".into(),
                    keys: vec![],
                },
            ],
            command_history: vec![
                CommandHistoryEntry {
                    command: "first".into(),
                    shell: ShellFlavor::Cmd,
                },
                CommandHistoryEntry {
                    command: "second".into(),
                    shell: ShellFlavor::Cmd,
                },
            ],
            ..Default::default()
        },
    );
    test.window.invoke_set_command("draft".into(), 5);
    ui::chord(&test.window, &[Key::Alt], Key::UpArrow);
    assert_eq!(test.window.get_command_text(), "draft");
    ui::chord(&test.window, &[Key::Alt], Key::PageUp);
    assert_eq!(test.window.get_command_text(), "first");
    ui::chord(&test.window, &[Key::Alt], Key::PageUp);
    assert_eq!(test.window.get_command_text(), "second");
    ui::chord(&test.window, &[Key::Alt], Key::DownArrow);
    assert_eq!(test.window.get_command_text(), "second");
    test.window.invoke_set_command("replacement".into(), 11);
    ui::chord(&test.window, &[Key::Alt], Key::PageUp);
    assert_eq!(test.window.get_command_text(), "first");
}

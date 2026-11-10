//! Exercise real splitter gestures, refreshes and the asynchronous preference boundary.
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::items::AccessibleRole;
use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::{
    ComponentHandle, LogicalPosition,
    platform::{Key, PointerEventButton, WindowEvent},
};
use std::{cell::RefCell, rc::Rc};

fn saved(commands: &[Command]) -> Vec<UserPreferences> {
    commands
        .iter()
        .filter_map(|command| match command {
            Command::SavePreferences { preferences, .. } => Some(*preferences.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn restored_split_stays_live_during_refresh_and_saves_only_finished_gestures() {
    let mut test = ui::TestUi::new();
    let p = Rc::new(RefCell::new(Presenter::new("left".into(), "right".into())));
    let sent = Rc::new(RefCell::new(Vec::new()));
    let captured = sent.clone();
    install(
        &test.window,
        p.clone(),
        Rc::new(move |command| {
            captured.borrow_mut().push(command);
            true
        }),
    );
    p.borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
            panel_split_basis_points: 6100,
            ..Default::default()
        }))));
    refresh(&test.window, &p.borrow());
    test.draw();
    assert!((test.window.get_panel_ratio() - 0.61).abs() < 0.0001);
    assert!(saved(&sent.borrow()).is_empty(), "restoring must not save");
    sent.borrow_mut().clear();

    let start = ui::root_position(&test.window, "Panel splitter", AccessibleRole::Button);
    ui::pointer_button(&test.window, start, PointerEventButton::Left, true);
    for delta in [-40.0, -80.0] {
        test.window
            .window()
            .dispatch_event(WindowEvent::PointerMoved {
                position: LogicalPosition::new(start.x + delta, start.y),
            });
        test.draw();
        let live = test.window.get_panel_ratio();
        assert!(live < 0.6);
        refresh(&test.window, &p.borrow());
        test.draw();
        assert_eq!(
            test.window.get_panel_ratio(),
            live,
            "refresh must not reset a drag"
        );
        assert!(
            saved(&sent.borrow()).is_empty(),
            "pointer moves must not write"
        );
    }
    ui::pointer_button(
        &test.window,
        LogicalPosition::new(start.x - 80.0, start.y),
        PointerEventButton::Left,
        false,
    );
    test.draw();
    let writes = saved(&sent.borrow());
    assert_eq!(writes.len(), 1, "one write per completed drag");
    let persisted = f32::from(writes[0].panel_split_basis_points) / 10_000.0;
    assert!((test.window.get_panel_ratio() - persisted).abs() < 0.0001);

    // A fresh presenter restores the committed value, independent of the window size.
    let mut reopened = Presenter::new("left".into(), "right".into());
    reopened.apply(Event::PreferencesLoaded(Ok(Box::new(writes[0].clone()))));
    test.window.set_panel_ratio(0.5);
    test.window.set_restored_panel_ratio(0.5);
    test.draw();
    test.resize(1600, 900);
    refresh(&test.window, &reopened);
    test.draw();
    assert!((test.window.get_panel_ratio() - persisted).abs() < 0.0001);
    sent.borrow_mut().clear();

    test.window.invoke_focus_files();
    ui::chord(&test.window, &[Key::Control], Key::F11);
    test.draw();
    assert!(test.window.get_panel_zoomed());
    assert!(
        saved(&sent.borrow()).is_empty(),
        "temporary zoom must not save a split"
    );
    ui::chord(&test.window, &[Key::Control], Key::F11);
    test.draw();
    assert!((test.window.get_panel_ratio() - persisted).abs() < 0.0001);

    let center = ui::root_position(&test.window, "Panel splitter", AccessibleRole::Button);
    ui::click(&test.window, center.x, center.y);
    ui::click(&test.window, center.x, center.y);
    test.draw();
    let writes = saved(&sent.borrow());
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].panel_split_basis_points, 5000);
    assert!((test.window.get_panel_ratio() - 0.5).abs() < 0.0001);
    sent.borrow_mut().clear();

    // Focus remains with the splitter after clicking; arrow adjustment is also saved.
    ui::key(&test.window, Key::LeftArrow);
    test.draw();
    let writes = saved(&sent.borrow());
    assert_eq!(writes.len(), 1);
    assert!(writes[0].panel_split_basis_points < 5000);
    test.window.hide().unwrap();
}

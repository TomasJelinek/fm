//! Modifier gestures are driven through the installed native event bridge.
#[path = "support/selection.rs"]
mod selection;
#[path = "support/ui.rs"]
mod ui;
use midnight_core::*;
use midnight_ui::{Presenter, Theme, install, refresh};
use slint::{
    ComponentHandle, Model,
    platform::{Key, PointerEventButton, WindowEvent},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};

fn modifier(window: &midnight_ui::MainWindow, key: Key, down: bool) {
    window.window().dispatch_event(if down {
        WindowEvent::KeyPressed { text: key.into() }
    } else {
        WindowEvent::KeyReleased { text: key.into() }
    });
}
fn click(ui: &mut ui::TestUi, row: usize, control: bool, shift: bool) {
    if control {
        modifier(&ui.window, Key::Control, true);
    }
    if shift {
        modifier(&ui.window, Key::Shift, true);
    }
    let position = slint::LogicalPosition::new(
        ui.window.get_left_list_left() + 35.0,
        ui.window.get_left_list_top()
            + (row as f32 + 0.5) * ui.window.global::<Theme>().get_row_height(),
    );
    for event in [
        WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        },
        WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        },
    ] {
        ui.window.window().dispatch_event(event);
    }
    if shift {
        modifier(&ui.window, Key::Shift, false);
    }
    if control {
        modifier(&ui.window, Key::Control, false);
    }
    ui.draw();
}
fn marked(ui: &ui::TestUi) -> Vec<usize> {
    (0..ui.window.get_left_rows().row_count())
        .filter(|row| ui.window.get_left_rows().row_data(*row).unwrap().selected)
        .collect()
}
#[test]
fn actual_modifier_clicks_preview_inclusive_ranges_and_reconcile_authoritative_counts() {
    let mut ui = ui::TestUi::new();
    let location = std::env::temp_dir().join("selection-gestures");
    let presenter = Rc::new(RefCell::new(Presenter::new(
        location.clone(),
        location.clone(),
    )));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let accepted = Rc::new(Cell::new(true));
    let capture = commands.clone();
    let accept = accepted.clone();
    install(
        &ui.window,
        presenter.clone(),
        Rc::new(move |cmd| {
            if !accept.get() {
                return false;
            }
            capture.borrow_mut().push(cmd);
            true
        }),
    );
    ui.window.invoke_action(0, 3);
    let navigation = commands
        .borrow()
        .iter()
        .rev()
        .find_map(|cmd| match cmd {
            Command::Navigate { navigation, .. } | Command::Refresh { navigation, .. } => {
                Some(*navigation)
            }
            _ => None,
        })
        .unwrap();
    let entries: Arc<[Entry]> = (0..6)
        .map(|row| {
            Entry::new(
                EntryId(row),
                location.join(format!("entry-{row}")),
                if row % 2 == 0 {
                    EntryKind::DirectoryLink
                } else {
                    EntryKind::File
                },
            )
        })
        .collect::<Vec<_>>()
        .into();
    presenter.borrow_mut().apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation,
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries: entries.clone(),
    });
    refresh(&ui.window, &presenter.borrow());
    ui.draw();
    click(&mut ui, 2, false, false);
    click(&mut ui, 4, false, true);
    assert_eq!(
        marked(&ui),
        [2, 3, 4],
        "Shift inclusive range is visible before any acknowledgement"
    );
    click(&mut ui, 6, true, true);
    assert_eq!(marked(&ui), [2, 3, 4, 5, 6]);
    click(&mut ui, 4, true, false);
    assert_eq!(marked(&ui), [2, 3, 5, 6]);
    accepted.set(false);
    click(&mut ui, 1, true, false);
    assert_eq!(
        marked(&ui),
        [2, 3, 5, 6],
        "queue refusal leaves displayed selection alone"
    );
    accepted.set(true);
    selection::acknowledge(
        &presenter,
        &commands,
        PaneId::Left,
        &[
            &entries[1].path,
            &entries[2].path,
            &entries[4].path,
            &entries[5].path,
        ],
    );
    refresh(&ui.window, &presenter.borrow());
    ui.draw();
    assert_eq!(marked(&ui), [2, 3, 5, 6]);
    assert!((0..ui.window.get_left_details().row_count()).any(|row| {
        ui.window.get_left_details().row_data(row).unwrap().value == "2 files, 2 folders"
    }));
    assert!(!ui.window.get_left_rows().row_data(0).unwrap().selected);
}

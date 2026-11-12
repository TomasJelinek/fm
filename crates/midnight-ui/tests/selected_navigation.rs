//! Native key dispatch through the installed presenter, preserving every selection mark.
#[path = "support/ui.rs"]
mod ui;

use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::{Model, platform::Key};
use std::{cell::RefCell, rc::Rc, sync::Arc};

#[test]
fn selected_navigation_real_keys_preserve_marks_in_all_layouts_and_respect_edit_focus() {
    let mut test = ui::TestUi::new();
    let base = std::env::temp_dir().join("selected navigation 文档");
    let presenter = Rc::new(RefCell::new(Presenter::new(
        base.clone(),
        base.join("right"),
    )));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let sent = commands.clone();
    install(
        &test.window,
        presenter.clone(),
        Rc::new(move |command| {
            sent.borrow_mut().push(command);
            true
        }),
    );
    let navigations: Vec<_> = commands
        .borrow()
        .iter()
        .filter_map(|command| match command {
            Command::Navigate {
                pane,
                navigation,
                path,
                ..
            } => Some((*pane, *navigation, path.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(navigations.len(), 2);
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::default())));
    for (pane, navigation, path) in navigations {
        let entries: Arc<[Entry]> = (0..8)
            .map(|row| {
                Entry::new(
                    EntryId(row + 1),
                    path.join(format!("{row} item")),
                    if row % 2 == 0 {
                        EntryKind::Directory
                    } else {
                        EntryKind::File
                    },
                )
            })
            .collect::<Vec<_>>()
            .into();
        presenter.borrow_mut().apply(Event::ListingFinished {
            pane,
            navigation,
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        });
        for row in [1, 4, 7] {
            presenter
                .borrow_mut()
                .select_row(pane, row, true, &|command| {
                    commands.borrow_mut().push(command);
                    true
                });
        }
        presenter.borrow_mut().select_row(pane, 1, false, &|_| true);
    }
    refresh(&test.window, &presenter.borrow());
    let cursors = || {
        [
            test.window.get_left_cursor(),
            test.window.get_right_cursor(),
        ]
    };
    for pane in [0, 1] {
        test.window.invoke_activate(pane);
        test.window.invoke_focus_files();
        for mode in [0, 1, 2] {
            test.window.invoke_view_mode(pane, mode);
            commands.borrow_mut().clear();
            for (key, expected) in [
                (Key::DownArrow, 4),
                (Key::DownArrow, 7),
                (Key::DownArrow, 7),
                (Key::UpArrow, 4),
                (Key::UpArrow, 1),
                (Key::UpArrow, 1),
            ] {
                let other = cursors()[1 - pane as usize];
                ui::chord(&test.window, &[Key::Alt], key);
                assert_eq!(
                    cursors()[pane as usize],
                    expected,
                    "pane {pane}, layout {mode}"
                );
                assert_eq!(cursors()[1 - pane as usize], other);
                let rows = if pane == 0 {
                    test.window.get_left_rows()
                } else {
                    test.window.get_right_rows()
                };
                let selected: Vec<_> = rows
                    .iter()
                    .enumerate()
                    .filter_map(|(row, value)| value.selected.then_some(row))
                    .collect();
                assert_eq!(selected, [1, 4, 7]);
            }
            assert!(commands.borrow().iter().all(|command| !matches!(
                command,
                Command::Selection(_) | Command::Navigate { .. } | Command::OpenEntry { .. }
            )));
        }
    }
    let before = cursors();
    test.window.invoke_set_command("untouched".into(), 9);
    for key in [Key::UpArrow, Key::DownArrow] {
        ui::chord(&test.window, &[Key::Alt], key);
    }
    assert_eq!(cursors(), before);
    assert_eq!(test.window.get_focused_pane(), -1);
    test.window.invoke_focus_files();
    ui::key(&test.window, "i");
    assert!(test.window.get_right_search_active());
    for key in [Key::UpArrow, Key::DownArrow] {
        ui::chord(&test.window, &[Key::Alt], key);
    }
    assert_eq!(cursors(), before);
    ui::key(&test.window, Key::Escape);
    test.window
        .invoke_keyboard_apply("panel.next-selected".into(), "Ctrl+Shift+J".into());
    assert!(test.window.get_keyboard_error().is_empty());
    test.window.invoke_focus_files();
    ui::chord(&test.window, &[Key::Alt], Key::DownArrow);
    assert_eq!(test.window.get_right_cursor(), 1);
    ui::chord(&test.window, &[Key::Control, Key::Shift], "j");
    assert_eq!(test.window.get_right_cursor(), 4);
    test.draw();
}

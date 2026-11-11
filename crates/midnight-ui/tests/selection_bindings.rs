//! Exercise the installed presenter bridge and explicit asynchronous worker results.
use midnight_core::*;
use midnight_ui::{MainWindow, Presenter, install, refresh};
use slint::{
    ComponentHandle, Model,
    platform::{
        Key, Platform, PlatformError, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{cell::RefCell, path::Path, rc::Rc, sync::Arc};
#[path = "support/selection.rs"]
mod selection;

struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

fn control(window: &MainWindow, text: &str) {
    chord(window, text, false);
}

fn chord(window: &MainWindow, text: &str, shift: bool) {
    if shift {
        window.window().dispatch_event(WindowEvent::KeyPressed {
            text: Key::Shift.into(),
        });
    }
    for event in [
        WindowEvent::KeyPressed {
            text: Key::Control.into(),
        },
        WindowEvent::KeyPressed { text: text.into() },
        WindowEvent::KeyReleased { text: text.into() },
        WindowEvent::KeyReleased {
            text: Key::Control.into(),
        },
    ] {
        window.window().dispatch_event(event);
    }
    if shift {
        window.window().dispatch_event(WindowEvent::KeyReleased {
            text: Key::Shift.into(),
        });
    }
}

#[test]
fn selection_shortcuts_use_the_backend_and_preserve_other_pane_parent_and_editor() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = MainWindow::new().unwrap();
    window.show().unwrap();
    native.set_size(slint::PhysicalSize::new(1340, 820));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(1340.0, 820.0),
    });
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let left = std::env::temp_dir().join("left 文档");
    let right = std::env::temp_dir().join("right 👩🏽‍💻");
    let presenter = Rc::new(RefCell::new(Presenter::new(left.clone(), right.clone())));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let captured = commands.clone();
    install(
        &window,
        presenter.clone(),
        Rc::new(move |command| {
            captured.borrow_mut().push(command);
            true
        }),
    );
    let paths = [
        [left.join("Café.txt"), left.join("文档")],
        [right.join("Cafe\u{301}.txt"), right.join("👩🏽‍💻")],
    ];
    for pane in [PaneId::Left, PaneId::Right] {
        window.invoke_action(pane.index() as i32, 3);
        let navigation = commands
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::Navigate {
                    pane: actual,
                    navigation,
                    ..
                }
                | Command::Refresh {
                    pane: actual,
                    navigation,
                    ..
                } if *actual == pane => Some(*navigation),
                _ => None,
            })
            .unwrap();
        let entries: Arc<[Entry]> = paths[pane.index()]
            .iter()
            .enumerate()
            .map(|(index, path)| {
                Entry::new(
                    EntryId(index as u64),
                    path.clone(),
                    if index == 0 {
                        EntryKind::File
                    } else {
                        EntryKind::Directory
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
    }
    refresh(&window, &presenter.borrow());
    window.invoke_set_command("echo draft".into(), 10);
    window.invoke_action(0, 0);
    commands.borrow_mut().clear();
    for (key, shift, operation, expected) in [
        ("a", false, SelectionOperation::FilesOnly, [true, false]),
        ("A", true, SelectionOperation::All, [true, true]),
        (
            "d",
            false,
            SelectionOperation::ClearFilesOnly,
            [false, true],
        ),
        ("a", false, SelectionOperation::FilesOnly, [true, false]),
        ("A", true, SelectionOperation::All, [true, true]),
        ("i", true, SelectionOperation::InvertAll, [false, false]),
        ("i", true, SelectionOperation::InvertAll, [true, true]),
        ("d", true, SelectionOperation::Clear, [false, false]),
        ("i", false, SelectionOperation::Invert, [true, false]),
        ("i", false, SelectionOperation::Invert, [false, false]),
    ] {
        chord(&window, key, shift);
        // Selection feedback must already be right before the worker replies.
        for (row, selected) in expected.into_iter().enumerate() {
            assert_eq!(
                window.get_left_rows().row_data(row + 1).unwrap().selected,
                selected
            );
        }
        let request = selection::acknowledge(
            &presenter,
            &commands,
            PaneId::Left,
            &paths[0]
                .iter()
                .zip(expected)
                .filter_map(|(path, selected)| selected.then_some(path.as_path()))
                .collect::<Vec<&Path>>(),
        );
        assert!(matches!(request.action, SelectionAction::Bulk(actual) if actual == operation));
        refresh(&window, &presenter.borrow());
        assert!(
            !window.get_left_rows().row_data(0).unwrap().selected,
            "parent is not selectable"
        );
        for (index, selected) in expected.into_iter().enumerate() {
            let row = index + 1;
            assert_eq!(
                window.get_left_rows().row_data(row).unwrap().selected,
                selected
            );
            assert!(!window.get_right_rows().row_data(row).unwrap().selected);
        }
        assert_eq!(window.get_command_text(), "echo draft");
    }
    window.invoke_action(1, 0);
    chord(&window, "a", true);
    selection::acknowledge(
        &presenter,
        &commands,
        PaneId::Right,
        &paths[1].iter().map(|p| p.as_path()).collect::<Vec<_>>(),
    );
    refresh(&window, &presenter.borrow());
    assert!(window.get_right_rows().row_data(1).unwrap().selected);
    assert!(!window.get_left_rows().row_data(1).unwrap().selected);
    control(&window, "i");
    selection::acknowledge(
        &presenter,
        &commands,
        PaneId::Right,
        &[paths[1][1].as_path()],
    );
    refresh(&window, &presenter.borrow());
    assert!(!window.get_right_rows().row_data(1).unwrap().selected);
    assert!(window.get_right_rows().row_data(2).unwrap().selected);
    assert!(!window.get_left_rows().row_data(1).unwrap().selected);
    commands.borrow_mut().clear();
    window.invoke_set_command("editable text".into(), 13);
    control(&window, "a");
    assert!(
        commands.borrow().is_empty(),
        "editor Ctrl+A cannot send a file operation"
    );
    assert_eq!(window.get_command_text(), "editable text");
    assert_ne!(window.get_command_cursor(), window.get_command_anchor());
    control(&window, "i");
    assert!(
        commands.borrow().is_empty(),
        "editor Ctrl+I cannot alter panel selection"
    );
    chord(&window, "A", true);
    assert!(
        commands.borrow().is_empty(),
        "editor Ctrl+Shift+A cannot alter panel selection"
    );
    window.hide().unwrap();
}

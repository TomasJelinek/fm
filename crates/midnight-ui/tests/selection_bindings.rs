//! Exercise the installed presenter bridge and explicit asynchronous worker results.
use crate::selection;
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

struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

fn control(window: &MainWindow, text: &str) {
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
            .map(|(index, path)| Entry::new(EntryId(index as u64), path.clone(), EntryKind::File))
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
    for (key, operation, expected) in [
        ("a", SelectionOperation::All, true),
        ("d", SelectionOperation::Clear, false),
        ("i", SelectionOperation::Invert, true),
        ("i", SelectionOperation::Invert, false),
    ] {
        control(&window, key);
        let request = selection::acknowledge(
            &presenter,
            &commands,
            PaneId::Left,
            &if expected {
                paths[0].iter().map(|p| p.as_path()).collect::<Vec<&Path>>()
            } else {
                vec![]
            },
        );
        assert!(matches!(request.action, SelectionAction::Bulk(actual) if actual == operation));
        refresh(&window, &presenter.borrow());
        assert!(
            !window.get_left_rows().row_data(0).unwrap().selected,
            "parent is not selectable"
        );
        for row in 1..3 {
            assert_eq!(
                window.get_left_rows().row_data(row).unwrap().selected,
                expected
            );
            assert!(!window.get_right_rows().row_data(row).unwrap().selected);
        }
        assert_eq!(window.get_command_text(), "echo draft");
    }
    window.invoke_action(1, 0);
    control(&window, "a");
    selection::acknowledge(
        &presenter,
        &commands,
        PaneId::Right,
        &paths[1].iter().map(|p| p.as_path()).collect::<Vec<_>>(),
    );
    refresh(&window, &presenter.borrow());
    assert!(window.get_right_rows().row_data(1).unwrap().selected);
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
    window.hide().unwrap();
}

use midnight_core::*;
#[path = "support/selection.rs"]
mod selection;
use midnight_ui::{MainWindow, Presenter, install, refresh};
use slint::{
    ComponentHandle, Model,
    platform::{
        Key, Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::Duration,
};

struct TestPlatform(Rc<MinimalSoftwareWindow>, Rc<Cell<Duration>>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
    fn duration_since_start(&self) -> Duration {
        self.1.get()
    }
}

struct Harness {
    native: Rc<MinimalSoftwareWindow>,
    time: Rc<Cell<Duration>>,
    window: MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    commands: Rc<RefCell<Vec<Command>>>,
    accepted: Rc<Cell<bool>>,
}
impl Harness {
    fn draw(&self) {
        refresh(&self.window, &self.presenter.borrow());
        let mut pixels = vec![slint::Rgb8Pixel::default(); 1340 * 820];
        self.native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 1340);
        });
    }
    fn position(&self, pane: PaneId, row: usize) -> slint::LogicalPosition {
        let (left, top, width, mode, columns, rows, row_height) = match pane {
            PaneId::Left => (
                self.window.get_left_list_left(),
                self.window.get_left_list_top(),
                self.window.get_left_list_width(),
                self.window.get_left_view_mode(),
                self.window.get_left_view_columns() as usize,
                self.window.get_left_view_rows() as usize,
                self.window.get_left_view_row_height(),
            ),
            PaneId::Right => (
                self.window.get_right_list_left(),
                self.window.get_right_list_top(),
                self.window.get_right_list_width(),
                self.window.get_right_view_mode(),
                self.window.get_right_view_columns() as usize,
                self.window.get_right_view_rows() as usize,
                self.window.get_right_view_row_height(),
            ),
        };
        let (column, band) = match mode {
            1 => (row / rows, row % rows),
            2 => (row % columns, row / columns),
            _ => (0, row),
        };
        slint::LogicalPosition::new(
            left + (column as f32 + 0.5) * width / columns as f32,
            top + row_height * (band as f32 + 0.5),
        )
    }
    fn advance(&self, millis: u64) {
        self.time
            .set(self.time.get() + Duration::from_millis(millis));
        slint::platform::update_timers_and_animations();
    }
    fn pointer(&self, pane: PaneId, row: usize, button: PointerEventButton, pressed: bool) {
        let position = self.position(pane, row);
        self.window.window().dispatch_event(if pressed {
            WindowEvent::PointerPressed { position, button }
        } else {
            WindowEvent::PointerReleased { position, button }
        });
    }
    fn click(&self, pane: PaneId, row: usize, button: PointerEventButton) {
        self.pointer(pane, row, button, true);
        self.advance(20);
        self.pointer(pane, row, button, false);
    }
    fn double_click(&self, pane: PaneId, row: usize, button: PointerEventButton) {
        self.advance(800);
        self.click(pane, row, button);
        self.draw();
        self.advance(70);
        self.click(pane, row, button);
    }
    fn modifier(&self, key: Key, pressed: bool) {
        self.window.window().dispatch_event(if pressed {
            WindowEvent::KeyPressed { text: key.into() }
        } else {
            WindowEvent::KeyReleased { text: key.into() }
        });
    }
    fn complete(&self, pane: PaneId, entries: Vec<Entry>) {
        let navigation = self
            .commands
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::Navigate {
                    pane: actual,
                    navigation,
                    ..
                }
                | Command::Resort {
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
        let index = Arc::new(DirectoryIndex::from_entries(&entries));
        self.presenter.borrow_mut().apply(Event::ListingFinished {
            pane,
            navigation,
            entries: entries.into(),
            index,
        });
        self.draw();
    }
    fn clear(&self) {
        let requests: Vec<_> = self
            .commands
            .borrow()
            .iter()
            .filter_map(|command| {
                if let Command::ShowShellMenu { request, .. } = command {
                    Some(*request)
                } else {
                    None
                }
            })
            .collect();
        for request in requests {
            self.presenter.borrow_mut().apply_with_sender(
                Event::ShellMenuFinished {
                    request,
                    result: Ok(midnight_core::ShellMenuOutcome::Dismissed),
                },
                &|_| true,
            );
        }
        self.commands.borrow_mut().clear();
    }
    fn assert_no_activation(&self, context: &str) {
        assert!(
            self.commands.borrow().iter().all(|command| !matches!(
                command,
                Command::Navigate { .. } | Command::OpenEntry { .. }
            )),
            "{context}"
        );
    }
    fn selection(&self, pane: PaneId, paths: &[&std::path::Path]) {
        selection::acknowledge(&self.presenter, &self.commands, pane, paths);
        refresh(&self.window, &self.presenter.borrow());
    }
}

#[test]
fn actual_mouse_selection_and_activation_preserve_entry_identity() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    let time = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(TestPlatform(native.clone(), time.clone()))).unwrap();
    let window = MainWindow::new().unwrap();
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    native.set_size(slint::PhysicalSize::new(1340, 820));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(1340.0, 820.0),
    });
    let base = PathBuf::from(std::path::MAIN_SEPARATOR_STR).join("文档 ' & $() 👩🏽‍💻");
    let file = base.join("Café & $(literal).txt");
    let folder = base.join("文件夹");
    let entries = || {
        let mut entries = vec![
            Entry::new(EntryId(7), file.clone(), EntryKind::File),
            Entry::new(EntryId(9), folder.clone(), EntryKind::Directory),
        ];
        entries.extend((10..130).map(|id| {
            Entry::new(
                EntryId(id),
                base.join(format!("other-{id:03}.txt")),
                EntryKind::File,
            )
        }));
        entries
    };
    let presenter = Rc::new(RefCell::new(Presenter::new(base.clone(), base.clone())));
    let commands = Rc::new(RefCell::new(vec![]));
    let accepted = Rc::new(Cell::new(true));
    let sent = commands.clone();
    let accept = accepted.clone();
    install(
        &window,
        presenter.clone(),
        Rc::new(move |command| {
            if !accept.get() {
                return false;
            }
            sent.borrow_mut().push(command);
            true
        }),
    );
    let h = Harness {
        native,
        time,
        window,
        presenter,
        commands,
        accepted,
    };
    for pane in [PaneId::Left, PaneId::Right] {
        h.complete(pane, entries());
    }
    h.clear();

    for pane in [PaneId::Left, PaneId::Right] {
        h.advance(800);
        h.click(pane, 1, PointerEventButton::Left);
        assert!(
            h.commands.borrow().is_empty(),
            "single click must never open"
        );
        assert_eq!(h.window.get_active_pane(), pane.index() as i32);
        assert_eq!(h.window.get_focused_pane(), pane.index() as i32);
        let rows = if pane == PaneId::Left {
            h.window.get_left_rows()
        } else {
            h.window.get_right_rows()
        };
        assert!(rows.row_data(1).unwrap().cursor);
        h.advance(800);
        h.modifier(Key::Control, true);
        h.click(pane, 1, PointerEventButton::Left);
        h.modifier(Key::Control, false);
        // The first worker result may arrive during the next pointer gesture.
        // Updating selection must not consume that second click.
        h.advance(800);
        h.modifier(Key::Control, true);
        h.pointer(pane, 2, PointerEventButton::Left, true);
        h.selection(pane, &[&file]);
        assert!(
            rows.row_data(1).unwrap().selected,
            "Ctrl+left must toggle selection"
        );
        h.advance(20);
        h.pointer(pane, 2, PointerEventButton::Left, false);
        h.modifier(Key::Control, false);
        h.selection(pane, &[&file, &folder]);
        assert!(
            rows.row_data(1).unwrap().selected && rows.row_data(2).unwrap().selected,
            "Ctrl+left must retain the other selected entries"
        );
        assert!(h.commands.borrow().is_empty());
        h.double_click(pane, 1, PointerEventButton::Left);
        assert_eq!(h.commands.borrow().len(), 1);
        assert!(
            matches!(&h.commands.borrow()[0], Command::OpenEntry { pane: actual, path, .. }
            if *actual == pane && path == &file)
        );
        h.clear();
        h.double_click(pane, 1, PointerEventButton::Right);
        h.assert_no_activation("right double click must never launch");
        h.clear();
        for modifier in [Key::Control, Key::Shift, Key::Alt, Key::Meta] {
            h.modifier(modifier, true);
            h.double_click(pane, 1, PointerEventButton::Left);
            h.modifier(modifier, false);
            h.commands
                .borrow_mut()
                .retain(|command| !matches!(command, Command::Selection(_)));
            assert!(
                h.commands.borrow().is_empty(),
                "modified double click must not launch"
            );
        }
    }

    // Rendering, hover and a completed refresh cannot execute an association.
    h.window.window().dispatch_event(WindowEvent::PointerMoved {
        position: h.position(PaneId::Left, 1),
    });
    h.draw();
    assert!(h.commands.borrow().is_empty());

    for pane in [PaneId::Left, PaneId::Right] {
        h.double_click(pane, 2, PointerEventButton::Left);
        assert_eq!(h.commands.borrow().len(), 1);
        assert!(
            matches!(&h.commands.borrow()[0], Command::Navigate { pane: actual, path, .. }
            if *actual == pane && path == &folder)
        );
        h.complete(pane, vec![]);
        h.clear();
        h.double_click(pane, 0, PointerEventButton::Left);
        assert!(
            matches!(&h.commands.borrow()[0], Command::Navigate { pane: actual, path, .. }
            if *actual == pane && path == &base)
        );
        h.complete(pane, entries());
        h.clear();
    }

    // A provider may recycle IDs when a pending refresh replaces the snapshot.
    h.window.invoke_action(0, 3);
    h.advance(800);
    h.click(PaneId::Left, 1, PointerEventButton::Left);
    h.complete(
        PaneId::Left,
        vec![
            Entry::new(EntryId(7), base.join("replacement.txt"), EntryKind::File),
            entries()[1].clone(),
        ],
    );
    h.clear();
    h.advance(70);
    h.click(PaneId::Left, 1, PointerEventButton::Left);
    assert!(
        h.commands.borrow().is_empty(),
        "refresh commit must invalidate recycled IDs"
    );
    h.window.invoke_action(0, 3);
    h.complete(PaneId::Left, entries());
    h.clear();

    // The global double-click counter must not retarget a row after refresh.
    h.advance(800);
    h.click(PaneId::Left, 1, PointerEventButton::Left);
    h.window.invoke_action(0, 3);
    h.complete(PaneId::Left, entries());
    h.clear();
    h.advance(70);
    h.click(PaneId::Left, 1, PointerEventButton::Left);
    assert!(
        h.commands.borrow().is_empty(),
        "refresh between clicks invalidates activation"
    );

    // A pending press must not select or open the replacement at the same index.
    h.advance(800);
    h.pointer(PaneId::Right, 1, PointerEventButton::Left, true);
    h.window.invoke_sort(1, 0);
    h.complete(
        PaneId::Right,
        vec![entries()[1].clone(), entries()[0].clone()],
    );
    h.clear();
    h.pointer(PaneId::Right, 1, PointerEventButton::Left, false);
    assert!(
        h.commands.borrow().is_empty(),
        "resort during press must not open replacement"
    );
    h.advance(70);
    h.click(PaneId::Right, 1, PointerEventButton::Left);
    assert!(
        h.commands.borrow().is_empty(),
        "resort must not arm replacement double click"
    );

    h.window.invoke_action(1, 3);
    h.complete(PaneId::Right, entries());
    h.clear();
    h.advance(800);
    h.click(PaneId::Right, 1, PointerEventButton::Left);
    h.modifier(Key::Return, true);
    h.modifier(Key::Return, false);
    assert_eq!(h.commands.borrow().len(), 1);
    assert!(
        matches!(&h.commands.borrow()[0], Command::OpenEntry { pane: PaneId::Right, path, .. }
        if path == &file)
    );
    h.clear();

    h.advance(800);
    h.click(PaneId::Right, 2, PointerEventButton::Left);
    h.modifier(Key::Return, true);
    h.modifier(Key::Return, false);
    assert!(h.commands.borrow().iter().any(|command| matches!(command,
        Command::Navigate { pane: PaneId::Right, path, .. } if path == &folder)));
    h.window.invoke_navigate(1, base.to_str().unwrap().into());
    h.complete(PaneId::Right, entries());
    h.clear();

    // Render both presses and deliver an earlier worker acknowledgement between
    // clicks. Native input keeps the same screen coordinates for both clicks.
    for mode in 0..3 {
        for pane in [PaneId::Left, PaneId::Right] {
            h.window.invoke_view_mode(pane.index() as i32, mode);
            h.draw();
            h.clear();
            for hold in [20, 120] {
                h.advance(800);
                h.click(pane, 1, PointerEventButton::Left);
                h.window.invoke_bulk_selection(pane.index() as i32, 1);
                let position = h.position(pane, 2);
                h.advance(800);
                for click in 0..2 {
                    h.window
                        .window()
                        .dispatch_event(WindowEvent::PointerMoved { position });
                    h.window
                        .window()
                        .dispatch_event(WindowEvent::PointerPressed {
                            position,
                            button: PointerEventButton::Left,
                        });
                    h.draw();
                    h.advance(hold);
                    h.window
                        .window()
                        .dispatch_event(WindowEvent::PointerMoved { position });
                    h.draw();
                    h.window
                        .window()
                        .dispatch_event(WindowEvent::PointerReleased {
                            position,
                            button: PointerEventButton::Left,
                        });
                    if click == 0 {
                        let cursor = if pane == PaneId::Left {
                            h.window.get_left_cursor()
                        } else {
                            h.window.get_right_cursor()
                        };
                        assert_eq!(cursor, 2, "first folder click must focus its actual row");
                        h.selection(pane, &[]);
                        h.draw();
                        assert!(h.commands.borrow().is_empty(), "first click only focuses");
                        h.advance(70);
                    }
                }
                assert_eq!(
                    h.commands.borrow().len(),
                    1,
                    "mode {mode}, pane {pane:?}, {hold} ms presses must activate once"
                );
                assert!(matches!(&h.commands.borrow()[0],
                    Command::Navigate { pane: actual, path, .. }
                    if *actual == pane && path == &folder));
                h.complete(pane, vec![]);
                h.window
                    .invoke_navigate(pane.index() as i32, base.to_str().unwrap().into());
                h.complete(pane, entries());
                h.clear();
            }
            h.double_click(pane, 1, PointerEventButton::Left);
            assert_eq!(h.commands.borrow().len(), 1, "file activation occurs once");
            assert!(matches!(&h.commands.borrow()[0],
                Command::OpenEntry { pane: actual, path, .. }
                if *actual == pane && path == &file));
            h.clear();
            for modifier in [Key::Control, Key::Shift, Key::Alt, Key::Meta] {
                h.modifier(modifier, true);
                h.double_click(pane, 2, PointerEventButton::Left);
                h.modifier(modifier, false);
                h.assert_no_activation("modified folder double click must never navigate");
                h.clear();
                // The SDK tracks button/time/position, not keyboard modifiers.
                // A modified first click must not arm a later plain activation.
                h.advance(800);
                h.modifier(modifier, true);
                h.click(pane, 2, PointerEventButton::Left);
                h.modifier(modifier, false);
                h.draw();
                h.advance(70);
                h.click(pane, 2, PointerEventButton::Left);
                h.assert_no_activation("modified then plain click must never navigate");
                h.clear();
                // A modifier pressed after pointer-down still makes the second
                // click modified; its release must not open the folder.
                h.advance(800);
                h.click(pane, 2, PointerEventButton::Left);
                h.advance(70);
                h.pointer(pane, 2, PointerEventButton::Left, true);
                h.modifier(modifier, true);
                h.pointer(pane, 2, PointerEventButton::Left, false);
                h.modifier(modifier, false);
                h.assert_no_activation("modifier during second press must never navigate");
                h.clear();
            }
            h.double_click(pane, 2, PointerEventButton::Right);
            h.assert_no_activation("right folder double click must never navigate");
            h.clear();

            h.advance(800);
            h.click(pane, 2, PointerEventButton::Left);
            h.window.invoke_action(pane.index() as i32, 3);
            h.complete(pane, entries());
            h.clear();
            h.advance(70);
            h.click(pane, 2, PointerEventButton::Left);
            h.assert_no_activation("refresh between folder clicks invalidates activation");
            h.clear();

            h.advance(800);
            h.pointer(pane, 2, PointerEventButton::Left, true);
            h.draw();
            h.window.invoke_sort(pane.index() as i32, 0);
            let mut reordered = entries();
            reordered.swap(0, 1);
            h.complete(pane, reordered);
            h.clear();
            h.pointer(pane, 2, PointerEventButton::Left, false);
            h.assert_no_activation("resort during folder press cannot open the replacement file");
            h.advance(70);
            h.click(pane, 2, PointerEventButton::Left);
            h.assert_no_activation("a stale folder press cannot arm the replacement file");
            h.window.invoke_action(pane.index() as i32, 3);
            h.complete(pane, entries());
            h.clear();
        }
    }

    // Once count snapshots are established, resizing must still rebuild the
    // layout when the number of columns really changes.
    let columns = h.window.get_left_view_columns();
    h.window.set_panel_ratio(0.0);
    h.draw();
    h.advance(20);
    h.draw();
    assert!(h.window.get_left_view_columns() < columns);
    h.double_click(PaneId::Left, 1, PointerEventButton::Left);
    assert_eq!(h.commands.borrow().len(), 1);
    assert!(matches!(&h.commands.borrow()[0],
        Command::OpenEntry { pane: PaneId::Left, path, .. } if path == &file));
    h.clear();
    h.window.set_panel_ratio(0.5);

    for pane in [PaneId::Left, PaneId::Right] {
        h.window.invoke_view_mode(pane.index() as i32, 0);
    }
    h.draw();
    h.clear();

    // Touch density uses the same guarded pointer path with its larger rows.
    h.window.invoke_preferences_changed(2, 0, 0);
    h.window.invoke_preferences_changed(2, 1, 1);
    h.draw();
    h.clear();
    h.double_click(PaneId::Left, 1, PointerEventButton::Left);
    assert_eq!(h.commands.borrow().len(), 1);
    assert!(matches!(&h.commands.borrow()[0], Command::OpenEntry { path, .. } if path == &file));
    h.clear();

    h.accepted.set(false);
    h.double_click(PaneId::Right, 1, PointerEventButton::Left);
    assert!(h.commands.borrow().is_empty());
    assert_eq!(h.window.get_message_title().as_str(), "Open queue busy");
    assert!(!h.window.get_right_loading());
}

use midnight_core::*;
#[path = "support/selection.rs"]
mod selection;
use midnight_ui::{MainWindow, Presenter, Theme, install, refresh};
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
        let theme = self.window.global::<Theme>();
        let (left, top) = match pane {
            PaneId::Left => (
                self.window.get_left_list_left(),
                self.window.get_left_list_top(),
            ),
            PaneId::Right => (
                self.window.get_right_list_left(),
                self.window.get_right_list_top(),
            ),
        };
        slint::LogicalPosition::new(
            left + 100.0,
            top + theme.get_row_height() * (row as f32 + 0.5),
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
        self.commands.borrow_mut().clear();
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
        vec![
            Entry::new(EntryId(7), file.clone(), EntryKind::File),
            Entry::new(EntryId(9), folder.clone(), EntryKind::Directory),
        ]
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
        assert!(
            h.commands.borrow().is_empty(),
            "right double click must never launch"
        );
        for modifier in [Key::Control, Key::Shift, Key::Alt] {
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

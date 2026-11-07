//! Installed drag bridge: raw capture, cancellation, panel geometry and internal fallback.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::items::AccessibleRole;
use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::{
    ComponentHandle,
    platform::{Key, PointerEventButton, WindowEvent},
};
use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc, time::Duration};

struct Harness {
    ui: ui::TestUi,
    presenter: Rc<RefCell<Presenter>>,
    commands: Rc<RefCell<Vec<Command>>>,
    send: Rc<dyn Fn(Command) -> bool>,
    right: PathBuf,
}

impl Harness {
    fn new() -> Self {
        let mut ui = ui::TestUi::new();
        let left = std::env::temp_dir().join("drag source 文档");
        let right = std::env::temp_dir().join("drag target & $(literal)");
        let presenter = Rc::new(RefCell::new(Presenter::new(left.clone(), right.clone())));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let captured = commands.clone();
        let send: Rc<dyn Fn(Command) -> bool> = Rc::new(move |command| {
            captured.borrow_mut().push(command);
            true
        });
        install(&ui.window, presenter.clone(), send.clone());
        for (pane, path) in [(PaneId::Left, left), (PaneId::Right, right.clone())] {
            ui.window.invoke_action(pane.index() as i32, 3);
            let navigation = commands
                .borrow()
                .iter()
                .rev()
                .find_map(|command| match command {
                    Command::Navigate {
                        pane: target,
                        navigation,
                        ..
                    } if *target == pane => Some(*navigation),
                    _ => None,
                })
                .unwrap();
            let entries = if pane == PaneId::Left {
                vec![Entry::new(
                    EntryId(1),
                    path.join("file 文档.bin"),
                    EntryKind::File,
                )]
            } else {
                vec![Entry::new(
                    EntryId(2),
                    path.join("hovered subfolder"),
                    EntryKind::Directory,
                )]
            };
            presenter.borrow_mut().apply(Event::ListingFinished {
                pane,
                navigation,
                index: Arc::new(DirectoryIndex::from_entries(&entries)),
                entries: entries.into(),
            });
        }
        refresh(&ui.window, &presenter.borrow());
        ui.draw();
        Self {
            ui,
            presenter,
            commands,
            send,
            right,
        }
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            std::thread::sleep(Duration::from_millis(20));
            slint::platform::update_timers_and_animations();
            self.ui.draw();
        }
    }

    fn start(&mut self) -> DragId {
        let start = ui::root_position(&self.ui.window, "file 文档.bin", AccessibleRole::ListItem);
        self.ui
            .window
            .window()
            .dispatch_event(WindowEvent::PointerPressed {
                position: start,
                button: PointerEventButton::Left,
            });
        self.ui
            .window
            .window()
            .dispatch_event(WindowEvent::PointerMoved {
                position: slint::LogicalPosition::new(start.x + 16.0, start.y),
            });
        self.commands
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::PrepareDrag { id, files, .. } => {
                    assert_eq!(
                        files.fallback.as_ref().unwrap().file_name().unwrap(),
                        "file 文档.bin"
                    );
                    Some(*id)
                }
                _ => None,
            })
            .expect("threshold crossing prepares the original native row")
    }

    fn prepared(&mut self, id: DragId) {
        self.presenter.borrow_mut().apply_with_sender(
            Event::DragDrop(DragDropEvent::Prepared { id, item_count: 1 }),
            self.send.as_ref(),
        );
        refresh(&self.ui.window, &self.presenter.borrow());
        self.ui.draw();
    }

    fn target(&self) -> slint::LogicalPosition {
        ui::root_position(
            &self.ui.window,
            "hovered subfolder",
            AccessibleRole::ListItem,
        )
    }

    fn release(&self) {
        self.ui
            .window
            .window()
            .dispatch_event(WindowEvent::PointerReleased {
                position: self.target(),
                button: PointerEventButton::Left,
            });
    }

    fn targets(&self) -> SharedDropTargets {
        self.commands
            .borrow()
            .iter()
            .find_map(|command| match command {
                Command::AttachDragDrop { targets, .. } => Some(targets.clone()),
                _ => None,
            })
            .unwrap()
    }
}

#[test]
fn internal_fallback_highlights_the_panel_and_commits_only_its_native_directory() {
    let mut h = Harness::new();
    let id = h.start();
    h.prepared(id);
    h.ui.window
        .window()
        .dispatch_event(WindowEvent::PointerMoved {
            position: h.target(),
        });
    h.ui.draw();
    assert_eq!(h.ui.window.get_drag_hover_pane(), 1);
    let highlight = ui::root_item(&h.ui.window, "Copy into this panel", AccessibleRole::Text);
    assert!(
        highlight.geometry().size.width > 300.0,
        "feedback covers the list, not a folder row"
    );
    render::snapshot(&h.ui, "drag-copy-target");
    h.release();
    assert!(!h.ui.window.get_drag_active());
    let commands = h.commands.borrow();
    let drops: Vec<_> = commands
        .iter()
        .filter_map(|command| match command {
            Command::InternalDrop {
                id: actual,
                target,
                targets_revision,
            } => {
                assert_eq!(*actual, id);
                assert!(*targets_revision > 0);
                Some(target)
            }
            _ => None,
        })
        .collect();
    assert_eq!(drops.len(), 1);
    assert_eq!(drops[0].pane, PaneId::Right);
    assert_eq!(*drops[0].directory, h.right);
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, Command::StartDrag(_) | Command::OpenEntry { .. }))
    );
}

#[test]
fn release_escape_and_navigation_cannot_be_revived_by_late_preparation() {
    let mut h = Harness::new();
    for kind in 0..3 {
        let id = h.start();
        match kind {
            0 => h.release(),
            1 => ui::key(&h.ui.window, Key::Escape),
            _ => h.ui.window.invoke_action(0, 2),
        }
        h.prepared(id);
        h.release();
        assert!(!h.ui.window.get_drag_active());
        assert!(!h.commands.borrow().iter().any(|command| matches!(
            command,
            Command::StartDrag(_) | Command::InternalDrop { .. }
        )));
        assert!(h.commands.borrow().iter().any(|command| matches!(command,
            Command::CancelDrag(actual) if *actual == id)));
    }
}

#[test]
fn published_regions_follow_swapping_zoom_and_modal_exclusion() {
    let mut h = Harness::new();
    let _ = h.start();
    h.release();
    h.settle();
    let targets = h.targets();
    assert_eq!(targets.lock().unwrap().targets.len(), 2);
    h.ui.window.invoke_swap_panels();
    h.settle();
    {
        let snapshot = targets.lock().unwrap();
        let left = snapshot
            .targets
            .iter()
            .find(|target| target.pane == PaneId::Left)
            .unwrap();
        let right = snapshot
            .targets
            .iter()
            .find(|target| target.pane == PaneId::Right)
            .unwrap();
        assert!(
            right.rect.x < left.rect.x,
            "native identities follow swapped geometry"
        );
        assert!(right.rect.y as f32 >= h.ui.window.get_right_list_top());
    }
    h.ui.window.invoke_toggle_panel_zoom(1);
    h.settle();
    assert_eq!(targets.lock().unwrap().targets.len(), 1);
    assert_eq!(targets.lock().unwrap().targets[0].pane, PaneId::Right);
    h.ui.window.invoke_show_options();
    h.settle();
    assert!(
        targets.lock().unwrap().targets.is_empty(),
        "a dialog cannot receive a file drop"
    );
    ui::key(&h.ui.window, Key::Escape);
    h.settle();
    assert!(!targets.lock().unwrap().targets.is_empty());
    h.ui.window.invoke_activate(0);
    h.ui.window.invoke_focus_files();
    ui::key(&h.ui.window, Key::End);
    ui::key(&h.ui.window, Key::F5);
    h.settle();
    assert!(!h.ui.window.get_copy_draft_token().is_empty());
    assert!(
        targets.lock().unwrap().targets.is_empty(),
        "the persistent copy overlay excludes native drops on every poll"
    );
    ui::key(&h.ui.window, Key::Escape);
    h.settle();
    assert!(!targets.lock().unwrap().targets.is_empty());
}

#[test]
fn touch_scrolling_and_right_button_movement_never_prepare_file_drags() {
    let mut h = Harness::new();
    for (touch, button) in [
        (true, PointerEventButton::Left),
        (false, PointerEventButton::Right),
    ] {
        h.ui.window.set_touch_contact(touch);
        h.ui.draw();
        let position = ui::root_position(&h.ui.window, "file 文档.bin", AccessibleRole::ListItem);
        h.ui.window
            .window()
            .dispatch_event(WindowEvent::PointerPressed { position, button });
        let moved = slint::LogicalPosition::new(position.x + 30.0, position.y + 8.0);
        h.ui.window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position: moved });
        h.ui.window
            .window()
            .dispatch_event(WindowEvent::PointerReleased {
                position: moved,
                button,
            });
        assert!(!h.ui.window.get_drag_active());
        assert!(
            !h.commands
                .borrow()
                .iter()
                .any(|command| matches!(command, Command::PrepareDrag { .. }))
        );
    }
}

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
use std::{cell::RefCell, rc::Rc, sync::Arc, time::Duration};

struct Harness {
    ui: ui::TestUi,
    presenter: Rc<RefCell<Presenter>>,
    commands: Rc<RefCell<Vec<Command>>>,
    send: Rc<dyn Fn(Command) -> bool>,
}

impl Harness {
    fn new() -> Self {
        Self::with_targets(1)
    }

    fn with_targets(count: usize) -> Self {
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
                vec![
                    Entry::new(EntryId(1), path.join("file 文档.bin"), EntryKind::File),
                    Entry::new(
                        EntryId(3),
                        path.join("local destination"),
                        EntryKind::Directory,
                    ),
                ]
            } else {
                (0..count)
                    .map(|index| {
                        Entry::new(
                            EntryId(index as u64 + 2),
                            path.join(if index == 0 {
                                "hovered subfolder".to_owned()
                            } else {
                                format!("Folder {index:03}")
                            }),
                            EntryKind::Directory,
                        )
                    })
                    .collect()
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
fn internal_drop_highlights_the_exact_folder_and_captures_its_identity() {
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
    assert_eq!(h.ui.window.get_drag_hover_row(), 1);
    assert!(h.ui.window.get_drag_label().contains("hovered subfolder"));
    render::snapshot(&h.ui, "drag-copy-target");
    h.release();
    assert!(!h.ui.window.get_drag_active());
    let commands = h.commands.borrow();
    let drops: Vec<_> = commands
        .iter()
        .filter_map(|command| match command {
            Command::InternalDrop(request) => {
                assert_eq!(request.id, id);
                assert_eq!(request.intent, InternalDropIntent::Auto);
                Some(&request.destination)
            }
            _ => None,
        })
        .collect();
    assert_eq!(drops.len(), 1);
    assert!(matches!(
        drops[0],
        InternalDropDestination::PanelEntry {
            pane: PaneId::Right,
            entry: EntryId(2),
            ..
        }
    ));
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
        assert!(
            !h.commands
                .borrow()
                .iter()
                .any(|command| matches!(command, Command::StartDrag(_) | Command::InternalDrop(_)))
        );
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

#[test]
fn folder_drops_request_volume_default_or_explicit_modifiers_and_reject_link_requests() {
    let mut h = Harness::new();
    for (control, shift, link, expected) in [
        (false, false, false, Some(InternalDropIntent::Auto)),
        (true, false, false, Some(InternalDropIntent::Copy)),
        (false, true, false, Some(InternalDropIntent::Move)),
        (true, true, false, None),
        (false, false, true, None),
    ] {
        let id = h.start();
        h.prepared(id);
        let modifiers: Vec<_> = [
            (control, Key::Control),
            (shift, Key::Shift),
            (link, Key::Alt),
        ]
        .into_iter()
        .filter_map(|(held, key)| held.then_some(key))
        .collect();
        for key in &modifiers {
            h.ui.window
                .window()
                .dispatch_event(WindowEvent::KeyPressed {
                    text: (*key).into(),
                });
        }
        let position =
            ui::root_position(&h.ui.window, "local destination", AccessibleRole::ListItem);
        h.ui.window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        h.ui.draw();
        assert_eq!(
            h.ui.window.get_drag_hover_pane(),
            if expected.is_some() { 0 } else { -1 }
        );
        h.ui.window
            .window()
            .dispatch_event(WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            });
        for key in modifiers.iter().rev() {
            h.ui.window
                .window()
                .dispatch_event(WindowEvent::KeyReleased {
                    text: (*key).into(),
                });
        }
        let commands = h.commands.borrow();
        let drop = commands.iter().find_map(|command| match command {
            Command::InternalDrop(request) if request.id == id => Some(request),
            _ => None,
        });
        assert_eq!(drop.map(|request| request.intent), expected);
        if let Some(request) = drop {
            assert!(matches!(
                request.destination,
                InternalDropDestination::PanelEntry {
                    pane: PaneId::Left,
                    entry: EntryId(3),
                    ..
                }
            ));
        }
    }
}

#[test]
fn folder_targets_follow_actual_scrolled_cells_in_details_brief_and_thumbnail_modes() {
    use slint::Model;
    let mut h = Harness::with_targets(100);
    for mode in [0, 1, 2] {
        h.ui.window.set_right_view_mode(mode);
        h.ui.window.invoke_scroll_selection(1, 100000.0);
        h.ui.draw();
        for offset in [0.0, -100.0] {
            h.ui.window.invoke_scroll_selection(1, offset);
            h.ui.draw();
            let first = h.ui.window.get_right_visible_row_start().max(1);
            let last = h.ui.window.get_right_visible_row_end();
            let mut checked = 0;
            let id = h.start();
            h.prepared(id);
            for row in first..last.min(first + 20) {
                let item = h.ui.window.get_right_rows().row_data(row as usize).unwrap();
                // Brief view's bounding row range also spans cells in other columns
                // that are not instantiated at this vertical scroll offset.
                let Some(position) =
                    ui::try_root_position(&h.ui.window, &item.name, AccessibleRole::ListItem)
                else {
                    continue;
                };
                let top = h.ui.window.get_right_list_top();
                if position.y < top || position.y >= top + h.ui.window.get_right_list_height() {
                    continue;
                }
                h.ui.window
                    .window()
                    .dispatch_event(WindowEvent::PointerMoved { position });
                h.ui.draw();
                assert_eq!(
                    h.ui.window.get_drag_hover_row(),
                    row,
                    "mode {mode}, scroll {offset}"
                );
                assert!(h.ui.window.get_drag_label().contains(item.name.as_str()));
                checked += 1;
            }
            assert!(checked > 0);
            ui::key(&h.ui.window, Key::Escape);
            h.ui.window
                .window()
                .dispatch_event(WindowEvent::PointerReleased {
                    position: slint::LogicalPosition::new(0.0, 0.0),
                    button: PointerEventButton::Left,
                });
            assert!(!h.commands.borrow().iter().any(
                |command| matches!(command, Command::InternalDrop(request) if request.id == id)
            ));
        }
    }
}

#[test]
fn scrolling_during_a_stationary_drag_drops_on_the_newly_visible_folder() {
    use slint::Model;
    let mut h = Harness::with_targets(100);
    for mode in [0, 1, 2] {
        h.ui.window.set_right_view_mode(mode);
        h.ui.window.invoke_scroll_selection(1, 100000.0);
        h.ui.draw();
        let id = h.start();
        h.prepared(id);
        let position = h.target();
        h.ui.window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        h.ui.draw();
        assert!(h.ui.window.get_drag_label().contains("hovered subfolder"));

        let next_row = if mode == 2 {
            1 + h.ui.window.get_right_view_columns()
        } else {
            2
        };
        let next =
            h.ui.window
                .get_right_rows()
                .row_data(next_row as usize)
                .unwrap();
        h.ui.window
            .invoke_scroll_selection(1, -h.ui.window.get_right_view_row_height());
        h.ui.draw();
        let next_position = ui::root_position(&h.ui.window, &next.name, AccessibleRole::ListItem);
        assert!((next_position.x - position.x).abs() < 1.0);
        assert!((next_position.y - position.y).abs() < 1.0);
        // No pointer-moved event follows scrolling. Release must read the live layout,
        // not the last highlighted folder or a row cached before the scroll.
        h.ui.window
            .window()
            .dispatch_event(WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            });
        let commands = h.commands.borrow();
        let request = commands.iter().find_map(|command| match command {
            Command::InternalDrop(request) if request.id == id => Some(request),
            _ => None,
        });
        assert!(matches!(
            &request.expect("stationary drag still drops into the visible folder").destination,
            InternalDropDestination::PanelEntry {
                pane: PaneId::Right,
                entry,
                ..
            } if *entry == EntryId(next_row as u64 + 1)
        ));
    }
}

#[test]
fn parent_file_rows_and_scrollbar_gutters_cannot_receive_internal_drops() {
    let mut h = Harness::new();
    for kind in 0..3 {
        let id = h.start();
        h.prepared(id);
        let position = match kind {
            0 => slint::LogicalPosition::new(
                h.ui.window.get_right_list_left() + 70.0,
                h.ui.window.get_right_list_top() + h.ui.window.get_right_view_row_height() / 2.0,
            ),
            1 => ui::root_position(&h.ui.window, "file 文档.bin", AccessibleRole::ListItem),
            _ => slint::LogicalPosition::new(
                h.ui.window.get_right_list_left() + h.ui.window.get_right_list_width() - 2.0,
                h.ui.window.get_right_list_top() + 20.0,
            ),
        };
        h.ui.window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        h.ui.draw();
        assert_eq!(h.ui.window.get_drag_hover_pane(), -1);
        h.ui.window
            .window()
            .dispatch_event(WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            });
        assert!(
            !h.commands.borrow().iter().any(
                |command| matches!(command, Command::InternalDrop(request) if request.id == id)
            )
        );
    }
}

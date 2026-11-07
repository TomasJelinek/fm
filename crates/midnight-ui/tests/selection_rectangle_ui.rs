//! Actual child-first mouse input through the installed bridge and virtualized panels.
#[path = "support/selection.rs"]
mod selection;
#[path = "support/ui.rs"]
mod ui;
use midnight_core::*;
use midnight_ui::{Presenter, SelectionRectangle, install, refresh};
use slint::{
    ComponentHandle, Model,
    platform::{Key, PointerEventButton, WindowEvent},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};

struct Harness {
    ui: ui::TestUi,
    presenter: Rc<RefCell<Presenter>>,
    commands: Rc<RefCell<Vec<Command>>>,
    refuse: Rc<Cell<bool>>,
    navigation: NavigationId,
}
impl Harness {
    fn new(count: usize) -> Self {
        let ui = ui::TestUi::new();
        let root = std::env::temp_dir().join("selection 文档");
        let presenter = Rc::new(RefCell::new(Presenter::new(
            root.clone(),
            root.join("right"),
        )));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let sent = commands.clone();
        let refuse = Rc::new(Cell::new(false));
        let denied = refuse.clone();
        install(
            &ui.window,
            presenter.clone(),
            Rc::new(move |command| {
                if denied.get()
                    && matches!(
                        command,
                        Command::Selection(_) | Command::ArchiveSelect { .. }
                    )
                {
                    return false;
                }
                sent.borrow_mut().push(command);
                true
            }),
        );
        let navigation = commands
            .borrow()
            .iter()
            .find_map(|command| match command {
                Command::Navigate {
                    pane: PaneId::Left,
                    navigation,
                    ..
                } => Some(*navigation),
                _ => None,
            })
            .unwrap();
        let entries: Arc<[Entry]> = (0..count)
            .map(|row| {
                Entry::new(
                    EntryId(100 + row as u64 * 17),
                    root.join(format!("é{row}.txt")),
                    EntryKind::File,
                )
            })
            .collect::<Vec<_>>()
            .into();
        let send = commands.clone();
        presenter.borrow_mut().apply_with_sender(
            Event::ListingFinished {
                pane: PaneId::Left,
                navigation,
                index: Arc::new(DirectoryIndex::from_entries(&entries)),
                entries,
            },
            &|command| {
                send.borrow_mut().push(command);
                true
            },
        );
        selection::acknowledge(&presenter, &commands, PaneId::Left, &[]);
        commands.borrow_mut().clear();
        let mut harness = Self {
            ui,
            presenter,
            commands,
            refuse,
            navigation,
        };
        harness.draw();
        harness
    }
    fn draw(&mut self) {
        refresh(&self.ui.window, &self.presenter.borrow());
        self.ui.draw();
    }
    fn pointer(&self, x: f32, y: f32, kind: i32) {
        let position = slint::LogicalPosition::new(x, y);
        self.ui.window.window().dispatch_event(match kind {
            0 => WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            },
            2 => WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            },
            _ => WindowEvent::PointerMoved { position },
        });
    }
    fn start(&self) -> (f32, f32) {
        let x =
            self.ui.window.get_left_list_left() + self.ui.window.get_left_list_grid_width() + 5.0;
        let y = self.ui.window.get_left_list_top() + self.ui.window.get_left_list_height() - 35.0;
        self.pointer(x, y, 1);
        self.pointer(x, y, 0);
        assert_eq!(
            self.ui.window.global::<SelectionRectangle>().get_pane(),
            0,
            "empty gutter starts the gesture"
        );
        (x, y)
    }
    fn top(&self) -> (f32, f32) {
        (
            self.ui.window.get_left_list_left() + 1.0,
            self.ui.window.get_left_list_top() + 1.0,
        )
    }
    fn flags(&self) -> Vec<bool> {
        let rows = self.ui.window.get_left_rows();
        (1..rows.row_count())
            .map(|row| rows.row_data(row).unwrap().selected)
            .collect()
    }
    fn range_request(&self) -> SelectionRequest {
        self.commands
            .borrow()
            .iter()
            .find_map(|command| match command {
                Command::Selection(request) => Some(request.clone()),
                _ => None,
            })
            .unwrap()
    }
}

#[test]
fn empty_gutter_selects_every_layout_with_immediate_preview_and_one_backend_commit() {
    let mut h = Harness::new(5);
    for mode in [0, 1, 2] {
        h.ui.window.invoke_view_mode(0, mode);
        h.draw();
        assert_eq!(h.ui.window.get_left_view_mode(), mode);
        h.start();
        let (x, y) = h.top();
        h.pointer(x, y, 1);
        assert!(h.ui.window.global::<SelectionRectangle>().get_active());
        assert_eq!(h.flags(), vec![true; 5]);
        assert!(
            h.commands
                .borrow()
                .iter()
                .all(|command| !matches!(command, Command::Selection(_))),
            "preview must not flood the backend"
        );
        h.pointer(x, y, 2);
        let requests: Vec<_> = h
            .commands
            .borrow()
            .iter()
            .filter_map(|command| match command {
                Command::Selection(request) => Some(request.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(requests.len(), 1);
        let SelectionAction::Ranges {
            ranges,
            mode: action_mode,
        } = &requests[0].action
        else {
            panic!()
        };
        assert_eq!(*action_mode, SelectionMode::Replace);
        assert_eq!(ranges.spans(), &[SelectionSpan { first: 0, last: 4 }]);
        selection::acknowledge(&h.presenter, &h.commands, PaneId::Left, &[]);
        h.draw();
    }
}

#[test]
fn escape_focus_loss_and_refused_release_remove_preview_without_phantom_selection() {
    let mut h = Harness::new(5);
    for focus_loss in [false, true] {
        h.start();
        let (x, y) = h.top();
        h.pointer(x, y, 1);
        h.ui.window.window().dispatch_event(if focus_loss {
            WindowEvent::WindowActiveChanged(false)
        } else {
            WindowEvent::KeyPressed {
                text: Key::Escape.into(),
            }
        });
        assert_eq!(h.ui.window.global::<SelectionRectangle>().get_pane(), -1);
        assert_eq!(h.flags(), vec![false; 5]);
        h.pointer(x, y, 2);
        h.ui.window
            .window()
            .dispatch_event(WindowEvent::WindowActiveChanged(true));
    }
    h.start();
    let (x, y) = h.top();
    h.pointer(x, y, 1);
    h.refuse.set(true);
    h.pointer(x, y, 2);
    assert_eq!(h.flags(), vec![false; 5]);
    assert!(
        h.commands
            .borrow()
            .iter()
            .all(|command| !matches!(command, Command::Selection(_)))
    );
    assert!(h.ui.window.get_left_error().contains("queue is busy"));
    h.draw();
}

#[test]
fn long_details_view_starts_in_gutter_autoscrolls_and_reaches_offscreen_rows() {
    let mut h = Harness::new(500);
    let (x, _) = h.start();
    let y = h.ui.window.get_left_list_top() + h.ui.window.get_left_list_height() - 1.0;
    h.pointer(h.ui.window.get_left_list_left() + 1.0, y, 1);
    for _ in 0..12 {
        std::thread::sleep(std::time::Duration::from_millis(35));
        slint::platform::update_timers_and_animations();
        h.ui.draw();
    }
    assert!(h.ui.window.get_left_list_content_y() < -100.0);
    h.pointer(x - h.ui.window.get_left_list_grid_width(), y, 2);
    let request = h
        .commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::Selection(request) => Some(request.clone()),
            _ => None,
        })
        .unwrap();
    let SelectionAction::Ranges { ranges, .. } = request.action else {
        panic!()
    };
    assert!(
        ranges.spans().last().unwrap().last > h.ui.window.get_left_view_rows() as usize,
        "rectangle must include newly reached offscreen cells"
    );
    selection::acknowledge(&h.presenter, &h.commands, PaneId::Left, &[]);
    h.draw();
    let before = h.ui.window.get_left_list_content_y();
    let bar_x = h.ui.window.get_left_list_left() + h.ui.window.get_left_list_width() - 7.0;
    let bar_y = h.ui.window.get_left_list_top() + h.ui.window.get_left_list_height() * 0.8;
    h.pointer(bar_x, bar_y, 1);
    h.pointer(bar_x, bar_y, 0);
    h.pointer(bar_x, bar_y + 20.0, 1);
    h.pointer(bar_x, bar_y + 20.0, 2);
    assert_eq!(h.ui.window.global::<SelectionRectangle>().get_pane(), -1);
    assert_ne!(
        h.ui.window.get_left_list_content_y(),
        before,
        "reserved scrollbar must still scroll independently"
    );
    assert!(
        h.commands
            .borrow()
            .iter()
            .all(|command| !matches!(command, Command::Selection(_)))
    );
}

#[test]
fn cancellation_during_presenter_borrow_defers_repaint_without_reentrant_borrow() {
    let h = Harness::new(5);
    h.start();
    let (x, y) = h.top();
    h.pointer(x, y, 1);
    let borrowed = h.presenter.borrow_mut();
    assert!(h.ui.window.global::<SelectionRectangle>().invoke_cancel());
    drop(borrowed);
    assert_eq!(h.flags(), vec![false; 5]);
    std::thread::sleep(std::time::Duration::from_millis(35));
    slint::platform::update_timers_and_animations();
    assert_eq!(h.ui.window.global::<SelectionRectangle>().get_pane(), -1);
    assert!(
        h.commands
            .borrow()
            .iter()
            .all(|command| !matches!(command, Command::Selection(_)))
    );
}

#[test]
fn captured_ctrl_shift_modifiers_survive_release_before_mouse_up() {
    let mut h = Harness::new(5);
    for (control, shift, expected_mode) in [
        (false, true, SelectionMode::Add),
        (true, false, SelectionMode::Toggle),
        (true, true, SelectionMode::Add),
    ] {
        for (enabled, key) in [(control, Key::Control), (shift, Key::Shift)] {
            if enabled {
                h.ui.window
                    .window()
                    .dispatch_event(WindowEvent::KeyPressed { text: key.into() });
            }
        }
        h.start();
        let (x, y) = h.top();
        h.pointer(x, y, 1);
        for (enabled, key) in [(control, Key::Control), (shift, Key::Shift)] {
            if enabled {
                h.ui.window
                    .window()
                    .dispatch_event(WindowEvent::KeyReleased { text: key.into() });
            }
        }
        h.pointer(x, y, 2);
        let request = h.range_request();
        let SelectionAction::Ranges { mode, .. } = request.action else {
            panic!()
        };
        assert_eq!(mode, expected_mode);
        assert_eq!(h.flags(), vec![expected_mode != SelectionMode::Toggle; 5]);
        let paths: Vec<_> = if expected_mode == SelectionMode::Add {
            request.view.unwrap().by_path.keys().cloned().collect()
        } else {
            vec![]
        };
        selection::acknowledge(
            &h.presenter,
            &h.commands,
            PaneId::Left,
            &paths.iter().map(|p| p.as_path()).collect::<Vec<_>>(),
        );
        h.draw();
    }
}

#[derive(Debug)]
struct ArchiveFixture;
impl ArchiveMount for ArchiveFixture {
    fn id(&self) -> u64 {
        77
    }
    fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        panic!("UI listed archive")
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        panic!("UI read archive")
    }
    fn validate(&self) -> Result<(), FsError> {
        panic!("UI validated archive")
    }
}

#[test]
fn archive_thumbnail_rectangle_keeps_disjoint_opaque_ids_and_preview_until_ack() {
    let mut h = Harness::new(5);
    let entries: Arc<[Entry]> = (0..5)
        .map(|row| {
            Entry::from_archive(ArchiveNode {
                id: VirtualEntryId {
                    mount: 77,
                    entry: 100 + row * 17,
                },
                parent: 1,
                raw_name: vec![0xff, row as u8].into(),
                display_name: "same 文档".into(),
                kind: EntryKind::File,
                size: Some(row),
                readable: true,
            })
        })
        .collect::<Vec<_>>()
        .into();
    h.presenter.borrow_mut().apply(Event::ArchiveListed {
        pane: PaneId::Left,
        navigation: h.navigation,
        result: Ok(ArchiveListing {
            location: ArchiveLocation {
                mount: Some(Arc::new(ArchiveFixture)),
                directory: 1,
                address: ArchiveAddress {
                    provider: "test-archive".into(),
                    container: std::env::temp_dir().join("文档.zip"),
                    source_version: vec![1],
                    components: vec![],
                },
                trail: vec![(1, "文档.zip".into())],
            },
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        }),
    });
    h.ui.window.invoke_view_mode(0, 2);
    h.draw();
    assert_eq!(h.ui.window.get_left_view_mode(), 2);
    let columns = h.ui.window.get_left_view_columns();
    assert!(columns >= 3);
    let cell = h.ui.window.get_left_list_grid_width() / columns as f32;
    let left = h.ui.window.get_left_list_left();
    let top = h.ui.window.get_left_list_top();
    let height = h.ui.window.get_left_view_row_height();
    let (sx, sy) = (left + 2.5 * cell, top + 1.5 * height);
    h.pointer(sx, sy, 1);
    h.pointer(sx, sy, 0);
    assert_eq!(h.ui.window.global::<SelectionRectangle>().get_pane(), 0);
    let (x, y) = (left + 1.5 * cell, top + 0.5 * height);
    h.pointer(x, y, 1);
    assert_eq!(h.flags(), vec![true, true, false, false, true]);
    h.pointer(x, y, 2);
    assert_eq!(h.flags(), vec![true, true, false, false, true]);
    let command = h.commands.borrow_mut().pop().unwrap();
    let Command::ArchiveSelect {
        revision,
        view,
        action,
        ..
    } = command
    else {
        panic!()
    };
    assert!(view.by_path.is_empty());
    let ArchiveSelectionAction::Ranges { ranges, mode } = action else {
        panic!()
    };
    assert_eq!(mode, SelectionMode::Replace);
    assert_eq!(
        ranges.spans(),
        &[
            SelectionSpan { first: 0, last: 1 },
            SelectionSpan { first: 4, last: 4 }
        ]
    );
    let mut totals = ArchiveSelectionTotals::default();
    for size in [0, 1, 4] {
        totals.change(EntryKind::File, Some(size), true);
    }
    h.presenter.borrow_mut().apply(Event::ArchiveSelection {
        pane: PaneId::Left,
        navigation: h.navigation,
        revision,
        view,
        result: Ok(ArchiveSelectionResult {
            selected: Arc::new(
                [EntryId(100), EntryId(117), EntryId(168)]
                    .into_iter()
                    .collect(),
            ),
            totals,
        }),
    });
    h.draw();
    assert_eq!(h.flags(), vec![true, true, false, false, true]);
}

#[test]
fn logical_gutter_and_grid_math_survive_density_font_and_window_changes() {
    let mut h = Harness::new(5);
    for (width, height, font) in [(1340, 820, 11), (900, 640, 20), (780, 550, 20)] {
        h.ui.resize(width, height);
        h.ui.window.invoke_font_options_changed(0, font, font, 1);
        for density in [0, 1] {
            h.ui.window
                .invoke_preferences_changed(h.ui.window.get_theme_index(), density, 1);
            for mode in [0, 1, 2] {
                h.ui.window.invoke_view_mode(0, mode);
                h.draw();
                assert_eq!(h.ui.window.get_left_view_mode(), mode);
                assert_eq!(h.ui.window.get_density_index(), density);
                assert_eq!(h.ui.window.get_desktop_font_size(), font);
                assert!(
                    (h.ui.window.get_left_list_width()
                        - h.ui.window.get_left_list_grid_width()
                        - 24.0)
                        .abs()
                        < 0.02
                );
                h.start();
                let (x, y) = h.top();
                h.pointer(x, y, 1);
                h.pointer(x, y, 2);
                let request = h.range_request();
                let SelectionAction::Ranges { ranges, .. } = request.action else {
                    panic!()
                };
                assert_eq!(ranges.len(), 5);
                selection::acknowledge(&h.presenter, &h.commands, PaneId::Left, &[]);
            }
        }
    }
}

#[test]
fn navigation_during_drag_rejects_late_release_without_a_selection_command() {
    let mut h = Harness::new(5);
    h.start();
    let (x, y) = h.top();
    h.pointer(x, y, 1);
    h.ui.window.invoke_navigate(
        0,
        std::env::temp_dir()
            .join("new source")
            .to_string_lossy()
            .as_ref()
            .into(),
    );
    h.draw();
    h.pointer(x, y, 2);
    assert_eq!(h.ui.window.global::<SelectionRectangle>().get_pane(), -1);
    assert!(h.commands.borrow().iter().all(|command| !matches!(
        command,
        Command::Selection(SelectionRequest {
            action: SelectionAction::Ranges { .. },
            ..
        })
    )));
}

#[test]
fn brief_partial_columns_produce_disjoint_ranges_through_actual_cells() {
    let mut h = Harness::new(100);
    h.ui.window.invoke_view_mode(0, 1);
    h.draw();
    assert_eq!(h.ui.window.get_left_view_mode(), 1);
    let rows = h.ui.window.get_left_view_rows() as usize;
    let columns = h.ui.window.get_left_view_columns();
    assert_eq!(columns, 3);
    assert!(rows > 5);
    let left = h.ui.window.get_left_list_left();
    let top = h.ui.window.get_left_list_top();
    let width = h.ui.window.get_left_list_grid_width();
    let height = h.ui.window.get_left_view_row_height();
    let (sx, sy) = (left + width + 5.0, top + 4.5 * height);
    h.pointer(sx, sy, 1);
    h.pointer(sx, sy, 0);
    assert_eq!(h.ui.window.global::<SelectionRectangle>().get_pane(), 0);
    let (x, y) = (left + width * 0.5, top + 1.5 * height);
    h.pointer(x, y, 1);
    h.pointer(x, y, 2);
    let request = h.range_request();
    let SelectionAction::Ranges { ranges, .. } = request.action else {
        panic!()
    };
    assert_eq!(
        ranges.spans(),
        &[
            SelectionSpan {
                first: rows,
                last: rows + 3
            },
            SelectionSpan {
                first: 2 * rows,
                last: 2 * rows + 3
            }
        ]
    );
    assert!(h.flags()[rows]);
    assert!(!h.flags()[rows + 4]);
}

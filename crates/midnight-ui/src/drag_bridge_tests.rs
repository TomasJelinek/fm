//! The bridge polls quickly only while drag work is pending. Refreshes, invalidations
//! and native release, wheel and touch input republish on the next loop turn, without
//! waiting for the slow safety net.
use super::*;
use crate::{BeginOwned, DeferredDrop, OwnedCommandScope};
use midnight_core::{DirectoryIndex, Entry, EntryId, EntryKind, Event};
use slint::platform::{
    Platform, PlatformError, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use std::{path::PathBuf, sync::Arc};

/// The first adapter is the fixture's main window; later windows get their own.
struct ClockPlatform {
    window: Rc<MinimalSoftwareWindow>,
    created: Cell<bool>,
    clock: Rc<Cell<Duration>>,
}

impl Platform for ClockPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        if self.created.replace(true) {
            return Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer));
        }
        Ok(self.window.clone())
    }

    fn duration_since_start(&self) -> Duration {
        self.clock.get()
    }
}

/// A non-reentrant owned scope whose refused commands stay with the fixture.
struct Scope {
    claimed: Rc<Cell<bool>>,
    send: Rc<dyn Fn(Command) -> bool>,
    retired: Rc<RefCell<Vec<DeferredDrop>>>,
}

impl OwnedCommandScope for Scope {
    fn send(&self, command: Command) -> bool {
        (self.send)(command)
    }

    fn retire(&self, value: DeferredDrop) {
        self.retired.borrow_mut().push(value);
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        self.claimed.set(false);
    }
}

struct Fixture {
    window: MainWindow,
    native: Rc<MinimalSoftwareWindow>,
    clock: Rc<Cell<Duration>>,
    presenter: Rc<RefCell<Presenter>>,
    sent: Rc<RefCell<Vec<Command>>>,
    refuse: Rc<Cell<bool>>,
    roots: [PathBuf; 2],
    _retired: Rc<RefCell<Vec<DeferredDrop>>>,
}

impl Fixture {
    /// Two listed native panes and an attached native drop host, with time frozen.
    fn new() -> Self {
        let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        let clock = Rc::new(Cell::new(Duration::ZERO));
        slint::platform::set_platform(Box::new(ClockPlatform {
            window: native.clone(),
            created: Cell::new(false),
            clock: clock.clone(),
        }))
        .unwrap();
        let window = MainWindow::new().unwrap();
        let base = std::env::temp_dir().join("drag bridge cadence 文档");
        let roots = [base.join("left"), base.join("right")];
        let presenter = Rc::new(RefCell::new(Presenter::new(
            roots[0].clone(),
            roots[1].clone(),
        )));
        presenter
            .borrow_mut()
            .apply(Event::PreferencesLoaded(Ok(Box::default())));
        let sent = Rc::new(RefCell::new(Vec::new()));
        let refuse = Rc::new(Cell::new(false));
        let (recorded, refused) = (sent.clone(), refuse.clone());
        let send: Rc<dyn Fn(Command) -> bool> = Rc::new(move |command| {
            if refused.get() {
                return false;
            }
            recorded.borrow_mut().push(command);
            true
        });
        let retired = Rc::new(RefCell::new(Vec::new()));
        let claimed = Rc::new(Cell::new(false));
        let (scope_send, scope_retired) = (send.clone(), retired.clone());
        let begin: BeginOwned = Rc::new(move || {
            if claimed.replace(true) {
                return None;
            }
            Some(Box::new(Scope {
                claimed: claimed.clone(),
                send: scope_send.clone(),
                retired: scope_retired.clone(),
            }))
        });
        crate::install_with_owned(&window, presenter.clone(), send.clone(), begin);
        let navigations = sent
            .borrow()
            .iter()
            .filter_map(|command| match command {
                Command::Navigate {
                    pane, navigation, ..
                } => Some((*pane, *navigation)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(navigations.len(), 2, "both panes navigate on install");
        for (pane, navigation) in navigations {
            let root = &roots[pane.index()];
            let entries: Arc<[Entry]> = [
                Entry::new(EntryId(1), root.join("folder"), EntryKind::Directory),
                Entry::new(EntryId(2), root.join("file 文档.txt"), EntryKind::File),
            ]
            .into();
            presenter.borrow_mut().apply(Event::ListingFinished {
                pane,
                navigation,
                index: Arc::new(DirectoryIndex::from_entries(&entries)),
                entries,
            });
        }
        // A software window has no native handle; attach one as the native bridge does.
        presenter
            .borrow_mut()
            .attach_drag_host(NativeDragWindow::Windows { hwnd: 1 }, send.as_ref());
        crate::refresh(&window, &presenter.borrow());
        window.show().unwrap();
        native.set_size(slint::PhysicalSize::new(1000, 700));
        window
            .window()
            .dispatch_event(slint::platform::WindowEvent::Resized {
                size: slint::LogicalSize::new(1000.0, 700.0),
            });
        let fixture = Self {
            window,
            native,
            clock,
            presenter,
            sent,
            refuse,
            roots,
            _retired: retired,
        };
        fixture.frame();
        fixture
    }

    /// Run due timers and change handlers, then render, without advancing time.
    fn frame(&self) {
        slint::platform::update_timers_and_animations();
        let mut pixels = vec![slint::Rgb8Pixel::default(); 1000 * 700];
        self.native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 1000);
        });
        slint::platform::update_timers_and_animations();
    }

    fn advance(&self, by: Duration) {
        self.clock.set(self.clock.get() + by);
        self.frame();
    }

    fn interval(&self) -> Duration {
        bridge(&self.window).unwrap().timer.interval()
    }

    fn targets(&self) -> (u64, Vec<PaneId>) {
        let p = self.presenter.borrow();
        let snapshot = p.drag_drop.targets.lock().unwrap();
        (
            snapshot.revision,
            snapshot.targets.iter().map(|target| target.pane).collect(),
        )
    }

    fn hover(&self, pane: Option<PaneId>) {
        let host_generation = self
            .presenter
            .borrow()
            .drag_drop
            .host
            .as_ref()
            .unwrap()
            .generation;
        self.presenter.borrow_mut().apply_with_sender(
            Event::DragDrop(DragDropEvent::Hover {
                host_generation,
                pane,
            }),
            &|_| true,
        );
        crate::refresh(&self.window, &self.presenter.borrow());
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.window.hide();
    }
}

#[test]
fn an_idle_bridge_polls_slowly_and_a_row_drag_polls_quickly_until_it_ends() {
    let fixture = Fixture::new();
    assert_eq!(fixture.interval(), IDLE_POLL);
    // Ten idle seconds cost forty safety-net turns instead of 625 polls.
    for _ in 0..40 {
        fixture.advance(IDLE_POLL);
    }
    assert_eq!(fixture.interval(), IDLE_POLL);

    let (row, identity) = {
        let p = fixture.presenter.borrow();
        let pane = &p.panes[0];
        let entry = pane.display_entry(1).unwrap();
        (
            pane.parent_offset() + 1,
            slint::SharedString::from(pane.row_identity(entry)),
        )
    };
    assert!(
        fixture
            .window
            .invoke_row_drag(0, row as i32, identity, 10.0, 10.0)
    );
    assert_eq!(fixture.interval(), ACTIVE_POLL, "a gesture polls quickly");
    // A full queue refuses the cancellation; it is retried at the quick cadence.
    fixture.refuse.set(true);
    assert!(fixture.window.invoke_drag_escape());
    assert_eq!(fixture.interval(), ACTIVE_POLL);
    fixture.advance(ACTIVE_POLL);
    assert_eq!(fixture.interval(), ACTIVE_POLL, "still refused");
    fixture.refuse.set(false);
    fixture.advance(ACTIVE_POLL);
    assert!(
        fixture
            .sent
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::CancelDrag(_)))
    );
    assert_eq!(
        fixture.interval(),
        IDLE_POLL,
        "an ended gesture polls slowly"
    );
}

#[test]
fn invalidation_republishes_targets_on_the_next_turn_and_unchanged_refreshes_keep_them() {
    let fixture = Fixture::new();
    let (published, panes) = fixture.targets();
    assert_eq!(panes, [PaneId::Left, PaneId::Right]);
    fixture.window.invoke_drag_invalidate_targets();
    let (invalidated, panes) = fixture.targets();
    assert!(panes.is_empty());
    assert!(invalidated > published);
    // No time passes: only the zero-delay turn can republish.
    slint::platform::update_timers_and_animations();
    let (republished, panes) = fixture.targets();
    assert_eq!(panes, [PaneId::Left, PaneId::Right]);
    assert!(republished > invalidated);

    crate::refresh(&fixture.window, &fixture.presenter.borrow());
    slint::platform::update_timers_and_animations();
    assert_eq!(
        fixture.targets(),
        (republished, vec![PaneId::Left, PaneId::Right]),
        "an unchanged refresh keeps the published revision"
    );

    // Navigation marks the left pane loading; its refresh withdraws that target.
    let left = fixture.roots[0].to_string_lossy().into_owned();
    fixture.window.invoke_navigate(0, left.into());
    slint::platform::update_timers_and_animations();
    assert_eq!(fixture.targets().1, [PaneId::Right]);
}

#[test]
fn native_release_wheel_and_touch_republish_without_invalidating() {
    use slint::winit_030::winit::{
        dpi::PhysicalPosition,
        event::{
            DeviceId, ElementState, MouseButton, MouseScrollDelta, Touch, TouchPhase,
            WindowEvent as Native,
        },
    };
    let fixture = Fixture::new();
    let events = [
        Native::MouseInput {
            device_id: DeviceId::dummy(),
            state: ElementState::Released,
            button: MouseButton::Left,
        },
        Native::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::LineDelta(0.0, -1.0),
            phase: TouchPhase::Moved,
        },
        Native::Touch(Touch {
            device_id: DeviceId::dummy(),
            phase: TouchPhase::Started,
            location: PhysicalPosition::new(10.0, 10.0),
            force: None,
            id: 1,
        }),
    ];
    for event in events {
        // The slow net is far away; a stale region must change on the next turn.
        let (revision, _) = fixture.targets();
        fixture.window.set_scroll_interaction_active(true);
        observe_native(&fixture.window, &event);
        assert_eq!(
            fixture.targets().0,
            revision,
            "{event:?} does not invalidate"
        );
        slint::platform::update_timers_and_animations();
        assert_eq!(
            fixture.targets().1,
            [],
            "{event:?} withdraws scrolled regions"
        );
        fixture.window.set_scroll_interaction_active(false);
        observe_native(&fixture.window, &event);
        slint::platform::update_timers_and_animations();
        assert_eq!(fixture.targets().1, [PaneId::Left, PaneId::Right]);
    }
}

#[test]
fn external_hover_is_presented_on_the_next_turn_and_polls_quickly_while_it_lasts() {
    let fixture = Fixture::new();
    assert_eq!(fixture.window.get_drag_hover_pane(), -1);
    fixture.hover(Some(PaneId::Right));
    slint::platform::update_timers_and_animations();
    assert_eq!(fixture.window.get_drag_hover_pane(), 1);
    assert_eq!(fixture.interval(), ACTIVE_POLL);
    fixture.hover(None);
    slint::platform::update_timers_and_animations();
    assert_eq!(fixture.window.get_drag_hover_pane(), -1);
    assert_eq!(fixture.interval(), IDLE_POLL);
}

#[test]
fn a_refused_drag_command_keeps_the_quick_retry_until_it_is_sent() {
    let fixture = Fixture::new();
    // A new host generation must be announced; the full queue refuses it.
    fixture.refuse.set(true);
    fixture
        .presenter
        .borrow_mut()
        .attach_drag_host(NativeDragWindow::Windows { hwnd: 2 }, &|_| false);
    fixture.window.invoke_drag_invalidate_targets();
    slint::platform::update_timers_and_animations();
    assert_eq!(fixture.interval(), ACTIVE_POLL);
    fixture.advance(ACTIVE_POLL);
    assert_eq!(fixture.interval(), ACTIVE_POLL, "still refused");
    fixture.refuse.set(false);
    fixture.advance(ACTIVE_POLL);
    assert!(fixture.sent.borrow().iter().any(|command| matches!(
        command,
        Command::AttachDragDrop { host, .. } if host.native == NativeDragWindow::Windows { hwnd: 2 }
    )));
    assert_eq!(fixture.interval(), IDLE_POLL);
}

#[test]
fn bridges_of_two_windows_on_one_thread_republish_only_their_own_targets() {
    let first = Fixture::new();
    let second_window = MainWindow::new().unwrap();
    let second = Rc::new(RefCell::new(Presenter::new(
        first.roots[0].clone(),
        first.roots[1].clone(),
    )));
    crate::install(&second_window, second.clone(), Rc::new(|_| true));
    // Installation refreshes the second window; let its own turn run first.
    slint::platform::update_timers_and_animations();
    let first_bridge = bridge(&first.window).unwrap();
    let second_bridge = bridge(&second_window).unwrap();
    assert!(!Rc::ptr_eq(&first_bridge, &second_bridge));
    assert!(first_bridge.owns(&first.window) && !first_bridge.owns(&second_window));
    assert!(!first_bridge.turn_scheduled.get() && !second_bridge.turn_scheduled.get());
    first.window.invoke_drag_invalidate_targets();
    assert!(first_bridge.turn_scheduled.get());
    assert!(!second_bridge.turn_scheduled.get());
    slint::platform::update_timers_and_animations();
    assert_eq!(first.targets().1, [PaneId::Left, PaneId::Right]);
}

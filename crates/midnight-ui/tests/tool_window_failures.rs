//! Native adapter refusal and release-queue backpressure must not recreate hidden windows.
use midnight_core::*;
use midnight_ui::{MainWindow, Presenter};
use slint::{
    ComponentHandle,
    platform::{
        Platform, PlatformError, WindowAdapter,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

struct FailedTools {
    main: Rc<MinimalSoftwareWindow>,
    attempts: Rc<Cell<usize>>,
    clock: Rc<Cell<Duration>>,
}
impl Platform for FailedTools {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let count = self.attempts.get() + 1;
        self.attempts.set(count);
        if count == 1 {
            Ok(self.main.clone())
        } else {
            Err(PlatformError::Other(
                "fixture native tool window refused".into(),
            ))
        }
    }
    fn duration_since_start(&self) -> Duration {
        self.clock.get()
    }
}
struct Harness {
    main: MainWindow,
    state: Rc<RefCell<Presenter>>,
    attempts: Rc<Cell<usize>>,
    releases: Rc<Cell<usize>>,
    accepting: Rc<Cell<bool>>,
    clock: Rc<Cell<Duration>>,
}
impl Harness {
    fn new() -> Self {
        let attempts = Rc::new(Cell::new(0));
        let releases = Rc::new(Cell::new(0));
        let accepting = Rc::new(Cell::new(false));
        let clock = Rc::new(Cell::new(Duration::ZERO));
        slint::platform::set_platform(Box::new(FailedTools {
            main: MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer),
            attempts: attempts.clone(),
            clock: clock.clone(),
        }))
        .unwrap();
        let main = MainWindow::new().unwrap();
        let root = std::env::temp_dir().join("tool failure 文档");
        let state = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
        let commands = Rc::new(RefCell::new(Vec::new()));
        midnight_ui::install(
            &main,
            state.clone(),
            Rc::new({
                let commands = commands.clone();
                let releases = releases.clone();
                let accepting = accepting.clone();
                move |command| {
                    if matches!(
                        command,
                        Command::ReleaseChecksums { .. } | Command::CloseFind(_)
                    ) {
                        releases.set(releases.get() + 1);
                        return accepting.get();
                    }
                    commands.borrow_mut().push(command);
                    true
                }
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
        let entries: Arc<[Entry]> = vec![Entry::new(
            EntryId(7),
            root.join("source.txt"),
            EntryKind::File,
        )]
        .into();
        state.borrow_mut().apply(Event::ListingFinished {
            pane: PaneId::Left,
            navigation,
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        });
        main.show().unwrap();
        midnight_ui::refresh(&main, &state.borrow());
        main.invoke_row_click(0, 1, false, false);
        Self {
            main,
            state,
            attempts,
            releases,
            accepting,
            clock,
        }
    }
    fn tick(&self, millis: u64) {
        self.clock.set(Duration::from_millis(millis));
        slint::platform::update_timers_and_animations();
    }
}

#[test]
fn failed_checksum_native_window_retries_only_release_at_bounded_rate_until_accepted() {
    let h = Harness::new();
    h.main.invoke_calculate_checksums(0);
    assert_eq!(h.attempts.get(), 2);
    h.tick(0);
    assert_eq!(h.releases.get(), 1);
    let report = h.main.get_message_generation();
    for _ in 0..16 {
        h.tick(0);
        midnight_ui::refresh(&h.main, &h.state.borrow());
    }
    assert_eq!(
        h.attempts.get(),
        3,
        "the failed tool and its error notice are each attempted only once"
    );
    assert_eq!(h.releases.get(), 1, "no zero-delay release retry loop");
    h.tick(249);
    assert_eq!(h.releases.get(), 1);
    h.tick(250);
    assert_eq!(h.releases.get(), 2);
    assert_eq!(
        h.main.get_message_generation(),
        report,
        "native failure is reported once"
    );
    let guard = h.state.borrow_mut();
    h.tick(500);
    drop(guard);
    assert_eq!(
        h.releases.get(),
        2,
        "a borrowed Presenter defers release safely"
    );
    h.accepting.set(true);
    h.tick(750);
    assert_eq!(h.releases.get(), 3);
    for _ in 0..8 {
        midnight_ui::refresh(&h.main, &h.state.borrow());
    }
    h.tick(2000);
    assert_eq!(h.attempts.get(), 3);
    assert_eq!(h.releases.get(), 3);
    h.main.invoke_close_checksum_windows();
    h.main.hide().unwrap();
}

#[test]
fn host_shutdown_stops_failed_checksum_retries_and_initial_find_failure_closes_locally() {
    let h = Harness::new();
    h.main.invoke_calculate_checksums(0);
    h.tick(0);
    assert_eq!(h.releases.get(), 1);
    h.main.invoke_close_checksum_windows();
    h.tick(1000);
    assert_eq!(h.releases.get(), 1);
    assert_eq!(h.attempts.get(), 3);
    h.main.invoke_find_files(0);
    assert_eq!(h.attempts.get(), 4);
    h.tick(1000);
    for _ in 0..8 {
        h.tick(1000);
        midnight_ui::refresh(&h.main, &h.state.borrow());
    }
    assert_eq!(
        h.attempts.get(),
        5,
        "unstarted Find and its notice each fail once without recreating either host"
    );
    h.main.invoke_close_find_windows();
    h.main.hide().unwrap();
}

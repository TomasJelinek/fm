//! Real UI -> Engine -> native filesystem fixture; never reads or saves user preferences.
#![allow(dead_code)]
#[path = "../../../midnight-explorer/src/command_reclaim.rs"]
mod command_reclaim;
#[path = "modeless.rs"]
mod modeless;
#[path = "engine_teardown.rs"]
mod teardown;
use midnight_core::*;
use midnight_engine::{Engine, reclaim};
use midnight_platform::LocalFileSystem;
use midnight_ui::{
    BeginOwned, DeferredDrop, OperationPresentation, OwnedCommandScope, Presenter,
    install_with_owned, refresh, with_owned_commands,
};
use slint::{
    ComponentHandle, Model,
    platform::{WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{
    cell::{Cell, RefCell},
    fs,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{runtime::Runtime, sync::mpsc, task::JoinHandle};

struct Settings(UserPreferences);

struct ObservedScope {
    inner: Box<dyn OwnedCommandScope>,
    internal_drops: Rc<Cell<usize>>,
}

impl OwnedCommandScope for ObservedScope {
    fn send(&self, command: Command) -> bool {
        if matches!(&command, Command::InternalDrop(_)) {
            self.internal_drops.set(self.internal_drops.get() + 1);
        }
        self.inner.send(command)
    }

    fn retire(&self, value: DeferredDrop) {
        self.inner.retire(value);
    }
}
impl SettingsService for Settings {
    fn load_preferences(&self) -> Result<UserPreferences, FsError> {
        Ok(self.0.clone())
    }
    fn save_preferences(&self, _: UserPreferences) -> Result<(), FsError> {
        Ok(())
    }

    fn load_preferences_for_acceptance(&self) -> Result<UserPreferences, FsError> {
        self.load_preferences()
    }

    fn save_preferences_prepared(
        &self,
        preferences: UserPreferences,
        proof: ComparatorValidationProof<'_>,
    ) -> Result<(), FsError> {
        if !proof.matches(&preferences.file_comparators) {
            return Err(FsError::from_io(
                "save settings",
                std::path::Path::new(""),
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Comparator validation proof does not match preferences.",
                ),
            ));
        }
        self.save_preferences(preferences)
    }
}

pub struct Harness {
    pub ui: super::ui::TestUi,
    windows: Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>,
    pub root: PathBuf,
    pub left: PathBuf,
    pub right: PathBuf,
    pub presenter: Rc<RefCell<Presenter>>,
    pub copies: Vec<Arc<CopyProgress>>,
    pub operations: Vec<Arc<OperationProgress>>,
    pub prepared_drags: usize,
    pub last_prepared_drag: Option<DragId>,
    pub finished_drags: Vec<(DragId, DragFinish)>,
    pub internal_drops: Rc<Cell<usize>>,
    pub send: Rc<dyn Fn(Command) -> bool>,
    events: mpsc::Receiver<Event>,
    commands: midnight_engine::CommandSender,
    begin_owned: BeginOwned,
    command_reclaim: command_reclaim::CommandReclaim,
    retire_queue: mpsc::Sender<reclaim::Retired>,
    committed: [Option<NavigationId>; 2],
    runtime: Option<Runtime>,
    engine: Option<JoinHandle<()>>,
}

impl Harness {
    pub fn new() -> Self {
        let parent = std::env::var_os("MIDNIGHT_NATIVE_TEST_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../_build/qa/operations-find")
            });
        fs::create_dir_all(&parent).unwrap();
        let parent = parent.canonicalize().unwrap();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        // Parallel tests can read the same clock tick: number the collisions.
        let root = (0..64)
            .find_map(|attempt| {
                let root = parent.join(format!(
                    "native-ui-{}-{stamp}-{attempt}",
                    std::process::id()
                ));
                match fs::create_dir(&root) {
                    Ok(()) => Some(root),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                    Err(error) => panic!("cannot create {}: {error}", root.display()),
                }
            })
            .expect("a unique native fixture name");
        assert_eq!(root.parent(), Some(parent.as_path()));
        let left = root.join("source");
        let right = root.join("destination");
        fs::create_dir_all(left.join("inside")).unwrap();
        fs::create_dir_all(right.join("target folder")).unwrap();
        fs::create_dir_all(left.join("nested/empty")).unwrap();
        for index in 0..16 {
            fs::write(
                left.join(format!("file-{index:02}.bin")),
                vec![index as u8; 4096 + index],
            )
            .unwrap();
        }
        fs::write(left.join("část 文档 🦀.txt"), "Owned Unicode source").unwrap();
        fs::write(left.join("nested/child.txt"), "Owned nested source").unwrap();
        fs::write(root.join("untouched-sentinel.txt"), "Must remain unchanged").unwrap();
        eprintln!("REAL UI FIXTURE: {}", root.display());

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();

        let (events_send, events) = mpsc::channel(64);
        let service = Arc::new(LocalFileSystem::new(None));
        let mut preferences = UserPreferences::default();
        preferences.tabs[0].paths = vec![left.clone()];
        preferences.tabs[1].paths = vec![right.clone()];
        let mut engine = Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            service.clone(),
            events_send,
        );
        engine
            .configure_audited_transfers()
            .unwrap()
            .install(service.clone());
        engine
            .configure_mutations()
            .unwrap()
            .install(service.clone());
        // As production does: Copy/Move conflicts need the audited slot.
        engine.configure_audited_locale().unwrap().install(service);
        engine
            .configure_settings()
            .unwrap()
            .install(Arc::new(Settings(preferences)));
        let (commands, receive) = engine.command_channel(64).unwrap();
        let retire_queue = reclaim::start(runtime.handle(), Arc::new(|| {}));
        let command_reclaim = command_reclaim::CommandReclaim::new(retire_queue.clone());
        let engine = runtime.spawn(engine.run(receive));
        let windows = Rc::new(RefCell::new(Vec::new()));
        slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
        let window = midnight_ui::MainWindow::new().unwrap();
        window.set_preferences_ready(true);
        midnight_ui::install_file_keys(&window);
        window.show().unwrap();
        window
            .window()
            .dispatch_event(WindowEvent::WindowActiveChanged(true));
        let native = windows.borrow()[0].clone();
        let mut ui = super::ui::TestUi {
            window,
            native,
            pixels: Vec::new(),
            width: 1340,
            height: 820,
        };
        ui.resize(1340, 820);
        let presenter = Rc::new(RefCell::new(Presenter::new(left.clone(), right.clone())));
        let sender = commands.clone();
        let internal_drops = Rc::new(Cell::new(0));
        let begin = command_reclaim.factory(commands.clone());
        let observed_drops = internal_drops.clone();
        let begin_owned: BeginOwned = Rc::new(move || {
            Some(Box::new(ObservedScope {
                inner: begin()?,
                internal_drops: observed_drops.clone(),
            }))
        });
        let drops = internal_drops.clone();
        let begin_controls = begin_owned.clone();
        let send: Rc<dyn Fn(Command) -> bool> = Rc::new(move |command| {
            if let Some(scope) = begin_controls() {
                return scope.send(command);
            }
            // Only provider-free controls may use the unavailable-claim fallback.
            if matches!(&command, Command::InternalDrop(_)) {
                drops.set(drops.get() + 1);
            }
            sender.try_send(command).is_ok()
        });
        install_with_owned(
            &ui.window,
            presenter.clone(),
            send.clone(),
            begin_owned.clone(),
        );
        let mut h = Self {
            ui,
            windows,
            root,
            left,
            right,
            presenter,
            copies: vec![],
            operations: vec![],
            prepared_drags: 0,
            last_prepared_drag: None,
            finished_drags: vec![],
            internal_drops,
            send,
            events,
            commands,
            begin_owned,
            command_reclaim,
            retire_queue,
            committed: [None; 2],
            runtime: Some(runtime),
            engine: Some(engine),
        };
        h.until(|h| {
            !h.ui.window.get_left_loading()
                && !h.ui.window.get_right_loading()
                && h.rows(PaneId::Left)
                    .iter()
                    .any(|row| row.name == "file-00.bin")
                && h.rows(PaneId::Right)
                    .iter()
                    .any(|row| row.name == "target folder")
        });
        h
    }

    pub fn dialog(&mut self) -> Rc<MinimalSoftwareWindow> {
        // Settle the embedded popup on Main without resizing its adapter away
        // from TestUi's current drawing buffer and layout dimensions.
        self.ui.draw();
        let children: Vec<_> = self
            .windows
            .borrow()
            .iter()
            .skip(1)
            .filter(|window| window.window().is_visible())
            .cloned()
            .collect();
        assert_eq!(
            children.len(),
            0,
            "blocking operation confirmations must share the main adapter"
        );
        assert!(
            self.ui.window.get_embedded_modal_open(),
            "the embedded confirmation must gate background application input"
        );
        assert!(
            !self.ui.window.get_dialog_input_blocked(),
            "the main input host must stay enabled for the embedded confirmation"
        );
        let form = self.ui.window.global::<OperationPresentation>();
        assert!(form.get_active() && !form.get_token().is_empty());
        assert_eq!(
            i_slint_core::window::WindowInner::from_pub(self.ui.window.window())
                .active_popups()
                .iter()
                .filter(|popup| !matches!(
                    popup.window_kind,
                    i_slint_core::window::WindowKind::ToolTip
                ))
                .count(),
            1,
            "the confirmation must be the only non-tooltip popup on Main"
        );
        self.ui.native.clone()
    }

    /// Lay out the newest visible modeless window (an operation decision) and click
    /// its control.
    pub fn click_decision(&mut self, label: &str) {
        let decision = self
            .windows
            .borrow()
            .iter()
            .skip(1)
            .rev()
            .find(|window| window.window().is_visible())
            .cloned()
            .expect("a visible decision window");
        modeless::layout(&decision, 900, 650);
        modeless::click(&decision, label);
        self.settle();
    }

    pub fn rows(&self, pane: PaneId) -> Vec<midnight_ui::FileRow> {
        match pane {
            PaneId::Left => self.ui.window.get_left_rows(),
            PaneId::Right => self.ui.window.get_right_rows(),
        }
        .iter()
        .collect()
    }

    pub fn focus(&mut self, pane: PaneId, name: &str, toggle: bool) {
        let index = self
            .rows(pane)
            .iter()
            .position(|row| row.name == name)
            .unwrap();
        self.ui.window.invoke_activate(pane.index() as i32);
        self.ui
            .window
            .invoke_row_click(pane.index() as i32, index as i32, toggle, false);
        self.ui.window.invoke_focus_files();
        self.settle();
    }

    fn apply(&mut self, event: Event, presenter: &mut Presenter, send: &dyn Fn(Command) -> bool) {
        match &event {
            Event::Copy(CopyEvent::Progress(progress)) => self.copies.push(progress.clone()),
            Event::Operation(OperationEvent::Progress(progress)) => {
                self.operations.push(progress.clone())
            }
            Event::DragDrop(DragDropEvent::Prepared { id, .. }) => {
                self.prepared_drags += 1;
                self.last_prepared_drag = Some(*id);
            }
            Event::DragDrop(DragDropEvent::Finished { id, outcome }) => {
                self.finished_drags.push((*id, *outcome));
            }
            _ => {}
        }
        if let Event::DirectoryChanged { pane, navigation } = event {
            presenter.refresh_if_changed(pane, navigation, send);
        } else {
            presenter.apply_with_sender(event, send);
        }
    }

    fn drain_one(&mut self) -> bool {
        let presenter = self.presenter.clone();
        let begin = self.begin_owned.clone();
        let consumed = with_owned_commands(&presenter, &begin, |presenter, send| {
            let event = match self.events.try_recv() {
                Ok(event) => event,
                Err(mpsc::error::TryRecvError::Empty) => return false,
                // A dead Engine is a failure, not "no events yet".
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    panic!("Engine event channel closed before the UI settled");
                }
            };
            self.apply(event, presenter, send);
            true
        }) == Some(true);
        self.command_reclaim.flush();
        if consumed {
            refresh(&self.ui.window, &self.presenter.borrow());
        }
        consumed
    }

    pub fn settle(&mut self) {
        for _ in 0..4 {
            let mut consumed = false;
            for _ in 0..128 {
                if !self.drain_one() {
                    break;
                }
                consumed = true;
            }
            let _ = with_owned_commands(&self.presenter, &self.begin_owned, |presenter, send| {
                presenter.flush_selection(send);
                presenter.flush_search(send);
            });
            self.sync_visible();
            refresh(&self.ui.window, &self.presenter.borrow());
            self.ui.draw();
            if !consumed {
                // Further rounds would repeat this idle one; `until` polls again.
                break;
            }
        }
    }

    fn sync_visible(&mut self) {
        // Claim before cloning a visible listing, then acknowledge through the
        // same typed sender as the installed operation callbacks.
        for pane in [PaneId::Left, PaneId::Right] {
            let committed = &mut self.committed;
            if with_owned_commands(&self.presenter, &self.begin_owned, |presenter, send| {
                if let Some(visible) = presenter.visible_directory(pane) {
                    let navigation = visible.navigation;
                    if committed[pane.index()] != Some(navigation)
                        && send(Command::PaneCommitted(visible))
                    {
                        committed[pane.index()] = Some(navigation);
                    }
                }
            })
            .is_none()
            {
                break;
            }
        }
    }

    #[track_caller]
    pub fn until(&mut self, condition: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + teardown::UI_WAIT;
        loop {
            self.settle();
            if condition(self) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "UI timed out after {:?}: left={} right={} notice={} copy={:?} operation={:?}",
                teardown::UI_WAIT,
                self.ui.window.get_left_error(),
                self.ui.window.get_right_error(),
                self.ui.window.get_message_title(),
                self.copies.last(),
                self.operations.last()
            );
            // Wait without taking an event or holding a claim across the await.
            self.runtime.as_ref().unwrap().block_on(async {
                tokio::time::sleep(Duration::from_millis(10)).await;
            });
        }
    }

    pub fn copy_finished(&mut self, after: usize) -> Arc<CopyProgress> {
        self.until(|h| h.copies.iter().skip(after).any(|p| p.state.is_terminal()));
        self.copies
            .iter()
            .skip(after)
            .find(|p| p.state.is_terminal())
            .unwrap()
            .clone()
    }

    pub fn assert_sentinel(&self) {
        assert_eq!(
            fs::read(self.root.join("untouched-sentinel.txt")).unwrap(),
            b"Must remain unchanged"
        );
    }

    pub fn shutdown(&mut self) {
        let Some(runtime) = self.runtime.take() else {
            return;
        };
        self.presenter.borrow_mut().retire_all();
        let mut retired = self.presenter.borrow_mut().take_retired();
        if let Some(pending) = self.command_reclaim.take_ready() {
            retired.extend(pending);
        }
        // Never panic before the final assertion: this usually runs while a failed
        // assertion unwinds, and a second panic would abort the test binary.
        let mut errors = Vec::new();
        let result = match self.engine.take() {
            Some(mut engine) => {
                let (copies, operations) = (&mut self.copies, &mut self.operations);
                let result = runtime.block_on(async {
                    if let Err(error) = teardown::retire_final(&self.retire_queue, retired).await {
                        errors.push(error);
                    }
                    teardown::shutdown(
                        &self.commands,
                        &mut engine,
                        &mut self.events,
                        &self.retire_queue,
                        |event| match event {
                            Event::Copy(CopyEvent::Progress(progress)) => {
                                copies.push(progress.clone());
                            }
                            Event::Operation(OperationEvent::Progress(progress)) => {
                                operations.push(progress.clone());
                            }
                            _ => {}
                        },
                    )
                    .await
                });
                if result.is_err() {
                    engine.abort();
                }
                result
            }
            None => Err("the Engine task was already taken".to_owned()),
        };
        let (_, empty) = mpsc::channel(1);
        let receiver = std::mem::replace(&mut self.events, empty);
        if let Err(error) =
            runtime.block_on(teardown::retire_remaining(&self.retire_queue, receiver))
        {
            errors.push(error);
        }
        runtime.shutdown_timeout(teardown::RUNTIME_SHUTDOWN);
        if !std::thread::panicking() {
            assert!(
                result.is_ok() && errors.is_empty(),
                "Engine shutdown did not complete: {result:?}, {errors:?}"
            );
            self.assert_sentinel();
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.shutdown();
        // Tests may inspect the files after an explicit shutdown; the owned fixture
        // is removed only when the harness goes away, and kept when a test failed.
        if !std::thread::panicking()
            && let Err(error) = fs::remove_dir_all(&self.root)
        {
            eprintln!(
                "could not remove native UI fixture {}: {error}",
                self.root.display()
            );
        }
    }
}

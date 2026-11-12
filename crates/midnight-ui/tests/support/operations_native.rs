//! Real UI -> Engine -> native filesystem fixture; never reads or saves user preferences.
#![allow(dead_code)]
#[path = "modeless.rs"]
mod modeless;
use midnight_core::*;
use midnight_engine::Engine;
use midnight_platform::LocalFileSystem;
use midnight_ui::{Presenter, install, refresh};
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
impl SettingsService for Settings {
    fn load_preferences(&self) -> Result<UserPreferences, FsError> {
        Ok(self.0.clone())
    }
    fn save_preferences(&self, _: UserPreferences) -> Result<(), FsError> {
        Ok(())
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
    commands: mpsc::Sender<Command>,
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
        let root = parent.join(format!("native-ui-{}-{stamp}", std::process::id()));
        fs::create_dir(&root).unwrap();
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
        let (commands, receive) = mpsc::channel(64);
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
            .configure_transfers()
            .unwrap()
            .install(service.clone());
        engine
            .configure_mutations()
            .unwrap()
            .install(service.clone());
        engine.configure_locale().unwrap().install(service);
        engine
            .configure_settings()
            .unwrap()
            .install(Arc::new(Settings(preferences)));
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
        let drops = internal_drops.clone();
        let send: Rc<dyn Fn(Command) -> bool> = Rc::new(move |command| {
            if matches!(&command, Command::InternalDrop(_)) {
                drops.set(drops.get() + 1);
            }
            sender.try_send(command).is_ok()
        });
        install(&ui.window, presenter.clone(), send.clone());
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

    pub fn dialog(&self) -> Rc<MinimalSoftwareWindow> {
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
        assert!(self.ui.window.get_dialog_input_blocked());
        assert!(
            !i_slint_core::window::WindowInner::from_pub(self.ui.window.window())
                .active_popups()
                .is_empty()
        );
        let child = self.windows.borrow()[0].clone();
        modeless::draw(&child, 640, 420, "native-filesystem-operation");
        child
            .window()
            .dispatch_event(WindowEvent::WindowActiveChanged(true));
        child
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

    fn apply(&mut self, event: Event) {
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
            self.presenter
                .borrow_mut()
                .refresh_if_changed(pane, navigation, self.send.as_ref());
        } else {
            self.presenter
                .borrow_mut()
                .apply_with_sender(event, self.send.as_ref());
        }
        refresh(&self.ui.window, &self.presenter.borrow());
    }

    pub fn settle(&mut self) {
        for _ in 0..4 {
            for _ in 0..128 {
                let Ok(event) = self.events.try_recv() else {
                    break;
                };
                self.apply(event);
            }
            {
                let mut presenter = self.presenter.borrow_mut();
                presenter.flush_selection(self.send.as_ref());
                presenter.flush_search(self.send.as_ref());
                let retired = presenter.take_retired();
                self.runtime
                    .as_ref()
                    .unwrap()
                    .spawn_blocking(move || drop(retired));
            }
            self.sync_visible();
            self.ui.draw();
        }
    }

    fn sync_visible(&mut self) {
        // Match the app's VisibleSync: native operations require the exact
        // displayed snapshot, acknowledged on the same FIFO command channel.
        // Retry a full queue without marking an unaccepted snapshot delivered.
        for pane in [PaneId::Left, PaneId::Right] {
            if let Some(visible) = self.presenter.borrow().visible_directory(pane) {
                let navigation = visible.navigation;
                if self.committed[pane.index()] != Some(navigation)
                    && (self.send)(Command::PaneCommitted(visible))
                {
                    self.committed[pane.index()] = Some(navigation);
                }
            }
        }
    }

    #[track_caller]
    pub fn until(&mut self, condition: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            self.settle();
            if condition(self) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "UI timed out: left={} right={} notice={} copy={:?} operation={:?}",
                self.ui.window.get_left_error(),
                self.ui.window.get_right_error(),
                self.ui.window.get_message_title(),
                self.copies.last(),
                self.operations.last()
            );
            if let Ok(Some(event)) = self.runtime.as_ref().unwrap().block_on(async {
                tokio::time::timeout(Duration::from_millis(10), self.events.recv()).await
            }) {
                self.apply(event);
            }
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
        let retired = self.presenter.borrow_mut().take_retired();
        runtime.spawn_blocking(move || drop(retired));
        let mut engine = self.engine.take().unwrap();
        let result = runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(8), async {
                self.commands.send(Command::Shutdown).await.unwrap();
                loop {
                    tokio::select! {
                        result = &mut engine => return result,
                        event = self.events.recv() => {
                            if let Some(event) = event {
                                match event {
                                    Event::Copy(CopyEvent::Progress(progress)) => self.copies.push(progress),
                                    Event::Operation(OperationEvent::Progress(progress)) => self.operations.push(progress),
                                    _ => {}
                                }
                            } else { return (&mut engine).await; }
                        }
                    }
                }
            }).await
        });
        if result.is_err() {
            engine.abort();
        }
        self.events.close();
        runtime.shutdown_timeout(Duration::from_secs(3));
        if !std::thread::panicking() {
            assert!(
                matches!(result, Ok(Ok(()))),
                "Engine shutdown did not complete: {result:?}"
            );
            self.assert_sentinel();
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.shutdown();
        // Retain only this owned fixture as inspectable QA evidence.
    }
}

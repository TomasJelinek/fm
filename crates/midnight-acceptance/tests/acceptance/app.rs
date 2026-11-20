//! The application under test: the real MainWindow, Presenter and Engine with the
//! native filesystem services the application installs, on software-rendered windows.
//!
//! The wiring follows `crates/midnight-explorer/src/main.rs` (`run_window`): the same
//! Engine services where they work headless, `install_with_owned`, the owned command
//! scope and the event pump of its `backend_ready` callback. Differences, all
//! deliberate and documented in docs/developer/ACCEPTANCE.md: settings are in memory
//! (the user's profile is never read or written), browsing uses the host-native
//! `LocalFileSystem` service rather than the installed native filesystem plugin, and
//! desktop launching, clipboard and drag-and-drop are not installed. Editors and user
//! commands are opt-in (`Config::editor`, `Config::user_commands`), and
//! `Config::plugins` opts a staged viewer scenario into the installed native filesystem
//! plugin (archives included), viewers, comparators and thumbnails (`viewers_support`).
// Production's owned-command reclaimer. Test targets compile with cfg(test), so its
// unit-test module comes along (never run here); allow what that leaves unused.
#[allow(dead_code, unused_imports)]
#[path = "../../../midnight-explorer/src/command_reclaim.rs"]
mod command_reclaim;

use super::{Context, report};
use midnight_core::*;
use midnight_engine::{Engine, reclaim};
use midnight_platform::LocalFileSystem;
use midnight_ui::{
    BeginOwned, DeferredDrop, MainWindow, OwnedCommandScope, Presenter, install_with_owned,
    refresh, with_owned_commands,
};
use slint::{
    ComponentHandle, Model,
    platform::{
        Platform, PlatformError, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{
    cell::RefCell,
    collections::VecDeque,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{runtime::Runtime, sync::mpsc, task::JoinHandle};

/// One wait for UI or Engine progress. Waits end as soon as their condition holds.
pub const WAIT: Duration = Duration::from_secs(60);
const ENGINE_SHUTDOWN: Duration = Duration::from_secs(8);

type Windows = Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>;

/// Every window the application creates gets a software adapter recorded here.
struct SoftwarePlatform(Windows);

impl Platform for SoftwarePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        self.0.borrow_mut().push(window.clone());
        Ok(window)
    }
}

thread_local! {
    // Slint's platform is per thread and can be set once; later Apps on the same
    // thread reuse it with a cleared window list.
    static PLATFORM: RefCell<Option<Windows>> = const { RefCell::new(None) };
}

fn platform_windows() -> Windows {
    PLATFORM.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(windows) = slot.as_ref() {
            windows.borrow_mut().clear();
            return windows.clone();
        }
        let windows: Windows = Rc::default();
        slint::platform::set_platform(Box::new(SoftwarePlatform(windows.clone())))
            .expect("the scenario thread's Slint platform");
        *slot = Some(windows.clone());
        windows
    })
}

/// In-memory settings: the configured preferences are loaded, saves are accepted
/// and forgotten.
struct Settings(UserPreferences);

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
                Path::new(""),
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "comparator proof mismatch",
                ),
            ));
        }
        self.save_preferences(preferences)
    }
}

/// Stands in for the operating system's file chooser, which cannot be driven headless:
/// answers queued with [`ScriptedPicker::answer`] are returned in order; without one it
/// accepts the suggested name in the suggested folder, as a user pressing Save does.
#[derive(Default)]
pub struct ScriptedPicker {
    answers: std::sync::Mutex<VecDeque<Option<PathBuf>>>,
    requests: std::sync::Mutex<Vec<PathPickerRequest>>,
}

impl ScriptedPicker {
    /// Queue the chooser's next answer (`None` is the user cancelling it).
    #[allow(dead_code)] // For scenarios that pick a path other than the suggestion.
    pub fn answer(&self, path: Option<PathBuf>) {
        self.answers.lock().expect("picker answers").push_back(path);
    }

    pub fn requests(&self) -> Vec<PathPickerRequest> {
        self.requests.lock().expect("picker requests").clone()
    }
}

impl PathPickerService for ScriptedPicker {
    fn pick_path(
        &self,
        request: &PathPickerRequest,
        _: &Cancellation,
    ) -> Result<Option<PathBuf>, FsError> {
        self.requests
            .lock()
            .expect("picker requests")
            .push(request.clone());
        let queued = self.answers.lock().expect("picker answers").pop_front();
        Ok(queued.unwrap_or_else(|| {
            request
                .kind
                .suggested_save_name()
                .zip(request.initial_directory.clone())
                .map(|(name, folder)| folder.join(name))
        }))
    }
}

/// Stands in for the desktop clipboard, which a headless run has none of: what Copy
/// (Ctrl+C) and Cut (Ctrl+X) put there stays in this process, and Paste (Ctrl+V)
/// reads it back, as the native clipboard adapter would. Opt-in (`Config::clipboard`).
#[derive(Default)]
pub struct MemoryClipboard {
    files: std::sync::Mutex<Option<FileClipboard>>,
    text: std::sync::Mutex<Option<String>>,
}

impl MemoryClipboard {
    /// The file list on the clipboard, if any.
    pub fn files(&self) -> Option<FileClipboard> {
        self.files.lock().expect("clipboard files").clone()
    }

    /// The text on the clipboard (Copy details, a copied path), if any.
    #[allow(dead_code)] // For scenarios that check copied text by content.
    pub fn text(&self) -> Option<String> {
        self.text.lock().expect("clipboard text").clone()
    }
}

struct MemoryClipboardSession(Arc<MemoryClipboard>);

impl ClipboardSession for MemoryClipboardSession {
    fn copy_text(&mut self, text: &str) -> Result<(), FsError> {
        *self.0.text.lock().expect("clipboard text") = Some(text.to_owned());
        Ok(())
    }

    fn read_text(&mut self) -> Result<String, FsError> {
        Ok(self
            .0
            .text
            .lock()
            .expect("clipboard text")
            .clone()
            .unwrap_or_default())
    }

    fn copy_files(&mut self, files: &FileClipboard) -> Result<(), FsError> {
        *self.0.files.lock().expect("clipboard files") = Some(files.clone());
        Ok(())
    }

    fn read_files(&mut self) -> Result<FileClipboard, FsError> {
        self.0
            .files
            .lock()
            .expect("clipboard files")
            .clone()
            .ok_or_else(|| {
                FsError::from_io(
                    "file clipboard",
                    Path::new(""),
                    std::io::Error::new(std::io::ErrorKind::NotFound, "no files on the clipboard"),
                )
            })
    }

    fn owns_clipboard(&self) -> bool {
        false
    }
}

/// The service the Engine opens a [`MemoryClipboard`] session from.
struct MemoryClipboardService(Arc<MemoryClipboard>);

impl ClipboardService for MemoryClipboardService {
    fn open(&self) -> Result<Box<dyn ClipboardSession>, FsError> {
        Ok(Box::new(MemoryClipboardSession(self.0.clone())))
    }
}

/// The last few events and commands, for failure messages.
type Trace = Rc<RefCell<VecDeque<String>>>;

fn trace_line(trace: &Trace, mut line: String) {
    if line.len() > 160 {
        let cut = (0..=160)
            .rev()
            .find(|index| line.is_char_boundary(*index))
            .unwrap_or(0);
        line.truncate(cut);
    }
    let mut trace = trace.borrow_mut();
    if trace.len() == 32 {
        trace.pop_front();
    }
    trace.push_back(line);
}

/// The production owned-command scope, recording what the presenter sends.
struct ObservedScope {
    inner: Box<dyn OwnedCommandScope>,
    trace: Trace,
}

impl OwnedCommandScope for ObservedScope {
    fn send(&self, command: Command) -> bool {
        let line = format!("→ {command:?}");
        let sent = self.inner.send(command);
        trace_line(
            &self.trace,
            if sent {
                line
            } else {
                format!("{line} (refused)")
            },
        );
        sent
    }

    fn retire(&self, value: DeferredDrop) {
        self.inner.retire(value);
    }
}

pub struct Config {
    pub left: PathBuf,
    pub right: PathBuf,
    pub theme: PreferenceTheme,
    pub density: UiDensity,
    pub width: usize,
    pub height: usize,
    /// Show sizes as exact bytes instead of the readable default.
    pub size_bytes: bool,
    /// Browse through the installed native filesystem plugin (archives included) and
    /// install viewers, comparators and thumbnails, keeping their state (the thumbnail
    /// cache) in this folder. Needs a staged process: see `viewers_stage`.
    pub plugins: Option<PathBuf>,
    /// Opt-in: settings that outlive this run (restarts load what it saved).
    pub settings: Option<Arc<crate::options_support::SettingsStore>>,
    /// Opt-in: install the native Custom Command launcher, as the application does.
    pub user_commands: bool,
    /// Opt-in: install the native external editor launcher (F4), as the application does.
    pub editor: bool,
    /// Opt-in: an in-process clipboard (`MemoryClipboard`) for Ctrl+C, Ctrl+X and Ctrl+V.
    pub clipboard: bool,
    /// Opt-in: restore the saved tab session (with `settings`), as a start without paths
    /// on the command line does. Off, the configured paths win, as with explicit paths.
    pub restore_tabs: bool,
    /// Opt-in: start as `run_window` does with `--left`/`--right` given (true) or not
    /// (false) per panel: `set_tab_restore_overrides(explicit)`.
    pub explicit_paths: Option<[bool; 2]>,
    /// Opt-in: the application's own settings store (`LocalFileSystem`) instead of the
    /// in-memory one, as production loads and saves preferences. Only in a staged process,
    /// whose `XDG_CONFIG_HOME` is a private scratch profile.
    pub real_settings: bool,
}

impl Config {
    pub fn new(left: impl Into<PathBuf>, right: impl Into<PathBuf>) -> Self {
        Self {
            left: left.into(),
            right: right.into(),
            theme: PreferenceTheme::default(),
            density: UiDensity::default(),
            width: 1340,
            height: 820,
            size_bytes: false,
            plugins: None,
            settings: None,
            user_commands: false,
            editor: false,
            clipboard: false,
            restore_tabs: false,
            explicit_paths: None,
            real_settings: false,
        }
    }
}

pub struct App {
    pub window: MainWindow,
    pub main: Rc<MinimalSoftwareWindow>,
    pub width: usize,
    pub height: usize,
    pixels: Vec<slint::Rgb8Pixel>,
    windows: Windows,
    pub presenter: Rc<RefCell<Presenter>>,
    pub picker: Arc<ScriptedPicker>,
    events: mpsc::Receiver<Event>,
    commands: midnight_engine::CommandSender,
    begin_owned: BeginOwned,
    command_reclaim: command_reclaim::CommandReclaim,
    retire_queue: mpsc::Sender<reclaim::Retired>,
    committed: [Option<NavigationId>; 2],
    runtime: Option<Runtime>,
    engine: Option<JoinHandle<()>>,
    /// Every Copy/Move progress report, in arrival order.
    pub copies: Vec<Arc<CopyProgress>>,
    /// Every mutation (delete, rename, create) progress report.
    pub operations: Vec<Arc<OperationProgress>>,
    /// Completed listings per pane.
    pub listings: [usize; 2],
    /// Finished checksum calculations and verifications.
    pub checksum_runs: Vec<Arc<ChecksumRun>>,
    /// Rejected checksum jobs and finished manifest saves (errors as text).
    pub checksum_errors: Vec<String>,
    pub manifests: Vec<Result<ChecksumManifestReceipt, String>>,
    /// The last pointer click on a panel row (double-click protection).
    pub last_click: Option<(Instant, slint::LogicalPosition)>,
    /// The last few events, for failure messages.
    trace: Trace,
    /// Viewers, comparators and thumbnails, when `Config::plugins` asked for them.
    pub plugins: Option<super::viewers_support::Plugins>,
    /// The in-process clipboard, when `Config::clipboard` asked for one.
    pub clipboard: Option<Arc<MemoryClipboard>>,
}

impl App {
    /// Start the application on `config` and wait until both panes show their folders.
    #[track_caller]
    pub fn start(config: Config) -> Self {
        let windows = platform_windows();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .max_blocking_threads(16)
            .enable_all()
            .build()
            .expect("tokio runtime");
        let (events_send, events) = mpsc::channel(64);
        let local = Arc::new(LocalFileSystem::new(None));
        let mut preferences = UserPreferences {
            theme: config.theme,
            density: config.density,
            size_display: if config.size_bytes {
                SizeDisplay::Bytes
            } else {
                SizeDisplay::Readable
            },
            ..UserPreferences::default()
        };
        preferences.tabs[0].paths = vec![config.left.clone()];
        preferences.tabs[1].paths = vec![config.right.clone()];
        // Viewer scenarios browse through the installed native filesystem plugin.
        let mut native = config
            .plugins
            .as_ref()
            .map(|_| super::viewers_support::Native::new(&local));
        let mut engine = match native.as_mut() {
            Some(native) => native.engine(
                Engine::lifetime().expect("engine cleanup worker"),
                events_send,
            ),
            None => Engine::new(
                Engine::lifetime().expect("engine cleanup worker"),
                local.clone(),
                events_send,
            ),
        };
        if let Some(native) = &native {
            native.configure(&mut engine);
        }
        engine
            .configure_navigation_fallback()
            .unwrap()
            .install(local.clone());
        engine
            .configure_audited_transfers()
            .unwrap()
            .install(local.clone());
        engine.configure_find().unwrap().install(local.clone());
        engine
            .configure_audited_locale()
            .unwrap()
            .install(local.clone());
        engine.configure_mutations().unwrap().install(local.clone());
        engine.configure_watch().unwrap().install(local.clone());
        install_opt_in_services(&mut engine, &config, &local);
        let clipboard = config
            .clipboard
            .then(|| Arc::new(MemoryClipboard::default()));
        if let Some(clipboard) = &clipboard {
            engine
                .configure_clipboard()
                .unwrap()
                .install(Arc::new(MemoryClipboardService(clipboard.clone())));
        }
        let picker = Arc::new(ScriptedPicker::default());
        engine
            .configure_path_picker()
            .unwrap()
            .install(picker.clone());
        let (settings, saved_tabs) = settings_service(&config, preferences, &local);
        engine.configure_settings().unwrap().install(settings);
        let (commands, receive) = engine.command_channel(32).unwrap();
        let retire_queue = reclaim::start(runtime.handle(), Arc::new(|| {}));
        let command_reclaim = command_reclaim::CommandReclaim::new(retire_queue.clone());
        let engine = runtime.spawn(engine.run(receive));

        let window = MainWindow::new().expect("main window");
        window.show().expect("show main window");
        window
            .window()
            .dispatch_event(WindowEvent::WindowActiveChanged(true));
        let main = windows.borrow()[0].clone();
        let presenter = Rc::new(RefCell::new(Presenter::new(
            config.left.clone(),
            config.right.clone(),
        )));
        // As `run_window` does with or without `--left`/`--right`: a restored panel takes
        // the saved tab session, an explicit one keeps its path. Without either option
        // the presenter keeps its defaults, as earlier runs did.
        let overrides = if config.restore_tabs {
            Some([false; 2])
        } else {
            config.explicit_paths
        };
        if let Some(explicit) = overrides {
            presenter.borrow_mut().set_tab_restore_overrides(explicit);
        }
        // The panels whose saved tabs the first read restores (as new tabs): those with a
        // saved session and no path of their own.
        let explicit = overrides.unwrap_or([false; 2]);
        let relisted = [0, 1].map(|slot| saved_tabs[slot] && !explicit[slot]);
        let trace: Trace = Rc::default();
        let begin = command_reclaim.factory(commands.clone());
        let observed = trace.clone();
        let begin_owned: BeginOwned = Rc::new(move || {
            Some(Box::new(ObservedScope {
                inner: begin()?,
                trace: observed.clone(),
            }) as Box<dyn OwnedCommandScope>)
        });
        let sender = commands.clone();
        let begin_controls = begin_owned.clone();
        // As in production: ownership-bearing sends use an owned scope; the fallback
        // carries provider-free controls only.
        let send: Rc<dyn Fn(Command) -> bool> = Rc::new(move |command| {
            if let Some(scope) = begin_controls() {
                return scope.send(command);
            }
            sender.try_send(command).is_ok()
        });
        install_with_owned(
            &window,
            presenter.clone(),
            send.clone(),
            begin_owned.clone(),
        );
        let plugins = native
            .as_ref()
            .zip(config.plugins.as_ref())
            .map(|(native, state)| {
                super::viewers_support::Plugins::install(
                    native,
                    &window,
                    &presenter,
                    send.clone(),
                    runtime.handle(),
                    local.clone(),
                    picker.clone(),
                    state,
                )
            });
        let startup_tabs = active_tabs(&presenter.borrow());
        let mut app = Self {
            window,
            main,
            width: config.width,
            height: config.height,
            pixels: Vec::new(),
            windows,
            presenter,
            picker,
            events,
            commands,
            begin_owned,
            command_reclaim,
            retire_queue,
            committed: [None; 2],
            runtime: Some(runtime),
            engine: Some(engine),
            copies: Vec::new(),
            operations: Vec::new(),
            listings: [0; 2],
            checksum_runs: Vec::new(),
            checksum_errors: Vec::new(),
            manifests: Vec::new(),
            last_click: None,
            trace,
            plugins,
            clipboard,
        };
        app.resize(config.width, config.height);
        // Installing requests the preferences; their tab restore lists both panes again.
        // Production keeps the window hidden until then, so wait before any input.
        let ready = |app: &Self| {
            let presenter = app.presenter.borrow();
            presenter.window_geometry_ready()
                && active_tabs(&presenter)
                    .iter()
                    .zip(&startup_tabs)
                    .zip(&relisted)
                    .all(|((active, startup), relisted)| !relisted || active != startup)
                && !app.window.get_left_loading()
                && !app.window.get_right_loading()
        };
        let deadline = Instant::now() + WAIT;
        while !ready(&app) {
            if Instant::now() >= deadline {
                let presenter = app.presenter.borrow();
                panic!(
                    "startup did not finish within {WAIT:?}: window geometry ready {}, active \
                     tabs {:?} (at construction {startup_tabs:?}, relisted {relisted:?}), \
                     loading {} / {}; panels {:?} / {:?}",
                    presenter.window_geometry_ready(),
                    active_tabs(&presenter),
                    app.window.get_left_loading(),
                    app.window.get_right_loading(),
                    app.location(PaneId::Left),
                    app.location(PaneId::Right)
                );
            }
            app.settle();
            app.idle(Duration::from_millis(5));
        }
        if let Some(plugins) = &app.plugins {
            plugins.settings_applied(&app.presenter);
        }
        app
    }

    pub fn resize(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
        self.main
            .set_size(slint::PhysicalSize::new(width as u32, height as u32));
        self.window.window().dispatch_event(WindowEvent::Resized {
            size: slint::LogicalSize::new(width as f32, height as f32),
        });
        self.pixels
            .resize(width * height, slint::Rgb8Pixel::default());
        self.draw();
    }

    /// Run due timers and software-render the main window, as a frame does.
    pub fn draw(&mut self) {
        slint::platform::update_timers_and_animations();
        self.main.request_redraw();
        let (pixels, width) = (&mut self.pixels, self.width);
        self.main.draw_if_needed(|renderer| {
            renderer.render(pixels, width);
        });
    }

    pub fn slint(&self) -> &slint::Window {
        self.window.window()
    }

    /// Visible windows other than the main window (Find, Checksums, decisions, ...).
    pub fn tool_windows(&self) -> Vec<Rc<MinimalSoftwareWindow>> {
        self.windows
            .borrow()
            .iter()
            .skip(1)
            .filter(|window| window.window().is_visible())
            .cloned()
            .collect()
    }

    pub fn rows(&self, pane: PaneId) -> Vec<midnight_ui::FileRow> {
        match pane {
            PaneId::Left => self.window.get_left_rows(),
            PaneId::Right => self.window.get_right_rows(),
        }
        .iter()
        .collect()
    }

    pub fn names(&self, pane: PaneId) -> Vec<String> {
        self.rows(pane)
            .iter()
            .map(|row| row.name.to_string())
            .collect()
    }

    pub fn cursor(&self, pane: PaneId) -> i32 {
        match pane {
            PaneId::Left => self.window.get_left_cursor(),
            PaneId::Right => self.window.get_right_cursor(),
        }
    }

    pub fn loading(&self, pane: PaneId) -> bool {
        match pane {
            PaneId::Left => self.window.get_left_loading(),
            PaneId::Right => self.window.get_right_loading(),
        }
    }

    pub fn error(&self, pane: PaneId) -> String {
        match pane {
            PaneId::Left => self.window.get_left_error(),
            PaneId::Right => self.window.get_right_error(),
        }
        .to_string()
    }

    /// The folder the pane shows, as the presenter knows it.
    pub fn location(&self, pane: PaneId) -> Option<PathBuf> {
        self.presenter
            .borrow()
            .visible_directory(pane)
            .map(|visible| visible.path.clone())
    }

    /// The last few events and commands (newest last), for diagnostics.
    pub fn recent_events(&self) -> Vec<String> {
        self.trace.borrow().iter().cloned().collect()
    }

    /// The displayed notice (title and body), when one is open. The window keeps the
    /// last message title after the notice closes; the notice presentation's token says
    /// whether it is shown.
    pub fn notice(&self) -> Option<(String, String)> {
        let notice = self.window.global::<midnight_ui::NoticePresentation>();
        if notice.get_token().is_empty() {
            return None;
        }
        Some((
            notice.get_title().to_string(),
            notice.get_body().to_string(),
        ))
    }

    fn record(&mut self, event: &Event) {
        match event {
            Event::Copy(CopyEvent::Progress(progress)) => self.copies.push(progress.clone()),
            Event::Operation(OperationEvent::Progress(progress)) => {
                self.operations.push(progress.clone())
            }
            Event::ListingFinished { pane, .. } => self.listings[pane.index()] += 1,
            Event::Checksum(event) => {
                if let ChecksumUpdate::Finished(run) = &event.update {
                    self.checksum_runs.push(run.clone());
                }
            }
            Event::ChecksumRejected(rejected) => {
                self.checksum_errors.push(rejected.error.to_string())
            }
            Event::ChecksumManifest(event) => match &event.update {
                ChecksumManifestUpdate::Finished(result) => {
                    self.manifests
                        .push(result.clone().map_err(|error| error.to_string()));
                }
                ChecksumManifestUpdate::Rejected(error) => {
                    self.checksum_errors.push(error.to_string())
                }
                _ => {}
            },
            _ => {}
        }
        trace_line(&self.trace, format!("{event:?}"));
    }

    fn drain_one(&mut self) -> bool {
        let presenter = self.presenter.clone();
        let begin = self.begin_owned.clone();
        let consumed = with_owned_commands(&presenter, &begin, |presenter, send| {
            // Claim precedes the ownership-bearing dequeue, as in production.
            let event = match self.events.try_recv() {
                Ok(event) => event,
                Err(mpsc::error::TryRecvError::Empty) => return false,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    panic!("the Engine stopped (its event channel closed)");
                }
            };
            self.record(&event);
            if let Event::DirectoryChanged { pane, navigation } = event {
                presenter.refresh_if_changed(pane, navigation, send);
            } else {
                presenter.apply_with_sender(event, send);
            }
            true
        }) == Some(true);
        self.command_reclaim.flush();
        if consumed {
            refresh(&self.window, &self.presenter.borrow());
        }
        consumed
    }

    /// Apply every pending Engine event and repaint, like idle frames of the event loop.
    pub fn settle(&mut self) {
        for _ in 0..4 {
            let mut consumed = false;
            for _ in 0..256 {
                if !self.drain_one() {
                    break;
                }
                consumed = true;
            }
            // What production's retry pass flushes after each frame.
            let _ = with_owned_commands(&self.presenter, &self.begin_owned, |presenter, send| {
                presenter.flush_selection(send);
                presenter.flush_filters(send);
                presenter.flush_filename_colors(send);
                presenter.flush_search(send);
                presenter.flush_comparison(send);
            });
            self.sync_visible();
            self.sample_visible_rows();
            refresh(&self.window, &self.presenter.borrow());
            self.draw();
            for window in self.tool_windows() {
                slint::platform::update_timers_and_animations();
                inspect_tree(&window);
            }
            if !consumed {
                break;
            }
        }
    }

    /// The visible-row sampling production's thumbnail pulse performs every frame,
    /// with or without thumbnails: it also tells the presenter each tree viewport.
    fn sample_visible_rows(&mut self) {
        let window = &self.window;
        let ranges = [
            (
                window.get_left_thumbnail_row_start().max(0) as usize,
                window.get_left_thumbnail_row_end().max(0) as usize,
            ),
            (
                window.get_right_thumbnail_row_start().max(0) as usize,
                window.get_right_thumbnail_row_end().max(0) as usize,
            ),
        ];
        let hovered = [
            window.get_left_hovered_row(),
            window.get_right_hovered_row(),
        ];
        let _ = self
            .presenter
            .borrow_mut()
            .thumbnail_targets(ranges, [hovered[0].as_str(), hovered[1].as_str()]);
    }

    /// Acknowledge each pane's visible listing, as production's visible sync does,
    /// so watches and maintenance follow what is shown.
    fn sync_visible(&mut self) {
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

    /// Wait without taking events or holding a claim.
    pub fn idle(&mut self, duration: Duration) {
        if let Some(runtime) = &self.runtime {
            // The timer must be created inside the runtime's context.
            runtime.block_on(async { tokio::time::sleep(duration).await });
        }
        self.settle();
    }

    /// Pump events until `condition` holds, or panic after [`WAIT`] with diagnostics.
    #[track_caller]
    pub fn until(&mut self, what: &str, condition: impl Fn(&Self) -> bool) {
        self.until_for(what, WAIT, condition);
    }

    #[track_caller]
    pub fn until_for(&mut self, what: &str, limit: Duration, condition: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + limit;
        loop {
            self.settle();
            if condition(self) {
                return;
            }
            if Instant::now() >= deadline {
                panic!(
                    "timed out after {limit:?} waiting for {what}\n  left: {:?} loading={} error={:?}\n  right: {:?} loading={} error={:?}\n  notice: {:?}\n  modal={} tool windows={}{}\n  last copy: {:?}\n  last operation: {:?}\n  recent events:\n    {}",
                    self.location(PaneId::Left),
                    self.loading(PaneId::Left),
                    self.error(PaneId::Left),
                    self.location(PaneId::Right),
                    self.loading(PaneId::Right),
                    self.error(PaneId::Right),
                    self.notice(),
                    self.window.get_embedded_modal_open(),
                    self.tool_windows().len(),
                    self.describe_tool_windows(),
                    self.copies.last().map(|progress| progress.state),
                    self.operations.last().map(|progress| progress.state),
                    self.trace
                        .borrow()
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("\n    ")
                );
            }
            if let Some(runtime) = &self.runtime {
                runtime.block_on(async { tokio::time::sleep(Duration::from_millis(5)).await });
            }
        }
    }

    /// What each open tool window shows (texts and buttons, shortened), for timeouts.
    fn describe_tool_windows(&self) -> String {
        let short = |text: &String| {
            let line = text.replace('\n', " / ");
            match line.char_indices().nth(120) {
                Some((end, _)) => format!("{}…", &line[..end]),
                None => line,
            }
        };
        self.tool_windows()
            .iter()
            .map(|window| {
                let texts: Vec<String> = super::inspect::visible_texts(window.window())
                    .iter()
                    .take(10)
                    .map(short)
                    .collect();
                let buttons = super::inspect::labels(
                    window.window(),
                    i_slint_core::items::AccessibleRole::Button,
                );
                format!("\n    window: texts {texts:?}; buttons {buttons:?}")
            })
            .collect()
    }

    /// Render the main window into the run's screenshots and record it.
    pub fn screenshot(&mut self, context: &mut Context, name: &str, caption: &str) -> String {
        self.settle();
        self.draw();
        let relative = format!("shots/{}/{name}.png", context_name());
        let path = context.run_dir.join(&relative);
        if let Err(error) = report::save_png(&path, self.width, self.height, &self.pixels) {
            context.note(format!("screenshot {name} failed: {error}"));
        } else {
            context.outcome.shots.push(report::Shot {
                file: relative.clone(),
                caption: caption.to_owned(),
                flags: 0,
            });
        }
        relative
    }

    /// Render a tool window at its current size into the run's screenshots.
    pub fn screenshot_window(
        &mut self,
        context: &mut Context,
        window: &MinimalSoftwareWindow,
        name: &str,
        caption: &str,
    ) -> String {
        self.settle();
        let (width, height, pixels) = render(window);
        let relative = format!("shots/{}/{name}.png", context_name());
        let path = context.run_dir.join(&relative);
        if let Err(error) = report::save_png(&path, width, height, &pixels) {
            context.note(format!("screenshot {name} failed: {error}"));
        } else {
            context.outcome.shots.push(report::Shot {
                file: relative.clone(),
                caption: caption.to_owned(),
                flags: 0,
            });
        }
        relative
    }

    /// Shut the Engine down in order (bounded); returns what went wrong, if anything.
    pub fn shutdown(&mut self) -> Result<(), String> {
        let Some(runtime) = self.runtime.take() else {
            return Ok(());
        };
        let plugins = self.plugins.take();
        if let Some(plugins) = &plugins {
            plugins.close();
        }
        midnight_ui::close_tool_windows(&self.window);
        self.presenter.borrow_mut().retire_all();
        let mut retired = self.presenter.borrow_mut().take_retired();
        if let Some(pending) = self.command_reclaim.take_ready() {
            retired.extend(pending);
        }
        let mut errors = Vec::new();
        if let Some(mut engine) = self.engine.take() {
            let commands = self.commands.clone();
            let queue = self.retire_queue.clone();
            let events = &mut self.events;
            let result = runtime.block_on(async {
                if let Ok(permit) = queue.reserve().await {
                    permit.send(retired);
                }
                let drain = async {
                    let shutdown = commands.send(Command::Shutdown);
                    tokio::pin!(shutdown);
                    let mut sent = false;
                    loop {
                        let Ok(permit) = queue.reserve().await else {
                            return Err("the reclamation queue closed during shutdown".to_owned());
                        };
                        tokio::select! {
                            biased;
                            result = &mut engine => {
                                return result.map_err(|error| format!("Engine task failed: {error}"));
                            }
                            _ = &mut shutdown, if !sent => sent = true,
                            event = events.recv() => match event {
                                Some(event) => permit.send(vec![Box::new(event)]),
                                None => {
                                    return (&mut engine).await.map_err(|error| format!("Engine task failed: {error}"));
                                }
                            }
                        }
                    }
                };
                tokio::time::timeout(ENGINE_SHUTDOWN, drain)
                    .await
                    .unwrap_or_else(|_| Err(format!("Engine shutdown exceeded {ENGINE_SHUTDOWN:?}")))
            });
            if let Err(error) = result {
                errors.push(error);
            }
            let (_, empty) = mpsc::channel(1);
            let receiver = std::mem::replace(&mut self.events, empty);
            let mut rest = reclaim::Retired::new();
            reclaim::retire_receiver(receiver, &mut rest);
            let queue = self.retire_queue.clone();
            runtime.block_on(async move {
                if let Ok(permit) = queue.reserve().await {
                    permit.send(rest);
                }
                let _ = tokio::time::timeout(ENGINE_SHUTDOWN, reclaim::barrier(&queue)).await;
            });
        }
        if let Some(plugins) = plugins
            && let Err(error) = plugins.finish(&runtime)
        {
            errors.push(error);
        }
        let _ = self.window.hide();
        runtime.shutdown_timeout(Duration::from_secs(3));
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        // Never panic here: this usually runs while a failed check unwinds.
        if let Err(error) = self.shutdown() {
            eprintln!("acceptance: application shutdown: {error}");
        }
    }
}

fn active_tabs(presenter: &Presenter) -> [Option<TabId>; 2] {
    [PaneId::Left, PaneId::Right].map(|pane| {
        presenter
            .tabs(pane)
            .into_iter()
            .find(|tab| tab.active)
            .map(|tab| tab.id)
    })
}

/// The current scenario's name (its thread is named after it).
pub fn context_name() -> String {
    std::thread::current()
        .name()
        .unwrap_or("scenario")
        .to_owned()
}

/// Size a tool window and settle its layout (no rendering).
pub fn layout_tool(window: &MinimalSoftwareWindow, width: u32, height: u32) {
    window.set_size(slint::PhysicalSize::new(width, height));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(width as f32, height as f32),
    });
    inspect_tree(window);
}

fn inspect_tree(window: &MinimalSoftwareWindow) {
    i_slint_core::window::WindowInner::from_pub(window.window()).ensure_tree_instantiated();
}

/// Software-render any window at its current size.
pub fn render(window: &MinimalSoftwareWindow) -> (usize, usize, Vec<slint::Rgb8Pixel>) {
    slint::platform::update_timers_and_animations();
    let size = window.window().size();
    let (width, height) = (size.width as usize, size.height as usize);
    let mut pixels = vec![slint::Rgb8Pixel::default(); width * height];
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, width);
    });
    (width, height, pixels)
}

/// The settings service of a run: the application's own store (opt-in), the opt-in
/// store shared across restarts, otherwise in-memory preferences whose saves are
/// forgotten. Also which panels have a saved tab session the first read restores.
fn settings_service(
    config: &Config,
    preferences: UserPreferences,
    local: &Arc<LocalFileSystem>,
) -> (Arc<dyn SettingsService>, [bool; 2]) {
    let restorable = |preferences: &UserPreferences| {
        preferences
            .tabs
            .each_ref()
            .map(|tabs| tabs.validate().is_ok() && tabs.paths.get(tabs.active).is_some())
    };
    if config.real_settings {
        // The real store writes `$XDG_CONFIG_HOME/midnight-explorer`: only a staged
        // process has a private one.
        assert!(
            std::env::var_os("MIDNIGHT_ACCEPTANCE_STAGED").is_some()
                && std::env::var_os("XDG_CONFIG_HOME").is_some(),
            "Config::real_settings needs a staged process with a private XDG_CONFIG_HOME"
        );
        // Read once before the start, as production's startup read does; the engine's
        // first load then reads the store again, asynchronously.
        let stored = SettingsService::load_preferences_for_acceptance(local.as_ref()).ok();
        return (
            local.clone(),
            stored.as_ref().map_or([false; 2], restorable),
        );
    }
    match &config.settings {
        Some(store) => {
            let service = store.service(preferences);
            (service, restorable(&store.current()))
        }
        None => {
            let saved = restorable(&preferences);
            (Arc::new(Settings(preferences)), saved)
        }
    }
}

/// Opt-in production services, installed exactly as `run_window` installs them. They
/// launch the programs a scenario configured, so only scenarios that configure
/// harmless commands inside their work folder enable them.
fn install_opt_in_services(engine: &mut Engine, config: &Config, local: &Arc<LocalFileSystem>) {
    if config.user_commands {
        engine
            .configure_user_commands()
            .unwrap()
            .install(local.clone());
    }
    if config.editor {
        engine.configure_editor().unwrap().install(local.clone());
    }
}

//! The application under test: the real MainWindow, Presenter and Engine with the
//! native filesystem services the application installs, on software-rendered windows.
//!
//! The wiring follows `crates/midnight-explorer/src/main.rs` (`run_window`): the same
//! Engine services where they work headless, `install_with_owned`, the owned command
//! scope, the event pump of its `backend_ready` callback and its startup deadline for a
//! slow settings read. Differences, all deliberate and documented in
//! docs/developer/ACCEPTANCE.md: settings are in memory (the user's profile is never read
//! or written), browsing uses the host-native `LocalFileSystem` service rather than the
//! installed native filesystem plugin, the clipboard is an opt-in in-process one
//! (`Config::clipboard`), the desktop service is an opt-in stand-in that records its
//! launches (`Config::desktop`, `launches.rs`), and the desktop drag-and-drop hand-off is
//! not installed. Editors and Custom Commands are opt-in (`Config::editor`,
//! `Config::user_commands`; a Custom Command's terminal is recorded, never started), and
//! `Config::plugins` opts a staged viewer scenario into the installed native filesystem
//! plugin (archives included), viewers, comparators and thumbnails (`viewers_support`).
//! Slint's timers and animations follow a [`Clock`] that a scenario can hold and advance.
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
/// How often [`App::pump`] and [`App::advance`] settle: about one frame.
pub const FRAME: Duration = Duration::from_millis(10);
/// Production's startup deadline (`crates/midnight-explorer/src/startup.rs`, `DEADLINE`):
/// a settings read slower than this no longer keeps the window hidden; the window shows
/// the defaults and says so until the read applies.
pub const SETTINGS_DEADLINE: Duration = Duration::from_secs(10);

type Windows = Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>;

/// The time Slint's timers and animations follow on this thread, and with them every
/// product rule timed on the platform clock: the drag bridge's 0.4 s quick-drop rule,
/// the time picker's repeat, the panels' rest and glide, the startup deadline. It runs
/// with real time until a scenario holds it ([`App::hold_clock`]); then it moves only by
/// [`App::advance`], until [`App::resume_clock`]. It never goes back.
#[derive(Clone)]
pub struct Clock(Rc<RefCell<ClockState>>);

struct ClockState {
    /// The clock's time at `since`.
    base: Duration,
    /// When it was created, last held or resumed (real time).
    since: Instant,
    held: bool,
}

impl Clock {
    fn new() -> Self {
        Self(Rc::new(RefCell::new(ClockState {
            base: Duration::ZERO,
            since: Instant::now(),
            held: false,
        })))
    }

    /// The platform's time since this thread's platform was installed.
    pub fn now(&self) -> Duration {
        let state = self.0.borrow();
        if state.held {
            state.base
        } else {
            state.base + state.since.elapsed()
        }
    }

    pub fn is_held(&self) -> bool {
        self.0.borrow().held
    }

    fn hold(&self) {
        let now = self.now();
        let mut state = self.0.borrow_mut();
        state.base = now;
        state.held = true;
    }

    /// Run with real time again, on from where the held clock stands.
    fn resume(&self) {
        let mut state = self.0.borrow_mut();
        if state.held {
            state.since = Instant::now();
            state.held = false;
        }
    }

    fn step(&self, by: Duration) {
        self.0.borrow_mut().base += by;
    }
}

/// Every window the application creates gets a software adapter recorded here; timers
/// and animations follow its [`Clock`].
struct SoftwarePlatform {
    windows: Windows,
    clock: Clock,
}

impl Platform for SoftwarePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        self.windows.borrow_mut().push(window.clone());
        Ok(window)
    }

    fn duration_since_start(&self) -> Duration {
        self.clock.now()
    }
}

thread_local! {
    // Slint's platform is per thread and can be set once; later Apps on the same
    // thread reuse it with a cleared window list and a running clock.
    static PLATFORM: RefCell<Option<(Windows, Clock)>> = const { RefCell::new(None) };
}

fn platform() -> (Windows, Clock) {
    PLATFORM.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some((windows, clock)) = slot.as_ref() {
            windows.borrow_mut().clear();
            clock.resume();
            return (windows.clone(), clock.clone());
        }
        let (windows, clock): (Windows, Clock) = (Rc::default(), Clock::new());
        slint::platform::set_platform(Box::new(SoftwarePlatform {
            windows: windows.clone(),
            clock: clock.clone(),
        }))
        .expect("the scenario thread's Slint platform");
        *slot = Some((windows.clone(), clock.clone()));
        (windows, clock)
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
    /// Every copy so far, also of what the clipboard already held.
    writes: std::sync::atomic::AtomicU64,
}

impl MemoryClipboard {
    /// How many times anything was copied (text or files): a copy is observable even
    /// when it puts the same text there again.
    pub fn writes(&self) -> u64 {
        self.writes.load(std::sync::atomic::Ordering::SeqCst)
    }

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
        self.0
            .writes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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
        self.0
            .writes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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
    /// Opt-in: the Custom Command launcher (`launches::UserCommands`): the platform's own
    /// validation, programs started as configured, terminals recorded in
    /// [`App::launches`] and never started.
    pub user_commands: bool,
    /// Opt-in: a stand-in desktop service (`launches::Desktop`): the platform's
    /// locations, icons, property details and link targets; the command line's and the
    /// terminal button's terminals, files opened with their association and system
    /// locations recorded in [`App::launches`] and never started; Properties shows the
    /// application's own summary unless `native_properties`. Without it the command line reports "Terminal
    /// unavailable" and opening a file is unavailable.
    pub desktop: bool,
    /// Opt-in, with `desktop`: Properties opens the platform's own dialog, as the
    /// application does: Windows' property sheet, which the scenario reads and closes
    /// (`native_windows.rs`). Linux has none and shows the application's summary.
    pub native_properties: bool,
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
    /// in-memory one, as production loads and saves preferences. Only in a
    /// `viewers_stage::run_private` child, whose settings folders (`XDG_CONFIG_HOME`;
    /// Windows: `USERPROFILE`, `LOCALAPPDATA`) lie in its private folder: `App::start`
    /// checks that they do.
    pub real_settings: bool,
    /// Opt-in: every settings read (of whichever store) answers this late, as from a
    /// slow profile folder. Longer than [`SETTINGS_DEADLINE`], the window shows the
    /// defaults first and `App::start` returns before the read ([`App::settings_late`]).
    pub settings_delay: Option<Duration>,
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
            desktop: false,
            native_properties: false,
            editor: false,
            clipboard: false,
            restore_tabs: false,
            explicit_paths: None,
            real_settings: false,
            settings_delay: None,
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
    /// Every answer to the Copy/Move dialog's question how links fare at a destination
    /// (`Event::TransferLinksInspected`), in arrival order: a step waits for one more.
    pub links_inspected: Vec<TransferLinksResult>,
    /// Every answer to an archive password (`Event::ArchiveUnlocked`), in arrival order:
    /// the outcome, or the error's text (`journeys_archive_roundtrip` waits for them).
    pub archive_unlocks: Vec<Result<ArchiveUnlock, String>>,
    /// The settings reads applied (`Event::PreferencesLoaded`, read or failed).
    pub settings_reads: usize,
    /// Every program, terminal, file association and system location the application
    /// asked to start ([`Config::user_commands`], [`Config::desktop`]), in order. No
    /// terminal is ever started: no platform launcher is installed for them.
    pub launches: Arc<super::launches::Launches>,
    /// The startup deadline passed before the first settings read applied: the window
    /// showed the defaults, as production's does ([`SETTINGS_DEADLINE`]).
    pub settings_late: bool,
    /// Whether each panel was seen loading since the start (`App::start`'s ready check).
    started_loading: [bool; 2],
    /// The last pointer click on a panel row (double-click protection).
    pub last_click: Option<(Instant, slint::LogicalPosition)>,
    /// The Slint clock of this thread ([`App::hold_clock`]).
    pub clock: Clock,
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
        let (windows, clock) = platform();
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
        let launches = Arc::new(super::launches::Launches::default());
        install_opt_in_services(&mut engine, &config, &local, &launches);
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
        let started = clock.now();
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
            links_inspected: Vec::new(),
            archive_unlocks: Vec::new(),
            settings_reads: 0,
            launches,
            settings_late: false,
            started_loading: [false; 2],
            last_click: None,
            clock,
            trace,
            plugins,
            clipboard,
        };
        app.resize(config.width, config.height);
        // Installing requests the preferences; their tab restore lists both panes again.
        // Production keeps the window hidden until the first read applied (read or
        // failed) or its startup deadline passed, so no input is sent before either.
        let ready = |app: &Self| {
            let presenter = app.presenter.borrow();
            let revealed = if app.settings_reads > 0 {
                active_tabs(&presenter)
                    .iter()
                    .zip(&startup_tabs)
                    .zip(&relisted)
                    .all(|((active, startup), relisted)| !relisted || active != startup)
            } else {
                app.settings_late
            };
            // A panel that has not started its first listing is not loading either: each
            // must show its folder, or have listed (loaded and stopped) without one.
            let listed = |pane: PaneId| {
                app.location(pane).is_some()
                    || (app.started_loading[pane.index()] && !app.loading(pane))
            };
            revealed
                && !app.window.get_left_loading()
                && !app.window.get_right_loading()
                && listed(PaneId::Left)
                && listed(PaneId::Right)
        };
        let deadline = Instant::now() + WAIT;
        loop {
            app.settle();
            for pane in [PaneId::Left, PaneId::Right] {
                app.started_loading[pane.index()] |= app.loading(pane);
            }
            if app.settings_reads == 0
                && !app.settings_late
                && app.clock.now() >= started + SETTINGS_DEADLINE
            {
                // What production's deadline does (`run_window`, `startup.arm`): the
                // window shows the defaults and its status says why.
                app.settings_late = true;
                app.presenter.borrow_mut().note_settings_pending();
                refresh(&app.window, &app.presenter.borrow());
            }
            if ready(&app) {
                break;
            }
            if Instant::now() >= deadline {
                let presenter = app.presenter.borrow();
                panic!(
                    "startup did not finish within {WAIT:?}: settings reads applied {}, late \
                     {}, window geometry ready {}, active tabs {:?} (at construction \
                     {startup_tabs:?}, relisted {relisted:?}), loading {} / {}; panels {:?} / \
                     {:?}",
                    app.settings_reads,
                    app.settings_late,
                    presenter.window_geometry_ready(),
                    active_tabs(&presenter),
                    app.window.get_left_loading(),
                    app.window.get_right_loading(),
                    app.location(PaneId::Left),
                    app.location(PaneId::Right)
                );
            }
            app.sleep(Duration::from_millis(5));
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

    /// Run due timers and render the main window only when Slint has a frame pending.
    pub fn frame(&mut self) {
        slint::platform::update_timers_and_animations();
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
            Event::TransferLinksInspected(result) => self.links_inspected.push((**result).clone()),
            Event::ArchiveUnlocked { result, .. } => self
                .archive_unlocks
                .push(result.clone().map_err(|error| error.to_string())),
            Event::PreferencesLoaded(_) => self.settings_reads += 1,
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

    /// Apply every pending Engine event and repaint what changed, like idle frames of the
    /// event loop.
    pub fn settle(&mut self) {
        let _phase = crate::timing::Span::new("app: settle");
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
            self.frame();
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

    /// Deliver an automatic watch hint for this flat pane and wait until its new
    /// backend listing commits. The directory itself may be unchanged.
    pub fn reread_unchanged_pane(&mut self, pane: PaneId) -> Result<(), String> {
        let old = self.committed[pane.index()].ok_or("pane has no committed listing")?;
        let sent = with_owned_commands(&self.presenter, &self.begin_owned, |presenter, send| {
            presenter.refresh_if_changed(pane, old, send)
        });
        if sent != Some(true) {
            return Err(format!("could not send a watch reread for {pane:?}"));
        }
        self.until("the watch reread to commit a new listing", |app| {
            app.committed[pane.index()].is_some_and(|current| current != old)
        });
        Ok(())
    }

    /// Hand `url` to the system's handler as a viewer hands a link the user followed
    /// (`Presenter::open_external_url`, `Command::OpenUrl`); settle until the backend
    /// answered (30 s at most) and return the status text the viewer would show: `Ok`
    /// once the handler took the link, `Err` with the reason otherwise. With
    /// `Config::desktop` the stand-in records it (`Launch::OpenUrl`) and nothing starts;
    /// without it the Engine has no desktop service and refuses.
    pub fn open_url(&mut self, url: &str) -> Result<Result<String, String>, String> {
        let url = ExternalUrl::parse(url).map_err(|refusal| format!("{url}: {refusal:?}"))?;
        let answer: Rc<RefCell<Option<Result<String, String>>>> = Rc::default();
        let done = answer.clone();
        with_owned_commands(
            &self.presenter,
            &self.begin_owned,
            move |presenter, send| {
                presenter.open_external_url(
                    url,
                    send,
                    Box::new(move |result| *done.borrow_mut() = Some(result)),
                );
            },
        )
        .ok_or("no command scope for the link")?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            self.settle();
            if let Some(result) = answer.borrow_mut().take() {
                return Ok(result);
            }
            if Instant::now() >= deadline {
                return Err("the link got no answer within 30 s".into());
            }
            self.sleep(Duration::from_millis(10));
        }
    }

    /// Wait without taking events or holding a claim, then settle once: timers and
    /// animations advance in one jump. Not a synchronisation: wait for what should
    /// happen with [`App::until`], or let time pass in frames with [`App::pump`].
    pub fn idle(&mut self, duration: Duration) {
        self.sleep(duration);
        self.settle();
    }

    /// Sleep in real time without taking events or holding a claim.
    fn sleep(&self, duration: Duration) {
        if let Some(runtime) = &self.runtime {
            // The timer must be created inside the runtime's context.
            runtime.block_on(async { tokio::time::sleep(duration).await });
        }
    }

    /// Let `duration` of real time pass the way the event loop does: settle about every
    /// [`FRAME`], so Engine events are applied as they come and timers and animations
    /// advance frame by frame (while the clock runs; a held clock stays where it is).
    pub fn pump(&mut self, duration: Duration) {
        let end = Instant::now() + duration;
        loop {
            self.settle();
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return;
            }
            self.sleep(left.min(FRAME));
        }
    }

    /// Stop the Slint clock (the [`Clock`]): from now on Slint's timers and animations,
    /// and every product rule timed on that clock, move only with [`App::advance`].
    /// Engine events are still applied by every settle. Two clicks at one place within
    /// the double-click interval of the clock's time are a double click, whatever real
    /// time passes between them: advance the clock past that interval in between.
    pub fn hold_clock(&mut self) {
        self.settle();
        self.clock.hold();
    }

    /// Move the held clock forward by `by` in steps of at most [`FRAME`], settling after
    /// each step: timers due in a step fire in it, animations take every step.
    #[track_caller]
    pub fn advance(&mut self, by: Duration) {
        assert!(
            self.clock.is_held(),
            "App::advance moves a held clock; call App::hold_clock first"
        );
        let mut left = by;
        while !left.is_zero() {
            let step = left.min(FRAME);
            self.clock.step(step);
            left -= step;
            self.settle();
        }
    }

    /// Let `by` pass for Slint's timers and animations. With the clock held it moves by
    /// `by` at once and one settle follows: an animation shows its state at that time and
    /// a timer due by then fires, without waiting in real time. Otherwise [`App::idle`].
    pub fn wait_ui(&mut self, by: Duration) {
        if self.clock.is_held() {
            self.clock.step(by);
            self.settle();
        } else {
            self.idle(by);
        }
    }

    /// Let the held clock run with real time again, on from where it stands.
    pub fn resume_clock(&mut self) {
        self.clock.resume();
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
                    "timed out after {limit:?} waiting for {what}{}\n  left: {:?} loading={} error={:?}\n  right: {:?} loading={} error={:?}\n  notice: {:?}\n  modal={} tool windows={}{}\n  last copy: {:?}\n  last operation: {:?}\n  recent events:\n    {}",
                    if self.clock.is_held() {
                        format!(" (the Slint clock is held at {:?})", self.clock.now())
                    } else {
                        String::new()
                    },
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
            self.sleep(Duration::from_millis(5));
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

    /// Render a tool window at its current size into the run's screenshots. A window
    /// that closed while the events were applied (a job window whose job ended) is noted
    /// instead: it has nothing left to render ([`shown`]).
    pub fn screenshot_window(
        &mut self,
        context: &mut Context,
        window: &MinimalSoftwareWindow,
        name: &str,
        caption: &str,
    ) -> String {
        self.settle();
        let relative = format!("shots/{}/{name}.png", context_name());
        if !shown(window) {
            context.note(format!(
                "screenshot {name} ({caption}) not taken: the window had closed"
            ));
            return relative;
        }
        let (width, height, pixels) = render(window);
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

/// Whether `window` is still shown with its content. A tool window that closed (a job
/// window whose job ended, a dialog answered) keeps its adapter but loses its component,
/// and Slint 1.18.1 panics (`WindowInner::component`, window.rs:781) when such a window
/// is resized or rendered; a panic there can leave Slint's state borrowed, and a second
/// one while the scenario unwinds aborts the whole run.
pub fn shown(window: &MinimalSoftwareWindow) -> bool {
    window.window().is_visible()
        && i_slint_core::window::WindowInner::from_pub(window.window())
            .try_component()
            .is_some()
}

/// Size a tool window and settle its layout (no rendering); false, and nothing done,
/// when the window has closed ([`shown`]).
pub fn layout_tool(window: &MinimalSoftwareWindow, width: u32, height: u32) -> bool {
    if !shown(window) {
        return false;
    }
    window.set_size(slint::PhysicalSize::new(width, height));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(width as f32, height as f32),
    });
    inspect_tree(window);
    true
}

fn inspect_tree(window: &MinimalSoftwareWindow) {
    i_slint_core::window::WindowInner::from_pub(window.window()).ensure_tree_instantiated();
}

/// Software-render any window at its current size; an empty frame (0 × 0) when the
/// window has closed ([`shown`]).
pub fn render(window: &MinimalSoftwareWindow) -> (usize, usize, Vec<slint::Rgb8Pixel>) {
    slint::platform::update_timers_and_animations();
    if !shown(window) {
        return (0, 0, Vec::new());
    }
    let size = window.window().size();
    let (width, height) = (size.width as usize, size.height as usize);
    let mut pixels = vec![slint::Rgb8Pixel::default(); width * height];
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, width);
    });
    (width, height, pixels)
}

/// Whether the application's own settings store would stay inside this process's
/// private folder, as `startup_paths` checks before it seeds the store: a
/// `viewers_stage::run_private` child, whose parent gave it `<work>/staged/run` as
/// `MIDNIGHT_ACCEPTANCE_RUN_DIR` and its profile below `<work>/staged`. The store lives
/// in `$XDG_CONFIG_HOME/midnight-explorer` (`$HOME/.config` when that is not absolute);
/// on Windows in the LocalAppData known folder, which resolves through `%USERPROFILE%`
/// (`%USERPROFILE%\AppData\Local`), and `LOCALAPPDATA` names it to the other services.
/// Every one of those folders must lie below `<work>/staged`; that they are merely set
/// proves nothing (Windows always sets `USERPROFILE`).
fn private_settings() -> Result<(), String> {
    if !crate::viewers_stage::is_staged() {
        return Err("this process is not staged".to_owned());
    }
    let absolute = |variable: &str| {
        std::env::var_os(variable)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    // `viewers_stage::parent` names these two folders.
    let staged = absolute("MIDNIGHT_ACCEPTANCE_RUN_DIR")
        .filter(|run| run.file_name().is_some_and(|name| name == "run"))
        .and_then(|run| run.parent().map(Path::to_path_buf))
        .filter(|staged| staged.file_name().is_some_and(|name| name == "staged"))
        .ok_or("MIDNIGHT_ACCEPTANCE_RUN_DIR is not the `<work>/staged/run` of a staged process")?;
    #[cfg(unix)]
    let folders = [(
        "XDG_CONFIG_HOME (or $HOME/.config)",
        absolute("XDG_CONFIG_HOME").or_else(|| absolute("HOME").map(|home| home.join(".config"))),
    )];
    #[cfg(not(unix))]
    let folders = [
        ("USERPROFILE", absolute("USERPROFILE")),
        ("LOCALAPPDATA", absolute("LOCALAPPDATA")),
    ];
    for (variable, folder) in folders {
        match folder {
            Some(folder) if folder.starts_with(&staged) => {}
            Some(folder) => {
                return Err(format!(
                    "{variable} {} is not below the private folder {}",
                    folder.display(),
                    staged.display()
                ));
            }
            None => return Err(format!("{variable} is not an absolute folder")),
        }
    }
    Ok(())
}

/// The settings service of a run: the application's own store (opt-in), the opt-in
/// store shared across restarts, otherwise in-memory preferences whose saves are
/// forgotten; slowed down by `Config::settings_delay`. Also which panels have a saved
/// tab session the first read restores.
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
    let (service, saved): (Arc<dyn SettingsService>, _) = if config.real_settings {
        if let Err(problem) = private_settings() {
            panic!("Config::real_settings needs a staged process with private settings: {problem}");
        }
        // Read once before the start, as production's startup read does; the engine's
        // first load then reads the store again, asynchronously.
        let stored = SettingsService::load_preferences_for_acceptance(local.as_ref()).ok();
        (
            local.clone(),
            stored.as_ref().map_or([false; 2], restorable),
        )
    } else {
        match &config.settings {
            Some(store) => (store.service(preferences), restorable(&store.current())),
            None => {
                let saved = restorable(&preferences);
                (Arc::new(Settings(preferences)), saved)
            }
        }
    };
    match config.settings_delay {
        Some(delay) => (
            Arc::new(SlowSettings {
                inner: service,
                delay,
            }),
            saved,
        ),
        None => (service, saved),
    }
}

/// A settings store whose reads answer `delay` late, as one in a slow profile folder;
/// everything else goes to `inner` unchanged. The Engine reads on a blocking worker, so
/// the delay holds up nothing but the read.
struct SlowSettings {
    inner: Arc<dyn SettingsService>,
    delay: Duration,
}

impl SettingsService for SlowSettings {
    fn font_families(&self) -> Result<Vec<String>, FsError> {
        self.inner.font_families()
    }

    fn load_preferences(&self) -> Result<UserPreferences, FsError> {
        std::thread::sleep(self.delay);
        self.inner.load_preferences()
    }

    fn save_preferences(&self, preferences: UserPreferences) -> Result<(), FsError> {
        self.inner.save_preferences(preferences)
    }

    fn load_preferences_for_acceptance(&self) -> Result<UserPreferences, FsError> {
        std::thread::sleep(self.delay);
        self.inner.load_preferences_for_acceptance()
    }

    fn save_preferences_prepared(
        &self,
        preferences: UserPreferences,
        proof: ComparatorValidationProof<'_>,
    ) -> Result<(), FsError> {
        self.inner.save_preferences_prepared(preferences, proof)
    }
}

/// Opt-in services that launch programs. Custom Commands and the external editor start
/// the programs a scenario configured, so only scenarios that configure harmless
/// commands inside their work folder enable them. No terminal is ever started: the
/// slots that launch terminals (Custom Commands' terminal mode, the desktop's command
/// line and terminal button) get only the recording stand-ins of `launches.rs`, never
/// the platform's own launcher.
fn install_opt_in_services(
    engine: &mut Engine,
    config: &Config,
    local: &Arc<LocalFileSystem>,
    launches: &Arc<super::launches::Launches>,
) {
    if config.user_commands {
        engine.configure_user_commands().unwrap().install(Arc::new(
            super::launches::UserCommands::new(local.clone(), launches.clone()),
        ));
    }
    if config.desktop {
        engine.configure_desktop().unwrap().install(Arc::new(
            super::launches::Desktop::new(local.clone(), launches.clone())
                .with_native_properties(config.native_properties),
        ));
    }
    if config.editor {
        engine.configure_editor().unwrap().install(local.clone());
    }
}

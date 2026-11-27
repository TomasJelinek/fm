//! The update check in the application (docs/developer/UPDATES.md): the update service
//! runs on its own thread; this module connects it to the toolbar button, the dialog and
//! the Options choices, runs the startup check and confirms a new version's start, and
//! hands "Restart now" to the updater (an extracted release folder) or "Install now" to
//! the Windows Installer hand-off (an installed MSI) once the window has closed.
use midnight_ui::{MainWindow, Presenter, UpdatePhase, UpdateState, UpdateView};
use midnight_update::{
    SystemTransport, apply,
    archive::BuildInfo,
    client::{Endpoints, UpdateClient},
    controller::{Environment, Phase, STARTUP_INTERVAL, Snapshot, UpdateController},
    install::{Installation, InstanceLock, NotUpdatable, Staged},
    installer::{self, InstalledCopy, ReadyInstaller},
    release::{PLATFORM, current_version},
    trust::TrustAnchor,
};
use slint::ComponentHandle;
use std::{
    cell::RefCell,
    ffi::OsString,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

/// The startup check waits this long after the window shows.
const STARTUP_DELAY: Duration = Duration::from_secs(5);
/// A run whose settings have not been read this long after its window showed makes no
/// startup check (the check follows the user's choice, which only the settings hold).
const PREFERENCES_WAIT: Duration = Duration::from_secs(120);

/// The startup work of the update service, ticked by a timer from the event loop: the
/// new version's confirmation and the startup check.
struct StartupSchedule {
    /// The update in progress whose start this run confirms: its version and folder.
    confirm: Option<(String, PathBuf)>,
    /// False for smoke and probe runs, which never check on their own.
    automatic: bool,
    shown_at: Option<Instant>,
    /// The startup check ran, or will not run in this process.
    checked: bool,
}

/// What one tick asks of the update service.
#[derive(Debug, Default)]
struct Tick {
    /// Confirm this version's start in this installation folder.
    confirm: Option<(String, PathBuf)>,
    /// Run the startup check with these choices.
    check: Option<midnight_core::UpdatePreferences>,
    /// Nothing remains to do: the timer stops.
    stop: bool,
}

impl StartupSchedule {
    fn new(confirm: Option<(String, PathBuf)>, automatic: bool) -> Self {
        Self {
            confirm,
            automatic,
            shown_at: None,
            checked: false,
        }
    }
    /// `revealed`: the main window shows. A tick runs on the event loop, so the new
    /// version then runs and is usable: it confirms its start whether or not its settings
    /// have been read (a refused or slow read leaves the defaults in place). A process
    /// that ends before its window shows never confirms, and the updater rolls back.
    fn tick(
        &mut self,
        now: Instant,
        revealed: bool,
        preferences: Option<midnight_core::UpdatePreferences>,
    ) -> Tick {
        if !revealed {
            return Tick::default();
        }
        let confirm = self.confirm.take();
        let shown = *self.shown_at.get_or_insert(now);
        let waited = now.saturating_duration_since(shown);
        let mut check = None;
        if !self.checked {
            match preferences {
                _ if !self.automatic => self.checked = true,
                Some(updates) if waited >= STARTUP_DELAY => {
                    self.checked = true;
                    check = updates.check_at_startup.then_some(updates);
                }
                None if waited >= PREFERENCES_WAIT => self.checked = true,
                _ => {}
            }
        }
        Tick {
            confirm,
            check,
            stop: self.checked,
        }
    }
}

/// What the process learns about updates before its window exists.
#[derive(Clone)]
pub(crate) struct AtStart {
    installation: Option<PathBuf>,
    report: apply::StartupReport,
}

/// The instance locks that the process holds for its whole life.
pub(crate) struct Instance {
    _folder: Option<InstanceLock>,
    _installer: Option<installer::InstanceLock>,
}

/// Takes the instance lock, which the caller holds for the life of the process (the
/// updater waits until no copy runs from the folder), and reads what an update left.
pub(crate) fn at_start() -> (Instance, AtStart) {
    let executable = std::env::current_exe().ok();
    let installation = executable
        .as_deref()
        .and_then(|executable| executable.parent().map(Path::to_path_buf));
    let instance = installation.as_deref().and_then(InstanceLock::acquire);
    // The installed release's own version: the updater put this BUILD-INFO.json in place
    // with the executable it describes (checked against its `binary_sha256`).
    let mut report = match (&installation, &instance) {
        (Some(folder), Some(_)) => {
            let version = BuildInfo::read(folder)
                .map(|info| info.version)
                .unwrap_or_else(|_| current_version());
            apply::startup(folder, &version.to_string())
        }
        _ => apply::StartupReport::default(),
    };
    // A copy that Windows Installer may manage (no BUILD-INFO.json): its instance lock
    // lives in the private folder, where the hand-off also leaves its outcome. Whether
    // Windows Installer really manages it is asked later, on the update thread.
    let mut installer_lock = None;
    if let (Some(executable), Some(folder)) = (&executable, &installation)
        && msi_candidate(executable, folder)
        && let Some(private) = midnight_update::private_folder()
    {
        installer_lock = installer::InstanceLock::acquire(&private, folder);
        report.installer = installer::startup(&private, folder);
    }
    (
        Instance {
            _folder: instance,
            _installer: installer_lock,
        },
        AtStart {
            installation,
            report,
        },
    )
}

/// Whether `executable` may be an MSI installation's: Windows, the MSI's executable
/// name, and no `BUILD-INFO.json` beside it.
fn msi_candidate(executable: &Path, folder: &Path) -> bool {
    cfg!(windows)
        && executable
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case(installer::EXECUTABLE))
        && !folder.join(midnight_update::archive::BUILD_INFO).exists()
}

/// What "Restart now" or "Install now" hands over once the window has closed.
pub(crate) enum Restart {
    /// The staged release that the updater swaps into the release folder.
    Swap(Installation, Staged),
    /// The verified MSI that the hand-off gives to Windows Installer.
    Installer(ReadyInstaller),
}

/// "Restart now" or "Install now" was chosen.
pub(crate) type RestartRequest = Rc<RefCell<Option<Restart>>>;

pub(crate) struct Updates {
    controller: RefCell<UpdateController>,
    _timer: slint::Timer,
}

fn environment(installation: Option<PathBuf>, report: apply::StartupReport) -> Environment {
    let endpoints = Endpoints::from_environment();
    let transport = Arc::new(SystemTransport::new(endpoints.policy.clone()));
    let executable = std::env::current_exe().ok();
    let installation = match (PLATFORM, &executable, installation) {
        (Some(platform), Some(executable), Some(_)) => Installation::detect(executable, &platform),
        _ => Err(NotUpdatable::NotAReleaseFolder),
    };
    let (installation, copy) = windows_installer(installation, executable.as_deref());
    let private_folder = midnight_update::private_folder()
        .unwrap_or_else(|| std::env::temp_dir().join("midnight-explorer-updates"));
    Environment {
        client: UpdateClient::new(transport, endpoints),
        anchor: TrustAnchor::built_in(),
        current: current_version(),
        platform: PLATFORM,
        installation,
        installer: copy,
        private_folder,
        startup: report,
    }
}

/// A copy without a release folder that Windows Installer installed updates with the
/// release's MSI; a recorded mismatch replaces the generic reason.
#[cfg(windows)]
fn windows_installer(
    installation: Result<Installation, NotUpdatable>,
    executable: Option<&Path>,
) -> (Result<Installation, NotUpdatable>, Option<InstalledCopy>) {
    match (installation, executable) {
        (Err(NotUpdatable::NotAReleaseFolder), Some(executable)) => {
            match installer::detect_system(executable, &current_version()) {
                Ok(copy) => (Err(NotUpdatable::NotAReleaseFolder), Some(copy)),
                Err(reason) => (Err(reason), None),
            }
        }
        (installation, _) => (installation, None),
    }
}

#[cfg(not(windows))]
fn windows_installer(
    installation: Result<Installation, NotUpdatable>,
    _executable: Option<&Path>,
) -> (Result<Installation, NotUpdatable>, Option<InstalledCopy>) {
    (installation, None)
}

/// The view of a snapshot, as the button and dialog show it.
pub(crate) fn view(snapshot: &Snapshot) -> UpdateView {
    let (phase, received, total, message) = match &snapshot.phase {
        Phase::Idle => (UpdatePhase::Idle, 0, 0, String::new()),
        Phase::Checking => (UpdatePhase::Checking, 0, 0, String::new()),
        Phase::UpToDate => (UpdatePhase::UpToDate, 0, 0, String::new()),
        Phase::Available => (UpdatePhase::Available, 0, 0, String::new()),
        Phase::Downloading { received, total } => {
            (UpdatePhase::Downloading, *received, *total, String::new())
        }
        Phase::Verifying => (UpdatePhase::Verifying, 0, 0, String::new()),
        Phase::Ready => (UpdatePhase::Ready, 0, 0, String::new()),
        Phase::Failed { message } => (UpdatePhase::Failed, 0, 0, message.clone()),
    };
    let offer = snapshot.offer.as_ref();
    let blocked = snapshot
        .unavailable
        .clone()
        .or_else(|| offer.and_then(|offer| offer.blocked.clone()))
        .unwrap_or_default();
    UpdateView {
        phase,
        current_version: snapshot.current.clone(),
        latest_version: offer.map(|offer| offer.version.clone()).unwrap_or_default(),
        notes: offer.map(|offer| offer.notes.clone()).unwrap_or_default(),
        notes_cut: offer.is_some_and(|offer| offer.notes_cut),
        preview: offer.is_some_and(|offer| offer.preview),
        skipped: offer.is_some_and(|offer| offer.skipped),
        published: offer
            .map(|offer| offer.published.clone())
            .unwrap_or_default(),
        last_checked: snapshot.last_checked,
        message,
        blocked,
        notice: snapshot.notice.clone().unwrap_or_default(),
        received,
        total,
        page_url: offer
            .map(|offer| offer.page_url.clone())
            .unwrap_or_default(),
        windows_installer: snapshot.installer.is_some(),
        needs_approval: snapshot.installer == Some(installer::Scope::Machine),
    }
}

impl Updates {
    /// Connects the update service to `window`. `automatic` is false for smoke and probe
    /// runs, which never contact GitHub on their own.
    pub(crate) fn install(
        window: &MainWindow,
        presenter: Rc<RefCell<Presenter>>,
        at_start: AtStart,
        automatic: bool,
        revealed: Rc<dyn Fn() -> bool>,
        restart: RestartRequest,
    ) -> Rc<Self> {
        let weak = window.as_weak();
        let confirm = at_start
            .report
            .confirm
            .clone()
            .zip(at_start.installation.clone());
        let installation = at_start.installation.clone();
        let report = at_start.report.clone();
        let controller = UpdateController::start_with(
            move || environment(installation, report),
            Box::new(move |snapshot| {
                let view = view(&snapshot);
                let _ = weak.upgrade_in_event_loop(move |window| {
                    midnight_ui::show_update(&window, &view);
                });
            }),
        );
        let updates = Rc::new(Self {
            controller: RefCell::new(controller),
            _timer: slint::Timer::default(),
        });
        updates.connect(window, &presenter, restart);
        updates.schedule_startup(presenter, automatic, revealed, confirm);
        updates
    }

    fn previews(presenter: &Rc<RefCell<Presenter>>) -> bool {
        presenter
            .try_borrow()
            .ok()
            .and_then(|p| p.loaded_update_preferences())
            .unwrap_or_default()
            .previews
    }

    fn connect(
        self: &Rc<Self>,
        window: &MainWindow,
        presenter: &Rc<RefCell<Presenter>>,
        restart: RestartRequest,
    ) {
        let state = window.global::<UpdateState>();
        let this = Rc::downgrade(self);
        let weak = window.as_weak();
        let preferences = presenter.clone();
        state.on_open_dialog(move |check| {
            let (Some(this), Some(window)) = (this.upgrade(), weak.upgrade()) else {
                return;
            };
            midnight_ui::open_update_dialog(&window);
            if check {
                this.controller
                    .borrow()
                    .check(true, Self::previews(&preferences));
            }
        });
        let this = Rc::downgrade(self);
        let preferences = presenter.clone();
        state.on_check_now(move || {
            if let Some(this) = this.upgrade() {
                this.controller
                    .borrow()
                    .check(true, Self::previews(&preferences));
            }
        });
        let this = Rc::downgrade(self);
        state.on_download(move || {
            if let Some(this) = this.upgrade() {
                this.controller.borrow().download();
            }
        });
        let this = Rc::downgrade(self);
        state.on_skip(move || {
            if let Some(this) = this.upgrade() {
                this.controller.borrow().skip();
            }
        });
        let this = Rc::downgrade(self);
        let weak = window.as_weak();
        state.on_restart(move || {
            let (Some(this), Some(window)) = (this.upgrade(), weak.upgrade()) else {
                return;
            };
            let controller = this.controller.borrow();
            let chosen = match (controller.staged(), controller.installer_ready()) {
                (Some((installation, staged)), _) => Restart::Swap(installation, staged),
                (None, Some(ready)) => Restart::Installer(ready),
                (None, None) => return,
            };
            drop(controller);
            *restart.borrow_mut() = Some(chosen);
            midnight_ui::close_update_dialog(&window);
            // The ordinary close: running jobs are offered first, the session is saved.
            window.invoke_exit();
        });
    }

    fn schedule_startup(
        self: &Rc<Self>,
        presenter: Rc<RefCell<Presenter>>,
        automatic: bool,
        revealed: Rc<dyn Fn() -> bool>,
        confirm: Option<(String, PathBuf)>,
    ) {
        let this = Rc::downgrade(self);
        let schedule = RefCell::new(StartupSchedule::new(confirm, automatic));
        self._timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(500),
            move || {
                let Some(this) = this.upgrade() else {
                    return;
                };
                let preferences = presenter
                    .try_borrow()
                    .ok()
                    .and_then(|p| p.loaded_update_preferences());
                let tick = schedule
                    .borrow_mut()
                    .tick(Instant::now(), revealed(), preferences);
                if let Some((version, folder)) = tick.confirm {
                    std::thread::spawn(move || {
                        if let Err(error) = apply::confirm(&folder, &version) {
                            eprintln!("Midnight Explorer: cannot confirm the update: {error}");
                        }
                    });
                }
                if let Some(updates) = tick.check {
                    this.controller
                        .borrow()
                        .startup_check(updates.previews, STARTUP_INTERVAL);
                }
                if tick.stop {
                    this._timer.stop();
                }
            },
        );
    }

    /// Stops a download in progress; the process is about to exit.
    pub(crate) fn stop(&self) {
        self.controller.borrow_mut().stop();
    }
}

/// Starts the updater or the Windows Installer hand-off for `restart` and returns; the
/// caller exits right after.
pub(crate) fn start_updater(
    restart: Restart,
    arguments: Vec<OsString>,
    working_folder: PathBuf,
) -> Result<(), String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let (installation, staged) = match restart {
        Restart::Swap(installation, staged) => (installation, staged),
        Restart::Installer(ready) => {
            let private = midnight_update::private_folder()
                .ok_or_else(|| "The private update folder cannot be used.".to_owned())?;
            let (helper, request) =
                installer::prepare(&ready, &executable, &private, working_folder, arguments)
                    .map_err(|error| format!("The installation cannot be started: {error}"))?;
            return installer::start(&helper, &request)
                .map_err(|error| format!("The installation cannot be started: {error}"));
        }
    };
    let (updater, request) = apply::prepare_restart(
        &installation,
        &staged.folder,
        &executable,
        working_folder,
        arguments,
    )
    .map_err(|error| format!("The update cannot be started: {error}"))?;
    apply::start_updater(&updater, &request)
        .map_err(|error| format!("The updater cannot be started: {error}"))
}

#[cfg(test)]
mod startup_tests {
    use super::*;
    use midnight_core::UpdatePreferences;

    const TICK: Duration = Duration::from_millis(500);
    const VERSION: &str = "0.5.2";

    /// An installation folder with an update in progress.
    fn installation(name: &str) -> PathBuf {
        let folder = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../_build/tmp")
            .join(format!("update-start-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(midnight_update::install::work_folder(&folder)).unwrap();
        folder
    }

    /// Whether the updater, which reads `confirmed` while it waits, sees this start
    /// confirmed.
    fn confirmed(folder: &Path) -> bool {
        std::fs::read_to_string(midnight_update::install::work_folder(folder).join("confirmed"))
            .is_ok_and(|text| text.trim() == VERSION)
    }

    /// Ticks the timer every 500 ms for `span`, as the event loop does: the window shows
    /// from `shown` on and the settings are read from `read` on (`None`: never). Returns
    /// when the start was confirmed, when the startup check ran and when the timer
    /// stopped, as times since the first tick.
    struct Run {
        confirmed: Option<Duration>,
        checked: Option<Duration>,
        stopped: Option<Duration>,
    }
    fn simulate(
        folder: &Path,
        automatic: bool,
        span: Duration,
        shown: Option<Duration>,
        read: Option<Duration>,
    ) -> Run {
        let start = Instant::now();
        let mut schedule =
            StartupSchedule::new(Some((VERSION.into(), folder.to_owned())), automatic);
        let mut result = Run {
            confirmed: None,
            checked: None,
            stopped: None,
        };
        let mut elapsed = Duration::ZERO;
        while elapsed <= span {
            let preferences = read
                .filter(|read| elapsed >= *read)
                .map(|_| UpdatePreferences {
                    check_at_startup: true,
                    previews: true,
                });
            let revealed = shown.is_some_and(|shown| elapsed >= shown);
            let tick = schedule.tick(start + elapsed, revealed, preferences);
            if let Some((version, installation)) = tick.confirm {
                assert!(result.confirmed.is_none(), "confirmed twice");
                apply::confirm(&installation, &version).unwrap();
                result.confirmed = Some(elapsed);
            }
            if tick.check.is_some() {
                assert!(result.checked.is_none(), "checked twice");
                result.checked = Some(elapsed);
            }
            if tick.stop {
                result.stopped = Some(elapsed);
                break;
            }
            elapsed += TICK;
        }
        result
    }

    #[test]
    fn a_settings_read_slower_than_the_confirmation_window_does_not_roll_the_update_back() {
        let window = apply::Timings::default().confirm;
        let folder = installation("slow-settings");
        // The window shows after the startup deadline; the settings arrive after the
        // updater stopped waiting.
        let late = window + Duration::from_secs(20);
        let run = simulate(
            &folder,
            true,
            late * 2,
            Some(Duration::from_secs(10)),
            Some(late),
        );
        let at = run.confirmed.expect("the start was never confirmed");
        assert!(at <= Duration::from_secs(10) + TICK, "{at:?}");
        assert!(at < window);
        assert!(confirmed(&folder));
        // Settings that arrive this late skip the startup check of this run.
        assert_eq!(run.checked, None);
        assert_eq!(
            run.stopped,
            Some(Duration::from_secs(10) + PREFERENCES_WAIT)
        );
        let _ = std::fs::remove_dir_all(&folder);
        // Read within the wait, they decide the startup check as before.
        let folder = installation("late-settings");
        let read = Duration::from_secs(60);
        let run = simulate(
            &folder,
            true,
            late,
            Some(Duration::from_secs(10)),
            Some(read),
        );
        assert_eq!(run.confirmed, Some(Duration::from_secs(10)));
        assert_eq!(run.checked, Some(read));
        assert_eq!(run.stopped, Some(read));
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_start_whose_window_never_shows_stays_unconfirmed_and_rolls_back() {
        let folder = installation("never-shown");
        let window = apply::Timings::default().confirm;
        let run = simulate(&folder, true, window * 2, None, Some(Duration::ZERO));
        assert_eq!(run.confirmed, None);
        assert_eq!(run.checked, None);
        // The updater finds no confirmation: when this process exits it rolls back.
        assert!(!confirmed(&folder));
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn the_timer_stops_once_confirmed_even_when_the_settings_never_arrive() {
        let folder = installation("no-settings");
        let run = simulate(
            &folder,
            true,
            Duration::from_secs(3600),
            Some(Duration::ZERO),
            None,
        );
        assert_eq!(run.confirmed, Some(Duration::ZERO));
        assert_eq!(run.checked, None);
        assert_eq!(run.stopped, Some(PREFERENCES_WAIT));
        // A run that never checks on its own stops as soon as it confirmed.
        let run = run_manual(&folder);
        assert_eq!(run.stopped, Some(Duration::ZERO));
        let _ = std::fs::remove_dir_all(&folder);
    }

    fn run_manual(folder: &Path) -> Run {
        simulate(
            folder,
            false,
            Duration::from_secs(60),
            Some(Duration::ZERO),
            None,
        )
    }
}

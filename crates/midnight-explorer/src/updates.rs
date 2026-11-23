//! The update check in the application (docs/developer/UPDATES.md): the update service
//! runs on its own thread; this module connects it to the toolbar button, the dialog and
//! the Options choices, runs the startup check and confirms a new version's start, and
//! hands "Restart now" to the updater once the window has closed.
use midnight_ui::{MainWindow, Presenter, UpdatePhase, UpdateState, UpdateView};
use midnight_update::{
    SystemTransport, apply,
    archive::BuildInfo,
    client::{Endpoints, UpdateClient},
    controller::{Environment, Phase, STARTUP_INTERVAL, Snapshot, UpdateController},
    install::{Installation, InstanceLock, NotUpdatable, Staged},
    release::{PLATFORM, current_version},
    trust::TrustAnchor,
};
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    ffi::OsString,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

/// The startup check waits this long after the window shows.
const STARTUP_DELAY: Duration = Duration::from_secs(5);

/// What the process learns about updates before its window exists.
#[derive(Clone)]
pub(crate) struct AtStart {
    installation: Option<PathBuf>,
    report: apply::StartupReport,
}

/// Takes the instance lock, which the caller holds for the life of the process (the
/// updater waits until no copy runs from the folder), and reads what an update left.
pub(crate) fn at_start() -> (Option<InstanceLock>, AtStart) {
    let installation = std::env::current_exe()
        .ok()
        .and_then(|executable| executable.parent().map(Path::to_path_buf));
    let instance = installation.as_deref().and_then(InstanceLock::acquire);
    // The installed release's own version: the updater put this BUILD-INFO.json in place
    // with the executable it describes (checked against its `binary_sha256`).
    let report = match (&installation, &instance) {
        (Some(folder), Some(_)) => {
            let version = BuildInfo::read(folder)
                .map(|info| info.version)
                .unwrap_or_else(|_| current_version());
            apply::startup(folder, &version.to_string())
        }
        _ => apply::StartupReport::default(),
    };
    (
        instance,
        AtStart {
            installation,
            report,
        },
    )
}

/// "Restart now" was chosen: what the updater installs once the window has closed.
pub(crate) type RestartRequest = Rc<RefCell<Option<(Installation, Staged)>>>;

pub(crate) struct Updates {
    controller: RefCell<UpdateController>,
    _timer: slint::Timer,
}

fn environment(installation: Option<PathBuf>, report: apply::StartupReport) -> Environment {
    let endpoints = Endpoints::from_environment();
    let transport = Arc::new(SystemTransport::new(endpoints.policy.clone()));
    let installation = match (PLATFORM, std::env::current_exe(), installation) {
        (Some(platform), Ok(executable), Some(_)) => Installation::detect(&executable, &platform),
        _ => Err(NotUpdatable::NotAReleaseFolder),
    };
    let private_folder = midnight_update::private_folder()
        .unwrap_or_else(|| std::env::temp_dir().join("midnight-explorer-updates"));
    Environment {
        client: UpdateClient::new(transport, endpoints),
        anchor: TrustAnchor::built_in(),
        current: current_version(),
        platform: PLATFORM,
        installation,
        private_folder,
        startup: report,
    }
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
            let Some(staged) = this.controller.borrow().staged() else {
                return;
            };
            *restart.borrow_mut() = Some(staged);
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
        let shown_at = Cell::new(None::<Instant>);
        let confirm = RefCell::new(confirm);
        self._timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(500),
            move || {
                let Some(this) = this.upgrade() else {
                    return;
                };
                let Some(updates) = presenter
                    .try_borrow()
                    .ok()
                    .and_then(|p| p.loaded_update_preferences())
                else {
                    return;
                };
                if !revealed() {
                    return;
                }
                // The new version's startup check: its window shows with the user's
                // settings, panels and tabs restored.
                if let Some((version, folder)) = confirm.borrow_mut().take() {
                    std::thread::spawn(move || {
                        if let Err(error) = apply::confirm(&folder, &version) {
                            eprintln!("Midnight Explorer: cannot confirm the update: {error}");
                        }
                    });
                }
                let shown = shown_at.get().unwrap_or_else(Instant::now);
                shown_at.set(Some(shown));
                if shown.elapsed() < STARTUP_DELAY {
                    return;
                }
                if automatic && updates.check_at_startup {
                    this.controller
                        .borrow()
                        .startup_check(updates.previews, STARTUP_INTERVAL);
                }
                this._timer.stop();
            },
        );
    }

    /// Stops a download in progress; the process is about to exit.
    pub(crate) fn stop(&self) {
        self.controller.borrow_mut().stop();
    }
}

/// Starts the updater for `restart` and returns; the caller exits right after.
pub(crate) fn start_updater(
    restart: (Installation, Staged),
    arguments: Vec<OsString>,
    working_folder: PathBuf,
) -> Result<(), String> {
    let (installation, staged) = restart;
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
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

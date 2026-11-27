//! Viewers, comparators, thumbnails and archive browsing through the installed plugin
//! packages, composed as `crates/midnight-explorer/src/main.rs` (`run_window`) composes
//! them: browsing and archive capture through the shared installed native filesystem
//! plugin, viewer and comparison services on their own resident workers, the F3/Alt+F3
//! and Compare callbacks, the content-appearance bridge and a thumbnail pulse.
//!
//! The application's own modules cannot be included by path here (their unit-test
//! modules need the explorer crate), so this mirrors them: F3 and Alt+F3 ask for an
//! encrypted member's archive password first and close an open filename search
//! (`view_needs_archive_password`, `view_command`), the viewer of an unlocked member
//! opens through the same callback once the dialog has closed, a typed zoom reads the
//! presenter's number format, and the thumbnail pulse offers program tiles for their
//! own artwork (`refresh_file_icons`). Deliberate differences, documented in
//! docs/developer/ACCEPTANCE.md: no parent-folder pin cache for viewer resources (each
//! open prepares its folder), no external comparator programs, and the thumbnail pulse
//! presents native files only (no archive members, command artwork, animation or
//! upload retries); the thumbnail cache lives in the scenario's folder.
//!
//! Plugin discovery is relative to the running executable, so this works only in a
//! staged child process (`viewers_stage`). The helpers below act on viewer and
//! comparator windows through keys and pointer events and read their state;
//! [`probe`] measures scrolling, rendered pixels and contrast, and [`generated`] writes
//! the content the viewer scenarios generate in their own work folders.
#[path = "viewers_fixtures.rs"]
pub mod generated;
#[path = "viewers_probe.rs"]
pub mod probe;

use super::{Context, app::App, input};
use midnight_compare_ui::{ComparatorWindow, ComparatorWindows};
use midnight_core::{
    Cancellation, Command, Event, FONT_SIZE_MAX, FONT_SIZE_MIN, FsError, NumberLocale, PaneId,
    PathPickerService, PreferenceTheme, ShortcutChord, StorageKind, StorageService,
    TransferService, UiDensity, ViewerAppearance, ViewerLaunch, ViewerLinkRequest,
};
use midnight_engine::{Engine, EngineLifetime};
use midnight_native_filesystem_host::{InstalledProvider, SharedInstalledProvider};
use midnight_platform::LocalFileSystem;
use midnight_thumbnail_host::service as thumbnail;
use midnight_ui::{
    ArchivePasswords, MainWindow, Presenter, Theme, ThumbnailContent, ThumbnailTarget,
};
use midnight_viewer_ui::{ViewerWindow, ViewerWindows};
use midnight_work_budget::resident;
use slint::{
    ComponentHandle, SharedString,
    platform::{Key, WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

/// Production's exit grace for content workers and the image spare.
const CONTENT_GRACE: Duration = Duration::from_millis(2500);
/// Production's thumbnail pulse interval.
const THUMBNAIL_PULSE: Duration = Duration::from_millis(100);

// ---------------------------------------------------------------------------------------
// Composition

/// The shared installed native filesystem plugin, created before the Engine.
pub struct Native {
    shared: Option<SharedInstalledProvider>,
    pub provider: Arc<InstalledProvider>,
    local: Arc<LocalFileSystem>,
}

thread_local! {
    /// Whether the next application this thread starts gets the desktop icon reader.
    static FILE_ICONS: Cell<bool> = const { Cell::new(false) };
    /// The application's recent events and commands as each password submission was
    /// answered ([`submit_password`]). The trace keeps only its last lines, so a leak
    /// check at the end of a scenario would no longer hold the unlock commands.
    static SUBMISSION_TRACES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// The trace lines [`submit_password`] kept since the last call, oldest first.
pub fn submission_traces() -> Vec<String> {
    SUBMISSION_TRACES.with(RefCell::take)
}

/// Give the next [`App::start`] of this thread the desktop service's icon reader, so
/// tiles of programs show their own artwork, as the application's desktop service
/// reads it (decision A3). Only reading: [`IconReader`] refuses to start programs or
/// terminals, opens no native Properties and no system location.
pub fn read_file_icons() {
    FILE_ICONS.with(|icons| icons.set(true));
}

/// The desktop service of a scenario that asked for [`read_file_icons`]: the platform's
/// own reader of the artwork listed programs carry (`load_file_icon`) and its read-only
/// queries (locations, link targets, property details). Type icons stay the semantic
/// ones the harness always shows (`load_icon` reads none), so no shell icon handler runs,
/// and everything that would start a program is refused.
pub struct IconReader(Arc<LocalFileSystem>);

fn refused(operation: &'static str) -> FsError {
    FsError::from_io(
        operation,
        Path::new(""),
        io::Error::new(
            io::ErrorKind::Unsupported,
            "The acceptance harness starts no programs.",
        ),
    )
}

impl midnight_core::IconService for IconReader {
    fn load_icon(
        &self,
        _: &midnight_core::IconKey,
    ) -> Result<Option<midnight_core::IconPixels>, FsError> {
        Ok(None)
    }

    fn icon_edge(&self, drawn: u16) -> u16 {
        midnight_core::IconService::icon_edge(self.0.as_ref(), drawn)
    }

    fn load_file_icon(
        &self,
        path: &Path,
        edge: u16,
        cancel: &Cancellation,
    ) -> Result<Option<midnight_core::IconPixels>, FsError> {
        midnight_core::IconService::load_file_icon(self.0.as_ref(), path, edge, cancel)
    }
}

impl midnight_core::TerminalService for IconReader {
    fn shell_flavor(&self) -> midnight_core::ShellFlavor {
        midnight_core::TerminalService::shell_flavor(self.0.as_ref())
    }

    fn launch_terminal(
        &self,
        _: &midnight_core::TerminalRequest,
    ) -> Result<Box<dyn midnight_core::TerminalProcess>, FsError> {
        Err(refused("terminal"))
    }
}

impl midnight_core::PropertyService for IconReader {
    fn show_properties(&self, _: &[PathBuf]) -> Result<bool, FsError> {
        // The metadata summary instead of the native dialog.
        Ok(false)
    }

    fn property_details(
        &self,
        path: &Path,
    ) -> Result<Option<midnight_core::PropertyDetails>, FsError> {
        midnight_core::PropertyService::property_details(self.0.as_ref(), path)
    }
}

impl midnight_core::LocationService for IconReader {
    fn locations(&self) -> Result<Vec<midnight_core::NavigationLocation>, FsError> {
        midnight_core::LocationService::locations(self.0.as_ref())
    }
}

impl midnight_core::OpenService for IconReader {
    fn open_entry(&self, path: &Path) -> Result<midnight_core::OpenEntryResult, FsError> {
        // A folder (or a link to one) stays in the file manager; a file would start the
        // program associated with it.
        if path.is_dir() {
            midnight_core::OpenService::open_entry(self.0.as_ref(), path)
        } else {
            Err(refused("open"))
        }
    }

    fn read_link_target(&self, link: &Path) -> Result<Option<PathBuf>, FsError> {
        midnight_core::OpenService::read_link_target(self.0.as_ref(), link)
    }
}

impl Native {
    pub fn new(local: &Arc<LocalFileSystem>) -> Self {
        // The process singleton, as in production; its installation is the folder of
        // the running (staged) executable.
        let shared = InstalledProvider::shared_composition(None);
        let provider = shared.service();
        provider.set_size_directory_service(local.clone());
        Self {
            shared: Some(shared),
            provider,
            local: local.clone(),
        }
    }

    /// The Engine browsing through the plugin, as `run_window` creates it.
    pub fn engine(&mut self, lifetime: EngineLifetime, events: mpsc::Sender<Event>) -> Engine {
        Engine::with_shared_native_filesystem(
            lifetime,
            self.shared
                .take()
                .expect("one Engine per installed composition"),
            events,
            Arc::new(|| {}),
        )
    }

    /// Archive browsing reads containers through the same plugin; a scenario that asked
    /// for [`read_file_icons`] gets the icon reader as the desktop service.
    pub fn configure(&self, engine: &mut Engine) {
        engine
            .configure_archive_capture()
            .expect("archive capture")
            .install(archive_capture(self.provider.clone()));
        if FILE_ICONS.with(|icons| icons.replace(false)) {
            engine
                .configure_desktop()
                .expect("desktop service")
                .install(Arc::new(IconReader(self.local.clone())));
        }
    }
}

/// `native_content::factory`: the comparator reads both files through the plugin.
fn plain_capture(filesystem: Arc<InstalledProvider>) -> midnight_compare_engine::NativeCapture {
    Arc::new(move |path, cancel| {
        filesystem
            .capture(path, &|| cancel.load(Ordering::Acquire))
            .map_err(|failure| format!("Native filesystem: {failure}"))
    })
}

/// `native_content::archive`: containers are captured through the installed plugin.
fn archive_capture(filesystem: Arc<InstalledProvider>) -> midnight_engine::ArchiveCapture {
    Arc::new(move |path, expected, cancel| {
        filesystem
            .capture_matching(path, expected, &|| cancel.is_cancelled())
            .map_err(|failure| {
                let error = if cancel.is_cancelled() {
                    io::ErrorKind::Interrupted.into()
                } else {
                    io::Error::other(format!("Native filesystem: {failure}"))
                };
                FsError::from_io("capture archive", path, error)
            })
    })
}

/// When a viewer reported its initial content (or failure), and the result.
pub type Ready = Rc<RefCell<Option<(Instant, Result<(), String>)>>>;

/// What the viewer and comparator launches record: the windows opened and the
/// refusals.
type Records<'a> = (&'a Rc<RefCell<Vec<Opened>>>, &'a Rc<RefCell<Vec<String>>>);

/// One viewer opened with F3 or Alt+F3.
#[derive(Clone)]
pub struct Opened {
    pub window: slint::Weak<ViewerWindow>,
    /// When the viewer window was created and shown (still inside the key event).
    pub constructed: Instant,
    pub ready: Ready,
}

/// One comparator opened with Compare.
#[derive(Clone)]
pub struct Compared {
    pub window: slint::Weak<ComparatorWindow>,
}

/// What the thumbnail pulse saw, for scenarios.
#[derive(Default)]
pub struct ThumbnailLog {
    /// Frames the presenter accepted.
    pub presented: usize,
    /// Notices the thumbnail service reported (for example missing thumbnailers).
    pub notices: Vec<String>,
    /// Identities of every presented thumbnail.
    pub identities: Vec<String>,
    /// Files of every presented thumbnail, in presentation order.
    pub paths: Vec<PathBuf>,
}

#[derive(Default)]
struct ThumbnailState {
    previous: Vec<ThumbnailTarget>,
    scale: f32,
    tokens: HashMap<String, u64>,
    paths: HashMap<u64, PathBuf>,
    next_token: u64,
}

pub struct Plugins {
    viewers: Rc<ViewerWindows>,
    comparators: Rc<ComparatorWindows>,
    viewer_service: midnight_viewer_engine::Service,
    spare: midnight_viewer_engine::ImageSpareRetirement,
    owners: Vec<(&'static str, resident::Owner)>,
    thumbnails: Rc<thumbnail::Service>,
    pulse: slint::Timer,
    pub thumbnail_log: Rc<RefCell<ThumbnailLog>>,
    /// Every viewer opened, oldest first.
    pub opened: Rc<RefCell<Vec<Opened>>>,
    /// Every comparator opened, oldest first.
    pub compared: Rc<RefCell<Vec<Compared>>>,
    /// Every text a viewer handed to the clipboard (Ctrl+C), oldest first.
    pub copied: Rc<RefCell<Vec<String>>>,
}

impl Plugins {
    /// Install after `install_with_owned`, before any input. `state` keeps the
    /// thumbnail cache (never the user's).
    #[allow(clippy::too_many_arguments)]
    pub fn install(
        native: &Native,
        window: &MainWindow,
        presenter: &Rc<RefCell<Presenter>>,
        send: Rc<dyn Fn(Command) -> bool>,
        runtime: &tokio::runtime::Handle,
        local: Arc<LocalFileSystem>,
        picker: Arc<dyn PathPickerService>,
        state: &Path,
    ) -> Self {
        let (viewer_owner, viewer_spawner) = resident::Owner::start_elastic(
            "midnight-viewer",
            midnight_viewer_engine::RESIDENT_WORKERS,
        )
        .expect("viewer workers");
        let (compare_owner, compare_spawner) =
            resident::Owner::start_elastic("midnight-compare", 2).expect("comparison workers");
        let viewer_service = midnight_viewer_engine::Service::with_native_capture(
            runtime.clone(),
            viewer_spawner,
            viewer_capture(native.provider.clone(), local),
        );
        let spare = viewer_service.image_spare_retirement();
        let viewers = Rc::new(ViewerWindows::new(viewer_service.clone()));
        // As in production: viewer windows move at once when the system asks for less
        // motion (the acceptance seam's `override_settings` can replace the answer).
        viewers.set_motion_preference(Rc::new(midnight_ui::system_reduces_motion));
        // A typed zoom percentage uses the user's decimal separator: the presenter's
        // number format, which [`use_number_locale`] can replace.
        let locale = presenter.clone();
        viewers.set_number_locale(Rc::new(move || {
            locale
                .try_borrow()
                .map(|presenter| presenter.number_locale().clone())
                .unwrap_or_default()
        }));
        let settings = presenter.clone();
        let settings_send = send.clone();
        viewers.set_preferences_listener(Rc::new(move |change| {
            settings
                .borrow_mut()
                .save_viewer_preferences(change, settings_send.as_ref());
        }));
        // As in production (`viewers::install_link_listener`): a web or mail link followed
        // in a viewer goes to the desktop service (recorded by `Config::desktop`'s
        // stand-in, never opened), a folder opens and any other file is shown in the
        // active panel; the viewer's status line hears how it went.
        let (links, link_send, link_window) = (presenter.clone(), send.clone(), window.as_weak());
        viewers.set_link_listener(Rc::new(move |request, reply| {
            let Some(main) = link_window.upgrade() else {
                return;
            };
            let (path, folder) = match request {
                ViewerLinkRequest::OpenExternal(url) => {
                    links.borrow_mut().open_external_url(
                        url,
                        link_send.as_ref(),
                        Box::new(move |outcome: Result<String, String>| {
                            reply(outcome.unwrap_or_else(|reason| reason));
                        }),
                    );
                    return;
                }
                ViewerLinkRequest::OpenFolder(path) => (path, true),
                ViewerLinkRequest::Reveal(path) => (path, false),
            };
            let shown = links
                .borrow_mut()
                .show_link_target(&path, folder, link_send.as_ref());
            midnight_ui::refresh(&main, &links.borrow());
            reply(shown);
        }));
        let opened: Rc<RefCell<Vec<Opened>>> = Rc::default();
        let copied: Rc<RefCell<Vec<String>>> = Rc::default();
        {
            let (manager, state, sender, record, weak, clipboard) = (
                viewers.clone(),
                presenter.clone(),
                send.clone(),
                opened.clone(),
                window.as_weak(),
                copied.clone(),
            );
            // `viewers::install`'s F3 and Alt+F3. The viewer of a member the archive
            // password dialog unlocked comes back here too: the dialog's bridge
            // (`archive_passwords::refresh`) invokes this callback once it has closed.
            window.on_view_file(move |pane, basic| {
                let Some(window) = weak.upgrade() else {
                    return;
                };
                let pane = if pane == 0 {
                    PaneId::Left
                } else {
                    PaneId::Right
                };
                // An encrypted member of an archive without its password yet: the
                // password dialog opens first, and the viewer once it is unlocked.
                if state.borrow_mut().view_needs_archive_password(pane, basic) {
                    midnight_ui::refresh(&window, &state.borrow());
                    return;
                }
                // An open filename search closes; it never stops the viewer.
                let launch = state
                    .borrow_mut()
                    .view_command(pane, basic, sender.as_ref());
                midnight_ui::refresh(&window, &state.borrow());
                let launch = match launch {
                    Ok(launch) => launch,
                    Err(reason) => {
                        if let Some(reason) = reason {
                            state.borrow_mut().report_viewer_error(reason);
                            midnight_ui::refresh(&window, &state.borrow());
                        }
                        return;
                    }
                };
                let ready: Ready = Rc::default();
                let marker = ready.clone();
                match open_viewer(
                    &window,
                    &state,
                    &sender,
                    &manager,
                    &clipboard,
                    launch,
                    Box::new(move |result| *marker.borrow_mut() = Some((Instant::now(), result))),
                ) {
                    Ok(viewer) => record.borrow_mut().push(Opened {
                        window: viewer,
                        constructed: Instant::now(),
                        ready,
                    }),
                    Err(error) => {
                        state.borrow_mut().report_viewer_error(error);
                        midnight_ui::refresh(&window, &state.borrow());
                    }
                }
            });
        }
        // Viewers that Find results, a fed list and panel warning reports queue on the
        // presenter, as `install_find_results` and `install_warning_reports` open them.
        window.on_refresh_find_viewer(pending_viewers(
            window,
            presenter,
            &send,
            &viewers,
            (&opened, &copied),
            Presenter::take_find_viewer,
        ));
        window.on_refresh_warning_viewer(pending_viewers(
            window,
            presenter,
            &send,
            &viewers,
            (&opened, &copied),
            Presenter::take_warning_viewer,
        ));
        window.on_refresh_file_list_viewer(file_list_viewer(
            window,
            presenter,
            &send,
            &viewers,
            (&opened, &copied),
        ));
        let storage = native.provider.clone();
        let compare_capture = native.provider.clone();
        let comparators = Rc::new(
            ComparatorWindows::new(
                midnight_compare_engine::Service::with_native_capture(
                    runtime.clone(),
                    compare_spawner,
                    plain_capture(compare_capture),
                )
                .with_storage_hint(Arc::new(move |path| {
                    storage
                        .storage_kind(path.parent().unwrap_or(path))
                        .is_ok_and(|kind| kind == StorageKind::LocalSsd)
                }))
                .with_path_picker(picker),
            )
            .with_picker_owner(midnight_ui::native_path_picker_owner),
        );
        let fonts = presenter.clone();
        let fonts_send = send.clone();
        // As the application: the comparators read the system's wheel setting and its
        // wish for less motion as the panels do.
        comparators.set_wheel_setting(Rc::new(midnight_ui::system_wheel_setting));
        comparators.set_motion_preference(Rc::new(midnight_ui::system_reduces_motion));
        comparators.set_font_listener(Rc::new(move |family| {
            fonts.borrow_mut().save_viewer_preferences(
                midnight_core::ViewerPreferenceChange::ComparisonFont(family),
                fonts_send.as_ref(),
            );
        }));
        let compared: Rc<RefCell<Vec<Compared>>> = Rc::default();
        {
            let (manager, state, record, weak) = (
                comparators.clone(),
                presenter.clone(),
                compared.clone(),
                window.as_weak(),
            );
            window.on_compare_files(move |pane| {
                let Some(window) = weak.upgrade() else {
                    return;
                };
                let result = (|| {
                    let launch = state.borrow().file_compare_launch(if pane == 0 {
                        PaneId::Left
                    } else {
                        PaneId::Right
                    })?;
                    let mut appearance = appearance(&window);
                    appearance.family = state.borrow().viewer_preferences().comparison_font.clone();
                    appearance.font_size = appearance.font_size.max(11.0);
                    let placement = midnight_compare_ui::WindowPlacement::capture(window.window());
                    let keys = state.clone();
                    manager.open(
                        launch,
                        appearance,
                        placement,
                        Rc::new(move |chord: &ShortcutChord, control| {
                            keys.borrow().comparison_key(chord, control)
                        }),
                    )
                })();
                match result {
                    Ok(comparator) => record.borrow_mut().push(Compared { window: comparator }),
                    Err(error) => {
                        state.borrow_mut().report_viewer_error(error);
                        midnight_ui::refresh(&window, &state.borrow());
                    }
                }
            });
        }
        install_appearance(window, &viewers, &comparators);
        let executable = std::env::current_exe().expect("the staged executable");
        let thumbnails = Rc::new(thumbnail::Service::start(
            executable,
            state.join("thumbnail-cache"),
        ));
        let thumbnail_log: Rc<RefCell<ThumbnailLog>> = Rc::default();
        let pulse = slint::Timer::default();
        {
            let (weak, presenter, service, log, provider, send) = (
                window.as_weak(),
                presenter.clone(),
                thumbnails.clone(),
                thumbnail_log.clone(),
                native.provider.clone(),
                send.clone(),
            );
            let state = RefCell::new(ThumbnailState {
                next_token: 1,
                ..ThumbnailState::default()
            });
            pulse.start(slint::TimerMode::Repeated, THUMBNAIL_PULSE, move || {
                if let Some(window) = weak.upgrade() {
                    thumbnail_pass(
                        &window,
                        &presenter,
                        &service,
                        &mut state.borrow_mut(),
                        &log,
                        (&provider, send.as_ref()),
                    );
                }
            });
        }
        Self {
            viewers,
            comparators,
            viewer_service,
            spare,
            owners: vec![("Viewer", viewer_owner), ("Comparison", compare_owner)],
            thumbnails,
            pulse,
            thumbnail_log,
            opened,
            compared,
            copied,
        }
    }

    /// The initial settings read was applied: prepare the image spare once and restore
    /// the loaded viewer and comparison preferences, as `SettingsLoaded::applied` does.
    pub fn settings_applied(&self, presenter: &RefCell<Presenter>) {
        let (policy, preferences, font) = {
            let state = presenter.borrow();
            (
                state.viewer_startup_policy(),
                Arc::new(state.viewer_preferences().clone()),
                state.viewer_preferences().comparison_font.clone(),
            )
        };
        if let Some((disabled, associations)) = policy {
            self.viewer_service
                .prepare_image_workers(Some(disabled), associations);
        }
        self.viewers.restore_preferences(preferences);
        self.comparators.restore_font(&font);
    }

    /// Close every content window and stop background work (UI thread, before the
    /// Engine shuts down).
    pub fn close(&self) {
        self.pulse.stop();
        self.thumbnails.stop();
        self.spare.stop_spare();
        self.viewers.close_all();
        self.comparators.close_all();
    }

    /// Await the image spare and the resident workers, within production's grace.
    pub fn finish(self, runtime: &tokio::runtime::Runtime) -> Result<(), String> {
        let Self {
            viewers,
            comparators,
            viewer_service,
            spare,
            owners,
            thumbnails,
            pulse,
            ..
        } = self;
        drop((viewers, comparators, viewer_service, thumbnails, pulse));
        let mut errors = Vec::new();
        match runtime.block_on(async { tokio::time::timeout(CONTENT_GRACE, spare.wait()).await }) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => errors.push(error),
            Err(_) => errors.push("the image spare did not retire within its grace".into()),
        }
        let waits: Vec<_> = owners
            .into_iter()
            .map(|(name, owner)| (name, owner.shutdown(runtime.handle())))
            .collect();
        for (name, mut shutdown) in waits {
            match runtime
                .block_on(async { tokio::time::timeout(CONTENT_GRACE, &mut shutdown).await })
            {
                Ok(Ok(())) => {}
                Ok(Err(error)) => errors.push(format!("{name} worker cleanup: {error}")),
                Err(_) => errors.push(overrun(name, runtime, shutdown)),
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}

/// How long a shutdown past [`CONTENT_GRACE`] is still watched, so that the failure says by
/// how much it overran (or that it did not end at all).
const OVERRUN_WATCH: Duration = Duration::from_secs(30);

/// The failure of a resident pool whose shutdown outlived [`CONTENT_GRACE`]: the jobs still
/// running then (thread, what the job last noted and for how long), each later change of
/// what runs, and when the pool retired, watched for up to [`OVERRUN_WATCH`] more.
fn overrun(
    name: &str,
    runtime: &tokio::runtime::Runtime,
    mut shutdown: resident::Shutdown,
) -> String {
    let phases = |running: &[resident::Running]| -> Vec<(String, String)> {
        running
            .iter()
            .map(|job| (job.thread.clone(), job.phase.clone()))
            .collect()
    };
    let listed = |running: &[resident::Running]| -> String {
        if running.is_empty() {
            "no job (the threads had not yet returned)".to_owned()
        } else {
            running
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        }
    };
    let at_grace = shutdown.running();
    let mut message = format!(
        "{name} workers exceeded the content exit grace ({:.1} s); still running then: {}",
        CONTENT_GRACE.as_secs_f64(),
        listed(&at_grace)
    );
    let mut last = phases(&at_grace);
    let watched = Instant::now();
    let ended = runtime.block_on(async {
        loop {
            match tokio::time::timeout(Duration::from_millis(20), &mut shutdown).await {
                Ok(result) => break Some(result),
                Err(_) if watched.elapsed() >= OVERRUN_WATCH => break None,
                Err(_) => {
                    let running = shutdown.running();
                    let now = phases(&running);
                    if now != last {
                        message.push_str(&format!(
                            "; +{:.2} s: {}",
                            watched.elapsed().as_secs_f64(),
                            listed(&running)
                        ));
                        last = now;
                    }
                }
            }
        }
    });
    match ended {
        Some(Ok(())) => message.push_str(&format!(
            "; all retired {:.2} s after the grace",
            watched.elapsed().as_secs_f64()
        )),
        Some(Err(error)) => message.push_str(&format!(
            "; retirement failed {:.2} s after the grace: {error}",
            watched.elapsed().as_secs_f64()
        )),
        None => message.push_str(&format!(
            "; still running {} s after the grace: {}",
            OVERRUN_WATCH.as_secs(),
            listed(&shutdown.running())
        )),
    }
    message
}

/// The viewer's content authority: the main file through the native plugin, local
/// resources (stylesheets, images) through a prepared parent folder.
fn viewer_capture(
    filesystem: Arc<InstalledProvider>,
    directories: Arc<dyn TransferService>,
) -> midnight_viewer_engine::NativeCapture {
    Arc::new(move |path, cancel| {
        let main = filesystem
            .capture(path, &|| cancel.load(Ordering::Acquire))
            .map_err(|failure| format!("Native filesystem: {failure}"))?;
        if cancel.load(Ordering::Acquire) {
            return Err("Viewer closed".into());
        }
        let Some(parent) = path.parent() else {
            return Ok(main);
        };
        let cancelled = Cancellation::observing(cancel.clone());
        let directory = directories
            .prepare_checksum_directory(parent, &cancelled)
            .ok();
        if cancel.load(Ordering::Acquire) {
            return Err("Viewer closed".into());
        }
        // As the application (`native_content.rs`): folders above the file's folder are
        // pinned by path when an asset first names them.
        let levels = parent
            .ancestors()
            .skip(1)
            .take(midnight_viewer_engine::MAX_ANCESTOR_LEVELS)
            .count();
        let (folder, folders, flag) = (parent.to_path_buf(), directories.clone(), cancel.clone());
        let pins: midnight_viewer_engine::AncestorPins = Arc::new(move |up| {
            let ancestor = folder.ancestors().nth(up)?;
            folders
                .prepare_checksum_directory(ancestor, &Cancellation::observing(flag.clone()))
                .ok()
        });
        match directory.and_then(|directory| {
            midnight_viewer_engine::captured_directory_resources_with_ancestors(
                directory,
                main.clone(),
                (levels > 0).then_some((levels, pins)),
            )
            .ok()
        }) {
            Some(resources) => Ok(main.with_resources(resources)),
            None => Ok(main),
        }
    })
}

/// The Make File List viewer, as `viewers.rs` opens it: `refresh` calls back while it
/// borrows the presenter, so a zero-delay timer takes the generated list's launch and
/// reports the viewer's result back to the presenter, deferred again. The viewer is
/// recorded like one opened with F3 (`records`: opened viewers, copied texts).
fn file_list_viewer(
    window: &MainWindow,
    presenter: &Rc<RefCell<Presenter>>,
    commands: &Rc<dyn Fn(Command) -> bool>,
    viewers: &Rc<ViewerWindows>,
    records: Records<'_>,
) -> impl Fn() + 'static {
    let scheduled = Rc::new(Cell::new(false));
    let (state, sender, manager, weak) = (
        presenter.clone(),
        commands.clone(),
        Rc::downgrade(viewers),
        window.as_weak(),
    );
    let (record, clipboard) = (records.0.clone(), records.1.clone());
    move || {
        if scheduled.replace(true) {
            return;
        }
        let (scheduled, state, sender, manager, weak, record, clipboard) = (
            scheduled.clone(),
            state.clone(),
            sender.clone(),
            manager.clone(),
            weak.clone(),
            record.clone(),
            clipboard.clone(),
        );
        slint::Timer::single_shot(Duration::ZERO, move || {
            scheduled.set(false);
            let (Some(window), Some(manager)) = (weak.upgrade(), manager.upgrade()) else {
                return;
            };
            let launch = state.borrow_mut().take_generated_file_list_viewer();
            let Some((id, generation, launch)) = launch else {
                return;
            };
            let ready: Ready = Rc::default();
            let marker = ready.clone();
            let (ready_state, ready_window) = (Rc::downgrade(&state), window.as_weak());
            let report = Box::new(move |result: Result<(), String>| {
                *marker.borrow_mut() = Some((Instant::now(), result.clone()));
                // Shutdown can close viewers while borrowing the presenter: defer both
                // success and cancellation, as production does.
                slint::Timer::single_shot(Duration::ZERO, move || {
                    let (Some(state), Some(window)) =
                        (ready_state.upgrade(), ready_window.upgrade())
                    else {
                        return;
                    };
                    state
                        .borrow_mut()
                        .generated_file_list_viewer_finished(id, generation, result);
                    midnight_ui::refresh(&window, &state.borrow());
                });
            });
            match open_viewer(
                &window, &state, &sender, &manager, &clipboard, launch, report,
            ) {
                Ok(viewer) => record.borrow_mut().push(Opened {
                    window: viewer,
                    constructed: Instant::now(),
                    ready,
                }),
                Err(error) => {
                    state.borrow_mut().generated_file_list_viewer_finished(
                        id,
                        generation,
                        Err(error),
                    );
                    midnight_ui::refresh(&window, &state.borrow());
                }
            }
        });
    }
}

/// `pending_viewers_callback` of `viewers.rs`: `refresh` calls back while it borrows
/// the presenter, so one zero-delay timer opens every queued launch afterwards. Each
/// viewer is recorded like one opened with F3 (`records`: opened viewers, copied texts).
fn pending_viewers(
    window: &MainWindow,
    presenter: &Rc<RefCell<Presenter>>,
    commands: &Rc<dyn Fn(Command) -> bool>,
    viewers: &Rc<ViewerWindows>,
    records: Records<'_>,
    take: fn(&mut Presenter) -> Option<ViewerLaunch>,
) -> impl Fn() + 'static {
    let scheduled = Rc::new(Cell::new(false));
    let (state, sender, manager, weak) = (
        presenter.clone(),
        commands.clone(),
        Rc::downgrade(viewers),
        window.as_weak(),
    );
    let (record, clipboard) = (records.0.clone(), records.1.clone());
    move || {
        if scheduled.replace(true) {
            return;
        }
        let (scheduled, state, sender, manager, weak, record, clipboard) = (
            scheduled.clone(),
            state.clone(),
            sender.clone(),
            manager.clone(),
            weak.clone(),
            record.clone(),
            clipboard.clone(),
        );
        slint::Timer::single_shot(Duration::ZERO, move || {
            scheduled.set(false);
            let (Some(window), Some(manager)) = (weak.upgrade(), manager.upgrade()) else {
                return;
            };
            loop {
                let launch = take(&mut state.borrow_mut());
                let Some(launch) = launch else { break };
                let ready: Ready = Rc::default();
                let marker = ready.clone();
                match open_viewer(
                    &window,
                    &state,
                    &sender,
                    &manager,
                    &clipboard,
                    launch,
                    Box::new(move |result| *marker.borrow_mut() = Some((Instant::now(), result))),
                ) {
                    Ok(viewer) => record.borrow_mut().push(Opened {
                        window: viewer,
                        constructed: Instant::now(),
                        ready,
                    }),
                    Err(error) => {
                        state.borrow_mut().report_viewer_error(error);
                        midnight_ui::refresh(&window, &state.borrow());
                    }
                }
            }
        });
    }
}

fn open_viewer(
    window: &MainWindow,
    presenter: &Rc<RefCell<Presenter>>,
    commands: &Rc<dyn Fn(Command) -> bool>,
    manager: &ViewerWindows,
    copied: &Rc<RefCell<Vec<String>>>,
    launch: ViewerLaunch,
    ready: midnight_viewer_ui::OpenReady,
) -> Result<slint::Weak<ViewerWindow>, String> {
    let theme = window.global::<Theme>();
    let mut appearance = appearance(window);
    appearance.family = theme.get_font_family().to_string();
    appearance.font_size = appearance.font_size.max(11.0);
    let keys = presenter.clone();
    let copy_state = presenter.clone();
    let sender = commands.clone();
    let record = copied.clone();
    manager.open_with_ready(
        launch,
        appearance,
        midnight_viewer_ui::WindowPlacement::capture(window.window()),
        Rc::new(move |chord: &ShortcutChord, search| keys.borrow().viewer_key(chord, search)),
        Rc::new(move |text: String| {
            // What the viewer hands to the application's clipboard path, for scenarios.
            record.borrow_mut().push(text.clone());
            copy_state
                .borrow_mut()
                .copy_viewer_text(text, sender.as_ref())
        }),
        ready,
    )
}

fn appearance(window: &MainWindow) -> ViewerAppearance {
    let theme = window.global::<Theme>();
    let rgba = |color: slint::Color| {
        let color = color.to_argb_u8();
        [color.red, color.green, color.blue, color.alpha]
    };
    ViewerAppearance {
        family: theme.get_effective_font_family().to_string(),
        font_size: theme.get_font_size(),
        foreground: rgba(theme.get_text()),
        background: rgba(theme.get_pane_background()),
        selection: rgba(theme.get_selection_background()),
        selection_text: rgba(theme.get_selection_text()),
        accent: rgba(theme.get_accent()),
    }
}

/// Copy the displayed theme to content windows, as `content_appearance::install` does.
fn install_appearance(
    window: &MainWindow,
    viewers: &Rc<ViewerWindows>,
    comparators: &Rc<ComparatorWindows>,
) {
    let viewers = Rc::downgrade(viewers);
    let comparators = Rc::downgrade(comparators);
    let weak = window.as_weak();
    let previous = RefCell::<Option<(i32, i32, SharedString, i32, i32)>>::new(None);
    window.on_refresh_content_appearance(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let theme = window.global::<Theme>();
        let key = (
            theme.get_scheme(),
            theme.get_density(),
            theme.get_effective_font_family(),
            theme.get_desktop_font_size(),
            theme.get_touch_font_size(),
        );
        if previous.borrow().as_ref() == Some(&key) {
            return;
        }
        let scheme = PreferenceTheme::from_index(key.0).unwrap_or_default();
        let density = if key.1 == 1 {
            UiDensity::Touch
        } else {
            UiDensity::Desktop
        };
        let size =
            |value: i32| value.clamp(i32::from(FONT_SIZE_MIN), i32::from(FONT_SIZE_MAX)) as u16;
        let (desktop, touch) = (size(key.3), size(key.4));
        *previous.borrow_mut() = Some(key);
        let current = appearance(&window);
        if let Some(viewers) = viewers.upgrade() {
            viewers.apply_appearance(&current, scheme, density, desktop, touch);
        }
        if let Some(comparators) = comparators.upgrade() {
            comparators.apply_appearance(&current, scheme, density, desktop, touch);
        }
    });
    window.invoke_refresh_content_appearance();
}

fn physical_edge(target: &ThumbnailTarget, scale: f32) -> u32 {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    midnight_thumbnail_host::cache::resolution_edge((target.edge as f32 * scale).ceil() as u32)
}

/// The visible row ranges and hovered rows of both panes, as `thumbnails::window_targets`
/// reads them: with one panel shown (zoomed or a compact window) only the active one's.
fn visible_ranges(window: &MainWindow) -> ([(usize, usize); 2], [SharedString; 2]) {
    let single_panel = window.get_panel_zoomed() || window.get_compact_window();
    let active_pane = window.get_active_pane();
    let (left_hover, left_range) = if !single_panel || active_pane == 0 {
        (
            window.get_left_hovered_row(),
            (
                window.get_left_thumbnail_row_start().max(0) as usize,
                window.get_left_thumbnail_row_end().max(0) as usize,
            ),
        )
    } else {
        (SharedString::default(), (0, 0))
    };
    let (right_hover, right_range) = if !single_panel || active_pane == 1 {
        (
            window.get_right_hovered_row(),
            (
                window.get_right_thumbnail_row_start().max(0) as usize,
                window.get_right_thumbnail_row_end().max(0) as usize,
            ),
        )
    } else {
        (SharedString::default(), (0, 0))
    };
    ([left_range, right_range], [left_hover, right_hover])
}

/// One pass of the thumbnail pulse: follow the visible rows, offer the tiles of
/// programs, whose own artwork the Engine's desktop actor reads (`refresh_file_icons`),
/// and take ready frames. `routes` are the native filesystem plugin and the owned
/// command sender.
fn thumbnail_pass(
    window: &MainWindow,
    presenter: &Rc<RefCell<Presenter>>,
    service: &thumbnail::Service,
    state: &mut ThumbnailState,
    log: &RefCell<ThumbnailLog>,
    routes: (&Arc<InstalledProvider>, &dyn Fn(Command) -> bool),
) {
    let (provider, send) = routes;
    if !window.window().is_visible() {
        return;
    }
    let (ranges, hovered) = visible_ranges(window);
    let Ok(mut presenter_state) = presenter.try_borrow_mut() else {
        return;
    };
    let mut refresh_all = false;
    if let Some(notice) = service.take_notice() {
        log.borrow_mut().notices.push(notice.clone());
        presenter_state.report_thumbnail_error(notice);
        refresh_all = true;
    }
    let _ = service.take_cache_location();
    let rows =
        presenter_state.thumbnail_targets(ranges, [hovered[0].as_str(), hovered[1].as_str()]);
    let scale = window.window().scale_factor();
    // Tiles of programs show their own artwork; a refused view is offered again on the
    // next pass, as production's pulse does.
    presenter_state.refresh_file_icons(&rows, scale, send);
    if rows != state.previous || scale != state.scale {
        // Keep a token while its row keeps its content: the service reuses its work.
        let previous = std::mem::take(&mut state.previous);
        state.tokens.retain(|identity, _| {
            previous
                .iter()
                .find(|row| &row.identity == identity)
                .is_some_and(|old| {
                    rows.iter().any(|new| {
                        new.identity == old.identity
                            && new.content == old.content
                            && new.confirmed_ssd == old.confirmed_ssd
                    })
                })
        });
        let live: std::collections::HashSet<u64> = state.tokens.values().copied().collect();
        state.paths.retain(|token, _| live.contains(token));
        let mut targets = Vec::new();
        for row in &rows {
            let ThumbnailContent::Native(path) = &row.content else {
                continue;
            };
            let token = match state.tokens.get(&row.identity) {
                Some(token) => *token,
                None => {
                    state.next_token += 1;
                    state.tokens.insert(row.identity.clone(), state.next_token);
                    state.next_token
                }
            };
            state.paths.insert(token, path.clone());
            let (capture_path, filesystem, ssd) =
                (path.clone(), provider.clone(), row.confirmed_ssd);
            let capture: thumbnail::Capture = Arc::new(move |cancel: &AtomicBool| {
                let file = filesystem
                    .capture_with(&capture_path, thumbnail::NATIVE_REVISIONS, &|| {
                        cancel.load(Ordering::Acquire)
                    })
                    .map_err(|failure| {
                        if cancel.load(Ordering::Acquire) {
                            io::ErrorKind::Interrupted.into()
                        } else {
                            io::Error::other(format!("Native filesystem: {failure}"))
                        }
                    })?;
                Ok(thumbnail::captured_source(file, ssd))
            });
            targets.push(thumbnail::Target {
                token,
                edge: physical_edge(row, scale),
                identity: path.as_os_str().as_encoded_bytes().to_vec(),
                extension: path
                    .extension()
                    .and_then(|suffix| suffix.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase(),
                selected: row.selected,
                hovered: row.hovered,
                confirmed_ssd: ssd,
                capture,
            });
        }
        service.set_view(thumbnail::View {
            generation: 1,
            thumbnailer_associations: Arc::default(),
            targets,
        });
        state.previous = rows;
        state.scale = scale;
    }
    let mut changed = Vec::new();
    for _ in 0..16 {
        let Some(ready) = service.poll() else {
            break;
        };
        let Some((identity, _)) = state
            .tokens
            .iter()
            .find(|(_, token)| **token == ready.token)
        else {
            continue;
        };
        if state
            .previous
            .iter()
            .find(|row| &row.identity == identity)
            .is_none_or(|row| ready.edge < physical_edge(row, scale))
        {
            continue;
        }
        let frame = &ready.frame;
        if presenter_state.receive_thumbnail_at_resolution(
            identity,
            frame.frame,
            frame.width,
            frame.height,
            ready.edge,
            &frame.rgba,
        ) {
            changed.push(identity.clone());
            let mut log = log.borrow_mut();
            log.presented += 1;
            log.identities.push(identity.clone());
            if let Some(path) = state.paths.get(&ready.token) {
                log.paths.push(path.clone());
            }
        }
    }
    drop(presenter_state);
    if refresh_all {
        midnight_ui::refresh(window, &presenter.borrow());
    } else if !changed.is_empty() {
        midnight_ui::refresh_thumbnail_rows(window, &presenter.borrow(), &changed);
    }
}

// ---------------------------------------------------------------------------------------
// Scenario helpers

pub fn plugins(app: &App) -> Result<&Plugins, String> {
    app.plugins
        .as_ref()
        .ok_or_else(|| "the application was started without the installed plugins".into())
}

/// An open viewer window and its software surface.
pub struct Viewing {
    pub window: ViewerWindow,
    pub software: Rc<MinimalSoftwareWindow>,
    /// Key press to the viewer's report of its initial content (or failure).
    pub latency: Duration,
    /// The part of `latency` spent creating and showing the window, synchronously in
    /// the key event; the rest is the backend producing the first content frame.
    pub window_time: Duration,
    /// The viewer's own report: Ok when content is shown, or its error.
    pub result: Result<(), String>,
}

/// The software window behind a component window, among the visible tool windows.
pub fn software_window(app: &App, window: &slint::Window) -> Option<Rc<MinimalSoftwareWindow>> {
    app.tool_windows()
        .into_iter()
        .find(|software| std::ptr::eq(software.window(), window))
}

/// Paint every tool window that asked for it, as an event loop does after a change.
pub fn paint_tool_windows(app: &App) {
    for window in app.tool_windows() {
        let size = window.window().size();
        let (width, height) = (size.width as usize, size.height as usize);
        if width == 0 || height == 0 {
            continue;
        }
        window.draw_if_needed(|renderer| {
            let mut pixels = vec![slint::Rgb8Pixel::default(); width * height];
            renderer.render(&mut pixels, width);
        });
    }
}

/// Press F3 (Alt+F3 for `basic`) in the focused panel and wait until the new viewer
/// reports its initial content or its failure, at most `limit`. Only the viewer's own
/// timers run meanwhile (no main-window frames), so the measured time is the viewer's.
pub fn view(app: &mut App, basic: bool, limit: Duration) -> Result<Viewing, String> {
    let before = plugins(app)?.opened.borrow().len();
    let started = Instant::now();
    if basic {
        input::chord(app.slint(), &[Key::Alt], Key::F3);
    } else {
        input::key(app.slint(), Key::F3);
    }
    let opened = plugins(app)?.opened.borrow().get(before).cloned();
    let Some(opened) = opened else {
        app.settle();
        return Err(format!(
            "{} opened no viewer; notice: {:?}",
            if basic { "Alt+F3" } else { "F3" },
            app.notice()
        ));
    };
    ready_viewer(app, opened, started, limit)
}

/// The number of viewers opened so far, to pass to [`next_viewer`].
pub fn viewers_opened(app: &App) -> usize {
    plugins(app).map_or(0, |plugins| plugins.opened.borrow().len())
}

/// Wait for the viewer opened after `before` others by any route (Find results and
/// their View button, a fed list, a warning report, Make File List), running the
/// application meanwhile, then until it reports its content or failure; at most `limit`
/// in all, counted from now.
pub fn next_viewer(app: &mut App, before: usize, limit: Duration) -> Result<Viewing, String> {
    let started = Instant::now();
    let opened = loop {
        let opened = plugins(app)?.opened.borrow().get(before).cloned();
        if let Some(opened) = opened {
            break opened;
        }
        if started.elapsed() >= limit {
            return Err(format!(
                "no viewer opened within {limit:?}; notice: {:?}",
                app.notice()
            ));
        }
        app.idle(Duration::from_millis(10));
    };
    ready_viewer(app, opened, started, limit)
}

/// Activate a newly opened viewer as a desktop does and wait for its first report.
fn ready_viewer(
    app: &mut App,
    opened: Opened,
    started: Instant,
    limit: Duration,
) -> Result<Viewing, String> {
    let window = opened
        .window
        .upgrade()
        .ok_or("the viewer window closed at once")?;
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let deadline = started + limit;
    let (at, result) = loop {
        slint::platform::update_timers_and_animations();
        if let Some(ready) = opened.ready.borrow().clone() {
            break ready;
        }
        if Instant::now() >= deadline {
            let status = window.get_status().to_string();
            app.settle();
            return Err(format!(
                "the viewer showed no content within {limit:?}; status {status:?}, caption {:?}",
                window.get_caption()
            ));
        }
        std::thread::sleep(Duration::from_millis(1));
    };
    app.settle();
    let software = software_window(app, window.window())
        .ok_or("the viewer window is not visible after opening")?;
    Ok(Viewing {
        window,
        software,
        latency: at.saturating_duration_since(started),
        window_time: opened.constructed.saturating_duration_since(started),
        result,
    })
}

/// Why this machine refuses the HTML viewer's browser, when `error` (the viewer's own
/// report) is one of the refusals that depend on the machine, not on Midnight Explorer
/// (`midnight-worker-platform`'s `browser.rs` and `browser_cgroup_linux.rs`): AppArmor
/// denies Chromium's sandbox, or no writable cgroup-v2 delegation exists. As in
/// `viewers_files` (ACCEPTANCE.md), a case refused so is skipped with this reason; any
/// other viewer error fails.
pub fn browser_refused(error: &str) -> Option<&'static str> {
    if error.contains("The HTML viewer needs permission for Chromium's sandbox") {
        Some(
            "AppArmor denies Chromium's sandbox to this installation (no \
             midnight-explorer-html-viewer profile covers the stage)",
        )
    } else if error.contains("Chromium requires a writable cgroup-v2 user delegation") {
        Some("no writable cgroup-v2 delegation for the browser on this machine")
    } else {
        None
    }
}

// ---------------------------------------------------------------------------------------
// The archive password dialog

/// The archive password dialog as the window shows it (`ArchivePasswords`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasswordDialog {
    /// The archive's shown name.
    pub archive: String,
    /// What the password lets go on, such as "Enter its password to view a.txt.".
    pub purpose: String,
    /// The reason under the field ("Wrong password — try again."); empty while none.
    pub error: String,
    /// Passwords refused so far.
    pub attempt: i32,
    /// A password is being checked.
    pub busy: bool,
}

impl PasswordDialog {
    /// The dialog's sentence: "<archive> is password-protected. <purpose>".
    pub fn sentence(&self) -> String {
        format!("{} is password-protected. {}", self.archive, self.purpose)
    }
}

/// The open archive password dialog, if any.
pub fn password_dialog(app: &App) -> Option<PasswordDialog> {
    let dialog = app.window.global::<ArchivePasswords>();
    if dialog.get_token().is_empty() {
        return None;
    }
    Some(PasswordDialog {
        archive: dialog.get_archive().to_string(),
        purpose: dialog.get_purpose().to_string(),
        error: dialog.get_error().to_string(),
        attempt: dialog.get_attempt(),
        busy: dialog.get_busy(),
    })
}

/// Wait, at most `limit`, until the archive password dialog is open.
pub fn wait_password_dialog(app: &mut App, limit: Duration) -> Result<PasswordDialog, String> {
    let deadline = Instant::now() + limit;
    loop {
        app.settle();
        if let Some(dialog) = password_dialog(app) {
            return Ok(dialog);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "no archive password dialog within {limit:?}; notice {:?}, viewers opened {}",
                app.notice(),
                viewers_opened(app)
            ));
        }
        app.idle(Duration::from_millis(10));
    }
}

/// How a password reaches the dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Submit {
    /// Typed into the field the dialog focused, then Enter.
    Enter,
    /// A click into the field, typed, then a click on OK.
    Pointer,
}

/// How the dialog is dismissed without a password.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dismiss {
    Escape,
    /// The footer's Cancel button.
    Cancel,
}

/// Give the open dialog `password` (`by` keyboard or pointer), then wait until the
/// archive's worker answered: the dialog closed (unlocked: `None`) or shows a new
/// reason (a refused password, or an empty field). Err when it opened none, or it
/// gave no answer within `limit`.
pub fn submit_password(
    app: &mut App,
    password: &str,
    by: Submit,
    limit: Duration,
) -> Result<Option<PasswordDialog>, String> {
    let before = password_dialog(app).ok_or("no archive password dialog is open")?;
    match by {
        Submit::Enter => {
            input::type_text(app.slint(), password);
            input::key(app.slint(), Key::Return);
        }
        Submit::Pointer => {
            input::fill(app.slint(), "Password", password)?;
            input::click_button(app.slint(), "OK")?;
        }
    }
    let deadline = Instant::now() + limit;
    loop {
        app.settle();
        let now = password_dialog(app);
        let answered = now.as_ref().is_none_or(|now| {
            !now.busy && (now.attempt != before.attempt || now.error != before.error)
        });
        if answered {
            let trace = app.recent_events();
            SUBMISSION_TRACES.with(|traces| traces.borrow_mut().extend(trace));
            return Ok(now);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "the archive password dialog gave no answer within {limit:?}: {now:?}"
            ));
        }
        app.idle(Duration::from_millis(10));
    }
}

/// Dismiss the open dialog without a password and wait until it has closed.
pub fn dismiss_password(app: &mut App, how: Dismiss) -> Result<(), String> {
    if password_dialog(app).is_none() {
        return Err("no archive password dialog is open".into());
    }
    match how {
        Dismiss::Escape => input::key(app.slint(), Key::Escape),
        Dismiss::Cancel => input::click_button(app.slint(), "Cancel")?,
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while password_dialog(app).is_some() {
        if Instant::now() >= deadline {
            return Err(format!("{how:?} did not close the archive password dialog"));
        }
        app.idle(Duration::from_millis(10));
    }
    Ok(())
}

/// F3 (Alt+F3 for `basic`) on an encrypted member of a locked archive: the password
/// dialog opens, `password` unlocks it by `by`, and the member's viewer opens once the
/// dialog has closed. The dialog the key opened is returned with the viewer.
pub fn view_unlocking(
    app: &mut App,
    basic: bool,
    password: &str,
    by: Submit,
    limit: Duration,
) -> Result<(PasswordDialog, Viewing), String> {
    let before = viewers_opened(app);
    if basic {
        input::chord(app.slint(), &[Key::Alt], Key::F3);
    } else {
        input::key(app.slint(), Key::F3);
    }
    let asked = wait_password_dialog(app, limit)?;
    if viewers_opened(app) != before {
        return Err("a viewer opened before the archive password was given".into());
    }
    if let Some(refused) = submit_password(app, password, by, limit)? {
        return Err(format!(
            "the archive password was refused: {:?} (attempt {})",
            refused.error, refused.attempt
        ));
    }
    let viewing = next_viewer(app, before, limit)?;
    Ok((asked, viewing))
}

// ---------------------------------------------------------------------------------------
// The user's number format

/// A number format with a decimal comma and space-grouped thousands, as many regional
/// settings have, for checks of typed numbers.
#[allow(dead_code)] // For the typed zoom checks (V2) of the viewer scenarios.
pub fn comma_locale() -> NumberLocale {
    NumberLocale {
        decimal_separator: ",".into(),
        group_separator: "\u{a0}".into(),
        grouping: vec![3],
        repeat_last_group: true,
    }
}

/// Make `locale` the user's number format, as the Engine's startup read of the regional
/// settings delivers it (`Event::LocaleLoaded`); typed zoom percentages read it from
/// then on. That read was answered long before (`App::start` waits for listings, which
/// follow it), so it cannot replace this one later; Err when the presenter does not
/// keep it.
#[allow(dead_code)] // For the typed zoom checks (V2) of the viewer scenarios.
pub fn use_number_locale(app: &mut App, locale: NumberLocale) -> Result<(), String> {
    app.presenter
        .borrow_mut()
        .apply(Event::LocaleLoaded(Ok(locale.clone())));
    app.settle();
    let kept = app.presenter.borrow().number_locale().clone();
    if kept == locale {
        Ok(())
    } else {
        Err(format!(
            "the presenter keeps the number format {kept:?}, not {locale:?}"
        ))
    }
}

impl Viewing {
    pub fn text(&self) -> String {
        self.window.get_accessible_text().to_string()
    }

    pub fn status(&self) -> String {
        self.window.get_status().to_string()
    }

    pub fn slint(&self) -> &slint::Window {
        self.window.window()
    }

    /// Run the application and paint the viewer until `condition` holds.
    pub fn wait(
        &self,
        app: &mut App,
        what: &str,
        limit: Duration,
        condition: impl Fn(&ViewerWindow) -> bool,
    ) -> Result<(), String> {
        let deadline = Instant::now() + limit;
        loop {
            app.settle();
            paint_tool_windows(app);
            if condition(&self.window) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "timed out after {limit:?} waiting for {what}; viewer status {:?}, text {:?}",
                    self.status(),
                    clip(&self.text(), 240)
                ));
            }
            app.idle(Duration::from_millis(10));
        }
    }

    /// Software-render the viewer: (width, height, pixels).
    pub fn render(&self) -> (usize, usize, Vec<slint::Rgb8Pixel>) {
        let _phase = crate::timing::Span::new("viewer: render");
        super::app::render(&self.software)
    }

    /// The rendered content canvas only.
    pub fn canvas(&self) -> Canvas {
        let (width, _height, pixels) = self.render();
        let scale = self.slint().scale_factor();
        let left = (self.window.get_content_left() * scale).max(0.0) as usize;
        let top = (self.window.get_content_top() * scale).max(0.0) as usize;
        let canvas_width = (self.window.get_content_width() * scale).max(0.0) as usize;
        let canvas_height = (self.window.get_content_height() * scale).max(0.0) as usize;
        Canvas::crop(&pixels, width, left, top, canvas_width, canvas_height)
    }

    /// Click the middle of the content, as a person does before using scroll keys.
    pub fn click_content(&self, app: &mut App) {
        let position = slint::LogicalPosition::new(
            self.window.get_content_left() + self.window.get_content_width() / 2.0,
            self.window.get_content_top() + self.window.get_content_height() / 2.0,
        );
        input::click_at(self.slint(), position);
        app.settle();
    }

    /// Escape closes the viewer; wait until its window is gone.
    pub fn close(self, app: &mut App) -> Result<(), String> {
        input::key(self.slint(), Key::Escape);
        let deadline = Instant::now() + Duration::from_secs(10);
        while software_window(app, self.slint()).is_some() {
            if Instant::now() >= deadline {
                return Err("Escape did not close the viewer".into());
            }
            app.idle(Duration::from_millis(10));
        }
        app.slint()
            .dispatch_event(WindowEvent::WindowActiveChanged(true));
        app.settle();
        Ok(())
    }
}

/// A rendered region's pixels, row by row.
#[derive(Clone)]
pub struct Canvas {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<slint::Rgb8Pixel>,
}

impl Canvas {
    /// The `width` x `height` region at `left`, `top` of a render `stride` pixels wide;
    /// pixels outside the render are black.
    pub fn crop(
        pixels: &[slint::Rgb8Pixel],
        stride: usize,
        left: usize,
        top: usize,
        width: usize,
        height: usize,
    ) -> Self {
        let mut cropped = Vec::with_capacity(width * height);
        for y in top..top + height {
            for x in left..left + width {
                let inside = x < stride;
                cropped.push(
                    pixels
                        .get(y * stride + x)
                        .filter(|_| inside)
                        .copied()
                        .unwrap_or_default(),
                );
            }
        }
        Self {
            width,
            height,
            pixels: cropped,
        }
    }

    /// The colour at `x`, `y`.
    pub fn at(&self, x: usize, y: usize) -> [u8; 3] {
        let pixel = self.pixels[y * self.width + x];
        [pixel.r, pixel.g, pixel.b]
    }

    /// Distinct colours, counting each only when it covers at least `minimum` pixels.
    pub fn colours(&self, minimum: usize) -> Vec<([u8; 3], usize)> {
        let mut counts: HashMap<[u8; 3], usize> = HashMap::new();
        for pixel in &self.pixels {
            *counts.entry([pixel.r, pixel.g, pixel.b]).or_default() += 1;
        }
        let mut colours: Vec<([u8; 3], usize)> = counts
            .into_iter()
            .filter(|(_, count)| *count >= minimum)
            .collect();
        colours.sort_by_key(|entry| std::cmp::Reverse(entry.1));
        colours
    }

    /// Pixels within `tolerance` (per channel) of `colour`.
    pub fn count_near(&self, colour: [u8; 3], tolerance: u8) -> usize {
        self.pixels
            .iter()
            .filter(|pixel| {
                pixel.r.abs_diff(colour[0]) <= tolerance
                    && pixel.g.abs_diff(colour[1]) <= tolerance
                    && pixel.b.abs_diff(colour[2]) <= tolerance
            })
            .count()
    }

    /// A cheap fingerprint of the pixels.
    pub fn fingerprint(&self) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for pixel in &self.pixels {
            for byte in [pixel.r, pixel.g, pixel.b] {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        hash
    }
}

/// Save a rendered region as one of this scenario's screenshots, to show what a failed
/// measurement saw; returns its path relative to the run folder.
pub fn save_canvas(context: &mut Context, canvas: &Canvas, name: &str, caption: &str) -> String {
    let relative = format!("shots/{}/{name}.png", super::app::context_name());
    let path = context.run_dir.join(&relative);
    match crate::report::save_png(&path, canvas.width, canvas.height, &canvas.pixels) {
        Ok(()) => context.outcome.shots.push(crate::report::Shot {
            file: relative.clone(),
            caption: caption.to_owned(),
            flags: 0,
        }),
        Err(error) => context.note(format!("screenshot {name} failed: {error}")),
    }
    relative
}

/// Record a reviewer-visible flag (the scenario then reports WARN) on the screenshot
/// `shot`, the path `App::screenshot` or `App::screenshot_window` returned.
pub fn flag_shot(context: &mut Context, shot: &str, kind: &'static str, detail: String) {
    if let Some(recorded) = context
        .outcome
        .shots
        .iter_mut()
        .find(|recorded| recorded.file == shot)
    {
        recorded.flags += 1;
    }
    context.outcome.flags.push(crate::report::Flag {
        shot: shot.to_owned(),
        kind,
        detail,
    });
}

/// At most `limit` characters, for messages.
pub fn clip(text: &str, limit: usize) -> String {
    let mut clipped: String = text.chars().take(limit).collect();
    if text.chars().count() > limit {
        clipped.push('…');
    }
    clipped
}

/// Milliseconds, for notes.
pub fn millis(duration: Duration) -> String {
    format!("{} ms", duration.as_millis())
}

/// The scenario's latency budget: slower opens are reported as findings.
pub const SLOW_OPEN: Duration = Duration::from_millis(500);

/// Checks a scenario carries on after, reported together at its end: failures, checks
/// of announced behavior that is not there yet (pending) and checks this environment
/// cannot make (skipped).
#[derive(Default)]
pub struct Failures {
    failed: RefCell<Vec<String>>,
    checks: Cell<usize>,
    /// Pending checks that still fail: "what (reason): failure".
    pending: RefCell<Vec<String>>,
    /// Pending checks that pass now: their pending mark should be removed.
    passing: RefCell<Vec<String>>,
    skipped: RefCell<Vec<String>>,
}

impl Failures {
    pub fn check(&self, what: &str, result: Result<(), String>) {
        self.checks.set(self.checks.get() + 1);
        if let Err(error) = result {
            self.failed.borrow_mut().push(format!("{what}: {error}"));
        }
    }

    /// A check of announced behavior that is not implemented yet (`reason` names what
    /// it waits for). It keeps its full strength: while it fails, the scenario reports
    /// `PENDING` when nothing else failed; once it passes, a note asks to remove the mark.
    #[allow(dead_code)] // No pending check now; the next one (V6, V7 until A1) uses it.
    pub fn pending(&self, what: &str, reason: &str, result: Result<(), String>) {
        self.checks.set(self.checks.get() + 1);
        match result {
            Err(error) => self
                .pending
                .borrow_mut()
                .push(format!("{what} ({reason}): {error}")),
            Ok(()) => self.passing.borrow_mut().push(format!("{what} ({reason})")),
        }
    }

    /// A check this environment cannot make, with the reason.
    pub fn skip(&self, what: &str, reason: &str) {
        self.skipped.borrow_mut().push(format!("{what}: {reason}"));
    }

    /// The scenario's result without its notes: failures first, then pending checks.
    pub fn into_result(self) -> Result<(), String> {
        let failures = self.failed.into_inner();
        let pending = self.pending.into_inner();
        if !failures.is_empty() {
            let mut message = format!(
                "{} of {} check(s) failed:\n{}",
                failures.len(),
                self.checks.get(),
                failures.join("\n")
            );
            if !pending.is_empty() {
                message.push_str(&format!(
                    "\nand {} pending check(s) still fail:\n{}",
                    pending.len(),
                    pending.join("\n")
                ));
            }
            return Err(message);
        }
        if pending.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "PENDING: {} check(s) wait for announced behavior:\n{}",
                pending.len(),
                pending.join("\n")
            ))
        }
    }

    /// [`Self::into_result`], with notes for every skipped, pending and newly passing
    /// pending check; skipped checks also become a passing scenario's message.
    pub fn finish(self, context: &mut Context) -> Result<(), String> {
        let skipped = std::mem::take(&mut *self.skipped.borrow_mut());
        for skip in &skipped {
            context.note(format!("SKIP {skip}"));
        }
        for pending in self.pending.borrow().iter() {
            context.note(format!("PENDING {pending}"));
        }
        for passing in self.passing.borrow().iter() {
            context.note(format!(
                "pending check passes now, remove its pending mark: {passing}"
            ));
        }
        context.note(format!(
            "{} check(s): {} failed, {} pending, {} skipped",
            self.checks.get(),
            self.failed.borrow().len(),
            self.pending.borrow().len(),
            skipped.len()
        ));
        if !skipped.is_empty() && context.outcome.message.is_empty() {
            context.outcome.message =
                format!("SKIP {} check(s): {}", skipped.len(), skipped.join("; "));
        }
        self.into_result()
    }
}

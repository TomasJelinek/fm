#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

mod command_artwork;
mod command_reclaim;
mod comparator;
mod comparator_external;
mod content_appearance;
mod idle_probe;
mod native_content;
mod native_events;
mod pulse;
#[cfg(test)]
mod resident_test_runtime;
mod retry_pass;
mod shutdown;
mod shutdown_preferences;
mod size_refresh;
mod smoke_settings;
mod smoke_timing;
mod startup;
mod startup_gate;
mod startup_recovery;
mod startup_settings;
mod thumbnails;
mod updates;
mod viewers;
mod visible_sync;
mod wake_gate;
mod window_backend;
mod window_state;

use midnight_core::{
    Command, ConsoleLauncher, DesktopServices, Event, NavigationId, PaneId, RUN_AND_WAIT_ARGUMENT,
    SettingsService, SortSpec, WatchService,
};
use midnight_engine::{Engine, reclaim, snapshot};
use midnight_platform::{LocalFileSystem, home_directory};
use midnight_ui::{MainWindow, Presenter};
use midnight_work_budget::resident;
use slint::ComponentHandle;
use std::{
    cell::{Cell, OnceCell, RefCell},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use visible_sync::VisibleSync;
use wake_gate::WakeGate;

/// Settings reconciliation, the final save and engine shutdown share this exit grace.
const EXIT_GRACE: Duration = Duration::from_millis(2500);
/// The engine always gets at least this long to process `Shutdown` and accepted saves.
const ENGINE_MINIMUM_GRACE: Duration = Duration::from_millis(500);
/// The last reclamation is awaited at most this long; blocked destructors are left to
/// the bounded runtime shutdown instead of keeping the hidden process alive.
const RECLAIM_GRACE: Duration = Duration::from_millis(500);

struct Options {
    explicit_paths: [bool; 2],
    left: PathBuf,
    right: PathBuf,
    smoke: bool,
    ui_smoke: bool,
    software: bool,
    snapshot: Option<PathBuf>,
}

fn options() -> Result<Option<Options>, String> {
    let current = std::env::current_dir().map_err(|error| error.to_string())?;
    let mut result = Options {
        explicit_paths: [false; 2],
        left: current.clone(),
        right: home_directory().unwrap_or_else(|| current.clone()),
        smoke: false,
        ui_smoke: false,
        software: false,
        snapshot: None,
    };
    let mut arguments = std::env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("--help" | "-h") => {
                println!(concat!(
                    "Midnight Explorer — dual-pane file manager\n\n",
                    "Usage: midnight-explorer [--left PATH] [--right PATH] [--software]\n",
                    "       midnight-explorer --smoke-test [--left PATH] [--right PATH]\n",
                    "       midnight-explorer --ui-smoke-test [--snapshot FILE.png] ",
                    "[--software]\n\n",
                    "Native browsing, installed viewers, external editing, rename, ",
                    "file/directory creation, confirmed deletion, verified copying, ",
                    "per-file verified moves, Find and checksums. ",
                    "A move removes each source only after its destination is verified."
                ));
                return Ok(None);
            }
            Some("--left" | "--right" | "--snapshot") => {
                let value = arguments
                    .next()
                    .ok_or_else(|| format!("{} requires a path", argument.to_string_lossy()))?;
                let path = PathBuf::from(value);
                let path = if path.is_absolute() {
                    path
                } else {
                    current.join(path)
                };
                match argument.to_str() {
                    Some("--left") => {
                        result.left = path;
                        result.explicit_paths[0] = true;
                    }
                    Some("--right") => {
                        result.right = path;
                        result.explicit_paths[1] = true;
                    }
                    _ => {
                        result.snapshot = Some(path);
                        result.ui_smoke = true;
                    }
                }
            }
            Some("--smoke-test") => result.smoke = true,
            Some("--ui-smoke-test") => result.ui_smoke = true,
            Some("--software") => result.software = true,
            _ => {
                return Err(format!(
                    "Unknown argument: {}. Use --help.",
                    argument.to_string_lossy()
                ));
            }
        }
    }
    Ok(Some(result))
}

fn main() -> std::process::ExitCode {
    // The updater that "Restart now" starts (a copy of the previous executable): it swaps
    // the installation and starts the new version, and never opens a window.
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    if midnight_update::apply::ApplyRequest::from_arguments(&arguments).is_some() {
        return std::process::ExitCode::from(midnight_update::apply::run(&arguments));
    }
    // The Console window launcher of a Run directly command: a console program, not the
    // file manager. It runs the program with exactly its arguments and waits for a key.
    let mut arguments = arguments.into_iter();
    let mode = arguments.next();
    #[cfg(windows)]
    if mode.as_deref() == Some(std::ffi::OsStr::new(midnight_platform::RUN_BATCH_ARGUMENT)) {
        std::process::exit(midnight_platform::run_captured_batch(arguments));
    }
    if mode.as_deref() == Some(std::ffi::OsStr::new(RUN_AND_WAIT_ARGUMENT)) {
        let code = midnight_platform::run_and_wait(arguments.collect());
        #[cfg(windows)]
        std::process::exit(code);
        #[cfg(not(windows))]
        return std::process::ExitCode::from(u8::try_from(code).unwrap_or(1));
    }
    // First, before any thread opens files: on Linux, raise the soft open-file limit to
    // the hard limit (at most 65,536), from which the transfers' folder budget is sized;
    // programs started for the user get the original limit back.
    let open_files = midnight_platform::raise_open_file_limit();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    if let Err(error) = open_files {
        tracing::warn!("The limit on open files could not be raised: {error}");
    }
    let result = run();
    match result {
        Ok(status) => std::process::ExitCode::from(status),
        Err(error) => {
            eprintln!("Midnight Explorer: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// The process exit status: 0, or the software-rendering child's own status.
fn run() -> Result<u8, String> {
    let Some(options) = options()? else {
        return Ok(0);
    };
    let engine_lifetime = Engine::lifetime().map_err(|error| error.to_string())?;
    // Console window output starts this executable as its launcher (`main`).
    if let Ok(executable) = std::env::current_exe() {
        midnight_platform::set_console_launcher(ConsoleLauncher {
            executable,
            leading: vec![RUN_AND_WAIT_ARGUMENT.into()],
        });
    }
    let local = Arc::new(LocalFileSystem::new(home_directory()));
    let shared_native =
        midnight_native_filesystem_host::InstalledProvider::shared_composition(home_directory());
    let filesystem = shared_native.service();
    filesystem.set_size_directory_service(local.clone());
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(16)
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    if options.smoke {
        let result = runtime.block_on(smoke_test(engine_lifetime, filesystem, &options));
        runtime.shutdown_timeout(Duration::from_secs(1));
        return result.map(|()| 0);
    }
    let restart_arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    let restart_cwd = std::env::current_dir().map_err(|error| error.to_string())?;
    // Held until the process exits: an updater waits until no copy runs from the folder.
    let (_instance, update_start) = updates::at_start();
    let update_restart = updates::RestartRequest::default();
    let software_status = Cell::new(0);
    // Prestart before run_window can adopt capture factories or start real actors.
    // Owners and retained window-stop fences remain outside UI unwinding.
    let mut content_workers = match ContentWorkers::start(&runtime) {
        Ok(workers) => workers,
        Err(error) => {
            runtime.shutdown_timeout(Duration::from_secs(1));
            return Err(error);
        }
    };
    let mut image_spare_retirement = None;
    // Keep runtime ownership and the registered spare fence outside UI unwinding,
    // so a panic cannot enter Tokio's unbounded blocking-pool destructor first.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_window(
            engine_lifetime,
            &runtime,
            local,
            shared_native,
            &options,
            &mut image_spare_retirement,
            &mut content_workers,
            (update_start.clone(), update_restart.clone()),
        )
    }));
    // Normal close drains mutation cancellation before hiding the window.
    // A failed startup also stops its runtime before another process is started.
    shutdown::finish_after_unwind(
        result,
        || {
            content_workers.stop_windows();
            let retirement_result = if let Some(retirement) = image_spare_retirement.as_ref() {
                retirement.stop_spare();
                // The event loop has closed. Await the terminal owner result before bounded
                // runtime shutdown; only successful retirement can permit software retry.
                runtime.block_on(retirement.wait())
            } else {
                Ok(())
            };
            let content_result = content_workers.finish(&runtime);
            let retirement_result = combine_cleanup(retirement_result, content_result);
            // Keep the ticket in this outer scope through runtime cleanup and fatal handling.
            (runtime, retirement_result)
        },
        |runtime| runtime.shutdown_timeout(Duration::from_secs(1)),
        || {
            let executable = std::env::current_exe().map_err(|error| error.to_string())?;
            // The parent stays until the child exits so launchers and scripts observe the
            // whole session; the child reports its own errors, and its status is forwarded.
            let mut command =
                startup_recovery::software_command(&executable, &restart_arguments, &restart_cwd);
            software_status.set(startup_recovery::run_child(&mut command)?);
            Ok(())
        },
    )?;
    // "Restart now": the window has closed and the session is saved; the updater waits
    // for this process to exit, installs the update and starts the new version.
    if let Some(restart) = update_restart.borrow_mut().take()
        && let Err(error) = updates::start_updater(restart, restart_arguments, restart_cwd)
    {
        eprintln!("Midnight Explorer: {error}");
    }
    Ok(software_status.get())
}

/// Neutral execution context; stop ownership never enters a Service or capture.
pub(crate) struct ContentExecution {
    transient: tokio::runtime::Handle,
    resident: resident::Spawner,
}

struct ContentWorkers {
    viewer_owner: resident::Owner,
    compare_owner: resident::Owner,
    viewer: ContentExecution,
    compare: ContentExecution,
    viewer_windows: Option<Rc<midnight_viewer_ui::ViewerWindows>>,
    compare_windows: Option<Rc<midnight_compare_ui::ComparatorWindows>>,
}

impl ContentWorkers {
    fn start(runtime: &tokio::runtime::Runtime) -> Result<Self, String> {
        let (viewer_owner, viewer) =
            resident::Owner::start("midnight-viewer", midnight_viewer_engine::RESIDENT_WORKERS)
                .map_err(|error| format!("Cannot start viewer workers: {error}"))?;
        // One worker per open comparator, mostly waiting for input: elastic, so any number
        // of comparators open. Their heavy preparation is bounded by the comparison service.
        let (compare_owner, compare) = match resident::Owner::start_elastic("midnight-compare", 2) {
            Ok(workers) => workers,
            Err(error) => {
                let cleanup = runtime.block_on(async {
                    let deadline = tokio::time::Instant::now() + Duration::from_millis(2500);
                    wait_content_workers(
                        "Viewer",
                        viewer_owner.shutdown(runtime.handle()),
                        deadline,
                    )
                    .await
                });
                let error = format!("Cannot start comparison workers: {error}");
                return Err(match cleanup {
                    Ok(()) => error,
                    Err(cleanup) => format!("{error}\n{cleanup}"),
                });
            }
        };
        Ok(Self {
            viewer_owner,
            compare_owner,
            viewer: ContentExecution {
                transient: runtime.handle().clone(),
                resident: viewer,
            },
            compare: ContentExecution {
                transient: runtime.handle().clone(),
                resident: compare,
            },
            viewer_windows: None,
            compare_windows: None,
        })
    }

    fn stop_windows(&mut self) {
        if let Some(windows) = self.viewer_windows.take() {
            windows.close_all();
        }
        if let Some(windows) = self.compare_windows.take() {
            windows.close_all();
        }
    }

    fn finish(self, runtime: &tokio::runtime::Runtime) -> Result<(), String> {
        // Seal both domains before polling either waiter. The original Runtime
        // drives timers and finite joins; each result survives its peer's timeout.
        let viewer = self.viewer_owner.shutdown(runtime.handle());
        let compare = self.compare_owner.shutdown(runtime.handle());
        let (viewer, compare) = runtime.block_on(async {
            // This content grace follows the unchanged Engine and spare waits; it does not
            // extend the Engine's own exit deadline (`EXIT_GRACE`, never under
            // `ENGINE_MINIMUM_GRACE`).
            let deadline = tokio::time::Instant::now() + Duration::from_millis(2500);
            tokio::join!(
                wait_content_workers("Viewer", viewer, deadline),
                wait_content_workers("Comparison", compare, deadline),
            )
        });
        combine_cleanup(viewer, compare)
    }
}

async fn wait_content_workers(
    name: &str,
    shutdown: resident::Shutdown,
    deadline: tokio::time::Instant,
) -> Result<(), String> {
    match tokio::time::timeout_at(deadline, shutdown).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(format!("{name} worker cleanup is unconfirmed: {error}")),
        Err(_) => Err(format!(
            "{name} worker cleanup exceeded the content exit grace."
        )),
    }
}

fn combine_cleanup(first: Result<(), String>, next: Result<(), String>) -> Result<(), String> {
    match (first, next) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(first), Err(next)) => Err(format!("{first}\n{next}")),
    }
}

async fn smoke_test(
    engine_lifetime: midnight_engine::EngineLifetime,
    filesystem: Arc<midnight_native_filesystem_host::InstalledProvider>,
    options: &Options,
) -> Result<(), String> {
    let (sender, mut events) = mpsc::channel(16);
    let mut engine = Engine::new(engine_lifetime, filesystem.clone(), sender);
    engine.enable_installation_notice();
    engine
        .configure_archive_capture()
        .map_err(|error| error.to_string())?
        .install(native_content::archive(filesystem));
    engine
        .configure_locale()
        .map_err(|error| error.to_string())?
        .install(midnight_platform::locale_service());
    let (commands, receiver) = engine
        .command_channel(16)
        .map_err(|error| error.to_string())?;
    let engine = tokio::spawn(engine.run(receiver));
    commands
        .send(Command::ListVolumes)
        .await
        .map_err(|error| error.to_string())?;
    for (pane, path) in [
        (PaneId::Left, &options.left),
        (PaneId::Right, &options.right),
    ] {
        commands
            .send(Command::Navigate {
                pane,
                navigation: NavigationId(1),
                path: path.clone(),
                sort: SortSpec::default(),
            })
            .await
            .map_err(|error| error.to_string())?;
    }
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let mut completed = [false; 2];
        let mut volumes_ready = false;
        let mut installation_checked = false;
        let mut failure = None;
        while !completed.into_iter().all(|done| done) || !volumes_ready || !installation_checked {
            match events
                .recv()
                .await
                .ok_or("engine stopped before completing smoke test")?
            {
                Event::VolumesUpdated(volumes) => {
                    println!("volumes: {}", volumes.len());
                    volumes_ready = true;
                }
                Event::VolumesFailed(error) => {
                    failure = Some(error.to_string());
                    volumes_ready = true;
                }
                Event::ListingFinished { pane, entries, .. } => {
                    println!("{pane:?}: {} entries", entries.len());
                    completed[pane.index()] = true;
                }
                Event::ListingFailed { pane, error, .. } => {
                    failure = Some(error.to_string());
                    completed[pane.index()] = true;
                }
                Event::Installation(event) => {
                    if let midnight_core::InstallationEvent::Checked(status) = *event {
                        match status {
                            None => println!("installation: private"),
                            Some(status) => println!(
                                "installation: shared; other accounts can change {} objects \
                                 in {}",
                                status.shared_count,
                                status.folder.display()
                            ),
                        }
                        installation_checked = true;
                    }
                }
                _ => {}
            }
        }
        failure.map_or(Ok(()), Err)
    })
    .await
    .map_err(|_| "smoke test timed out".to_string())
    .and_then(|result| result);
    let _ = commands.send(Command::Shutdown).await;
    drop(events);
    let _ = engine.await;
    result
}

#[expect(
    clippy::too_many_arguments,
    reason = "Owners that outlive the window's unwinding stay with `run`, one argument each."
)]
fn run_window(
    engine_lifetime: midnight_engine::EngineLifetime,
    runtime: &tokio::runtime::Runtime,
    local: Arc<LocalFileSystem>,
    shared_native: midnight_native_filesystem_host::SharedInstalledProvider,
    options: &Options,
    image_spare_retirement: &mut Option<midnight_viewer_engine::ImageSpareRetirement>,
    content_workers: &mut ContentWorkers,
    (update_start, update_restart): (updates::AtStart, updates::RestartRequest),
) -> Result<startup_recovery::Attempt, String> {
    let filesystem = shared_native.service();
    // Desktop integration and mutations keep their explicit platform services.
    // Browsing, volume queries and viewer reads route only through the native plugin.
    let desktop: Arc<dyn DesktopServices> = local.clone();
    let watch: Arc<dyn WatchService> = local.clone();
    let settings: Arc<dyn SettingsService> = local.clone();
    let mutations: Arc<dyn midnight_core::MutationService> = local.clone();
    let may_retry = startup_recovery::retry_allowed(
        options.software,
        std::env::var_os("SLINT_BACKEND").is_some(),
        false,
    );
    let wakeup_probe = idle_probe::from_env();
    let main_events = Rc::new(native_events::MainWindowEvents::default());
    let observer = native_events::Observer::new(
        wakeup_probe.map(idle_probe::Probe::new),
        main_events.clone(),
    );
    // The renderer reads its text fitting before the first window, earlier than the
    // asynchronous preference load, which then reuses this one read. Probes and
    // smoke runs keep the defaults and never read the user's profile.
    let prefetched = if options.ui_smoke || wakeup_probe.is_some() {
        None
    } else {
        startup_settings::prefetch(settings.as_ref())
    };
    let stored_fitting = prefetched
        .as_ref()
        .map(|preferences| preferences.glyph_fitting)
        .unwrap_or_default();
    if let Err(error) = window_backend::select(options.software, observer, stored_fitting) {
        return startup_recovery::native_failure(
            format!("Cannot initialize renderer: {error}"),
            may_retry,
        );
    }
    let window = match MainWindow::new() {
        Ok(window) => window,
        Err(error) => {
            let error = format!("Cannot create window: {error}");
            return startup_recovery::native_failure(error, may_retry);
        }
    };
    if let Some(preferences) = &prefetched {
        window.set_app_icon_index(preferences.app_icon.index());
    }
    main_events.bind(&window);
    // This owner aborts/wakes a pending settings write on every early return.
    let startup_owner = startup_gate::Owner::new();
    let settings = match wakeup_probe {
        Some(probe) => smoke_settings::for_probe(probe.thumbnails),
        None => smoke_settings::for_run(options.ui_smoke, settings),
    };
    let settings = startup_settings::for_startup(settings, startup_owner.gate(), prefetched);
    let package_version = env!("CARGO_PKG_VERSION");
    let display_version = package_version
        .strip_suffix(".0")
        .unwrap_or(package_version);
    window.set_application_version(display_version.into());
    let presenter = Rc::new(RefCell::new(Presenter::new(
        options.left.clone(),
        options.right.clone(),
    )));
    presenter
        .borrow_mut()
        .set_tab_restore_overrides(if options.ui_smoke {
            [true; 2]
        } else {
            options.explicit_paths
        });
    let network = midnight_platform::network_service();
    window.set_network_available(network.is_some());
    let (event_sender, event_receiver) = mpsc::channel(32);
    let receiver = Rc::new(RefCell::new(event_receiver));
    let gate = Arc::new(WakeGate::default());
    let weak = window.as_weak();
    let notify_gate = gate.clone();
    let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        if notify_gate.request()
            && weak
                .upgrade_in_event_loop(|window| window.invoke_backend_ready())
                .is_err()
        {
            notify_gate.release();
        }
    });
    let retire_queue = reclaim::start(runtime.handle(), notify.clone());
    // Refused sends and unavailable owned scopes leave work for the retry pass.
    let refusals = Rc::new(pulse::Refusals::default());
    let scope_refusals = refusals.clone();
    let command_reclaim = command_reclaim::CommandReclaim::new(retire_queue.clone())
        .observe_refusals(Rc::new(move || scope_refusals.refused()));
    let thumbnail_routing = Arc::new(thumbnails::Routing::default());
    let routing_commands = Rc::new(thumbnails::RoutingCommands::new(
        thumbnail_routing.clone(),
        midnight_core::PathPickerService::command_icon_resource_requires_import(local.as_ref()),
    ));
    let mut engine = Engine::with_shared_native_filesystem(
        engine_lifetime,
        shared_native,
        event_sender,
        notify.clone(),
    );
    engine
        .configure_archive_capture()
        .map_err(|error| error.to_string())?
        .install(native_content::archive(filesystem.clone()));
    engine
        .configure_elevated_directories()
        .map_err(|error| error.to_string())?
        .install(Arc::new(midnight_elevation::ElevatedDirectories));
    engine
        .configure_navigation_fallback()
        .map_err(|error| error.to_string())?
        .install(local.clone());
    engine
        .configure_shell_menu()
        .map_err(|error| error.to_string())?
        .install(local.clone());
    engine
        .configure_path_picker()
        .map_err(|error| error.to_string())?
        .install(command_artwork::installed(
            local.clone(),
            filesystem.clone(),
            thumbnail_routing.clone(),
        ));
    engine
        .configure_audited_transfers()
        .map_err(|error| error.to_string())?
        .install(local.clone());
    engine
        .configure_find()
        .map_err(|error| error.to_string())?
        .install(local.clone());
    engine
        .configure_user_commands()
        .map_err(|error| error.to_string())?
        .install(local.clone());
    engine
        .configure_editor()
        .map_err(|error| error.to_string())?
        .install(local.clone());
    engine
        .configure_drag_drop()
        .map_err(|error| error.to_string())?
        .install(local.clone());
    engine
        .configure_audited_locale()
        .map_err(|error| error.to_string())?
        .install(local.clone());
    engine
        .configure_mutations()
        .map_err(|error| error.to_string())?
        .install(mutations);
    engine
        .configure_desktop()
        .map_err(|error| error.to_string())?
        .install(desktop);
    engine
        .configure_clipboard()
        .map_err(|error| error.to_string())?
        .install(Arc::new(midnight_platform::NativeClipboard));
    engine
        .configure_watch()
        .map_err(|error| error.to_string())?
        .install(watch);
    engine
        .configure_settings()
        .map_err(|error| error.to_string())?
        .install(settings);
    engine.enable_installation_notice();
    if let Some(network) = network {
        engine
            .configure_network()
            .map_err(|error| error.to_string())?
            .install(network);
    }
    let (commands, command_receiver) = engine
        .command_channel(32)
        .map_err(|error| error.to_string())?;
    let comparator_backend =
        comparator_external::Backend::new(&engine, commands.clone(), filesystem.clone());
    let begin_owned = routing_commands.track_owned(command_reclaim.factory(commands.clone()));
    let control_commands = commands.clone();
    let control_refusals = refusals.clone();
    let control_routing = routing_commands.clone();
    let control_send: Rc<dyn Fn(Command) -> bool> = Rc::new(move |command| {
        // Captured owners and listing acknowledgements use an owned scope.
        // This fallback receives only provider-free controls and metadata.
        let sent = control_routing.dispatch(
            command,
            |command| match control_commands.try_send(command) {
                Ok(()) => true,
                Err(error) => {
                    drop(error.into_inner());
                    false
                }
            },
            drop,
        );
        if !sent {
            control_refusals.refused();
        }
        sent
    });
    // Dropped when the engine returns or unwinds; a hidden startup then stops waiting.
    let (engine_stopped, engine_stop) = tokio::sync::oneshot::channel::<()>();
    let engine = runtime.spawn(async move {
        let _stopped = engine_stopped;
        engine.run(command_receiver).await
    });

    let completed = Rc::new(Cell::new([false; 2]));
    let smoke_scheduled = Rc::new(Cell::new(false));
    let smoke_error = Rc::new(RefCell::new(None::<String>));
    let snapshot = Rc::new(RefCell::new(None));
    let weak = window.as_weak();
    let state = presenter.clone();
    let ui_receiver = receiver.clone();
    let drain_gate = gate.clone();
    let reschedule = notify.clone();
    let done = completed.clone();
    let scheduled = smoke_scheduled.clone();
    let output = snapshot.clone();
    let error_state = smoke_error.clone();
    let ui_smoke = options.ui_smoke;
    let capture = options.snapshot.is_some();
    let visible_sync = Rc::new(RefCell::new(VisibleSync::default()));
    let ui_visible_sync = visible_sync.clone();
    let ui_commands = commands.clone();
    let ui_begin_owned = begin_owned.clone();
    let ui_reclaim = command_reclaim.clone();
    let final_retire_queue = retire_queue.clone();
    let final_command_reclaim = command_reclaim.clone();
    // Both panes share one size pulse, armed by the first size-only event after a
    // presentation. Interactive/backend events still repaint immediately and include
    // the latest cached totals.
    let size_window = window.as_weak();
    let size_state = presenter.clone();
    let size_updates = size_refresh::SizeRefresh::new(move || {
        if let Some(window) = size_window.upgrade() {
            midnight_ui::refresh(&window, &size_state.borrow());
        }
    });
    let pending_sizes = size_updates.clone();
    let window_state = Rc::new(window_state::WindowState::install(
        &window,
        presenter.clone(),
        control_send.clone(),
        !options.ui_smoke,
    ));
    let startup = Rc::new(startup::Startup::install(
        &window,
        presenter.clone(),
        window_state.clone(),
        startup_owner.gate(),
    )?);
    let pending_state = Rc::downgrade(&presenter);
    let pending_window = window.as_weak();
    startup.arm(
        startup::DEADLINE,
        engine_stop,
        Box::new(move || {
            // The deadline reveals defaults before the settings read answered.
            let (Some(state), Some(window)) = (pending_state.upgrade(), pending_window.upgrade())
            else {
                return;
            };
            let Ok(mut presenter) = state.try_borrow_mut() else {
                return;
            };
            presenter.note_settings_pending();
            drop(presenter);
            midnight_ui::refresh(&window, &state.borrow());
        }),
    )?;
    let startup_ready = startup.clone();
    // Bound after the viewers are installed; the event loop has not run before then.
    let settings_loaded: Rc<OnceCell<viewers::SettingsLoaded>> = Rc::default();
    let loaded_settings = settings_loaded.clone();
    // Bound after the comparators are installed, likewise.
    let comparators_loaded: Rc<OnceCell<std::rc::Weak<midnight_compare_ui::ComparatorWindows>>> =
        Rc::default();
    let loaded_comparators = comparators_loaded.clone();
    window.on_backend_ready(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let start = Instant::now();
        let mut changed = false;
        let mut preferences_applied = false;
        let mut thumbnail_routes_changed = false;
        let mut can_drain = ui_reclaim.flush();
        {
            let mut events = ui_receiver.borrow_mut();
            for _ in 0..16 {
                if !can_drain {
                    break;
                }
                let consumed =
                    midnight_ui::with_owned_commands(&state, &ui_begin_owned, |p, send| {
                        // Claim precedes the ownership-bearing dequeue, including
                        // stale events and immediate follow-up capture submissions.
                        let Ok(mut event) = events.try_recv() else {
                            return false;
                        };
                        if routing_commands.apply(&mut event) {
                            p.clear_thumbnail_images();
                            thumbnail_routes_changed = true;
                        }
                        preferences_applied |= matches!(&event, Event::PreferencesLoaded(_));
                        let size_only = matches!(
                            &event,
                            Event::FolderSizeUpdated { .. }
                                | Event::FolderSizesFinished { .. }
                                | Event::ArchiveSizes(_)
                        );
                        if let Event::ListingFinished { pane, .. }
                        | Event::ListingFailed { pane, .. } = &event
                        {
                            let mut finished = done.get();
                            finished[pane.index()] = true;
                            done.set(finished);
                        }
                        if let Event::DirectoryChanged { pane, navigation } = event {
                            p.refresh_if_changed(pane, navigation, send);
                        } else {
                            p.apply_with_sender(event, send);
                        }
                        if size_only {
                            pending_sizes.changed();
                        } else {
                            changed = true;
                        }
                        true
                    });
                can_drain = ui_reclaim.flush();
                if consumed != Some(true)
                    || !can_drain
                    || start.elapsed() >= Duration::from_millis(3)
                {
                    break;
                }
            }
        }
        if changed {
            midnight_ui::refresh(&window, &state.borrow());
            pending_sizes.painted();
            let _ = retry_pass::sync_visible(&state, &ui_begin_owned, &ui_visible_sync);
        }
        if thumbnail_routes_changed {
            thumbnails::wake_pulse();
        }
        if preferences_applied {
            if let Some(loaded) = loaded_settings.get() {
                loaded.applied(&state);
            }
            if let Some(comparators) = loaded_comparators.get().and_then(std::rc::Weak::upgrade) {
                comparator::settings_applied(&comparators, &state);
            }
            // Both successful and failed reads must complete startup, after
            // their appearance/error presentation has reached the hidden UI.
            startup_ready.preferences_applied();
        }
        // Clear before rechecking the queue. A producer racing this check either
        // schedules its own callback or leaves data seen by this recheck.
        drain_gate.release();
        can_drain = ui_reclaim.flush();
        if (can_drain && !ui_receiver.borrow().is_empty())
            || (!can_drain && !retire_queue.is_closed() && retire_queue.capacity() > 0)
        {
            reschedule();
        }
        if ui_smoke && done.get().into_iter().all(|value| value) && !scheduled.replace(true) {
            let weak = window.as_weak();
            let state = state.clone();
            let output = output.clone();
            let errors = error_state.clone();
            let shutdown_commands = ui_commands.clone();
            let dwell = smoke_timing::dwell_duration(
                std::env::var("MIDNIGHT_UI_SMOKE_DWELL_MS").ok().as_deref(),
            );
            slint::Timer::single_shot(dwell, move || {
                if state.borrow().has_errors() {
                    *errors.borrow_mut() =
                        Some("UI smoke test encountered a listing/volume error".into());
                }
                if let Some(window) = weak.upgrade() {
                    if window.get_message_generation() != 0 {
                        *errors.borrow_mut() = Some(format!(
                            "UI smoke test reported {}: {}",
                            window.get_message_title(),
                            window.get_message_body()
                        ));
                    }
                    // Exercise real Slint focus routing without changing paths:
                    // empty Back history still uses the toolbar action callback.
                    window.invoke_action(1, 0);
                    if window.get_focused_pane() != 1 {
                        *errors.borrow_mut() =
                            Some("Right toolbar action did not focus the right list".into());
                    }
                    window.invoke_action(0, 0);
                    if window.get_focused_pane() != 0 {
                        *errors.borrow_mut() =
                            Some("Left toolbar action did not focus the left list".into());
                    }
                }
                if capture && let Some(window) = weak.upgrade() {
                    match window.window().take_snapshot() {
                        Ok(pixels) => *output.borrow_mut() = Some(pixels),
                        Err(error) => {
                            *errors.borrow_mut() = Some(format!("Snapshot failed: {error}"))
                        }
                    }
                }
                state
                    .borrow_mut()
                    .shutdown_drag_drop(&|command| shutdown_commands.try_send(command).is_ok());
                let _ = slint::quit_event_loop();
            });
        }
    });

    let send_controls = control_send.clone();
    let startup_request = startup.clone();
    let preferences_requested = Rc::new(Cell::new(false));
    let initial_read = preferences_requested.clone();
    midnight_ui::install_with_owned(
        &window,
        presenter.clone(),
        Rc::new(move |command| {
            let initial_preferences = matches!(&command, Command::LoadPreferences);
            let sent = send_controls(command);
            if initial_preferences {
                initial_read.set(initial_read.get() || sent);
                if !sent {
                    // install reports the rejected request and refreshes before
                    // returning to the event loop. Do not leave startup waiting.
                    startup_request.preferences_applied();
                }
            }
            sent
        }),
        begin_owned.clone(),
    );
    let revealed_startup = startup.clone();
    let updates = updates::Updates::install(
        &window,
        presenter.clone(),
        update_start,
        !options.ui_smoke && wakeup_probe.is_none(),
        Rc::new(move || revealed_startup.revealed()),
        update_restart,
    );
    let (viewers, image_preparation) = viewers::install(
        &window,
        presenter.clone(),
        control_send.clone(),
        &content_workers.viewer,
        filesystem.clone(),
        local.clone(),
        &mut *image_spare_retirement,
    );
    content_workers.viewer_windows = Some(viewers.clone());
    let _ = settings_loaded.set(viewers::SettingsLoaded::new(&viewers, &image_preparation));
    let thumbnails = thumbnails::install(
        &window,
        presenter.clone(),
        filesystem.clone(),
        control_send.clone(),
        thumbnail_routing,
    );
    let comparators = comparator::install(
        &window,
        presenter.clone(),
        control_send.clone(),
        &content_workers.compare,
        local,
        filesystem,
        comparator_backend,
    );
    content_workers.compare_windows = Some(comparators.clone());
    content_appearance::install(&window, &viewers, &comparators);
    let _host_appearance = midnight_ui::observe_host_appearance(&window, &presenter);
    let _ = comparators_loaded.set(Rc::downgrade(&comparators));
    let close_viewers = viewers.clone();
    let close_comparators = comparators.clone();
    let retry = retry_pass::RetryPass {
        presenter: presenter.clone(),
        controls: control_send.clone(),
        begin_owned: begin_owned.clone(),
        window: window.as_weak(),
        close_viewers: Box::new(move || {
            close_viewers.close_all();
            close_comparators.close_all();
        }),
        preparation: Rc::downgrade(&image_preparation),
        visible: visible_sync,
        refusals: refusals.clone(),
    };
    let retry_pulse = pulse::Pulse::start(retry_pass::INTERVAL, move || retry.run());
    thumbnails.follow_with(&retry_pulse);
    let pulses = [thumbnails.pulse(), Rc::downgrade(&retry_pulse)];
    for pulse in &pulses {
        refusals.watch(pulse.clone());
    }
    let presentation = pulses.clone();
    let imported_geometry = (Rc::downgrade(&window_state), Rc::downgrade(&presenter));
    window.on_presentation_changed(move || {
        pulse::wake_all(&presentation);
        // A committed settings import moves the window from the geometry sampler.
        if let (Some(state), Some(presenter)) =
            (imported_geometry.0.upgrade(), imported_geometry.1.upgrade())
            && presenter
                .try_borrow()
                .is_ok_and(|p| p.imported_window_geometry_pending())
        {
            state.geometry_changed();
        }
    });
    main_events.on_visibility(move || pulse::wake_all(&pulses));
    let geometry_state = Rc::downgrade(&window_state);
    let scale = [thumbnails.pulse()];
    main_events.on_geometry(move || {
        if let Some(state) = geometry_state.upgrade() {
            state.geometry_changed();
        }
        // A scale change changes the decode edge of every visible thumbnail.
        pulse::wake_all(&scale);
    });
    if options.ui_smoke {
        let error_state = smoke_error.clone();
        let state = presenter.clone();
        let shutdown_commands = commands.clone();
        slint::Timer::single_shot(Duration::from_secs(30), move || {
            *error_state.borrow_mut() = Some("UI smoke test timed out".into());
            state
                .borrow_mut()
                .shutdown_drag_drop(&|command| shutdown_commands.try_send(command).is_ok());
            let _ = slint::quit_event_loop();
        });
    }
    // Run while hidden so preferences can arrive without displaying defaults.
    // Startup restores native geometry before its first show. Keep hide explicit
    // so native drop targets become invalid first, including loop errors.
    let result = slint::run_event_loop()
        .map_err(|error| error.to_string())
        .and(startup.finish());
    let revealed = startup.revealed();
    // Abort before joining any worker. Reads may have queued normalization saves.
    startup_owner.gate().abort();
    if revealed {
        window_state.finish(&window, &mut presenter.borrow_mut());
    }
    let retry = may_retry && !revealed && result.is_err();
    presenter
        .borrow_mut()
        .shutdown_drag_drop(&|command| commands.try_send(command).is_ok());
    let result = result.and(window.hide().map_err(|error| error.to_string()));
    window.invoke_close_information_windows();
    drop(image_preparation);
    if let Some(retirement) = image_spare_retirement.as_ref() {
        retirement.stop_spare();
    }
    viewers.close_all();
    comparators.close_all();
    window.invoke_close_find_windows();
    presenter
        .borrow_mut()
        .flush_find_window_preferences(&|command| commands.try_send(command).is_ok());
    window.invoke_close_checksum_windows();
    window.invoke_close_file_list_window();
    retry_pulse.stop();
    updates.stop();
    size_updates.stop();
    thumbnails.stop();
    drop(thumbnails);
    // Detach the receiver from dormant UI callbacks without holding a RefCell
    // borrow across the post-window asynchronous exit grace.
    let (_, empty_events) = mpsc::channel(1);
    let mut closing_events = std::mem::replace(&mut *receiver.borrow_mut(), empty_events);
    drop(receiver);
    let mut retired: Vec<midnight_ui::DeferredDrop> = Vec::new();
    runtime.block_on(async {
        let deadline = tokio::time::Instant::now() + EXIT_GRACE;
        if revealed {
            shutdown_preferences::reconcile(
                &presenter,
                &mut closing_events,
                &commands,
                deadline,
                &mut retired,
                preferences_requested.get(),
            )
            .await;
            shutdown_preferences::flush_final(
                &presenter,
                &mut closing_events,
                &commands,
                deadline,
                &mut retired,
            )
            .await;
        }
        reclaim::retire_receiver(closing_events, &mut retired);
        presenter.borrow_mut().retire_all();
        retired.extend(presenter.borrow_mut().take_retired());
        if let Some(pending) = final_command_reclaim.take_ready() {
            retired.extend(pending);
        }
        let sent_shutdown = commands.try_send(Command::Shutdown).is_ok();
        // Reconciliation and accepted-save draining share one total deadline, but a slow
        // settings read cannot take the engine's own minimum shutdown grace.
        let engine_deadline = deadline.max(tokio::time::Instant::now() + ENGINE_MINIMUM_GRACE);
        let _ = tokio::time::timeout_at(engine_deadline, async {
            if !sent_shutdown {
                let _ = commands.send(Command::Shutdown).await;
            }
            let _ = engine.await;
        })
        .await;
        shutdown::retire_final(
            &final_retire_queue,
            std::mem::take(&mut retired),
            tokio::time::Instant::now() + RECLAIM_GRACE,
        )
        .await;
    });
    if let Err(error) = result {
        return startup_recovery::native_failure(error, retry);
    }
    if let Some(error) = smoke_error.borrow_mut().take() {
        return Err(error);
    }
    if let Some(path) = &options.snapshot {
        let pixels = snapshot
            .borrow_mut()
            .take()
            .ok_or("UI smoke test did not capture a snapshot")?;
        runtime.block_on(snapshot::save(
            path.clone(),
            pixels.width(),
            pixels.height(),
            pixels.as_bytes().to_vec(),
        ))?;
        println!("snapshot: {}", path.display());
    }
    Ok(startup_recovery::Attempt::Finished)
}

#[cfg(test)]
mod content_worker_tests {
    use super::*;

    struct Release(Vec<std::sync::mpsc::Sender<()>>);

    impl Drop for Release {
        fn drop(&mut self) {
            for sender in self.0.drain(..) {
                let _ = sender.send(());
            }
        }
    }

    #[test]
    fn content_domains_keep_original_transient_runtime_available_through_retirement() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .max_blocking_threads(16)
            .enable_all()
            .build()
            .unwrap();
        let workers = ContentWorkers::start(&runtime).unwrap();
        let mut release = Release(Vec::new());
        let (ready, received) = std::sync::mpsc::channel::<(&str, bool)>();
        for (name, execution, count) in [
            ("viewer", &workers.viewer, 5),
            ("comparison", &workers.compare, 2),
        ] {
            for _ in 0..count {
                let (stop, stopped) = std::sync::mpsc::channel();
                release.0.push(stop);
                let ready = ready.clone();
                assert!(
                    execution
                        .resident
                        .spawn(
                            &execution.transient,
                            Box::new(move || {
                                ready
                                    .send((name, tokio::runtime::Handle::try_current().is_ok()))
                                    .unwrap();
                                let _ = stopped.recv();
                            }),
                        )
                        .is_ok()
                );
            }
        }
        let held = (0..7)
            .map(|_| received.recv_timeout(Duration::from_secs(5)).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(held.iter().filter(|(name, _)| *name == "viewer").count(), 5);
        assert_eq!(
            held.iter()
                .filter(|(name, _)| *name == "comparison")
                .count(),
            2
        );
        assert!(held.iter().all(|(_, entered)| *entered));
        assert_eq!(
            runtime.block_on(async {
                tokio::time::timeout(Duration::from_secs(2), tokio::task::spawn_blocking(|| 42))
                    .await
                    .unwrap()
                    .unwrap()
            }),
            42,
            "all seven held content jobs must leave the original transient pool usable"
        );
        drop(release);
        workers.finish(&runtime).unwrap();
        assert_eq!(runtime.block_on(async { 43 }), 43);
        runtime.shutdown_timeout(Duration::from_secs(1));
    }
}

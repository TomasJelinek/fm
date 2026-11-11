#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

mod comparator;
mod native_content;
mod shutdown_preferences;
mod size_refresh;
mod smoke_settings;
mod startup;
mod startup_gate;
mod startup_recovery;
mod startup_settings;
mod thumbnails;
mod viewers;
mod visible_sync;
mod wake_gate;
mod window_backend;
mod window_state;

use midnight_core::{
    Command, DesktopServices, Event, NavigationId, PaneId, SettingsService, SortSpec, WatchService,
};
use midnight_engine::{Engine, reclaim, snapshot};
use midnight_platform::{LocalFileSystem, home_directory};
use midnight_ui::{MainWindow, Presenter};
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use visible_sync::VisibleSync;
use wake_gate::WakeGate;

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
                    "Usage: me [--left PATH] [--right PATH] [--software]\n",
                    "       me --smoke-test [--left PATH] [--right PATH]\n",
                    "       me --ui-smoke-test [--snapshot FILE.png] [--software]\n\n",
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
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    let result = run();
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Midnight Explorer: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let Some(options) = options()? else {
        return Ok(());
    };
    let local = Arc::new(LocalFileSystem::new(home_directory()));
    let filesystem = midnight_native_filesystem_host::InstalledProvider::shared(home_directory());
    filesystem.set_size_directory_service(local.clone());
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(16)
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    if options.smoke {
        let result = runtime.block_on(smoke_test(filesystem, &options));
        runtime.shutdown_timeout(Duration::from_secs(1));
        return result;
    }
    let restart_arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    let restart_cwd = std::env::current_dir().map_err(|error| error.to_string())?;
    let result = run_window(&runtime, local, filesystem, &options);
    // Normal close drains mutation cancellation before hiding the window.
    // A failed startup also stops its runtime before another process is started.
    startup_recovery::finish(
        result,
        || runtime.shutdown_timeout(Duration::from_secs(1)),
        || {
            let executable = std::env::current_exe().map_err(|error| error.to_string())?;
            let status =
                startup_recovery::software_command(&executable, &restart_arguments, &restart_cwd)
                    .status()
                    .map_err(|error| error.to_string())?;
            if status.success() {
                Ok(())
            } else {
                Err(format!("Software process exited with {status}"))
            }
        },
    )
}

async fn smoke_test(
    filesystem: Arc<midnight_native_filesystem_host::InstalledProvider>,
    options: &Options,
) -> Result<(), String> {
    let (commands, receiver) = mpsc::channel(16);
    let (sender, mut events) = mpsc::channel(16);
    let engine = tokio::spawn(
        Engine::new(filesystem.clone(), sender)
            .with_archive_capture(native_content::archive(filesystem))
            .with_locale(midnight_platform::locale_service())
            .run(receiver),
    );
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
        let mut failure = None;
        while !completed.into_iter().all(|done| done) || !volumes_ready {
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

fn run_window(
    runtime: &tokio::runtime::Runtime,
    local: Arc<LocalFileSystem>,
    filesystem: Arc<midnight_native_filesystem_host::InstalledProvider>,
    options: &Options,
) -> Result<startup_recovery::Attempt, String> {
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
    if let Err(error) = window_backend::select(options.software) {
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
    // This owner aborts/wakes a pending settings write on every early return.
    let startup_owner = startup_gate::Owner::new();
    let settings = smoke_settings::for_run(options.ui_smoke, settings);
    let settings = startup_settings::for_startup(settings, startup_owner.gate());
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
    let (commands, command_receiver) = mpsc::channel(32);
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
    let retire_pending = Rc::new(RefCell::new(None::<reclaim::Retired>));
    let engine = runtime.spawn(
        Engine::with_wakeup(filesystem.clone(), event_sender, notify.clone())
            .with_archive_capture(native_content::archive(filesystem.clone()))
            .with_navigation_fallback(local.clone())
            .with_shell_menu(local.clone())
            .with_path_picker(local.clone())
            .with_transfers(local.clone())
            .with_find(local.clone())
            .with_user_commands(local.clone())
            .with_editor(local.clone())
            .with_drag_drop(local.clone())
            .with_locale(midnight_platform::locale_service())
            .with_mutations(mutations)
            .with_network(network)
            .with_desktop(desktop)
            .with_clipboard(Arc::new(midnight_platform::NativeClipboard))
            .with_watch(watch)
            .with_settings(settings)
            .run(command_receiver),
    );

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
    let final_retire_queue = retire_queue.clone();
    let final_retire_pending = retire_pending.clone();
    let size_updates = Rc::new(RefCell::new(size_refresh::SizeRefresh::default()));
    let pending_sizes = size_updates.clone();
    let window_state = Rc::new(window_state::WindowState::install(
        &window,
        presenter.clone(),
        commands.clone(),
        !options.ui_smoke,
    ));
    let startup = Rc::new(startup::Startup::install(
        &window,
        presenter.clone(),
        window_state.clone(),
        startup_owner.gate(),
    )?);
    let startup_ready = startup.clone();
    window.on_backend_ready(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let start = Instant::now();
        let mut changed = false;
        let mut preferences_applied = false;
        let mut can_drain = reclaim::flush_pending(&retire_queue, &mut retire_pending.borrow_mut());
        {
            let mut events = ui_receiver.borrow_mut();
            let mut presenter = state.borrow_mut();
            if can_drain {
                let retired = presenter.take_retired();
                if !retired.is_empty() {
                    *retire_pending.borrow_mut() = Some(retired);
                    can_drain =
                        reclaim::flush_pending(&retire_queue, &mut retire_pending.borrow_mut());
                }
            }
            for _ in 0..16 {
                if !can_drain {
                    break;
                }
                let Ok(event) = events.try_recv() else {
                    break;
                };
                preferences_applied |= matches!(&event, Event::PreferencesLoaded(_));
                let size_only = matches!(
                    &event,
                    Event::FolderSizeUpdated { .. }
                        | Event::FolderSizesFinished { .. }
                        | Event::ArchiveSizes(_)
                );
                if let Event::ListingFinished { pane, .. } | Event::ListingFailed { pane, .. } =
                    &event
                {
                    let mut finished = done.get();
                    finished[pane.index()] = true;
                    done.set(finished);
                }
                if let Event::DirectoryChanged { pane, navigation } = event {
                    presenter.refresh_if_changed(pane, navigation, &|command| {
                        ui_commands.try_send(command).is_ok()
                    });
                } else {
                    presenter
                        .apply_with_sender(event, &|command| ui_commands.try_send(command).is_ok());
                }
                if size_only {
                    pending_sizes.borrow_mut().changed();
                } else {
                    changed = true;
                }
                let retired = presenter.take_retired();
                if !retired.is_empty() {
                    *retire_pending.borrow_mut() = Some(retired);
                    can_drain =
                        reclaim::flush_pending(&retire_queue, &mut retire_pending.borrow_mut());
                }
                if !can_drain || start.elapsed() >= Duration::from_millis(3) {
                    break;
                }
            }
        }
        if changed {
            midnight_ui::refresh(&window, &state.borrow());
            pending_sizes.borrow_mut().painted();
            sync_visible(&state.borrow(), &ui_commands, &ui_visible_sync);
        }
        if preferences_applied {
            // Both successful and failed reads must complete startup, after
            // their appearance/error presentation has reached the hidden UI.
            startup_ready.preferences_applied();
        }
        // Clear before rechecking the queue. A producer racing this check either
        // schedules its own callback or leaves data seen by this recheck.
        drain_gate.release();
        if (can_drain && !ui_receiver.borrow().is_empty())
            || (!can_drain && retire_queue.capacity() > 0)
        {
            reschedule();
        }
        if ui_smoke && done.get().into_iter().all(|value| value) && !scheduled.replace(true) {
            let weak = window.as_weak();
            let state = state.clone();
            let output = output.clone();
            let errors = error_state.clone();
            let shutdown_commands = ui_commands.clone();
            slint::Timer::single_shot(Duration::from_millis(350), move || {
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

    let sender = commands.clone();
    let startup_request = startup.clone();
    midnight_ui::install(
        &window,
        presenter.clone(),
        Rc::new(move |command| {
            let initial_preferences = matches!(&command, Command::LoadPreferences);
            let sent = sender.try_send(command).is_ok();
            if initial_preferences && !sent {
                // install reports the rejected request and refreshes before
                // returning to the event loop. Do not leave startup waiting.
                startup_request.preferences_applied();
            }
            sent
        }),
    );
    let viewers = viewers::install(
        &window,
        presenter.clone(),
        commands.clone(),
        runtime.handle(),
        filesystem.clone(),
        local.clone(),
    );
    let thumbnail_timer = thumbnails::install(&window, presenter.clone(), filesystem.clone());
    let comparators = comparator::install(
        &window,
        presenter.clone(),
        commands.clone(),
        runtime.handle(),
        local,
        filesystem,
    );
    // Both panes share a four-Hz size pulse. Interactive/backend events still
    // repaint immediately and include the latest cached totals.
    let size_timer = slint::Timer::default();
    let size_state = presenter.clone();
    let size_window = window.as_weak();
    size_timer.start(
        slint::TimerMode::Repeated,
        size_refresh::INTERVAL,
        move || {
            let changed = size_updates.borrow_mut().take_pending();
            if changed && let Some(window) = size_window.upgrade() {
                midnight_ui::refresh(&window, &size_state.borrow());
            }
        },
    );
    // Bounded queue saturation cannot permanently lose the accepted-listing ack.
    // Directory rows are shared; this timer only reads identities and tries sends.
    let visible_timer = slint::Timer::default();
    let sync_state = presenter.clone();
    let sync_commands = commands.clone();
    let sync_window = window.as_weak();
    let sync_viewers = viewers.clone();
    let sync_comparators = comparators.clone();
    visible_timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(100),
        move || {
            let mut state = sync_state.borrow_mut();
            if state.continue_operation_shutdown(&|command| sync_commands.try_send(command).is_ok())
                && let Some(window) = sync_window.upgrade()
            {
                state.shutdown_drag_drop(&|command| sync_commands.try_send(command).is_ok());
                sync_viewers.close_all();
                sync_comparators.close_all();
                window.invoke_close_find_windows();
                window.invoke_close_checksum_windows();
                window.invoke_close_file_list_window();
                window.invoke_close_information_windows();
                let _ = window.hide();
            }
            if sync_window
                .upgrade()
                .is_none_or(|window| !window.window().is_visible())
            {
                sync_viewers.close_all();
                sync_comparators.close_all();
            }
            state.flush_selection(&|command| sync_commands.try_send(command).is_ok());
            state.flush_filters(&|command| sync_commands.try_send(command).is_ok());
            state.flush_filename_colors(&|command| sync_commands.try_send(command).is_ok());
            state.flush_search(&|command| sync_commands.try_send(command).is_ok());
            state.flush_comparison(&|command| sync_commands.try_send(command).is_ok());
            sync_visible(&state, &sync_commands, &visible_sync);
        },
    );
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
    viewers.close_all();
    comparators.close_all();
    window.invoke_close_find_windows();
    window.invoke_close_checksum_windows();
    window.invoke_close_file_list_window();
    visible_timer.stop();
    size_timer.stop();
    thumbnail_timer.stop();
    drop(thumbnail_timer);
    // Detach the receiver from dormant UI callbacks without holding a RefCell
    // borrow across the post-window asynchronous exit grace.
    let (_, empty_events) = mpsc::channel(1);
    let mut closing_events = std::mem::replace(&mut *receiver.borrow_mut(), empty_events);
    drop(receiver);
    let mut retired: Vec<midnight_ui::DeferredDrop> = Vec::new();
    runtime.block_on(async {
        let deadline = tokio::time::Instant::now() + Duration::from_millis(2500);
        if revealed {
            shutdown_preferences::reconcile(
                &presenter,
                &mut closing_events,
                &commands,
                deadline,
                &mut retired,
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
        if let Some(pending) = final_retire_pending.borrow_mut().take() {
            retired.extend(pending);
        }
        let sent_shutdown = commands.try_send(Command::Shutdown).is_ok();
        // Reconciliation and accepted-save draining share one total deadline.
        let _ = tokio::time::timeout_at(deadline, async {
            if !sent_shutdown {
                let _ = commands.send(Command::Shutdown).await;
            }
            let _ = engine.await;
        })
        .await;
        match final_retire_queue.send(std::mem::take(&mut retired)).await {
            Ok(()) => {
                let _ = reclaim::barrier(&final_retire_queue).await;
            }
            Err(error) => {
                let _ = tokio::task::spawn_blocking(move || drop(error.0)).await;
            }
        }
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

fn sync_visible(
    presenter: &Presenter,
    commands: &mpsc::Sender<Command>,
    sync: &RefCell<VisibleSync>,
) {
    for pane in [PaneId::Left, PaneId::Right] {
        if let Some(visible) = presenter.visible_directory(pane) {
            sync.borrow_mut().send(visible, commands);
        }
    }
}

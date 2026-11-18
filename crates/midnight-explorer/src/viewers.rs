use midnight_core::{Command, PaneId, ViewerAppearance, ViewerAssociations, ViewerLaunch};
use midnight_ui::{MainWindow, Presenter, Theme};
use midnight_viewer_ui::{ViewerWindows, WindowPlacement};
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

pub(crate) fn finish_after_image_spare<T>(
    result: Result<crate::startup_recovery::Attempt, String>,
    retire_spare: impl FnOnce() -> (T, Result<(), String>),
    shutdown_runtime: impl FnOnce(T),
    retry: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    // Transfer the runtime only after a terminal spare result, including startup errors.
    let (runtime, retirement) = retire_spare();
    let result = match retirement {
        Ok(()) => result,
        Err(error) => Err(match result {
            Ok(crate::startup_recovery::Attempt::NativeStartupFailed(startup)) | Err(startup) => {
                format!("{startup}\n{error}")
            }
            Ok(crate::startup_recovery::Attempt::Finished) => error,
        }),
    };
    // Existing runtime cleanup still runs on failure; Err cannot enter software retry.
    crate::startup_recovery::finish(result, || shutdown_runtime(runtime), retry)
}

#[cfg(test)]
mod image_spare_ordering_tests {
    use super::*;
    use crate::startup_recovery::Attempt;
    use std::{
        cell::Cell,
        sync::{Arc, Mutex, mpsc},
        time::Duration,
    };

    #[test]
    fn image_spare_acknowledgement_precedes_runtime_shutdown_and_software_retry() {
        let phases = Arc::new(Mutex::new(Vec::new()));
        let recorded = phases.clone();
        let (stopping, stopped) = mpsc::sync_channel(1);
        let (release, reaped) = mpsc::sync_channel(1);
        let (finished, completion) = mpsc::sync_channel(1);
        let task = std::thread::spawn(move || {
            let result = finish_after_image_spare(
                Ok(Attempt::NativeStartupFailed("renderer unavailable".into())),
                || {
                    recorded.lock().unwrap().push("spare stop");
                    stopping.send(()).unwrap();
                    reaped.recv_timeout(Duration::from_secs(5)).unwrap();
                    recorded.lock().unwrap().push("spare reaped");
                    (41u8, Ok(()))
                },
                |owned_runtime| {
                    assert_eq!(owned_runtime, 41);
                    recorded.lock().unwrap().push("runtime shutdown");
                },
                || {
                    recorded.lock().unwrap().push("software retry");
                    Ok(())
                },
            );
            finished.send(result).unwrap();
        });
        stopped.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(*phases.lock().unwrap(), ["spare stop"]);
        assert!(completion.try_recv().is_err());
        release.send(()).unwrap();
        completion
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        task.join().unwrap();
        assert_eq!(
            *phases.lock().unwrap(),
            [
                "spare stop",
                "spare reaped",
                "runtime shutdown",
                "software retry"
            ]
        );
    }

    #[test]
    fn image_spare_failure_preserves_ownership_through_runtime_shutdown_and_blocks_retry() {
        struct HeldSpare<'a>(&'a Cell<bool>);
        impl Drop for HeldSpare<'_> {
            fn drop(&mut self) {
                self.0.set(true);
            }
        }

        for outcome in [
            Ok(Attempt::NativeStartupFailed("renderer unavailable".into())),
            Ok(Attempt::Finished),
            Err("startup failed after registration".into()),
        ] {
            let released = Cell::new(false);
            let held = HeldSpare(&released);
            let phases = RefCell::new(Vec::new());
            let result = finish_after_image_spare(
                outcome,
                || {
                    let _retained = &held;
                    phases.borrow_mut().push("spare owner failed");
                    (41u8, Err("OwnerFailed; cleanup unconfirmed".into()))
                },
                |owned_runtime| {
                    assert_eq!(owned_runtime, 41);
                    assert!(!released.get());
                    phases.borrow_mut().push("runtime shutdown");
                },
                || panic!("failed retirement must suppress a renderer-qualified retry"),
            );
            let error = result.unwrap_err();
            assert!(error.contains("OwnerFailed"));
            assert!(error.contains("cleanup unconfirmed"));
            assert_eq!(*phases.borrow(), ["spare owner failed", "runtime shutdown"]);
            assert!(!released.get(), "ownership must survive the fatal result");
            drop(held);
            assert!(released.get());
        }
    }
}

pub(crate) fn install(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    commands: Rc<dyn Fn(Command) -> bool>,
    execution: &crate::ContentExecution,
    filesystem: std::sync::Arc<midnight_native_filesystem_host::InstalledProvider>,
    resources: std::sync::Arc<dyn midnight_core::TransferService>,
    retirement: &mut Option<midnight_viewer_engine::ImageSpareRetirement>,
) -> (Rc<ViewerWindows>, Rc<ImagePreparation>) {
    let service = midnight_viewer_engine::Service::with_native_capture(
        execution.transient.clone(),
        execution.resident.clone(),
        crate::native_content::factory_with_resources(filesystem, resources, &execution.transient),
    );
    // The outer run owns this ticket on every later Result/early-return path.
    // Register before the settings response or ViewerWindows can start catalog preparation.
    debug_assert!(retirement.is_none());
    *retirement = Some(service.image_spare_retirement());
    let image_preparation = Rc::new(ImagePreparation::new(service.clone()));
    let viewers = Rc::new(ViewerWindows::new(service));
    let settings = presenter.clone();
    let settings_commands = commands.clone();
    viewers.set_preferences_listener(Rc::new(move |change| {
        settings
            .borrow_mut()
            .save_viewer_preferences(change, settings_commands.as_ref());
    }));
    {
        // Refresh is called with an immutable Presenter borrow. Schedule the
        // handoff after it returns rather than re-entering that borrow.
        let scheduled = Rc::new(Cell::new(false));
        let state = presenter.clone();
        let sender = commands.clone();
        let manager = viewers.clone();
        let weak = window.as_weak();
        window.on_refresh_file_list_viewer(move || {
            if scheduled.replace(true) {
                return;
            }
            let scheduled = scheduled.clone();
            let state = state.clone();
            let sender = sender.clone();
            let manager = Rc::downgrade(&manager);
            let weak = weak.clone();
            slint::Timer::single_shot(Duration::ZERO, move || {
                scheduled.set(false);
                let (Some(window), Some(manager)) = (weak.upgrade(), manager.upgrade()) else {
                    return;
                };
                let launch = state.borrow_mut().take_generated_file_list_viewer();
                let Some((id, generation, launch)) = launch else {
                    return;
                };
                let ready_state = Rc::downgrade(&state);
                let ready_window = window.as_weak();
                let ready = Box::new(move |result| {
                    // Shutdown can close viewers while borrowing Presenter. Defer both
                    // success and cancellation, retaining the exact originating session.
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
                if let Err(error) = open(&window, &state, &sender, &manager, launch, Some(ready)) {
                    state.borrow_mut().generated_file_list_viewer_finished(
                        id,
                        generation,
                        Err(error),
                    );
                    midnight_ui::refresh(&window, &state.borrow());
                }
            });
        });
    }
    install_warning_reports(window, &presenter, &commands, &viewers);
    install_find_results(window, &presenter, &commands, &viewers);
    let manager = viewers.clone();
    let weak = window.as_weak();
    window.on_view_file(move |pane, basic| {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let pane = if pane == 0 {
            PaneId::Left
        } else {
            PaneId::Right
        };
        let launch = presenter.borrow().viewer_launch(pane, basic);
        let Some(launch) = launch else {
            let reason = presenter.borrow().viewer_unavailable_reason(pane);
            if let Some(reason) = reason {
                presenter.borrow_mut().report_viewer_error(reason);
                midnight_ui::refresh(&window, &presenter.borrow());
            }
            return;
        };
        if let Err(error) = open(&window, &presenter, &commands, &manager, launch, None) {
            presenter.borrow_mut().report_viewer_error(error);
            midnight_ui::refresh(&window, &presenter.borrow());
        }
    });
    (viewers, image_preparation)
}

type Prepare = Box<dyn Fn(Option<Arc<[String]>>, Arc<ViewerAssociations>)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Preparation {
    Waiting,
    Prepared,
    Unprepared,
    Revoked,
}

/// The spare image worker is prepared once, when the initial settings response is
/// applied; discovery and native work run on the backend. A failed preparation is not
/// retried: the first image view then starts its worker on demand. An exit request that
/// waits for live jobs revokes the prepared spare once.
pub(crate) struct ImagePreparation {
    prepare: Prepare,
    state: Cell<Preparation>,
}

impl ImagePreparation {
    fn new(service: midnight_viewer_engine::Service) -> Self {
        Self::with(move |disabled, associations| {
            service.prepare_image_workers(disabled, associations);
        })
    }

    fn with(prepare: impl Fn(Option<Arc<[String]>>, Arc<ViewerAssociations>) + 'static) -> Self {
        Self {
            prepare: Box::new(prepare),
            state: Cell::new(Preparation::Waiting),
        }
    }

    /// The initial settings response, successful or failed, reached the presenter.
    /// A failed or blocked read leaves image viewers without a spare for the session.
    pub(crate) fn settings_applied(&self, presenter: &Presenter) {
        if self.state.get() != Preparation::Waiting {
            return;
        }
        if let Some((disabled, associations)) = presenter.viewer_startup_policy() {
            self.state.set(Preparation::Prepared);
            (self.prepare)(Some(disabled), associations);
        } else {
            self.state.set(Preparation::Unprepared);
        }
    }

    /// Release a prepared spare once shutdown was requested, instead of holding it
    /// while live jobs drain until final retirement.
    pub(crate) fn exit_requested(&self, presenter: &Presenter) {
        if self.state.get() == Preparation::Prepared && presenter.viewer_startup_policy().is_none()
        {
            self.state.set(Preparation::Revoked);
            (self.prepare)(None, Arc::default());
        }
    }
}

/// Work that waits for the initial settings response. The backend-ready handler is
/// installed before the viewers, so it reaches them through this late-bound handle.
pub(crate) struct SettingsLoaded {
    viewers: std::rc::Weak<ViewerWindows>,
    preparation: std::rc::Weak<ImagePreparation>,
}

impl SettingsLoaded {
    pub(crate) fn new(viewers: &Rc<ViewerWindows>, preparation: &Rc<ImagePreparation>) -> Self {
        Self {
            viewers: Rc::downgrade(viewers),
            preparation: Rc::downgrade(preparation),
        }
    }

    /// Call once the initial settings response, successful or failed, was applied.
    pub(crate) fn applied(&self, presenter: &RefCell<Presenter>) {
        let preferences = {
            let state = presenter.borrow();
            if let Some(preparation) = self.preparation.upgrade() {
                preparation.settings_applied(&state);
            }
            Arc::new(state.viewer_preferences().clone())
        };
        // Viewers opened before the read show defaults plus their own edits, which the
        // presenter merged into these values. Restoration saves nothing, reports no
        // edit and runs without a Presenter borrow.
        if let Some(viewers) = self.viewers.upgrade() {
            viewers.restore_preferences(preferences);
        }
    }
}

#[cfg(test)]
mod preparation_tests {
    use super::*;
    use midnight_core::{Event, FsError, UserPreferences};
    use std::sync::atomic::{AtomicBool, Ordering};

    struct CaptureLifetime(Arc<AtomicBool>);

    impl Drop for CaptureLifetime {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    #[test]
    fn dropping_the_preparation_releases_the_service_without_an_event_loop_callback() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let (owner, spawner) = midnight_work_budget::resident::Owner::start(
            "viewer-preparation-test",
            midnight_viewer_engine::RESIDENT_WORKERS,
        )
        .unwrap();
        let released = Arc::new(AtomicBool::new(false));
        let lifetime = CaptureLifetime(released.clone());
        let capture: midnight_viewer_engine::NativeCapture = Arc::new(move |_, _| {
            let _lifetime = &lifetime;
            Err("Lifetime-only capture must not be invoked".into())
        });
        let service = midnight_viewer_engine::Service::with_native_capture(
            runtime.handle().clone(),
            spawner.clone(),
            capture,
        );
        let preparation = Rc::new(ImagePreparation::new(service));
        let viewers = Rc::new(ViewerWindows::new(
            midnight_viewer_engine::Service::with_native_capture(
                runtime.handle().clone(),
                spawner,
                Arc::new(|_, _| Err("No content is read".into())),
            ),
        ));
        let loaded = SettingsLoaded::new(&viewers, &preparation);
        assert!(!released.load(Ordering::Acquire));
        drop(preparation);
        assert!(
            released.load(Ordering::Acquire),
            "Shutdown must drop the service with its handle"
        );
        // A late response after shutdown reaches nothing.
        let root = std::env::current_dir().unwrap();
        loaded.applied(&RefCell::new(Presenter::new(root.clone(), root)));
        drop(viewers);
        runtime.block_on(owner.shutdown(runtime.handle())).unwrap();
    }

    type Calls = Rc<RefCell<Vec<Option<Arc<[String]>>>>>;

    fn recording() -> (ImagePreparation, Calls) {
        let calls = Calls::default();
        let recorded = calls.clone();
        let preparation =
            ImagePreparation::with(move |disabled, _| recorded.borrow_mut().push(disabled));
        (preparation, calls)
    }

    fn presenter(read: Result<UserPreferences, FsError>) -> Presenter {
        let root = std::env::current_dir().unwrap();
        let mut presenter = Presenter::new(root.clone(), root);
        let accepted = read
            .map(|preferences| {
                Box::new(midnight_core::AcceptedPreferences {
                    preferences,
                    comparators: Default::default(),
                })
            })
            .map_err(Into::into);
        presenter.apply(Event::PreferencesLoaded(accepted));
        presenter
    }

    #[test]
    fn loaded_settings_prepare_the_spare_once_and_an_exit_request_revokes_it_once() {
        let (preparation, calls) = recording();
        let mut presenter = presenter(Ok(UserPreferences {
            disabled_viewer_plugins: vec!["example.viewer.disabled".into()],
            ..Default::default()
        }));
        preparation.settings_applied(&presenter);
        preparation.settings_applied(&presenter);
        let disabled: Arc<[String]> = Arc::from(["example.viewer.disabled".to_owned()]);
        assert_eq!(*calls.borrow(), [Some(disabled)]);
        preparation.exit_requested(&presenter);
        assert_eq!(calls.borrow().len(), 1, "no exit was requested");
        presenter.request_operation_shutdown(&|_| true);
        preparation.exit_requested(&presenter);
        preparation.exit_requested(&presenter);
        assert_eq!(calls.borrow().len(), 2);
        assert_eq!(calls.borrow()[1], None, "the spare is revoked once");
        preparation.settings_applied(&presenter);
        assert_eq!(calls.borrow().len(), 2);
    }

    #[test]
    fn a_failed_settings_read_prepares_and_revokes_nothing() {
        let (preparation, calls) = recording();
        let mut presenter = presenter(Err(FsError::from_io(
            "preferences",
            std::path::Path::new("preferences.json"),
            std::io::Error::other("unreadable"),
        )));
        preparation.settings_applied(&presenter);
        presenter.request_operation_shutdown(&|_| true);
        preparation.exit_requested(&presenter);
        assert!(calls.borrow().is_empty());
    }
}

#[cfg(test)]
mod restore_tests {
    use super::*;
    use midnight_core::{
        DirectoryIndex, Entry, EntryId, EntryKind, Event, UserPreferences, ViewerPreferences,
    };
    use midnight_viewer_ui::ViewerWindow;
    use slint::platform::{
        Platform, PlatformError, WindowAdapter,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    };

    struct Windows;
    impl Platform for Windows {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
            Ok(MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer))
        }
    }

    fn open(manager: &ViewerWindows) -> ViewerWindow {
        let path = std::env::temp_dir().join("restore before read 文档.txt");
        let entries: Arc<[Entry]> =
            vec![Entry::new(EntryId(1), path.clone(), EntryKind::File)].into();
        manager
            .open(
                ViewerLaunch {
                    find: None,
                    associations: Default::default(),
                    tree: None,
                    generated: None,
                    // Opened during the hidden startup: the presenter still has defaults.
                    preferences: Arc::new(ViewerPreferences::default()),
                    focused: path,
                    view: Arc::new(DirectoryIndex::from_entries(&entries)),
                    entries,
                    archive: None,
                    basic: true,
                    disabled_plugins: Arc::from([]),
                },
                ViewerAppearance {
                    family: "monospace".into(),
                    font_size: 14.0,
                    foreground: [255; 4],
                    background: [0, 0, 0, 255],
                    selection: [80, 80, 80, 255],
                    accent: [100, 100, 255, 255],
                },
                slint::LogicalSize::new(640.0, 480.0),
                Rc::new(|_, _| None),
                Rc::new(|_| {}),
            )
            .unwrap()
            .upgrade()
            .unwrap()
    }

    #[test]
    fn the_settings_read_restores_merged_choices_into_viewers_opened_before_it() {
        slint::platform::set_platform(Box::new(Windows)).unwrap();
        let runtime = crate::resident_test_runtime::TestRuntime::new(
            tokio::runtime::Runtime::new().unwrap(),
            "viewer-settings-restore",
            midnight_viewer_engine::RESIDENT_WORKERS,
        );
        let service = midnight_viewer_engine::Service::with_native_capture(
            runtime.handle().clone(),
            runtime.spawner(),
            Arc::new(|_, _| Err("Content is deliberately unavailable.".into())),
        );
        let viewers = Rc::new(ViewerWindows::new(service));
        let root = std::env::current_dir().unwrap();
        let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root)));
        let edits = Rc::new(Cell::new(0));
        let (settings, counted) = (presenter.clone(), edits.clone());
        viewers.set_preferences_listener(Rc::new(move |change| {
            counted.set(counted.get() + 1);
            settings
                .borrow_mut()
                .save_viewer_preferences(change, &|_| true);
        }));
        let window = open(&viewers);
        assert!(window.get_line_numbers());
        assert_eq!(window.get_viewer_font_size(), 14);
        // An edit before the read is retained and merged into the loaded values.
        window.invoke_change_case_sensitive(true);
        assert_eq!(edits.get(), 1);
        let mut loaded = ViewerPreferences {
            line_numbers: false,
            ..Default::default()
        };
        loaded.text_font.size = 19;
        presenter
            .borrow_mut()
            .apply(Event::PreferencesLoaded(Ok(Box::new(
                midnight_core::AcceptedPreferences {
                    preferences: UserPreferences {
                        viewer: loaded,
                        ..Default::default()
                    },
                    comparators: Default::default(),
                },
            ))));
        let preparation = Rc::new(ImagePreparation::with(|_, _| {}));
        SettingsLoaded::new(&viewers, &preparation).applied(&presenter);
        let presented = (
            window.get_line_numbers(),
            window.get_viewer_font_size(),
            window.get_case_sensitive(),
            edits.get(),
        );
        // Close before asserting: an unwinding test must not wait on live sessions.
        viewers.close_all();
        assert_eq!(
            presented,
            (false, 19, true, 1),
            "the loaded choices reach the open viewer, its earlier edit survives the \
             merge, and restoration reports no edit"
        );
    }
}

fn open(
    window: &MainWindow,
    presenter: &Rc<RefCell<Presenter>>,
    commands: &Rc<dyn Fn(Command) -> bool>,
    manager: &ViewerWindows,
    launch: ViewerLaunch,
    ready: Option<midnight_viewer_ui::OpenReady>,
) -> Result<(), String> {
    let theme = window.global::<Theme>();
    let mut appearance = crate::content_appearance::capture(window);
    // Preserve the opening document snapshot; only the UI cache uses the resolved UI family.
    appearance.family = theme.get_font_family().to_string();
    appearance.font_size = appearance.font_size.max(11.0);
    let key_state = presenter.clone();
    let keys = Rc::new(move |chord: &_, search| key_state.borrow().viewer_key(chord, search));
    let copy_state = presenter.clone();
    let sender = commands.clone();
    let copy = Rc::new(move |text| {
        copy_state
            .borrow_mut()
            .copy_viewer_text(text, sender.as_ref())
    });
    let placement = WindowPlacement::capture(window.window());
    match ready {
        Some(ready) => manager.open_with_ready(launch, appearance, placement, keys, copy, ready),
        None => manager.open(launch, appearance, placement, keys, copy),
    }
    .map(|_| ())
}

#[cfg(test)]
fn service(
    runtime: &tokio::runtime::Handle,
    viewer_spawner: midnight_work_budget::resident::Spawner,
    filesystem: std::sync::Arc<midnight_native_filesystem_host::InstalledProvider>,
) -> midnight_viewer_engine::Service {
    let capture = crate::native_content::factory(filesystem);
    midnight_viewer_engine::Service::with_native_capture(
        runtime.clone(),
        viewer_spawner,
        std::sync::Arc::new(move |path, closed| capture(path, closed)),
    )
}

#[cfg(test)]
#[path = "viewer_filesystem_tests.rs"]
mod tests;

fn install_warning_reports(
    window: &MainWindow,
    presenter: &Rc<RefCell<Presenter>>,
    commands: &Rc<dyn Fn(Command) -> bool>,
    viewers: &Rc<ViewerWindows>,
) {
    window.on_refresh_warning_viewer(pending_viewers_callback(
        window,
        presenter,
        commands,
        viewers,
        Presenter::take_warning_viewer,
    ));
}

fn install_find_results(
    window: &MainWindow,
    presenter: &Rc<RefCell<Presenter>>,
    commands: &Rc<dyn Fn(Command) -> bool>,
    viewers: &Rc<ViewerWindows>,
) {
    window.on_refresh_find_viewer(pending_viewers_callback(
        window,
        presenter,
        commands,
        viewers,
        Presenter::take_find_viewer,
    ));
}

fn pending_viewers_callback(
    window: &MainWindow,
    presenter: &Rc<RefCell<Presenter>>,
    commands: &Rc<dyn Fn(Command) -> bool>,
    viewers: &Rc<ViewerWindows>,
    take: fn(&mut Presenter) -> Option<ViewerLaunch>,
) -> impl Fn() + use<> {
    let scheduled = Rc::new(Cell::new(false));
    let state = presenter.clone();
    let sender = commands.clone();
    let manager = viewers.clone();
    let weak = window.as_weak();
    move || {
        if scheduled.replace(true) {
            return;
        }
        let (scheduled, state, sender, manager, weak) = (
            scheduled.clone(),
            state.clone(),
            sender.clone(),
            Rc::downgrade(&manager),
            weak.clone(),
        );
        slint::Timer::single_shot(Duration::ZERO, move || {
            scheduled.set(false);
            let (Some(window), Some(manager)) = (weak.upgrade(), manager.upgrade()) else {
                return;
            };
            loop {
                let launch = take(&mut state.borrow_mut());
                let Some(launch) = launch else { break };
                if let Err(error) = open(&window, &state, &sender, &manager, launch, None) {
                    state.borrow_mut().report_viewer_error(error);
                    midnight_ui::refresh(&window, &state.borrow());
                }
            }
        });
    }
}

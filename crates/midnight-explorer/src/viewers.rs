use midnight_core::{Command, PaneId, ViewerAppearance, ViewerLaunch};
use midnight_ui::{MainWindow, Presenter, Theme};
use midnight_viewer_ui::{ViewerWindows, WindowPlacement};
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};
use tokio::sync::mpsc;

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

pub(crate) fn install(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    commands: mpsc::Sender<Command>,
    runtime: &tokio::runtime::Handle,
    filesystem: std::sync::Arc<midnight_native_filesystem_host::InstalledProvider>,
    resources: std::sync::Arc<dyn midnight_core::TransferService>,
    retirement: &mut Option<midnight_viewer_engine::ImageSpareRetirement>,
) -> (Rc<ViewerWindows>, slint::Timer) {
    let service = midnight_viewer_engine::Service::with_native_capture(
        runtime.clone(),
        crate::native_content::factory_with_resources(filesystem, resources),
    );
    // The outer run owns this ticket on every later Result/early-return path.
    // Register before timers or ViewerWindows can start catalog preparation.
    debug_assert!(retirement.is_none());
    *retirement = Some(service.image_spare_retirement());
    let image_preparation =
        prepare_images_after_settings(window.as_weak(), Rc::downgrade(&presenter), service.clone());
    let viewers = Rc::new(ViewerWindows::new(service));
    let settings = presenter.clone();
    let settings_commands = commands.clone();
    viewers.set_preferences_listener(Rc::new(move |change| {
        settings
            .borrow_mut()
            .save_viewer_preferences(change, &|command| {
                settings_commands.try_send(command).is_ok()
            });
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

fn prepare_images_after_settings(
    window: slint::Weak<MainWindow>,
    presenter: std::rc::Weak<RefCell<Presenter>>,
    service: midnight_viewer_engine::Service,
) -> slint::Timer {
    // Wait only by scheduling: panel startup, preference loading and UI input keep running.
    // The service suppresses unchanged memory-only policy; discovery and native work are backend.
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(200),
        move || {
            let (Some(_window), Some(state)) = (window.upgrade(), presenter.upgrade()) else {
                return;
            };
            let policy = state.borrow().viewer_startup_policy();
            if let Some((disabled, associations)) = policy {
                service.prepare_image_workers(Some(disabled), associations);
            } else {
                service.prepare_image_workers(None, Default::default());
            }
        },
    );
    timer
}

#[cfg(test)]
mod preparation_lifetime_tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    struct ClockOnly;

    impl slint::platform::Platform for ClockOnly {
        fn create_window_adapter(
            &self,
        ) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::platform::PlatformError>
        {
            panic!("Preparation lifetime test must not create a native window");
        }

        fn duration_since_start(&self) -> Duration {
            Duration::ZERO
        }
    }

    struct CaptureLifetime(Arc<AtomicBool>);

    impl Drop for CaptureLifetime {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    #[test]
    fn dropping_preparation_timer_releases_service_without_an_event_loop_callback() {
        slint::platform::set_platform(Box::new(ClockOnly)).unwrap();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let released = Arc::new(AtomicBool::new(false));
        let lifetime = CaptureLifetime(released.clone());
        let capture: midnight_viewer_engine::NativeCapture = Arc::new(move |_, _| {
            let _lifetime = &lifetime;
            Err("Lifetime-only capture must not be invoked".into())
        });
        let service =
            midnight_viewer_engine::Service::with_native_capture(runtime.handle().clone(), capture);
        let timer =
            prepare_images_after_settings(slint::Weak::default(), std::rc::Weak::new(), service);
        assert!(!released.load(Ordering::Acquire));
        timer.stop();
        assert!(
            !released.load(Ordering::Acquire),
            "Stopping retains its callback"
        );
        drop(timer);
        assert!(
            released.load(Ordering::Acquire),
            "Shutdown must drop the owned timer"
        );
    }
}

fn open(
    window: &MainWindow,
    presenter: &Rc<RefCell<Presenter>>,
    commands: &mpsc::Sender<Command>,
    manager: &ViewerWindows,
    launch: ViewerLaunch,
    ready: Option<midnight_viewer_ui::OpenReady>,
) -> Result<(), String> {
    let theme = window.global::<Theme>();
    let rgba = |color: slint::Color| {
        let color = color.to_argb_u8();
        [color.red, color.green, color.blue, color.alpha]
    };
    let appearance = ViewerAppearance {
        family: theme.get_font_family().to_string(),
        font_size: theme.get_font_size().max(11.0),
        foreground: rgba(theme.get_text()),
        background: rgba(theme.get_pane_background()),
        selection: rgba(theme.get_selection_background()),
        accent: rgba(theme.get_accent()),
    };
    let key_state = presenter.clone();
    let keys = Rc::new(move |chord: &_, search| key_state.borrow().viewer_key(chord, search));
    let copy_state = presenter.clone();
    let sender = commands.clone();
    let copy = Rc::new(move |text| {
        copy_state
            .borrow_mut()
            .copy_viewer_text(text, &|command| sender.try_send(command).is_ok())
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
    filesystem: std::sync::Arc<midnight_native_filesystem_host::InstalledProvider>,
) -> midnight_viewer_engine::Service {
    let capture = crate::native_content::factory(filesystem);
    midnight_viewer_engine::Service::with_native_capture(
        runtime.clone(),
        std::sync::Arc::new(move |path, closed| capture(path, closed)),
    )
}

#[cfg(test)]
#[path = "viewer_filesystem_tests.rs"]
mod tests;

fn install_warning_reports(
    window: &MainWindow,
    presenter: &Rc<RefCell<Presenter>>,
    commands: &mpsc::Sender<Command>,
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
    commands: &mpsc::Sender<Command>,
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
    commands: &mpsc::Sender<Command>,
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

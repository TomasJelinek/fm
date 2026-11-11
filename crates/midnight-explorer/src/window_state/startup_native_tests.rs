use super::*;
use crate::startup::Startup;
use midnight_core::{Command, Event, FsError, PreferenceTheme, UserPreferences, WindowGeometry};
use slint::winit_030::winit::platform::windows::EventLoopBuilderExtWindows;
use std::{cell::Cell, time::Duration};

fn backend() {
    let mut event_loop = slint::winit_030::winit::event_loop::EventLoop::<
        slint::winit_030::SlintEvent,
    >::with_user_event();
    event_loop.with_any_thread(true);
    let mut graphics = slint::wgpu_30::WGPUSettings::default();
    graphics.backends = slint::wgpu_30::wgpu::Backends::DX12;
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("femtovg-wgpu".into())
        .require_wgpu_30(slint::wgpu_30::WGPUConfiguration::Automatic(graphics))
        .with_winit_event_loop_builder(event_loop)
        .select()
        .unwrap();
}

#[derive(Clone, Copy)]
enum Load {
    Saved,
    Maximized,
    RemovedMonitor,
    Failed,
}

fn first_frame(load: Load) {
    backend();
    let window = MainWindow::new().unwrap();
    let presenter = Rc::new(RefCell::new(Presenter::new("left".into(), "right".into())));
    let (commands, mut received) = tokio::sync::mpsc::channel(32);
    let state = Rc::new(WindowState::install(
        &window,
        presenter.clone(),
        commands,
        true,
    ));
    let startup_owner = crate::startup_gate::Owner::new();
    let startup = Rc::new(
        Startup::install(
            &window,
            presenter.clone(),
            state.clone(),
            startup_owner.gate(),
        )
        .unwrap(),
    );
    let expected = Rc::new(RefCell::new(None::<WindowGeometry>));
    let rendered = Rc::new(Cell::new(false));
    let observed = rendered.clone();
    let bounds = expected.clone();
    let weak = window.as_weak();
    let observed_presenter = presenter.clone();
    window
        .window()
        .set_rendering_notifier(move |phase, _| {
            if !matches!(phase, slint::RenderingState::BeforeRendering) || observed.replace(true) {
                return;
            }
            let window = weak.upgrade().unwrap();
            let theme = if matches!(load, Load::Failed) {
                PreferenceTheme::Blue
            } else {
                PreferenceTheme::Nord
            };
            assert_eq!(window.get_theme_index(), theme.index(), "first frame theme");
            let split = if matches!(load, Load::Failed) {
                0.5
            } else {
                0.61
            };
            assert!(
                (window.get_panel_ratio() - split).abs() < 0.0001,
                "first frame split"
            );
            window
                .window()
                .with_winit_window(|native| {
                    if let Some(expected) = bounds.borrow().as_ref() {
                        assert_eq!(native.is_maximized(), expected.maximized);
                        if !expected.maximized {
                            assert_normal_geometry(native, expected);
                        }
                    }
                    assert_eq!(
                        native.is_visible(),
                        Some(false),
                        "pre-render precedes reveal"
                    );
                    println!(
                        "first frame: theme={}, position={:?}, size={:?}, native_visible={:?}",
                        window.get_theme_index(),
                        native.outer_position(),
                        native.inner_size(),
                        native.is_visible()
                    );
                })
                .unwrap();
            if matches!(load, Load::Failed) {
                assert!(!observed_presenter.borrow().window_geometry_ready());
                assert!(
                    observed_presenter
                        .borrow()
                        .pending_preferences_save()
                        .is_none()
                );
            }
        })
        .unwrap();
    let weak = window.as_weak();
    let ready = startup.clone();
    let loaded = presenter.clone();
    let before = rendered.clone();
    // A deliberately late settings response must not expose a default frame.
    slint::Timer::single_shot(Duration::from_millis(250), move || {
        let window = weak.upgrade().unwrap();
        assert!(!window.window().is_visible());
        assert!(!before.get(), "rendered before the preferences response");
        window
            .window()
            .with_winit_window(|native| {
                assert_eq!(native.is_visible(), Some(false));
            })
            .unwrap();
        let result = if matches!(load, Load::Failed) {
            Err(FsError::from_io(
                "load",
                std::path::Path::new("settings"),
                std::io::ErrorKind::PermissionDenied.into(),
            ))
        } else {
            let (displays, decoration) = crate::window_state::displays(&window);
            let display = displays
                .iter()
                .find(|display| !display.primary)
                .or_else(|| displays.first())
                .unwrap();
            let mut saved = WindowGeometry {
                normal: midnight_core::WindowRect {
                    x: display.work.x + 120,
                    y: display.work.y + 90,
                    width: 840,
                    height: 570,
                },
                monitor: Some(display.identity.clone()),
                maximized: matches!(load, Load::Maximized),
                fullscreen: false,
            };
            if matches!(load, Load::RemovedMonitor) {
                saved.normal.x = -29000;
                let monitor = saved.monitor.as_mut().unwrap();
                monitor.name = Some("removed startup test monitor".into());
                monitor.bounds.x = -30000;
            }
            *expected.borrow_mut() = Some(crate::window_state::policy::restore(
                &saved, &displays, decoration,
            ));
            Ok(Box::new(UserPreferences {
                theme: PreferenceTheme::Nord,
                window_geometry: Some(saved),
                panel_split_basis_points: 6100,
                ..Default::default()
            }))
        };
        loaded
            .borrow_mut()
            .apply_with_sender(Event::PreferencesLoaded(result), &|_| true);
        midnight_ui::refresh(&window, &loaded.borrow());
        ready.preferences_applied();
        let weak = window.as_weak();
        slint::Timer::single_shot(Duration::from_millis(400), move || {
            let window = weak.upgrade().unwrap();
            assert!(window.window().is_visible());
            assert!(before.get(), "no first frame was rendered");
            if matches!(load, Load::Maximized) {
                window.window().set_maximized(false);
                let weak = window.as_weak();
                slint::Timer::single_shot(Duration::from_millis(200), move || {
                    let window = weak.upgrade().unwrap();
                    window
                        .window()
                        .with_winit_window(|native| {
                            assert_normal_geometry(native, expected.borrow().as_ref().unwrap());
                        })
                        .unwrap();
                    window.hide().unwrap();
                });
            } else {
                if let Some(expected) = expected.borrow().as_ref() {
                    window
                        .window()
                        .with_winit_window(|native| {
                            assert_normal_geometry(native, expected);
                        })
                        .unwrap();
                }
                window.hide().unwrap();
            }
        });
    });
    slint::Timer::single_shot(Duration::from_secs(5), || {
        panic!("startup did not reveal or close the window");
    });
    slint::run_event_loop().unwrap();
    startup.finish().unwrap();
    state.finish(&window, &mut presenter.borrow_mut());
    if matches!(load, Load::Failed) {
        assert!(!presenter.borrow().window_geometry_ready());
        assert!(presenter.borrow().pending_preferences_save().is_none());
        while let Ok(command) = received.try_recv() {
            assert!(!matches!(command, Command::SavePreferences { .. }));
        }
    }
}

fn assert_normal_geometry(
    native: &slint::winit_030::winit::window::Window,
    expected: &WindowGeometry,
) {
    let position = native.outer_position().unwrap();
    let size = native.inner_size();
    assert_eq!(
        (position.x, position.y),
        (expected.normal.x, expected.normal.y)
    );
    assert_eq!(
        (size.width, size.height),
        (expected.normal.width, expected.normal.height)
    );
}

#[test]
#[ignore = "requires an interactive Windows desktop; run by itself with --test-threads=1"]
fn native_first_frame_waits_for_saved_theme_and_monitor_geometry() {
    first_frame(Load::Saved);
}

#[test]
#[ignore = "requires an interactive Windows desktop; run by itself with --test-threads=1"]
fn native_first_frame_uses_primary_when_saved_monitor_is_removed() {
    first_frame(Load::RemovedMonitor);
}

#[test]
#[ignore = "requires an interactive Windows desktop; run by itself with --test-threads=1"]
fn native_first_frame_restores_maximized_state_and_normal_bounds() {
    first_frame(Load::Maximized);
}

#[test]
#[ignore = "requires an interactive Windows desktop; run by itself with --test-threads=1"]
fn native_failed_preferences_show_defaults_without_overwriting_settings() {
    first_frame(Load::Failed);
}

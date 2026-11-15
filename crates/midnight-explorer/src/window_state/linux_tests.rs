//! Opt-in qualification against an X11 window manager. This owns one clearly titled
//! software-rendered window; preferences and captured saves remain entirely in memory.
use super::*;
use midnight_core::{Event, UserPreferences};
use slint::winit_030::winit::{
    platform::x11::EventLoopBuilderExtX11,
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
    window::Window,
};
use std::{cell::Cell, process::Command as ProcessCommand};

fn normal_hints(window: &Window) -> Result<String, String> {
    let identity = match window
        .window_handle()
        .map_err(|error| error.to_string())?
        .as_raw()
    {
        RawWindowHandle::Xlib(handle) => format!("0x{:x}", handle.window),
        RawWindowHandle::Xcb(handle) => format!("0x{:x}", handle.window.get()),
        _ => return Err("The position regression requires an X11 window".into()),
    };
    // The exact XID belongs to this fixture; fixed commands/arguments never use a shell.
    let output = ProcessCommand::new("/usr/bin/timeout")
        .args(["3s", "/usr/bin/xprop", "-id", &identity, "WM_NORMAL_HINTS"])
        .env("LC_ALL", "C")
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "Unable to read the fixture's WM_NORMAL_HINTS: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    String::from_utf8(output.stdout).map_err(|error| error.to_string())
}

fn without_position(hints: &str) -> Vec<&str> {
    hints
        .lines()
        .filter(|line| !line.contains("user specified location:"))
        .collect()
}

fn expect_bounds(window: &MainWindow, expected: WindowRect) -> Result<(), String> {
    let actual = capture(window, None).ok_or("Unable to capture the fixture's normal bounds")?;
    if actual.maximized || actual.fullscreen || actual.normal != expected {
        return Err(format!("Expected normal {expected:?}; captured {actual:?}"));
    }
    Ok(())
}

#[test]
#[ignore = "requires an X11 window manager and xprop; run alone with --ignored --test-threads=1"]
fn restored_x11_position_survives_first_map_and_normal_capture_has_no_drift() {
    let mut builder: slint::winit_030::EventLoopBuilder =
        slint::winit_030::winit::event_loop::EventLoop::with_user_event();
    builder.with_any_thread(true).with_x11();
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .with_winit_event_loop_builder(builder)
        .with_winit_window_attributes_hook(|attributes| attributes.with_active(false))
        .select()
        .unwrap();
    let window = MainWindow::new().unwrap();
    window.set_application_version("POSITION RESTORE REGRESSION TEST".into());
    let weak = window.as_weak();
    let outcome = Rc::new(RefCell::new(None::<Result<(), String>>));
    let result = outcome.clone();
    let phase = Rc::new(Cell::new(0));
    let observed_phase = phase.clone();
    let started = Instant::now();
    let mut ready_at = started;
    let mut expected = None::<WindowRect>;
    let mut state = State::default();
    let mut presenter = Presenter::new("position test left".into(), "position test right".into());
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(50),
        move || {
            let Some(window) = weak.upgrade() else {
                *result.borrow_mut() = Some(Err("Position fixture was lost".into()));
                let _ = slint::quit_event_loop();
                return;
            };
            let step = (|| -> Result<bool, String> {
                if started.elapsed() > Duration::from_secs(8) {
                    return Err(format!(
                        "Position regression timed out at phase {}",
                        phase.get()
                    ));
                }
                if !window.window().has_winit_window() || Instant::now() < ready_at {
                    return Ok(false);
                }
                match phase.get() {
                    0 => {
                        let before = window
                            .window()
                            .with_winit_window(normal_hints)
                            .ok_or("Native fixture is unavailable")??;
                        let (displays, _) = displays(&window);
                        let display = displays
                            .iter()
                            .find(|display| display.primary)
                            .or_else(|| displays.first())
                            .ok_or("No monitor is available for the owned fixture")?;
                        let bounds = WindowRect {
                            x: display.work.x + 180,
                            y: display.work.y + 150,
                            width: 720,
                            height: 460,
                        };
                        presenter.apply_with_sender(
                            Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
                                window_geometry: Some(WindowGeometry {
                                    normal: bounds,
                                    maximized: false,
                                    fullscreen: false,
                                    monitor: Some(display.identity.clone()),
                                }),
                                ..Default::default()
                            }))),
                            &|_| true,
                        );
                        state.restore(&window, &presenter);
                        let after = window
                            .window()
                            .with_winit_window(normal_hints)
                            .ok_or("Native fixture is unavailable")??;
                        if !after.contains(&format!(
                            "user specified location: {}, {}",
                            bounds.x, bounds.y
                        )) || without_position(&before) != without_position(&after)
                        {
                            return Err(format!(
                                "WM_NORMAL_HINTS changed incorrectly:\n{before}\n{after}"
                            ));
                        }
                        println!("Owned pre-map WM_NORMAL_HINTS:\n{after}");
                        expected = Some(bounds);
                        window.show().map_err(|error| error.to_string())?;
                        phase.set(1);
                    }
                    1 => {
                        let bounds = expected.ok_or("Missing expected initial geometry")?;
                        expect_bounds(&window, bounds)?;
                        println!("Owned first-map physical bounds: {bounds:?}");
                        for _ in 0..20 {
                            state.sample(&window, &mut presenter, &|_| true, false);
                        }
                        let moved = WindowRect {
                            x: bounds.x + 73,
                            y: bounds.y + 51,
                            ..bounds
                        };
                        if !midnight_ui::restore_window_position(
                            window.window(),
                            slint::PhysicalPosition::new(moved.x, moved.y),
                        ) {
                            return Err("X11 declined the owned fixture's physical move".into());
                        }
                        expected = Some(moved);
                        phase.set(2);
                    }
                    2 => {
                        let bounds = expected.ok_or("Missing expected moved geometry")?;
                        expect_bounds(&window, bounds)?;
                        for _ in 0..20 {
                            state.sample(&window, &mut presenter, &|_| true, false);
                        }
                        window.window().set_maximized(true);
                        phase.set(3);
                    }
                    3 => {
                        state.sample(&window, &mut presenter, &|_| true, false);
                        let geometry = state.last.as_ref().ok_or("Missing maximized sample")?;
                        if !geometry.maximized || Some(geometry.normal) != expected {
                            return Err(format!(
                                "Maximized sample lost normal geometry: {geometry:?}"
                            ));
                        }
                        window.window().set_maximized(false);
                        phase.set(4);
                    }
                    _ => {
                        let bounds = expected.ok_or("Missing expected restored geometry")?;
                        expect_bounds(&window, bounds)?;
                        let saved = RefCell::new(None);
                        state.sample(
                            &window,
                            &mut presenter,
                            &|command| {
                                if let Command::SavePreferences { preferences, .. } = command {
                                    *saved.borrow_mut() = preferences.window_geometry;
                                }
                                true
                            },
                            true,
                        );
                        if saved.borrow().as_ref().map(|geometry| geometry.normal) != Some(bounds) {
                            return Err("Final in-memory save lost restored physical bounds".into());
                        }
                        println!("Owned move/maximize/restore/final capture: {bounds:?}");
                        return Ok(true);
                    }
                }
                ready_at = Instant::now() + Duration::from_millis(400);
                Ok(false)
            })();
            if !matches!(step, Ok(false)) {
                let mut step = step.map(|_| ());
                if let Err(error) = window.hide() {
                    step = Err(format!("{step:?}; unable to hide owned fixture: {error}"));
                }
                *result.borrow_mut() = Some(step);
                let _ = slint::quit_event_loop();
            }
        },
    );
    let event_loop = slint::run_event_loop_until_quit();
    timer.stop();
    let cleanup = window.hide();
    assert!(event_loop.is_ok(), "{event_loop:?}");
    assert!(cleanup.is_ok(), "{cleanup:?}");
    let result = outcome.borrow_mut().take().unwrap_or_else(|| {
        Err(format!(
            "Event loop exited at phase {} without a result",
            observed_phase.get()
        ))
    });
    assert!(result.is_ok(), "{result:?}");
}

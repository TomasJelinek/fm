use super::*;
use crate::native_events::{MainWindowEvents, Observer};
use midnight_core::{AcceptedPreferences, Event, UserPreferences};
use slint::winit_030::winit::platform::windows::EventLoopBuilderExtWindows;
use std::cell::Cell;

/// A native move and a native resize of the main window are each saved after they
/// settle. Only the explorer's application handler wakes the sampler: the test never
/// calls `geometry_changed` itself.
#[test]
#[ignore = "requires an interactive Windows desktop; run by itself with --test-threads=1"]
fn native_moves_and_resizes_reach_the_sampler_through_the_application_handler() {
    let mut event_loop = slint::winit_030::winit::event_loop::EventLoop::<
        slint::winit_030::SlintEvent,
    >::with_user_event();
    event_loop.with_any_thread(true);
    let events = Rc::new(MainWindowEvents::default());
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .with_winit_event_loop_builder(event_loop)
        .with_winit_custom_application_handler(Observer::new(None, events.clone()))
        .select()
        .unwrap();
    let window = MainWindow::new().unwrap();
    events.bind(&window);
    let mut presenter = Presenter::new("left".into(), "right".into());
    presenter.apply_with_sender(
        Event::PreferencesLoaded(Ok(Box::new(AcceptedPreferences::default()))),
        &|_| true,
    );
    let presenter = Rc::new(RefCell::new(presenter));
    // Each settled placement is saved once; every save starts the next step.
    let saves = Rc::new(RefCell::new(Vec::<WindowGeometry>::new()));
    let step: Rc<dyn Fn()> = {
        let (weak, saves) = (window.as_weak(), saves.clone());
        Rc::new(move || {
            let window = weak.upgrade().unwrap();
            let (count, first) = {
                let saves = saves.borrow();
                (saves.len(), saves[0].normal)
            };
            match count {
                1 => window
                    .window()
                    .set_position(slint::PhysicalPosition::new(first.x + 40, first.y + 30)),
                2 => window.window().set_size(slint::PhysicalSize::new(
                    first.width - 60,
                    first.height - 40,
                )),
                _ => {
                    let _ = slint::quit_event_loop();
                }
            }
        })
    };
    let recorded = saves.clone();
    let sampler = Rc::new(WindowState::install(
        &window,
        presenter.clone(),
        Rc::new(move |command| {
            if let Command::SavePreferences { preferences, .. } = command
                && let Some(geometry) = preferences.window_geometry
            {
                recorded.borrow_mut().push(geometry);
                // Leave the sampler's tick before moving the window again.
                let step = step.clone();
                slint::Timer::single_shot(Duration::ZERO, move || step());
            }
            true
        }),
        true,
    ));
    let woken = Rc::new(Cell::new(0));
    let (geometry, counted) = (Rc::downgrade(&sampler), woken.clone());
    events.on_geometry(move || {
        counted.set(counted.get() + 1);
        if let Some(geometry) = geometry.upgrade() {
            geometry.geometry_changed();
        }
    });
    // Startup restores the (absent) saved placement before the first show; the sampler
    // then saves the first placement as the first session does.
    sampler.restore(&window, &presenter.borrow());
    window.show().unwrap();
    let timed_out = Rc::new(Cell::new(false));
    let expired = timed_out.clone();
    // Only elapses on failure: three settled saves take about two seconds.
    let deadline = slint::Timer::default();
    deadline.start(
        slint::TimerMode::SingleShot,
        Duration::from_secs(15),
        move || {
            expired.set(true);
            let _ = slint::quit_event_loop();
        },
    );
    slint::run_event_loop().unwrap();
    let _ = window.hide();
    let saves = saves.borrow();
    println!(
        "saved placements: {saves:?}; geometry events: {}",
        woken.get()
    );
    assert!(
        !timed_out.get(),
        "only {} placements were saved",
        saves.len()
    );
    let first = saves[0].normal;
    assert_eq!(
        (saves[1].normal.x, saves[1].normal.y),
        (first.x + 40, first.y + 30),
        "the moved window was saved"
    );
    assert_eq!(
        (saves[2].normal.width, saves[2].normal.height),
        (first.width - 60, first.height - 40),
        "the resized window was saved"
    );
    assert!(woken.get() >= 2, "native events woke the sampler");
}

#[test]
#[ignore = "requires an interactive Windows desktop; run by itself with --test-threads=1"]
fn native_maximized_restore_keeps_normal_bounds_and_repeated_capture_has_no_drift() {
    let mut event_loop = slint::winit_030::winit::event_loop::EventLoop::<
        slint::winit_030::SlintEvent,
    >::with_user_event();
    event_loop.with_any_thread(true);
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .with_winit_event_loop_builder(event_loop)
        .select()
        .unwrap();
    let window = MainWindow::new().unwrap();
    window.show().unwrap();
    let failures = Rc::new(RefCell::new(Vec::new()));
    let observed_failures = failures.clone();
    let weak = window.as_weak();
    // Winit realizes the native window when its event loop first runs.
    slint::Timer::single_shot(Duration::from_millis(200), move || {
        let window = weak.upgrade().unwrap();
        let monitors = displays(&window).0;
        let selected = monitors
            .iter()
            .find(|display| display.primary)
            .unwrap_or(&monitors[0]);
        let expected = WindowRect {
            x: selected.work.x + 180,
            y: selected.work.y + 150,
            width: 720,
            height: 460,
        };
        let saved = WindowGeometry {
            normal: expected,
            monitor: Some(selected.identity.clone()),
            maximized: true,
            fullscreen: false,
        };
        let mut presenter = Presenter::new("left".into(), "right".into());
        presenter.apply_with_sender(
            Event::PreferencesLoaded(Ok(Box::new(AcceptedPreferences {
                preferences: UserPreferences {
                    window_geometry: Some(saved),
                    ..Default::default()
                },
                comparators: Default::default(),
            }))),
            &|_| true,
        );
        let presenter = Rc::new(RefCell::new(presenter));
        let state = Rc::new(RefCell::new(State::default()));
        state.borrow_mut().restore(&window, &presenter.borrow());
        let weak = window.as_weak();
        slint::Timer::single_shot(Duration::from_millis(400), move || {
            let window = weak.upgrade().unwrap();
            state
                .borrow_mut()
                .sample(&window, &mut presenter.borrow_mut(), &|_| true, false);
            let captured = state.borrow().last.clone().unwrap();
            println!("maximized capture: {captured:?}");
            if !captured.maximized || captured.normal != expected {
                observed_failures
                    .borrow_mut()
                    .push("maximized capture lost saved normal bounds");
            }
            window.window().set_maximized(false);
            slint::Timer::single_shot(Duration::from_millis(350), move || {
                for _ in 0..20 {
                    state.borrow_mut().sample(
                        &window,
                        &mut presenter.borrow_mut(),
                        &|_| true,
                        false,
                    );
                }
                let captured = state.borrow().last.clone().unwrap();
                println!("restored capture: {captured:?}");
                if captured.maximized || captured.normal != expected {
                    observed_failures
                        .borrow_mut()
                        .push("normal restore/capture drifted");
                }
                let saved = RefCell::new(None);
                state.borrow_mut().sample(
                    &window,
                    &mut presenter.borrow_mut(),
                    &|command| {
                        if let Command::SavePreferences { preferences, .. } = command {
                            *saved.borrow_mut() = preferences.window_geometry;
                        }
                        true
                    },
                    true,
                );
                if saved.borrow().as_ref().map(|geometry| geometry.normal) != Some(expected) {
                    observed_failures
                        .borrow_mut()
                        .push("final save lost normal rectangle");
                }
                window.hide().unwrap();
                slint::quit_event_loop().unwrap();
            });
        });
    });
    slint::run_event_loop().unwrap();
    assert!(failures.borrow().is_empty(), "{:?}", failures.borrow());
}

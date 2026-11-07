use super::*;
use midnight_core::{Event, UserPreferences};
use slint::winit_030::winit::platform::windows::EventLoopBuilderExtWindows;

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
            Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
                window_geometry: Some(saved),
                ..Default::default()
            }))),
            &|_| true,
        );
        let presenter = Rc::new(RefCell::new(presenter));
        let state = Rc::new(RefCell::new(State::default()));
        state
            .borrow_mut()
            .sample(&window, &mut presenter.borrow_mut(), &|_| true, false);
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

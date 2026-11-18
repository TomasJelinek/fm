//! The geometry sampler runs only after a geometry event or while a save is pending.
use super::*;
use midnight_core::{Event, UserPreferences};
use slint::platform::{
    Platform, PlatformError, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use std::cell::Cell;

struct ClockWindow {
    window: Rc<MinimalSoftwareWindow>,
    clock: Rc<Cell<Duration>>,
}

impl Platform for ClockWindow {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }

    fn duration_since_start(&self) -> Duration {
        self.clock.get()
    }
}

#[test]
fn the_sampler_stops_when_quiet_and_runs_after_geometry_events_or_for_a_pending_save() {
    let clock = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(ClockWindow {
        window: MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer),
        clock: clock.clone(),
    }))
    .unwrap();
    let advance = |by: Duration| {
        clock.set(clock.get() + by);
        slint::platform::update_timers_and_animations();
    };
    let window = MainWindow::new().unwrap();
    let root = std::env::current_dir().unwrap();
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root)));
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::new(
            UserPreferences::default(),
        ))));
    let geometry = WindowState::install(&window, presenter, Rc::new(|_| true), true);
    window.show().unwrap();
    let pulse = geometry.pulse.clone().unwrap();
    assert!(pulse.running(), "the installed sampler takes one look");
    advance(POLL);
    assert!(!pulse.running(), "nothing is dirty");
    for _ in 0..100 {
        advance(POLL);
        assert!(!pulse.running(), "ten idle seconds cost no tick");
    }

    geometry.geometry_changed();
    assert!(pulse.running());
    advance(POLL);
    assert!(!pulse.running(), "a quiet sample stops again");

    // A refused save keeps the sampler retrying until it is sent.
    geometry.state.borrow_mut().retry_at = Some(Instant::now());
    geometry.geometry_changed();
    for _ in 0..5 {
        advance(POLL);
        assert!(pulse.running());
    }
    geometry.state.borrow_mut().retry_at = None;
    advance(POLL);
    assert!(!pulse.running());
    window.hide().unwrap();
}

#[test]
fn automated_runs_install_no_sampler() {
    slint::platform::set_platform(Box::new(ClockWindow {
        window: MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer),
        clock: Rc::default(),
    }))
    .unwrap();
    let window = MainWindow::new().unwrap();
    let root = std::env::current_dir().unwrap();
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root)));
    let geometry = WindowState::install(&window, presenter.clone(), Rc::new(|_| true), false);
    assert!(geometry.pulse.is_none());
    geometry.geometry_changed();
    geometry.restore(&window, &presenter.borrow());
    geometry.finish(&window, &mut presenter.borrow_mut());
}

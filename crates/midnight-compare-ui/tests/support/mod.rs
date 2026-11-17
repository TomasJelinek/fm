use slint::platform::{
    Platform, PlatformError, WindowAdapter, software_renderer::MinimalSoftwareWindow,
};
use std::{
    rc::Rc,
    time::{Duration, Instant},
};

pub struct TestPlatform(pub Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

/// Pump Slint timers until `done`, failing with the comparator's visible state. The
/// deadline elapses only on failure; the status names the worker phase that is pending.
#[allow(dead_code)]
pub fn wait_for(
    window: &midnight_compare_ui::ComparatorWindow,
    what: &str,
    mut done: impl FnMut() -> bool,
) {
    let started = Instant::now();
    while !done() {
        if started.elapsed() > Duration::from_secs(30) {
            let text = window.get_accessible_text();
            panic!(
                "Comparator did not {what} within {:?}; status {:?}, fonts ready {}, \
                 choosing files {} ({:?}), first row {:?}",
                started.elapsed(),
                window.get_status().as_str(),
                window.get_fonts_ready(),
                window.get_choosing_files(),
                window.get_choice_message().as_str(),
                text.lines().next().unwrap_or(""),
            );
        }
        std::thread::sleep(Duration::from_millis(5));
        slint::platform::update_timers_and_animations();
    }
}

//! The explorer's winit application handler. It sees every event before Slint and never
//! changes its dispatch: geometry events of the main window wake the work that samples
//! them, and the opt-in idle probe counts wakeups.
use crate::idle_probe::Probe;
use midnight_ui::MainWindow;
use slint::{
    ComponentHandle,
    winit_030::{
        CustomApplicationHandler, EventResult,
        winit::{
            event::{StartCause, WindowEvent},
            event_loop::ActiveEventLoop,
            window::{Window, WindowId},
        },
    },
};
use std::{cell::RefCell, rc::Rc};

/// Main-window hooks bound once the window exists; events before that wake nothing.
#[derive(Default)]
pub(crate) struct MainWindowEvents {
    window: RefCell<Option<slint::Weak<MainWindow>>>,
    geometry: RefCell<Option<Box<dyn Fn()>>>,
}

impl MainWindowEvents {
    pub(crate) fn bind(&self, window: &MainWindow) {
        *self.window.borrow_mut() = Some(window.as_weak());
    }

    /// Called for each native resize, move or scale change of the main window.
    pub(crate) fn on_geometry(&self, wake: impl Fn() + 'static) {
        *self.geometry.borrow_mut() = Some(Box::new(wake));
    }

    fn is_main(&self, window: &slint::Window) -> bool {
        self.window
            .borrow()
            .as_ref()
            .and_then(slint::Weak::upgrade)
            .is_some_and(|main| std::ptr::eq(main.window(), window))
    }

    fn geometry_changed(&self, window: &slint::Window) {
        if self.is_main(window)
            && let Some(wake) = self.geometry.borrow().as_ref()
        {
            wake();
        }
    }
}

pub(crate) struct Observer {
    probe: Option<Probe>,
    main: Rc<MainWindowEvents>,
}

impl Observer {
    pub(crate) fn new(probe: Option<Probe>, main: Rc<MainWindowEvents>) -> Self {
        Self { probe, main }
    }
}

impl CustomApplicationHandler for Observer {
    fn new_events(&mut self, _: &ActiveEventLoop, cause: StartCause) -> EventResult {
        if let Some(probe) = &self.probe {
            probe.wakeup(cause);
        }
        EventResult::Propagate
    }

    fn window_event(
        &mut self,
        _: &ActiveEventLoop,
        _: WindowId,
        _: Option<&Window>,
        window: Option<&slint::Window>,
        event: &WindowEvent,
    ) -> EventResult {
        if matches!(
            event,
            WindowEvent::Resized(_) | WindowEvent::Moved(_) | WindowEvent::ScaleFactorChanged { .. }
        ) && let Some(window) = window
        {
            self.main.geometry_changed(window);
        }
        EventResult::Propagate
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) -> EventResult {
        if let Some(probe) = &mut self.probe {
            probe.arm();
        }
        EventResult::Propagate
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::platform::{
        Platform, PlatformError, WindowAdapter,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    };
    use std::cell::Cell;

    struct Windows;
    impl Platform for Windows {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
            Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
        }
    }

    #[test]
    fn only_geometry_events_of_the_bound_main_window_wake_its_samplers() {
        slint::platform::set_platform(Box::new(Windows)).unwrap();
        let main = MainWindow::new().unwrap();
        let other = MainWindow::new().unwrap();
        let events = MainWindowEvents::default();
        let wakes = Rc::new(Cell::new(0));
        let counted = wakes.clone();
        events.on_geometry(move || counted.set(counted.get() + 1));
        events.geometry_changed(main.window());
        assert_eq!(wakes.get(), 0, "nothing is bound before the window exists");
        events.bind(&main);
        events.geometry_changed(other.window());
        assert_eq!(wakes.get(), 0, "a tool or viewer window is not the main window");
        events.geometry_changed(main.window());
        assert_eq!(wakes.get(), 1);
        drop(main);
        events.geometry_changed(other.window());
        assert_eq!(wakes.get(), 1);
    }
}

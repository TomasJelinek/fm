//! The explorer's winit application handler. It sees every event before Slint and never
//! changes its dispatch: geometry events and visibility changes of the main window wake
//! the work that depends on them, and the opt-in idle probe counts wakeups.
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
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

type Wake = RefCell<Option<Box<dyn Fn()>>>;

/// Main-window hooks bound once the window exists; events before that wake nothing.
#[derive(Default)]
pub(crate) struct MainWindowEvents {
    window: RefCell<Option<slint::Weak<MainWindow>>>,
    geometry: Wake,
    visibility: Wake,
    visible: Cell<Option<bool>>,
}

impl MainWindowEvents {
    pub(crate) fn bind(&self, window: &MainWindow) {
        *self.window.borrow_mut() = Some(window.as_weak());
    }

    /// Called for each native resize, move or scale change of the main window.
    pub(crate) fn on_geometry(&self, wake: impl Fn() + 'static) {
        *self.geometry.borrow_mut() = Some(Box::new(wake));
    }

    /// Called when the main window was shown or hidden by any callback, timer or event.
    pub(crate) fn on_visibility(&self, wake: impl Fn() + 'static) {
        *self.visibility.borrow_mut() = Some(Box::new(wake));
    }

    fn main(&self) -> Option<MainWindow> {
        self.window.borrow().as_ref().and_then(slint::Weak::upgrade)
    }

    fn geometry_changed(&self, window: &slint::Window) {
        if self
            .main()
            .is_some_and(|main| std::ptr::eq(main.window(), window))
            && let Some(wake) = self.geometry.borrow().as_ref()
        {
            wake();
        }
    }

    /// The loop is about to wait: compare the main window's visibility with the last turn.
    fn loop_turn_ended(&self) {
        let Some(visible) = self.main().map(|main| main.window().is_visible()) else {
            return;
        };
        if self.visible.replace(Some(visible)) != Some(visible)
            && let Some(wake) = self.visibility.borrow().as_ref()
        {
            wake();
        }
    }
}

pub(crate) struct Observer {
    probe: Option<Probe>,
    main: Rc<MainWindowEvents>,
    presentation: midnight_ui_input::NativePresentation,
}

impl Observer {
    pub(crate) fn new(probe: Option<Probe>, main: Rc<MainWindowEvents>) -> Self {
        Self {
            probe,
            main,
            presentation: Default::default(),
        }
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
        id: WindowId,
        native: Option<&Window>,
        window: Option<&slint::Window>,
        event: &WindowEvent,
    ) -> EventResult {
        self.presentation.observe(id, native, window, event);
        if matches!(
            event,
            WindowEvent::Resized(_)
                | WindowEvent::Moved(_)
                | WindowEvent::ScaleFactorChanged { .. }
        ) && let Some(window) = window
        {
            self.main.geometry_changed(window);
        }
        EventResult::Propagate
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) -> EventResult {
        self.main.loop_turn_ended();
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

    struct Windows;
    impl Platform for Windows {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
            Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
        }
    }

    fn counter() -> (Rc<Cell<usize>>, impl Fn() + 'static) {
        let count = Rc::new(Cell::new(0));
        let counted = count.clone();
        (count, move || counted.set(counted.get() + 1))
    }

    #[test]
    fn only_geometry_events_of_the_bound_main_window_wake_its_samplers() {
        slint::platform::set_platform(Box::new(Windows)).unwrap();
        let main = MainWindow::new().unwrap();
        let other = MainWindow::new().unwrap();
        let events = MainWindowEvents::default();
        let (wakes, wake) = counter();
        events.on_geometry(wake);
        events.geometry_changed(main.window());
        assert_eq!(wakes.get(), 0, "nothing is bound before the window exists");
        events.bind(&main);
        events.geometry_changed(other.window());
        assert_eq!(
            wakes.get(),
            0,
            "a tool or viewer window is not the main window"
        );
        events.geometry_changed(main.window());
        assert_eq!(wakes.get(), 1);
        drop(main);
        events.geometry_changed(other.window());
        assert_eq!(wakes.get(), 1);
    }

    #[test]
    fn a_shown_or_hidden_main_window_wakes_once_per_change() {
        slint::platform::set_platform(Box::new(Windows)).unwrap();
        let main = MainWindow::new().unwrap();
        let events = MainWindowEvents::default();
        let (wakes, wake) = counter();
        events.on_visibility(wake);
        events.bind(&main);
        events.loop_turn_ended();
        assert_eq!(wakes.get(), 1, "the first turn reports the initial state");
        events.loop_turn_ended();
        assert_eq!(wakes.get(), 1);
        main.show().unwrap();
        events.loop_turn_ended();
        events.loop_turn_ended();
        assert_eq!(wakes.get(), 2);
        // A callback hides the window without any presenter refresh.
        main.hide().unwrap();
        events.loop_turn_ended();
        assert_eq!(wakes.get(), 3);
    }
}

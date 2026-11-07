//! Native geometry is sampled on the UI; serialization and storage stay on the
//! existing bounded settings worker. Never publish defaults over an unread file.
use midnight_core::{Command, WindowGeometry, WindowMonitor, WindowRect};
use midnight_ui::{MainWindow, Presenter};
use slint::{ComponentHandle, winit_030::WinitWindowAccessor};
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
#[cfg(all(test, windows))]
mod native_tests;
mod policy;

const POLL: Duration = Duration::from_millis(100);
const IDLE: Duration = Duration::from_millis(600);
const MAX_DELAY: Duration = Duration::from_secs(2);

#[derive(Default)]
struct State {
    initialized: bool,
    last: Option<WindowGeometry>,
    dirty_since: Option<Instant>,
    changed_at: Option<Instant>,
    retry_at: Option<Instant>,
}

pub(crate) struct WindowState {
    timer: slint::Timer,
    state: Rc<RefCell<State>>,
    enabled: bool,
}

impl WindowState {
    pub(crate) fn install(
        window: &MainWindow,
        presenter: Rc<RefCell<Presenter>>,
        commands: mpsc::Sender<Command>,
        enabled: bool,
    ) -> Self {
        let state = Rc::new(RefCell::new(State::default()));
        let timer = slint::Timer::default();
        if enabled {
            let weak = window.as_weak();
            let shared = state.clone();
            timer.start(slint::TimerMode::Repeated, POLL, move || {
                let Some(window) = weak.upgrade() else { return };
                if window.window().is_visible() {
                    shared.borrow_mut().sample(
                        &window,
                        &mut presenter.borrow_mut(),
                        &|command| commands.try_send(command).is_ok(),
                        false,
                    );
                }
            });
        }
        Self {
            timer,
            state,
            enabled,
        }
    }

    pub(crate) fn finish(&self, window: &MainWindow, presenter: &mut Presenter) {
        self.timer.stop();
        if self.enabled {
            // Leave the final command for the bounded asynchronous shutdown grace.
            self.state
                .borrow_mut()
                .sample(window, presenter, &|_| false, true);
        }
    }
}

impl State {
    fn sample(
        &mut self,
        window: &MainWindow,
        presenter: &mut Presenter,
        send: &dyn Fn(Command) -> bool,
        closing: bool,
    ) {
        if !presenter.window_geometry_ready() {
            return;
        }
        if !self.initialized {
            if closing {
                return;
            }
            self.initialized = true;
            if let Some(saved) = presenter.window_geometry() {
                let (displays, decoration) = displays(window);
                let restored = policy::restore(saved, &displays, decoration);
                let bounds = restored.normal;
                window
                    .window()
                    .set_position(slint::PhysicalPosition::new(bounds.x, bounds.y));
                window
                    .window()
                    .set_size(slint::PhysicalSize::new(bounds.width, bounds.height));
                window.window().set_maximized(restored.maximized);
                window.window().set_fullscreen(restored.fullscreen);
                if &restored != saved {
                    let now = Instant::now();
                    self.changed_at = Some(now);
                    self.dirty_since = Some(now);
                }
                self.last = Some(restored);
                return;
            }
        }
        let Some(geometry) = capture(window, self.last.as_ref())
            .or_else(|| closing.then(|| self.last.clone()).flatten())
        else {
            return;
        };
        let now = Instant::now();
        if self.last.as_ref() != Some(&geometry) {
            self.last = Some(geometry.clone());
            self.dirty_since.get_or_insert(now);
            self.changed_at = Some(now);
        }
        let due = closing
            || self
                .changed_at
                .is_some_and(|at| now.duration_since(at) >= IDLE)
            || self
                .dirty_since
                .is_some_and(|at| now.duration_since(at) >= MAX_DELAY);
        if due && presenter.window_geometry() != Some(&geometry) {
            let accepted = std::cell::Cell::new(false);
            presenter.save_window_geometry(geometry, &|command| {
                let sent = send(command);
                accepted.set(sent);
                sent
            });
            self.retry_at = (!accepted.get()).then_some(now + MAX_DELAY);
            self.changed_at = None;
            self.dirty_since = None;
        }
        if !closing && self.retry_at.is_some_and(|at| now >= at) {
            self.retry_at = presenter
                .pending_preferences_save()
                .and_then(|command| (!send(command)).then_some(now + MAX_DELAY));
        }
    }
}

fn monitor(handle: slint::winit_030::winit::monitor::MonitorHandle) -> WindowMonitor {
    let position = handle.position();
    let size = handle.size();
    WindowMonitor {
        name: handle
            .name()
            .filter(|name| name.len() <= 1024 && !name.chars().any(char::is_control)),
        bounds: WindowRect {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
        },
    }
}

fn capture(window: &MainWindow, previous: Option<&WindowGeometry>) -> Option<WindowGeometry> {
    window
        .window()
        .with_winit_window(|native| {
            if native.is_minimized() == Some(true) {
                return None;
            }
            let maximized = native.is_maximized();
            let fullscreen = native.fullscreen().is_some();
            let size = native.inner_size();
            if size.width == 0 || size.height == 0 {
                return None;
            }
            // outer_position is unavailable on Wayland; its compositor owns placement.
            let position = native.outer_position().ok();
            let current = WindowRect {
                x: position.map_or(0, |point| point.x),
                y: position.map_or(0, |point| point.y),
                width: size.width,
                height: size.height,
            };
            let normal = if maximized || fullscreen {
                previous.map_or(current, |geometry| geometry.normal)
            } else {
                current
            };
            let geometry = WindowGeometry {
                normal,
                maximized,
                fullscreen,
                monitor: native.current_monitor().map(monitor),
            };
            geometry.validate().ok().map(|()| geometry)
        })
        .flatten()
}

fn displays(window: &MainWindow) -> (Vec<policy::Display>, (u32, u32)) {
    window
        .window()
        .with_winit_window(|native| {
            let primary = native.primary_monitor();
            let client = native.inner_size();
            let outer = native.outer_size();
            let decoration = (
                outer.width.saturating_sub(client.width),
                outer.height.saturating_sub(client.height),
            );
            let displays = native
                .available_monitors()
                .take(32)
                .map(|handle| {
                    let is_primary = primary.as_ref() == Some(&handle);
                    let identity = monitor(handle);
                    let point = identity.bounds;
                    let work = midnight_platform::monitor_work_area(
                        point.x.saturating_add(1),
                        point.y.saturating_add(1),
                    )
                    .unwrap_or(point);
                    policy::Display {
                        identity,
                        work,
                        primary: is_primary,
                    }
                })
                .collect();
            (displays, decoration)
        })
        .unwrap_or_default()
}

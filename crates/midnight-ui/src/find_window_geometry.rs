//! Find placement uses physical bounds; capture and restoration stay on the UI thread.
use super::{FindWindow, Presenter};
use crate::dialog_host::DialogHost;
use midnight_core::{Command, WindowGeometry, WindowMonitor, WindowRect};
use slint::{
    ComponentHandle,
    winit_030::{EventResult, WinitWindowAccessor, winit},
};
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    time::Duration,
};

pub(super) struct Placement {
    last: RefCell<Option<WindowGeometry>>,
    pending: Rc<RefCell<Option<WindowGeometry>>>,
    ready: Cell<bool>,
    state: Weak<RefCell<Presenter>>,
    restore_timer: slint::Timer,
    timer: slint::Timer,
}

#[derive(Debug, PartialEq, Eq)]
enum Restoration {
    Stop,
    WaitNative,
    Apply,
}

fn restoration(
    restored: bool,
    visible: bool,
    preferences_ready: bool,
    native_hook: bool,
    host_realized: bool,
) -> Restoration {
    if restored || !visible || !preferences_ready {
        Restoration::Stop
    } else if native_hook && !host_realized {
        Restoration::WaitNative
    } else {
        Restoration::Apply
    }
}

impl Placement {
    pub(super) fn install(
        window: &FindWindow,
        state: &Rc<RefCell<Presenter>>,
        send: &Rc<dyn Fn(Command) -> bool>,
    ) -> Rc<Self> {
        let placement = Rc::new(Self {
            last: RefCell::new(None),
            pending: state.borrow().finds.pending_window_geometry.clone(),
            ready: Cell::new(false),
            state: Rc::downgrade(state),
            restore_timer: slint::Timer::default(),
            timer: slint::Timer::default(),
        });
        let track = placement.clone();
        let state = Rc::downgrade(state);
        let send = send.clone();
        window.window().on_winit_window_event(move |window, event| {
            if track.ready.get()
                && matches!(
                    event,
                    winit::event::WindowEvent::Moved(_)
                        | winit::event::WindowEvent::Resized(_)
                        | winit::event::WindowEvent::ScaleFactorChanged { .. }
                )
            {
                track.capture(window);
                let state = state.clone();
                let send = send.clone();
                let weak = Rc::downgrade(&track);
                // Only geometry changes wake this timer; a stationary dialog has no poll.
                track.timer.start(
                    slint::TimerMode::Repeated,
                    Duration::from_millis(300),
                    move || {
                        let Some(track) = weak.upgrade() else { return };
                        let Some(state) = state.upgrade() else {
                            track.timer.stop();
                            return;
                        };
                        if let Ok(mut p) = state.try_borrow_mut() {
                            p.flush_find_window_preferences(send.as_ref());
                            track.timer.stop();
                        }
                    },
                );
            }
            EventResult::Propagate
        });
        placement
    }

    pub(super) fn ensure_restored(
        self: &Rc<Self>,
        window: &FindWindow,
        host: &Rc<DialogHost>,
        state: &Rc<RefCell<Presenter>>,
    ) {
        if self.ready.get() || !window.window().is_visible() {
            self.restore_timer.stop();
            return;
        }
        let Ok(p) = state.try_borrow() else {
            return;
        };
        match restoration(
            self.ready.get(),
            window.window().is_visible(),
            p.window_geometry_ready(),
            crate::dialog_native::uses_native_hook(),
            host.is_realized(),
        ) {
            Restoration::Stop => self.restore_timer.stop(),
            Restoration::WaitNative => {
                drop(p);
                if self.restore_timer.running() {
                    return;
                }
                let placement = Rc::downgrade(self);
                let window = window.as_weak();
                let host = Rc::downgrade(host);
                let state = Rc::downgrade(state);
                // Only a shown, settings-ready window awaiting native placement polls.
                self.restore_timer.start(
                    slint::TimerMode::Repeated,
                    Duration::from_millis(16),
                    move || {
                        let Some(placement) = placement.upgrade() else {
                            return;
                        };
                        let (Some(window), Some(host), Some(state)) =
                            (window.upgrade(), host.upgrade(), state.upgrade())
                        else {
                            placement.restore_timer.stop();
                            return;
                        };
                        placement.ensure_restored(&window, &host, &state);
                    },
                );
            }
            Restoration::Apply => {
                self.restore_timer.stop();
                *self.last.borrow_mut() = p.preferences.values.find.window_geometry.clone();
                drop(p);
                self.restore(window.window());
            }
        }
    }

    fn restore(&self, window: &slint::Window) {
        if let Some(saved) = self.last.borrow().as_ref() {
            let bounds = window
                .with_winit_window(|native| {
                    let monitor = native
                        .available_monitors()
                        .take(32)
                        .find(|monitor| {
                            let bounds = monitor_bounds(monitor);
                            saved
                                .monitor
                                .as_ref()
                                .is_some_and(|old| old.bounds == bounds)
                                || contains(bounds, saved.normal.x, saved.normal.y)
                        })
                        .or_else(|| native.current_monitor())
                        .or_else(|| native.primary_monitor());
                    let Some(monitor) = monitor else {
                        return saved.normal;
                    };
                    let client = native.inner_size();
                    let outer = native.outer_size();
                    let area = if native.current_monitor().as_ref() == Some(&monitor) {
                        crate::dialog_native::work_area(window)
                            .unwrap_or_else(|| monitor_bounds(&monitor))
                    } else {
                        monitor_bounds(&monitor)
                    };
                    fit(
                        saved.normal,
                        area,
                        (
                            outer.width.saturating_sub(client.width),
                            outer.height.saturating_sub(client.height),
                        ),
                    )
                })
                .unwrap_or(saved.normal);
            window.set_size(slint::PhysicalSize::new(bounds.width, bounds.height));
            crate::restore_window_position(
                window,
                slint::PhysicalPosition::new(bounds.x, bounds.y),
            );
            window.set_maximized(saved.maximized);
            window.set_fullscreen(saved.fullscreen);
        }
        self.ready.set(true);
        // Initial native placement happened before capture was enabled. Retain
        // normal bounds now so the first maximize/fullscreen event can use them.
        let needs_initial_bounds = self.last.borrow().is_none();
        if needs_initial_bounds {
            self.capture(window);
        }
    }

    pub(super) fn capture(&self, window: &slint::Window) {
        if !self.ready.get()
            || !self
                .state
                .upgrade()
                .is_some_and(|state| state.try_borrow().is_ok_and(|p| p.window_geometry_ready()))
        {
            return;
        }
        let previous = self.last.borrow().clone();
        let sampled = window
            .with_winit_window(|native| {
                if native.is_minimized() == Some(true) {
                    return None;
                }
                let maximized = native.is_maximized();
                let fullscreen = native.fullscreen().is_some();
                let position = native.outer_position().ok();
                let size = native.inner_size();
                let normal = WindowRect {
                    x: position
                        .as_ref()
                        .map_or_else(|| previous.as_ref().map_or(0, |p| p.normal.x), |p| p.x),
                    y: position
                        .as_ref()
                        .map_or_else(|| previous.as_ref().map_or(0, |p| p.normal.y), |p| p.y),
                    width: size.width,
                    height: size.height,
                };
                let monitor = if position.is_some() {
                    native.current_monitor().map(|m| WindowMonitor {
                        name: m.name().filter(|name| {
                            name.len() <= 1024 && !name.chars().any(char::is_control)
                        }),
                        bounds: monitor_bounds(&m),
                    })
                } else {
                    previous.as_ref().and_then(|p| p.monitor.clone())
                };
                sampled_geometry(previous.as_ref(), normal, monitor, maximized, fullscreen)
            })
            .flatten();
        if let Some(geometry) = sampled {
            *self.last.borrow_mut() = Some(geometry);
        }
        if let Some(geometry) = self.last.borrow().as_ref() {
            *self.pending.borrow_mut() = Some(geometry.clone());
        }
    }
}

fn sampled_geometry(
    previous: Option<&WindowGeometry>,
    normal: WindowRect,
    monitor: Option<WindowMonitor>,
    maximized: bool,
    fullscreen: bool,
) -> Option<WindowGeometry> {
    let geometry = WindowGeometry {
        normal: if maximized || fullscreen {
            previous?.normal
        } else {
            normal
        },
        monitor,
        maximized,
        fullscreen,
    };
    geometry.validate().ok().map(|()| geometry)
}

fn monitor_bounds(monitor: &winit::monitor::MonitorHandle) -> WindowRect {
    let position = monitor.position();
    let size = monitor.size();
    WindowRect {
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
    }
}

fn contains(bounds: WindowRect, x: i32, y: i32) -> bool {
    i64::from(x) >= i64::from(bounds.x)
        && i64::from(y) >= i64::from(bounds.y)
        && i64::from(x) < i64::from(bounds.x) + i64::from(bounds.width)
        && i64::from(y) < i64::from(bounds.y) + i64::from(bounds.height)
}

fn fit(saved: WindowRect, area: WindowRect, decoration: (u32, u32)) -> WindowRect {
    let width = saved
        .width
        .min(area.width.saturating_sub(decoration.0).max(1));
    let height = saved
        .height
        .min(area.height.saturating_sub(decoration.1).max(1));
    WindowRect {
        x: i64::from(saved.x).clamp(
            i64::from(area.x),
            i64::from(area.x)
                + i64::from(
                    area.width
                        .saturating_sub(width.saturating_add(decoration.0)),
                ),
        ) as i32,
        y: i64::from(saved.y).clamp(
            i64::from(area.y),
            i64::from(area.y)
                + i64::from(
                    area.height
                        .saturating_sub(height.saturating_add(decoration.1)),
                ),
        ) as i32,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_normal_capture_survives_first_maximize_or_fullscreen_and_close() {
        let normal = WindowRect {
            x: 100,
            y: 80,
            width: 900,
            height: 640,
        };
        let expanded = WindowRect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        for (maximized, fullscreen) in [(true, false), (false, true)] {
            let initial = sampled_geometry(None, normal, None, false, false).unwrap();
            let changed =
                sampled_geometry(Some(&initial), expanded, None, maximized, fullscreen).unwrap();
            let closed =
                sampled_geometry(Some(&changed), expanded, None, maximized, fullscreen).unwrap();
            assert_eq!(closed.normal, normal);
            assert_eq!(
                (closed.maximized, closed.fullscreen),
                (maximized, fullscreen)
            );
        }
    }

    #[test]
    fn restoration_waits_for_preferences_and_native_placement_without_idle_polling() {
        assert_eq!(
            restoration(false, true, false, true, false),
            Restoration::Stop
        );
        assert_eq!(
            restoration(false, true, false, false, true),
            Restoration::Stop
        );
        assert_eq!(
            restoration(false, false, true, true, false),
            Restoration::Stop
        );
        assert_eq!(
            restoration(true, true, true, true, false),
            Restoration::Stop
        );
        assert_eq!(
            restoration(false, true, true, true, false),
            Restoration::WaitNative
        );
        assert_eq!(
            restoration(false, true, true, true, true),
            Restoration::Apply
        );
        assert_eq!(
            restoration(false, true, true, false, false),
            Restoration::Apply
        );
    }

    #[test]
    fn removed_monitor_placement_fits_the_current_desktop_and_frame() {
        let saved = WindowRect {
            x: -2600,
            y: 3000,
            width: 1800,
            height: 1400,
        };
        let area = WindowRect {
            x: 100,
            y: 50,
            width: 1200,
            height: 800,
        };
        assert_eq!(
            fit(saved, area, (20, 40)),
            WindowRect {
                x: 100,
                y: 50,
                width: 1180,
                height: 760,
            }
        );
        let saved = WindowRect {
            x: 300,
            y: 100,
            width: 700,
            height: 500,
        };
        assert_eq!(fit(saved, area, (20, 40)), saved);
    }
}

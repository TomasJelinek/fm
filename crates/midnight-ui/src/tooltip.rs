//! Slint 1.18.1 host coordinates for embedded, input-transparent tooltip popups.
//!
//! Winit uses the default create_child_window_adapter (None), so every app popup
//! shares its host coordinates. Recheck this contract when upgrading Slint or
//! adding a backend with native popup windows. This observer never consumes input.
use crate::{ChecksumWindow, FindWindow, MainWindow, TooltipPlacement};
use i_slint_core::{
    items::TooltipArea,
    lengths::LogicalLength,
    window::{WindowInner, WindowKind},
};
use slint::{
    ComponentHandle, PlatformError,
    platform::{PointerEventButton, WindowEvent},
};
use std::cell::RefCell;

#[cfg(test)]
#[path = "tooltip_tests.rs"]
mod tests;

#[derive(Default)]
struct Registry {
    installed: bool,
    targets: Vec<Target>,
}
thread_local! { static REGISTRY: RefCell<Registry> = RefCell::default(); }

#[derive(Clone)]
enum Target {
    Main(slint::Weak<MainWindow>),
    Find(slint::Weak<FindWindow>),
    Checksum(slint::Weak<ChecksumWindow>),
}
enum Host {
    Main(MainWindow),
    Find(FindWindow),
    Checksum(ChecksumWindow),
}
impl Target {
    fn upgrade(&self) -> Option<Host> {
        match self {
            Self::Main(window) => window.upgrade().map(Host::Main),
            Self::Find(window) => window.upgrade().map(Host::Find),
            Self::Checksum(window) => window.upgrade().map(Host::Checksum),
        }
    }
}
impl Host {
    fn window(&self) -> &slint::Window {
        match self {
            Self::Main(window) => window.window(),
            Self::Find(window) => window.window(),
            Self::Checksum(window) => window.window(),
        }
    }
    fn placement(&self) -> TooltipPlacement<'_> {
        match self {
            Self::Main(window) => window.global::<TooltipPlacement>(),
            Self::Find(window) => window.global::<TooltipPlacement>(),
            Self::Checksum(window) => window.global::<TooltipPlacement>(),
        }
    }
    fn synchronize(&self) -> bool {
        synchronize_host(self.window(), self.placement())
    }
}

pub(crate) fn install(window: &MainWindow) -> Result<(), PlatformError> {
    register(Target::Main(window.as_weak()))
}
pub(crate) fn install_find(window: &FindWindow) -> Result<(), PlatformError> {
    register(Target::Find(window.as_weak()))
}
pub(crate) fn install_checksum(window: &ChecksumWindow) -> Result<(), PlatformError> {
    register(Target::Checksum(window.as_weak()))
}

pub(crate) fn synchronize_checksum(window: &ChecksumWindow) {
    synchronize_host(window.window(), window.global::<TooltipPlacement>());
}
fn register(target: Target) -> Result<(), PlatformError> {
    let Some(host) = target.upgrade() else {
        return Ok(());
    };
    let callback = target.clone();
    host.placement()
        .on_synchronize(move || callback.upgrade().is_some_and(|host| host.synchronize()));
    let first = REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        registry.targets.retain(|target| target.upgrade().is_some());
        if !registry
            .targets
            .iter()
            .filter_map(Target::upgrade)
            .any(|target| std::ptr::eq(target.window(), host.window()))
        {
            registry.targets.push(target);
        }
        !registry.installed
    });
    if !first {
        return Ok(());
    }
    let previous = i_slint_core::context::set_window_event_hook(None)?;
    i_slint_core::context::set_window_event_hook(Some(Box::new(move |adapter, event, result| {
        // Drop the registry borrow before evaluating any Slint properties.
        let targets = REGISTRY.with(|registry| {
            let mut registry = registry.borrow_mut();
            let mut targets = Vec::with_capacity(registry.targets.len());
            registry.targets.retain(|target| {
                if let Some(target) = target.upgrade() {
                    targets.push(target);
                    true
                } else {
                    false
                }
            });
            targets
        });
        for host in targets {
            if !std::ptr::eq(adapter.window(), host.window()) {
                continue;
            }
            // Flickable may defer child pointer-down by 100ms. Preserve the footer's
            // actual press action/pane before modifiers or focus can change.
            if let Host::Main(window) = &host {
                match event {
                    WindowEvent::PointerPressed {
                        position,
                        button: PointerEventButton::Left,
                    } => {
                        window.invoke_capture_function_press(position.x, position.y);
                    }
                    WindowEvent::PointerReleased {
                        button: PointerEventButton::Left,
                        ..
                    }
                    | WindowEvent::PointerExited
                    | WindowEvent::WindowActiveChanged(false) => {
                        window.invoke_release_function_press();
                    }
                    _ => {}
                }
            }
            let global = host.placement();
            let size = match event {
                WindowEvent::Resized { size } => *size,
                _ => adapter.size().to_logical(host.window().scale_factor()),
            };
            if size.width > 0.0 && size.height > 0.0 {
                global.set_surface_width(size.width);
                global.set_surface_height(size.height);
            }
            match event {
                WindowEvent::PointerMoved { position }
                | WindowEvent::PointerPressed { position, .. }
                | WindowEvent::PointerReleased { position, .. }
                | WindowEvent::PointerScrolled { position, .. } => {
                    global.set_cursor_x(position.x);
                    global.set_cursor_y(position.y);
                }
                WindowEvent::PointerExited => {
                    global.set_cursor_x(-1.0);
                    global.set_cursor_y(-1.0);
                }
                _ => {}
            }
            host.synchronize();
            if let Host::Main(window) = &host {
                crate::panel_pointer::observe(window, event);
                crate::drag_bridge::observe(window, event);
            }
        }
        if let Some(previous) = previous.as_ref() {
            previous(adapter, event, result);
        }
    })))?;
    REGISTRY.with(|registry| registry.borrow_mut().installed = true);
    Ok(())
}

/// Refresh only active anchors after input or a presentation layout update.
/// The show callback also calls this after its first stock placement; a true
/// result requests one stock retry before the first paint, preserving ownership.
pub(crate) fn synchronize(window: &MainWindow) -> bool {
    synchronize_host(window.window(), window.global::<TooltipPlacement>())
}
pub(crate) fn synchronize_find(window: &FindWindow) -> bool {
    synchronize_host(window.window(), window.global::<TooltipPlacement>())
}
fn synchronize_host(window: &slint::Window, global: TooltipPlacement<'_>) -> bool {
    let cursor = (global.get_cursor_x(), global.get_cursor_y());
    if cursor.0 < 0.0 || cursor.1 < 0.0 {
        return false;
    }
    // A property write can update popup geometry; release popup storage first.
    let parents = {
        let inner = WindowInner::from_pub(window);
        inner
            .active_popups()
            .iter()
            .filter(|popup| matches!(popup.window_kind, WindowKind::ToolTip))
            .filter_map(|popup| popup.parent_item.upgrade())
            .collect::<Vec<_>>()
    };
    let mut changed = false;
    for parent in parents {
        let Some(area) = parent.downcast::<TooltipArea>() else {
            continue;
        };
        let origin = parent.map_to_native_window(parent.geometry().origin);
        // Ignore sub-pixel rounding when deciding whether initial placement needs retry.
        changed |= (area.as_pin_ref().mouse_x().get() - (cursor.0 - origin.x)).abs() > 0.01
            || (area.as_pin_ref().mouse_y().get() - (cursor.1 - origin.y)).abs() > 0.01;
        area.as_pin_ref()
            .mouse_x
            .set(LogicalLength::new(cursor.0 - origin.x));
        area.as_pin_ref()
            .mouse_y
            .set(LogicalLength::new(cursor.1 - origin.y));
    }
    changed
}

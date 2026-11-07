//! Slint 1.18.1 host coordinates for embedded, input-transparent tooltip popups.
//!
//! Winit uses the default create_child_window_adapter (None), so every app popup
//! shares its host coordinates. Recheck this contract when upgrading Slint or
//! adding a backend with native popup windows. This observer never consumes input.
use crate::{MainWindow, TooltipPlacement};
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

#[derive(Default)]
struct Registry {
    installed: bool,
    targets: Vec<slint::Weak<MainWindow>>,
}
thread_local! { static REGISTRY: RefCell<Registry> = RefCell::default(); }

pub(crate) fn install(window: &MainWindow) -> Result<(), PlatformError> {
    let weak = window.as_weak();
    window
        .global::<TooltipPlacement>()
        .on_synchronize(move || weak.upgrade().is_some_and(|window| synchronize(&window)));
    let first = REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        registry.targets.retain(|target| target.upgrade().is_some());
        if !registry
            .targets
            .iter()
            .filter_map(slint::Weak::upgrade)
            .any(|target| std::ptr::eq(target.window(), window.window()))
        {
            registry.targets.push(window.as_weak());
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
        for window in targets {
            if !std::ptr::eq(adapter.window(), window.window()) {
                continue;
            }
            // Flickable may defer child pointer-down by 100ms. Preserve the footer's
            // actual press action/pane before modifiers or focus can change.
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
            let global = window.global::<TooltipPlacement>();
            let size = match event {
                WindowEvent::Resized { size } => *size,
                _ => adapter.size().to_logical(window.window().scale_factor()),
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
            synchronize(&window);
            crate::panel_pointer::observe(&window, event);
            crate::drag_bridge::observe(&window, event);
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
    let global = window.global::<TooltipPlacement>();
    let cursor = (global.get_cursor_x(), global.get_cursor_y());
    if cursor.0 < 0.0 || cursor.1 < 0.0 {
        return false;
    }
    // A property write can update popup geometry; release popup storage first.
    let parents = {
        let inner = WindowInner::from_pub(window.window());
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

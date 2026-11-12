//! Native dialog operations. Native windows are borrowed only for one UI operation.
use crate::dialog_host::DialogOwner;
use i_slint_core::window::WindowInner;
use slint::winit_030::{WinitWindowAccessor, winit::window::WindowAttributes};
use std::cell::{Cell, RefCell};

#[cfg(windows)]
#[path = "dialog_native_windows.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "dialog_native_linux.rs"]
mod platform;
#[cfg(test)]
#[path = "dialog_native_tests.rs"]
mod tests;

thread_local! {
    static CREATION_OWNER: RefCell<Option<(DialogOwner, WindowRole)>> = const { RefCell::new(None) };
    static NATIVE_HOOK_USED: Cell<bool> = const { Cell::new(false) };
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum WindowRole {
    Dialog,
    WorkingTool,
}

pub(crate) struct CreationScope(Option<(DialogOwner, WindowRole)>);
impl CreationScope {
    pub(crate) fn enter(owner: DialogOwner, role: WindowRole) -> Self {
        Self(CREATION_OWNER.with(|slot| slot.replace(Some((owner, role)))))
    }
}
impl Drop for CreationScope {
    fn drop(&mut self) {
        CREATION_OWNER.with(|slot| slot.replace(self.0.take()));
    }
}

pub(crate) fn uses_native_hook() -> bool {
    NATIVE_HOOK_USED.with(Cell::get)
}

/// Compose with the application's existing Winit hook. Non-dialog attributes pass through.
pub fn configure_dialog_window_attributes(attributes: WindowAttributes) -> WindowAttributes {
    NATIVE_HOOK_USED.with(|used| used.set(true));
    let owner = CREATION_OWNER.with(|slot| slot.borrow().clone());
    let Some((owner, role)) = owner else {
        return attributes;
    };
    owner
        .with_window(|owner| {
            owner.with_winit_window(|native| {
                platform::configure(attributes.clone(), native, role)
                    .with_active(native.has_focus())
            })
        })
        .flatten()
        .unwrap_or(attributes)
}

pub(crate) use platform::set_enabled;

pub(crate) fn attach(owner: &slint::Window, child: &slint::Window, role: WindowRole) {
    owner.with_winit_window(|owner| {
        child.with_winit_window(|child| platform::attach(owner, child, role));
    });
}

pub(crate) fn has_focus(window: &slint::Window) -> bool {
    window
        .with_winit_window(|native| native.has_focus())
        .unwrap_or_else(|| WindowInner::from_pub(window).active())
}

pub(crate) fn activate(window: &slint::Window) -> bool {
    window
        .with_winit_window(platform::activate)
        .unwrap_or(false)
}

pub(crate) fn place_initial(owner: &slint::Window, child: &slint::Window) {
    owner.with_winit_window(|native| {
        let Some((area_x, area_y, area_width, area_height)) = platform::work_area(native) else {
            // Wayland placement belongs to the compositor.
            return;
        };
        let position = native.outer_position().unwrap_or_default();
        let owner_size = native.outer_size();
        let requested = child.size();
        let scale = owner.scale_factor();
        let preferred = if requested.width > 0 && requested.height > 0 {
            requested
        } else {
            let inner = WindowInner::from_pub(child);
            let component_rc = inner.component();
            let component = i_slint_core::item_tree::ItemTreeRc::borrow_pin(&component_rc);
            let horizontal = component
                .as_ref()
                .layout_info(i_slint_core::layout::Orientation::Horizontal);
            let vertical = component
                .as_ref()
                .layout_info(i_slint_core::layout::Orientation::Vertical);
            slint::LogicalSize::new(horizontal.preferred_bounded(), vertical.preferred_bounded())
                .to_physical(scale)
        };
        let (position, size) = centered_bounds(
            (position.x, position.y, owner_size.width, owner_size.height),
            (preferred.width, preferred.height),
            (area_x, area_y, area_width, area_height),
        );
        child.set_size(slint::PhysicalSize::new(size.0, size.1));
        child.set_position(slint::PhysicalPosition::new(position.0, position.1));
    });
}

/// Complete first placement using the native frame's actual extents, once realized.
pub(crate) fn place_realized(owner: &slint::Window, child: &slint::Window) {
    owner.with_winit_window(|native_owner| {
        let Some(area) = platform::work_area(native_owner) else {
            return;
        };
        let owner_position = native_owner.outer_position().unwrap_or_default();
        let owner_size = native_owner.outer_size();
        child.with_winit_window(|native_child| {
            let inner = native_child.inner_size();
            let outer = native_child.outer_size();
            let (position, size) = decorated_bounds(
                (
                    owner_position.x,
                    owner_position.y,
                    owner_size.width,
                    owner_size.height,
                ),
                (inner.width, inner.height),
                (
                    outer.width.saturating_sub(inner.width),
                    outer.height.saturating_sub(inner.height),
                ),
                area,
            );
            child.set_size(slint::PhysicalSize::new(size.0, size.1));
            child.set_position(slint::PhysicalPosition::new(position.0, position.1));
        });
    });
}

fn decorated_bounds(
    owner: (i32, i32, u32, u32),
    preferred_client: (u32, u32),
    frame: (u32, u32),
    area: (i32, i32, u32, u32),
) -> ((i32, i32), (u32, u32)) {
    let (position, outer) = centered_bounds(
        owner,
        (
            preferred_client.0.saturating_add(frame.0),
            preferred_client.1.saturating_add(frame.1),
        ),
        area,
    );
    (
        position,
        (
            outer.0.saturating_sub(frame.0).max(1),
            outer.1.saturating_sub(frame.1).max(1),
        ),
    )
}

fn centered_bounds(
    owner: (i32, i32, u32, u32),
    preferred: (u32, u32),
    area: (i32, i32, u32, u32),
) -> ((i32, i32), (u32, u32)) {
    let width = preferred.0.min(area.2).max(1);
    let height = preferred.1.min(area.3).max(1);
    let center = |origin: i32, extent: u32, size: u32, edge: i32, available: u32| {
        let wanted = i64::from(origin) + (i64::from(extent) - i64::from(size)) / 2;
        let minimum = i64::from(edge);
        let maximum = minimum + i64::from(available.saturating_sub(size));
        wanted
            .clamp(minimum, maximum)
            .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
    };
    (
        (
            center(owner.0, owner.2, width, area.0, area.2),
            center(owner.1, owner.3, height, area.1, area.3),
        ),
        (width, height),
    )
}

use slint::{BackendSelector, winit_030::winit::platform::windows::WindowAttributesExtWindows};

pub(super) fn initialize_identity() -> Result<(), slint::PlatformError> {
    // The executable resource and Slint window icon supply Windows branding.
    Ok(())
}

pub(super) fn configure(selector: BackendSelector, default_renderer: bool) -> BackendSelector {
    // Our backend STA owns OLE admission and acknowledges drops only after
    // reserving copy capacity. Winit's eager UI-thread target must not register.
    let selector = selector
        .with_winit_window_attributes_hook(|attributes| attributes.with_drag_and_drop(false));
    if default_renderer && std::env::var_os("WGPU_BACKEND").is_none() {
        // Use the native Windows API. Vulkan device creation fails on the QA
        // driver while this same executable renders correctly through Direct3D.
        // An explicit backend override remains available for driver diagnostics.
        selector.require_d3d()
    } else {
        selector
    }
}

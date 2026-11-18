//! Renderer policy shared by startup paths; native window options stay per OS.

#[cfg(target_os = "linux")]
#[path = "window_backend_linux.rs"]
mod native;
#[cfg(windows)]
#[path = "window_backend_windows.rs"]
mod native;

/// `observer` sees winit events before Slint dispatches them; it changes no dispatch.
pub fn select(software: bool, observer: crate::native_events::Observer) -> Result<(), String> {
    let explicit_renderer = std::env::var_os("SLINT_BACKEND").is_some();
    let mut selector =
        slint::BackendSelector::new().with_winit_custom_application_handler(observer);
    if software {
        selector = selector
            .backend_name("winit".into())
            .renderer_name("software".into());
    } else if !explicit_renderer {
        selector = selector
            .backend_name("winit".into())
            .renderer_name("femtovg-wgpu".into());
    }
    native::configure(selector, !software && !explicit_renderer)
        .select()
        .map_err(|error| error.to_string())?;
    native::initialize_identity().map_err(|error| error.to_string())
}

//! Renderer policy shared by startup paths; native window options stay per OS.

#[cfg(target_os = "linux")]
#[path = "window_backend_linux.rs"]
mod native;
#[cfg(windows)]
#[path = "window_backend_windows.rs"]
mod native;

/// `probe` observes event-loop wakeups for the opt-in idle measurement only.
pub fn select(software: bool, probe: Option<crate::idle_probe::Probe>) -> Result<(), String> {
    let explicit_renderer = std::env::var_os("SLINT_BACKEND").is_some();
    let mut selector = slint::BackendSelector::new();
    if let Some(probe) = probe {
        selector = selector.with_winit_custom_application_handler(probe);
    }
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

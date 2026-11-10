use slint::BackendSelector;

pub(super) fn initialize_identity() -> Result<(), slint::PlatformError> {
    // Wayland resolves the launcher icon through this matching desktop entry.
    slint::set_xdg_app_id("io.github.TomasJelinek.MidnightExplorer")
}

pub(super) fn configure(selector: BackendSelector, _default_renderer: bool) -> BackendSelector {
    // Winit selects X11/Wayland; the worker-owned XDND proxy is attached after
    // the actual X11 window exists. Keep the caller's renderer configuration.
    selector.with_winit_window_attributes_hook(midnight_ui::configure_dialog_window_attributes)
}

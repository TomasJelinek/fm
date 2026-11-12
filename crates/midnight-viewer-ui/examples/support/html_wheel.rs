//! Reuse the probe's native, own-window wheel injection.
#[cfg(target_os = "windows")]
#[allow(dead_code)]
#[path = "../performance_probe/wheel_windows.rs"]
mod native;
pub(super) fn send(window: &midnight_viewer_ui::ViewerWindow) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        native::send(window, -120, &crate::timeline::Trace)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = window;
        Err("native HTML wheel injection is Windows-only".into())
    }
}

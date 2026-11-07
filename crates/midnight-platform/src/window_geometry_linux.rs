/// Winit exposes monitor bounds on Linux; desktop work areas are not portable
/// across X11 window managers and Wayland compositors. Use those bounds instead.
pub fn monitor_work_area(_x: i32, _y: i32) -> Option<midnight_core::WindowRect> {
    None
}

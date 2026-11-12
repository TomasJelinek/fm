//! Small UI-thread display query; no files, processes or network access.
use midnight_core::WindowRect;
use windows_sys::Win32::{
    Foundation::POINT,
    Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromPoint},
};

/// Work area of the display containing a physical desktop point.
pub fn monitor_work_area(x: i32, y: i32) -> Option<WindowRect> {
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: both APIs borrow initialized value data; the returned display
    // handle is borrowed and GetMonitorInfoW writes exactly the sized struct.
    let available = unsafe {
        let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONULL);
        !monitor.is_null() && GetMonitorInfoW(monitor, &mut info) != 0
    };
    if !available {
        return None;
    }
    let area = info.rcWork;
    Some(WindowRect {
        x: area.left,
        y: area.top,
        width: u32::try_from(area.right.checked_sub(area.left)?).ok()?,
        height: u32::try_from(area.bottom.checked_sub(area.top)?).ok()?,
    })
}

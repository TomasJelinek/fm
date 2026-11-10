//! GetAsyncKeyState reports physical buttons, unlike logical mouse messages.
use super::{DragHost, FsError, error, validate_host};

#[link(name = "user32")]
unsafe extern "system" {
    fn AttachThreadInput(first: u32, second: u32, attach: i32) -> i32;
}

/// OLE's modal source loop must share the initiating window's input queue.
/// COM, payload admission and filesystem work remain on the backend STA.
pub(super) struct InputQueue(Option<(u32, u32)>);
impl InputQueue {
    pub(super) fn attach(host: &DragHost) -> Result<Self, FsError> {
        let hwnd = validate_host(host)?;
        // SAFETY: host validation restricts the live HWND to this process. Both
        // threads have message queues before a native session can start.
        let owner = unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(
                hwnd,
                std::ptr::null_mut(),
            )
        };
        let actor = unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() };
        if owner == actor {
            return Ok(Self(None));
        }
        if owner == 0 || unsafe { AttachThreadInput(actor, owner, 1) } == 0 {
            return Err(error(std::io::Error::last_os_error()));
        }
        Ok(Self(Some((actor, owner))))
    }
}
impl Drop for InputQueue {
    fn drop(&mut self) {
        if let Some((actor, owner)) = self.0.take() {
            // SAFETY: detach only the pair successfully attached by this guard,
            // on the creating actor. A closed owner thread is already detached.
            unsafe {
                AttachThreadInput(actor, owner, 0);
            }
        }
    }
}

pub(super) fn primary_pressed(swapped: bool, physical_pressed: impl Fn(i32) -> bool) -> bool {
    // VK_LBUTTON=1 / VK_RBUTTON=2 are physical keys for GetAsyncKeyState.
    physical_pressed(if swapped { 2 } else { 1 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_start_checks_the_configured_primary_physical_button() {
        assert!(primary_pressed(false, |key| key == 1));
        assert!(!primary_pressed(false, |key| key == 2));
        assert!(primary_pressed(true, |key| key == 2));
        assert!(!primary_pressed(true, |key| key == 1));
    }
}

//! Slint 1.18 stores the small icon in winit's attributes before a window exists. It sets
//! both Windows icons on subsequent changes, but its cache skips the first large icon.
use super::MainWindowEvents;
use slint::winit_030::winit::{
    event::WindowEvent,
    platform::windows::WindowExtWindows,
    window::{Icon, Window, WindowId},
};
use std::collections::HashSet;

#[cfg(test)]
#[path = "taskbar_icons_windows_tests.rs"]
mod tests;

#[derive(Default)]
pub(super) struct TaskbarIcons {
    initialized: HashSet<WindowId>,
}

impl TaskbarIcons {
    pub(super) fn observe(
        &mut self,
        main: &MainWindowEvents,
        id: WindowId,
        native: Option<&Window>,
        slint_window: Option<&slint::Window>,
        event: &WindowEvent,
    ) {
        if matches!(event, WindowEvent::Destroyed) {
            self.initialized.remove(&id);
            return;
        }
        if !matches!(event, WindowEvent::RedrawRequested) || self.initialized.contains(&id) {
            return;
        }
        // Winit supplies both handles for Slint application windows. Ignore system dialogs,
        // native menus and other windows outside Slint, and leave frameless popups alone.
        let (Some(native), Some(_), Some(main)) = (native, slint_window, main.main()) else {
            return;
        };
        self.initialized.insert(id);
        if native.is_decorated()
            && let Some(pixels) = main.get_current_app_icon().to_rgba8()
            && let Ok(icon) =
                Icon::from_rgba(pixels.as_bytes().to_vec(), pixels.width(), pixels.height())
        {
            native.set_taskbar_icon(Some(icon));
        }
    }
}

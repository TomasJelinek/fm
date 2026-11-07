//! UI-thread window geometry snapshots; no filesystem or backend work.
use slint::{LogicalSize, PhysicalPosition, Window, WindowSize};

/// Initial child-window bounds and native state captured from its parent.
/// Position is the outer frame origin; size is the client area in physical pixels.
#[derive(Clone, Debug)]
pub struct WindowPlacement {
    size: WindowSize,
    logical_size: LogicalSize,
    position: Option<PhysicalPosition>,
    maximized: bool,
    fullscreen: bool,
}

impl WindowPlacement {
    /// Capture the parent's native bounds. Wayland may leave placement to its compositor.
    pub fn capture(parent: &Window) -> Self {
        let size = parent.size();
        Self {
            size: size.into(),
            logical_size: size.to_logical(parent.scale_factor()),
            position: Some(parent.position()),
            maximized: parent.is_maximized(),
            fullscreen: parent.is_fullscreen(),
        }
    }

    /// Initial logical content size, before the native backend reports the child's monitor scale.
    pub fn logical_size(&self) -> LogicalSize {
        self.logical_size
    }

    /// Apply before showing the child so the native backend can use the requested initial bounds.
    pub fn apply(&self, child: &Window) {
        if let Some(position) = self.position {
            child.set_position(position);
        }
        child.set_size(self.size.clone());
        child.set_maximized(self.maximized);
        child.set_fullscreen(self.fullscreen);
    }
}

impl From<LogicalSize> for WindowPlacement {
    fn from(size: LogicalSize) -> Self {
        Self {
            size: size.into(),
            logical_size: size,
            position: None,
            maximized: false,
            fullscreen: false,
        }
    }
}

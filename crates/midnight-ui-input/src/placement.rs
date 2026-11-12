//! UI-thread window geometry snapshots; no filesystem or backend work.
use slint::{LogicalSize, PhysicalPosition, Window, WindowSize};

/// Initial child-window bounds and native state captured from its parent.
/// Position is the outer frame origin; size is the client area in physical pixels.
#[derive(Clone, Debug)]
pub struct WindowPlacement {
    size: WindowSize,
    logical_size: LogicalSize,
    scale_factor: Option<f32>,
    position: Option<PhysicalPosition>,
    maximized: bool,
    fullscreen: bool,
}

impl WindowPlacement {
    /// Capture the parent's native bounds. Wayland may leave placement to its compositor.
    pub fn capture(parent: &Window) -> Self {
        let size = parent.size();
        let scale_factor = parent.scale_factor();
        Self {
            size: size.into(),
            logical_size: size.to_logical(scale_factor),
            scale_factor: Some(scale_factor),
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
        // Native child creation can wait until the next event-loop boundary.
        // Seed its captured monitor scale before physical sizing establishes the
        // first logical layout and backend font metrics; native events replace it.
        if let Some(scale_factor) = self.scale_factor {
            child.dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor });
        }
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
            scale_factor: None,
            position: None,
            maximized: false,
            fullscreen: false,
        }
    }
}

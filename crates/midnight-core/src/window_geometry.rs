//! Persisted physical window bounds, independent of any UI toolkit or platform.

/// Outer-frame origin and client-area size, in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl WindowRect {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.x.unsigned_abs() > 16_777_216
            || self.y.unsigned_abs() > 16_777_216
            || !(1..=131_072).contains(&self.width)
            || !(1..=131_072).contains(&self.height)
        {
            return Err("Window geometry exceeds the supported physical desktop bounds.");
        }
        Ok(())
    }
}

/// Display identity plus the desktop bounds at capture time. Names may be absent
/// or duplicated, so restoration also compares the saved physical bounds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowMonitor {
    pub name: Option<String>,
    pub bounds: WindowRect,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowGeometry {
    /// Never replace these bounds with a maximized or minimized rectangle.
    pub normal: WindowRect,
    pub monitor: Option<WindowMonitor>,
    pub maximized: bool,
    pub fullscreen: bool,
}

impl WindowGeometry {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.normal.validate()?;
        if let Some(monitor) = &self.monitor {
            monitor.bounds.validate()?;
            if monitor
                .name
                .as_ref()
                .is_some_and(|name| name.len() > 1024 || name.chars().any(char::is_control))
            {
                return Err("Monitor names must fit 1024 UTF-8 bytes without controls.");
            }
        }
        Ok(())
    }
}

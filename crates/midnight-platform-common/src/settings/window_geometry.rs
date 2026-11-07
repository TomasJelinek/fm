use midnight_core::{WindowGeometry, WindowMonitor, WindowRect};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredWindowGeometry {
    normal: StoredRect,
    monitor: Option<StoredMonitor>,
    maximized: bool,
    fullscreen: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredRect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredMonitor {
    name: Option<String>,
    bounds: StoredRect,
}

impl From<WindowRect> for StoredRect {
    fn from(value: WindowRect) -> Self {
        Self {
            x: value.x,
            y: value.y,
            width: value.width,
            height: value.height,
        }
    }
}
impl From<StoredRect> for WindowRect {
    fn from(value: StoredRect) -> Self {
        Self {
            x: value.x,
            y: value.y,
            width: value.width,
            height: value.height,
        }
    }
}
impl From<WindowGeometry> for StoredWindowGeometry {
    fn from(value: WindowGeometry) -> Self {
        Self {
            normal: value.normal.into(),
            monitor: value.monitor.map(|monitor| StoredMonitor {
                name: monitor.name,
                bounds: monitor.bounds.into(),
            }),
            maximized: value.maximized,
            fullscreen: value.fullscreen,
        }
    }
}
impl From<StoredWindowGeometry> for WindowGeometry {
    fn from(value: StoredWindowGeometry) -> Self {
        Self {
            normal: value.normal.into(),
            monitor: value.monitor.map(|monitor| WindowMonitor {
                name: monitor.name,
                bounds: monitor.bounds.into(),
            }),
            maximized: value.maximized,
            fullscreen: value.fullscreen,
        }
    }
}

use midnight_core::{WindowGeometry, WindowMonitor, WindowRect};

/// Current display data from the UI backend, including its available work area.
pub(super) struct Display {
    pub identity: WindowMonitor,
    pub work: WindowRect,
    pub primary: bool,
}

pub(super) fn restore(
    saved: &WindowGeometry,
    displays: &[Display],
    decoration: (u32, u32),
) -> WindowGeometry {
    let mut restored = saved.clone();
    let matched = saved.monitor.as_ref().and_then(|monitor| {
        displays
            .iter()
            .find(|display| display.identity == *monitor)
            .or_else(|| {
                let mut same_name = displays.iter().filter(|display| {
                    monitor.name.is_some() && display.identity.name == monitor.name
                });
                let first = same_name.next()?;
                same_name.next().is_none().then_some(first)
            })
    });
    if let Some(display) = matched
        && saved.monitor.as_ref() == Some(&display.identity)
    {
        return restored;
    }
    let Some(display) = matched
        .or_else(|| displays.iter().find(|display| display.primary))
        .or_else(|| displays.first())
    else {
        return restored;
    };
    let area = display.work;
    let width = saved
        .normal
        .width
        .min(area.width.saturating_sub(decoration.0).max(1));
    let height = saved
        .normal
        .height
        .min(area.height.saturating_sub(decoration.1).max(1));
    let (x, y) = if matched.is_some() {
        let old = saved
            .monitor
            .as_ref()
            .map(|monitor| monitor.bounds)
            .unwrap_or(area);
        (
            i64::from(display.identity.bounds.x) + i64::from(saved.normal.x) - i64::from(old.x),
            i64::from(display.identity.bounds.y) + i64::from(saved.normal.y) - i64::from(old.y),
        )
    } else {
        // A removed display must not leave an otherwise valid off-screen origin.
        (i64::from(area.x), i64::from(area.y))
    };
    let right = i64::from(area.x) + i64::from(area.width.saturating_sub(width + decoration.0));
    let bottom = i64::from(area.y) + i64::from(area.height.saturating_sub(height + decoration.1));
    restored.normal = WindowRect {
        x: x.clamp(i64::from(area.x), right) as i32,
        y: y.clamp(i64::from(area.y), bottom) as i32,
        width,
        height,
    };
    restored.monitor = Some(display.identity.clone());
    restored
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(name: &str, x: i32, width: u32, primary: bool) -> Display {
        let bounds = WindowRect {
            x,
            y: 0,
            width,
            height: 1080,
        };
        Display {
            identity: WindowMonitor {
                name: Some(name.into()),
                bounds,
            },
            work: WindowRect {
                height: 1040,
                ..bounds
            },
            primary,
        }
    }
    fn saved(display: &Display) -> WindowGeometry {
        WindowGeometry {
            normal: WindowRect {
                x: -1800,
                y: 123,
                width: 970,
                height: 610,
            },
            monitor: Some(display.identity.clone()),
            maximized: true,
            fullscreen: false,
        }
    }
    #[test]
    fn unchanged_monitor_restores_exact_physical_origin_size_and_state_without_y_drift() {
        let displays = [
            display("left", -1920, 1920, false),
            display("main", 0, 1920, true),
        ];
        let original = saved(&displays[0]);
        let mut current = original.clone();
        for _ in 0..20 {
            current = restore(&current, &displays, (16, 39));
        }
        assert_eq!(current, original);
    }
    #[test]
    fn removed_monitor_falls_back_to_primary_work_area_including_frame() {
        let original = saved(&display("removed", -1920, 1920, false));
        let displays = [
            display("other", 1920, 1920, false),
            display("main", 0, 800, true),
        ];
        let restored = restore(&original, &displays, (16, 39));
        assert_eq!(
            restored.normal,
            WindowRect {
                x: 0,
                y: 0,
                width: 784,
                height: 610
            }
        );
        assert_eq!(restored.monitor, Some(displays[1].identity.clone()));
        assert!(restored.maximized);
    }
    #[test]
    fn monitor_move_uses_saved_offset_and_bounds_reduction_clamps_frame() {
        let original = saved(&display("left", -1920, 1920, false));
        let displays = [display("left", 100, 800, false)];
        let restored = restore(&original, &displays, (16, 39));
        assert_eq!(
            restored.normal,
            WindowRect {
                x: 100,
                y: 123,
                width: 784,
                height: 610
            }
        );
    }

    #[test]
    fn moved_monitor_offset_does_not_add_a_taskbar_offset_twice() {
        let original = saved(&display("left", -1920, 1920, false));
        let mut current = display("left", 100, 1920, false);
        current.work.x += 40;
        current.work.width -= 40;
        let restored = restore(&original, &[current], (16, 39));
        assert_eq!(restored.normal.x, 220);
        assert_eq!(restored.normal.y, 123);
    }
}

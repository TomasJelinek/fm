use super::*;

fn previous() -> WindowGeometry {
    WindowGeometry {
        normal: WindowRect {
            x: 2200,
            y: 119,
            width: 970,
            height: 610,
        },
        maximized: false,
        fullscreen: false,
        monitor: Some(display("previous", 1920)),
    }
}

fn display(name: &str, x: i32) -> WindowMonitor {
    WindowMonitor {
        name: Some(name.into()),
        bounds: WindowRect {
            x,
            y: 0,
            width: 1920,
            height: 1080,
        },
    }
}

#[test]
fn unavailable_native_position_preserves_the_previous_origin_and_monitor_together() {
    let previous = previous();
    assert_eq!(
        placement(None, Some(display("current", 0)), Some(&previous)),
        (2200, 119, previous.monitor)
    );
}

#[test]
fn actual_native_position_uses_the_actual_monitor_instead_of_the_previous_one() {
    let current = display("current", 0);
    assert_eq!(
        placement(Some((317, -269)), Some(current.clone()), Some(&previous())),
        (317, -269, Some(current))
    );
    assert_eq!(
        placement(Some((317, -269)), None, Some(&previous())),
        (317, -269, None)
    );
}

#[test]
fn unavailable_initial_position_does_not_pair_an_invented_origin_with_a_current_monitor() {
    assert_eq!(
        placement(None, Some(display("current", 0)), None),
        (0, 0, None)
    );
}

#[test]
fn unavailable_restore_retains_saved_monitor_identity_with_its_saved_coordinates() {
    let saved = previous();
    let mut restored = WindowGeometry {
        normal: WindowRect {
            x: 10,
            y: 20,
            width: 720,
            height: 460,
        },
        monitor: Some(display("current", 0)),
        ..saved.clone()
    };
    (restored.normal.x, restored.normal.y, restored.monitor) = placement(None, None, Some(&saved));
    assert_eq!((restored.normal.x, restored.normal.y), (2200, 119));
    assert_eq!(restored.monitor, saved.monitor);
    assert_eq!((restored.normal.width, restored.normal.height), (720, 460));
}

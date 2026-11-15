use super::*;
use slint::winit_030::winit::raw_window_handle::{WaylandDisplayHandle, WaylandWindowHandle};

fn existing_hints() -> xlib::XSizeHints {
    xlib::XSizeHints {
        flags: xlib::PSize
            | xlib::PMinSize
            | xlib::PMaxSize
            | xlib::PResizeInc
            | xlib::PAspect
            | xlib::PBaseSize
            | xlib::PWinGravity,
        x: 13,
        y: 17,
        width: 900,
        height: 700,
        min_width: 320,
        min_height: 200,
        max_width: 2400,
        max_height: 1600,
        width_inc: 2,
        height_inc: 3,
        min_aspect: xlib::AspectRatio { x: 4, y: 3 },
        max_aspect: xlib::AspectRatio { x: 16, y: 9 },
        base_width: 640,
        base_height: 480,
        win_gravity: xlib::NorthWestGravity,
    }
}

#[test]
fn restored_origin_adds_user_position_and_preserves_all_other_wm_hints() {
    let original = existing_hints();
    let position = slint::PhysicalPosition::new(-1837, 211);
    let mut updated = position_hints(original, position);
    assert_eq!(updated.flags, original.flags | xlib::USPosition);
    assert_eq!((updated.x, updated.y), (position.x, position.y));
    updated.flags = original.flags;
    updated.x = original.x;
    updated.y = original.y;
    assert_eq!(updated, original);
}

#[test]
fn repeated_position_hints_keep_physical_coordinates_without_decoration_or_scale_drift() {
    let position = slint::PhysicalPosition::new(-1825, -357);
    let expected = position_hints(existing_hints(), position);
    let mut updated = expected;
    for _ in 0..20 {
        updated = position_hints(updated, position);
    }
    assert_eq!(updated, expected);
}

#[test]
fn wayland_handles_do_not_claim_absolute_placement_support() {
    let pointer = std::ptr::NonNull::dangling();
    let display = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(pointer));
    let window = RawWindowHandle::Wayland(WaylandWindowHandle::new(pointer));
    assert!(xlib_identity(display, window).is_none());
}

use super::{Correction, correction, learned_offset};

/// A window manager's placement model in physical pixels: where a requested outer
/// origin puts the client, and the outer origin it then reports back.
#[derive(Clone, Copy, Debug)]
struct Manager {
    /// Client offset inside the frame the manager places at the requested origin.
    placed_border: (i32, i32),
    /// Client offset inside the frame the manager reports (frame extents / window rect).
    reported_border: (i32, i32),
}

impl Manager {
    fn client(self, request: (i32, i32)) -> (i32, i32) {
        (
            request.0 + self.placed_border.0,
            request.1 + self.placed_border.1,
        )
    }
    fn reported_outer(self, client: (i32, i32)) -> (i32, i32) {
        (
            client.0 - self.reported_border.0,
            client.1 - self.reported_border.1,
        )
    }
}

fn scaled(value: i32, scale: f32) -> i32 {
    (value as f32 * scale).round() as i32
}

/// Decorations of the supported desktops at one scale factor.
fn managers(scale: f32) -> Vec<(&'static str, Manager)> {
    let border = scaled(6, scale);
    let title = scaled(27, scale);
    let shadow = scaled(32, scale);
    let resize = scaled(7, scale);
    vec![
        // Mutter/KWin server-side frames: _NET_FRAME_EXTENTS and gravity both use
        // the visible frame.
        (
            "x11-ssd-consistent",
            Manager {
                placed_border: (border, title),
                reported_border: (border, title),
            },
        ),
        // Weston's XWayland manager (WSLg): extents include the shadow, gravity does not.
        (
            "x11-ssd-shadow-extents",
            Manager {
                placed_border: (border, title),
                reported_border: (border + shadow, title + shadow),
            },
        ),
        // Windows: GetWindowRect includes the invisible DWM resize borders on both paths.
        (
            "windows-dwm",
            Manager {
                placed_border: (resize + 1, title + resize),
                reported_border: (resize + 1, title + resize),
            },
        ),
        // Undecorated or reparenting-free managers.
        (
            "x11-no-frame",
            Manager {
                placed_border: (0, 0),
                reported_border: (0, 0),
            },
        ),
    ]
}

/// Open a child over a parent placed at `parent_request`; return the child's client
/// origin after creation and after at most one correction, plus the learned offset.
fn open_child(
    manager: Manager,
    parent_request: (i32, i32),
    learned: (i32, i32),
) -> ((i32, i32), (i32, i32), (i32, i32)) {
    let parent_client = manager.client(parent_request);
    let parent_outer = manager.reported_outer(parent_client);
    let request = (parent_outer.0 + learned.0, parent_outer.1 + learned.1);
    let first = manager.client(request);
    match correction(request, parent_client, first) {
        Correction::Aligned => (first, first, learned),
        Correction::Elsewhere => panic!("a frame-extents error is decoration-sized"),
        Correction::Move(corrected) => (
            first,
            manager.client(corrected),
            learned_offset(learned, request, corrected),
        ),
    }
}

#[test]
fn one_correction_aligns_the_client_area_on_every_frame_model_and_scale() {
    for scale in [1.0, 1.25, 2.25] {
        for (name, manager) in managers(scale) {
            for parent_request in [(180, 150), (-1837, 211), (0, 0), (3839, -357)] {
                let parent_client = manager.client(parent_request);
                let (_, aligned, _) = open_child(manager, parent_request, (0, 0));
                assert_eq!(
                    aligned, parent_client,
                    "{name} at {scale}: {parent_request:?}"
                );
            }
        }
    }
}

#[test]
fn consistent_frames_need_no_move_and_learn_nothing() {
    for scale in [1.0, 1.25, 2.25] {
        for (name, manager) in managers(scale) {
            if name == "x11-ssd-shadow-extents" {
                continue;
            }
            let (first, _, learned) = open_child(manager, (37, 61), (0, 0));
            assert_eq!(first, manager.client((37, 61)), "{name} at {scale}");
            assert_eq!(learned, (0, 0), "{name} at {scale}");
        }
    }
}

#[test]
fn a_learned_shadow_offset_opens_later_children_aligned_without_a_move() {
    for scale in [1.0, 1.25, 2.25] {
        let manager = managers(scale)
            .into_iter()
            .find(|(name, _)| *name == "x11-ssd-shadow-extents")
            .unwrap()
            .1;
        let shadow = scaled(32, scale);
        let (first, aligned, learned) = open_child(manager, (180, 150), (0, 0));
        let parent_client = manager.client((180, 150));
        assert_eq!(
            first,
            (parent_client.0 - shadow, parent_client.1 - shadow),
            "the measured WSLg drift at {scale}"
        );
        assert_eq!(aligned, parent_client);
        assert_eq!(learned, (shadow, shadow));
        for parent_request in [(500, 400), (-1200, 90)] {
            let (first, _, again) = open_child(manager, parent_request, learned);
            assert_eq!(first, manager.client(parent_request), "{scale}");
            assert_eq!(again, learned, "an aligned child keeps the learned offset");
        }
    }
}

#[test]
fn a_child_far_from_the_parent_is_left_where_the_window_manager_put_it() {
    // A cascaded child, or the offscreen origin XWayland reports before placement.
    assert_eq!(
        correction((100, 100), (106, 127), (406, 427)),
        Correction::Elsewhere
    );
    assert_eq!(
        correction((148, 118), (186, 177), (-32730, -32709)),
        Correction::Elsewhere
    );
    // Learned offsets stay decoration-sized.
    assert_eq!(learned_offset((100, 100), (0, 0), (60, 0)), (100, 100));
    assert_eq!(learned_offset((32, 32), (132, 132), (100, 100)), (0, 0));
}

#[test]
fn corrections_saturate_at_the_coordinate_limits() {
    assert_eq!(
        correction((i32::MAX, i32::MIN), (10, -10), (0, 0)),
        Correction::Move((i32::MAX, i32::MIN))
    );
    assert_eq!(correction((5, 5), (1, 2), (1, 2)), Correction::Aligned);
}

fn placement(
    position: Option<(i32, i32)>,
    client: Option<(i32, i32)>,
    maximized: bool,
    fullscreen: bool,
) -> super::WindowPlacement {
    let size = slint::PhysicalSize::new(2250, 1575);
    super::WindowPlacement {
        size: size.into(),
        logical_size: size.to_logical(2.25),
        scale_factor: Some(2.25),
        position: position.map(|(x, y)| slint::PhysicalPosition::new(x, y)),
        client: client.map(|(x, y)| (slint::PhysicalPosition::new(x, y), size)),
        maximized,
        fullscreen,
    }
}

#[test]
fn only_normal_frames_with_native_origins_are_verified_after_showing() {
    let normal = placement(Some((148, 118)), Some((186, 177)), false, false);
    let requested = normal.requested((32, 32));
    assert_eq!(requested, Some(slint::PhysicalPosition::new(180, 150)));
    assert_eq!(
        normal.alignment(requested),
        Some((
            slint::PhysicalPosition::new(180, 150),
            slint::PhysicalPosition::new(186, 177),
            slint::PhysicalSize::new(2250, 1575)
        ))
    );
    for (maximized, fullscreen) in [(true, false), (false, true)] {
        let state = placement(Some((-14, -14)), Some((0, 51)), maximized, fullscreen);
        assert_eq!(state.alignment(state.requested((0, 0))), None);
    }
    // Wayland (client-side decorations): no origin to request or to verify.
    let wayland = placement(None, None, false, false);
    assert_eq!(wayland.requested((32, 32)), None);
    assert_eq!(wayland.alignment(None), None);
    // Headless platforms: Slint's origin is requested, nothing is verified.
    let headless = placement(Some((10, 20)), None, false, false);
    assert_eq!(headless.alignment(headless.requested((0, 0))), None);
}

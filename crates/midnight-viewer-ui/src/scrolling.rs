//! Convert keyboard navigation into backend viewport movements, and tell touchpad
//! momentum where the content stands.
use midnight_core::ViewerInput;

/// The content position per axis (0 at the start, 1 at the end) for touchpad momentum,
/// which stops an axis whose position stops changing: given only at an edge. Between the
/// edges a text page can take a while to arrive from the backend, and the shown position
/// then stands still although the document goes on; momentum must not take that pause
/// for the document's end.
pub(super) fn momentum_position(horizontal: f32, vertical: f32) -> [Option<f64>; 2] {
    const EDGE: f32 = 1e-6;
    [horizontal, vertical]
        .map(|position| (position <= EDGE || position >= 1.0 - EDGE).then_some(f64::from(position)))
}

pub(super) fn keyboard(action: &str, content_height: f32, scale: f32) -> Option<ViewerInput> {
    let (x, y) = match action {
        "viewer.up" => (0.0, -28.0),
        "viewer.down" => (0.0, 28.0),
        "viewer.left" => (-40.0, 0.0),
        "viewer.right" => (40.0, 0.0),
        "viewer.page-up" => (0.0, -content_height),
        "viewer.page-down" => (0.0, content_height),
        _ => return None,
    };
    Some(ViewerInput::Scroll {
        x: x * scale,
        y: y * scale,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_keys_move_one_physical_viewport_at_fractional_and_double_scale() {
        for (scale, expected) in [(0.5, 300.0), (1.0, 600.0), (1.25, 750.0), (2.0, 1200.0)] {
            for (action, direction) in [("viewer.page-down", 1.0), ("viewer.page-up", -1.0)] {
                let Some(ViewerInput::Scroll { x, y }) = keyboard(action, 600.0, scale) else {
                    panic!("page keys must scroll the viewport");
                };
                assert_eq!((x, y), (0.0, expected * direction), "{action} at {scale}");
            }
        }
    }

    #[test]
    fn arrow_keys_keep_their_logical_distance_on_scaled_displays() {
        for (action, expected) in [
            ("viewer.up", (0.0, -56.0)),
            ("viewer.down", (0.0, 56.0)),
            ("viewer.left", (-80.0, 0.0)),
            ("viewer.right", (80.0, 0.0)),
        ] {
            let Some(ViewerInput::Scroll { x, y }) = keyboard(action, 600.0, 2.0) else {
                panic!("arrow keys must scroll the viewport");
            };
            assert_eq!((x, y), expected);
        }
        assert!(keyboard("viewer.copy", 600.0, 2.0).is_none());
    }
}

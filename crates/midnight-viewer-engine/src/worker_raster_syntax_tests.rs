use super::*;

#[test]
fn source_refresh_keeps_world_camera_pending_pan_and_selection_without_resetting_surface() {
    let mut retained = Retained {
        surface: 6,
        pixel: 10,
        layout: 4,
        base: [100.0, 200.0],
        target: [126.0, 240.0],
        sequence: 37,
        exact: false,
        continuing: Some([124.0, 241.0]),
        rebase_ack: Some(11),
        selected_source: Some([7000, 8000]),
        selected_text: Some("東京".into()),
        selection_valid: true,
        ..Default::default()
    };
    retained.source_refreshed([20.0, 30.0], [7.0, 9.0]);
    assert_eq!(retained.camera([7.0, 9.0]), [120.0, 230.0]);
    assert_eq!(retained.target, [126.0, 240.0]);
    assert_eq!(retained.sequence, 37);
    assert!(!retained.exact);
    assert_eq!(retained.continuing, Some([124.0, 241.0]));
    assert_eq!(retained.rebase_ack, Some(11));
    assert_eq!(retained.selected_source, Some([7000, 8000]));
    assert_eq!(retained.selected_text.as_deref(), Some("東京"));
    assert!(retained.selection_valid);
    assert_eq!(retained.surface, 6);
    assert_eq!(retained.layout, 5);
    assert_eq!(retained.first_pixel, 11);
    assert!(retained.invalid);
}

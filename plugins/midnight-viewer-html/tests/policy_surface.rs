use midnight_viewer_html::policy::{Access, DocumentPolicy};
use midnight_viewer_html::surface::Surface;

#[test]
fn offline_origin_never_falls_through_to_network() {
    let policy = DocumentPolicy::new(1, false, false).unwrap();
    assert_eq!(
        policy.access("https://mewv-1.invalid/index.html"),
        Access::Captured
    );
    assert_eq!(policy.access("https://example.com/"), Access::Deny);
    for url in [
        "file:///C:/secret",
        "javascript:alert(1)",
        "data:text/html,x",
        "https://mewv-1.invalid.evil/a",
        "https://mewv-1.invalid@evil/a",
    ] {
        assert_eq!(policy.access(url), Access::Deny, "{url}");
    }
}

#[test]
fn online_only_exposes_main_document_and_scripts_require_network() {
    assert!(DocumentPolicy::new(1, false, true).is_err());
    let policy = DocumentPolicy::new(2, true, true).unwrap();
    assert_eq!(policy.access("https://mewv-2.invalid/"), Access::Captured);
    assert_eq!(
        policy.access("https://mewv-2.invalid/index.html"),
        Access::Deny
    );
    assert_eq!(
        policy.access("https://mewv-2.invalid/sibling.css"),
        Access::Deny
    );
    assert_eq!(policy.access("https://example.com/asset"), Access::Network);
    assert_eq!(policy.access("file:///tmp/a"), Access::Deny);
}

#[test]
fn paint_is_owned_rgba_and_rejects_stale_resize() {
    let mut surface = Surface::new(7, 1, 1).unwrap();
    let mut bgra = vec![10, 20, 30, 128];
    let frame = surface.paint(7, 1, 1, &bgra).unwrap();
    bgra.fill(255);
    assert_eq!(frame, vec![30, 20, 10, 128]);
    surface.resize(8, 2, 1).unwrap();
    assert!(surface.paint(7, 1, 1, &bgra).is_none());
    assert!(Surface::new(1, 8193, 1).is_err());
    assert!(Surface::new(1, 8192, 8192).is_err());
}

#[test]
fn fractional_dpi_rounding_crops_only_the_extra_native_edge() {
    let mut surface = Surface::new(3, 3, 1).unwrap();
    assert_eq!(
        surface
            .paint_cropped(
                3,
                4,
                1,
                &[
                    0, 0, 10, 255, 0, 20, 0, 255, 30, 0, 0, 255, 255, 255, 255, 255
                ]
            )
            .unwrap(),
        [10, 0, 0, 255, 0, 20, 0, 255, 0, 0, 30, 255]
    );
    assert!(surface.paint_cropped(2, 4, 1, &[0; 16]).is_none());
    assert!(surface.paint_cropped(3, 2, 1, &[0; 8]).is_none());
}

#[test]
fn popup_composes_premultiplied_pixels_and_restores_base_on_hide() {
    let mut layers = midnight_viewer_html::surface::Layers::default();
    layers.base(2, 1, vec![100, 100, 100, 255, 100, 100, 100, 255]);
    layers.popup_rect(-1, 0);
    layers.show(true);
    assert!(layers.popup(2, 1, &[0, 0, 0, 0, 0, 0, 128, 128]));
    assert_eq!(
        layers.frame().unwrap(),
        [178, 50, 50, 255, 100, 100, 100, 255]
    );
    layers.show(false);
    assert_eq!(
        layers.frame().unwrap(),
        [100, 100, 100, 255, 100, 100, 100, 255]
    );
    assert!(!layers.popup(8192, 8192, &[]));
}

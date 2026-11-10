use midnight_viewer_render::{TextOptions, TextRenderer, installed_families, monospace_families};
use std::sync::Arc;

#[test]
fn installed_catalog_includes_proportional_families_and_preserves_literal_names() {
    let names = installed_families(|| false).unwrap();
    let mut fonts = parley::FontContext::default();
    let expected: Vec<_> = fonts.collection.family_names().map(str::to_owned).collect();
    assert!(!expected.is_empty(), "the test host needs installed fonts");
    for name in &expected {
        if !name.trim().is_empty() && name.len() <= 256 && !name.chars().any(char::is_control) {
            assert!(names.contains(name), "missing installed family {name:?}");
        }
    }
    assert!(Arc::ptr_eq(&names, &installed_families(|| false).unwrap()));
    let fixed = monospace_families(|| false).unwrap();
    assert!(names.iter().any(|name| !fixed.contains(name)));
    assert!(
        names.windows(2).all(|pair| {
            (pair[0].to_lowercase(), &pair[0]) < (pair[1].to_lowercase(), &pair[1])
        })
    );
}

#[test]
fn cancelled_installed_catalog_request_never_returns_a_completed_result() {
    assert!(installed_families(|| false).is_some());
    assert!(installed_families(|| true).is_none());
}

#[test]
fn installed_monospace_catalog_is_cached_and_lays_out_equal_width_columns() {
    let families = monospace_families(|| false).unwrap();
    assert!(Arc::ptr_eq(
        &families,
        &monospace_families(|| false).unwrap()
    ));
    assert!(
        !families
            .iter()
            .any(|name| name.eq_ignore_ascii_case("Arial"))
    );
    let mut renderer = TextRenderer::default();
    // Test every available fixed-pitch family, including Consolas where installed.
    for family in families.iter() {
        let options = TextOptions {
            family,
            literal_family: true,
            size: 16.0,
            ..TextOptions::default()
        };
        let narrow = renderer
            .layout_with_options("iiiiiiii", &options, &[])
            .unwrap();
        let wide = renderer
            .layout_with_options("WWWWWWWW", &options, &[])
            .unwrap();
        assert!(
            (narrow.width() - wide.width()).abs() < 0.1,
            "{family}: {} vs {}",
            narrow.width(),
            wide.width()
        );
    }
}

use midnight_viewer_render::{
    TextOptions, TextRenderer, fixed_pitch_family, installed_families, monospace_families,
};
use std::{sync::Arc, time::Instant};

#[test]
fn fixed_pitch_family_agrees_with_the_catalog() {
    let catalog = monospace_families(|| false).unwrap();
    assert!(!catalog.is_empty(), "the test host needs a code font");
    let first = catalog[0].as_str();
    for name in catalog.iter().take(8).map(String::as_str) {
        assert_eq!(fixed_pitch_family(&[name], || false).as_deref(), Some(name));
        // The first listed candidate wins, as the chooser resolves a saved family.
        assert_eq!(
            fixed_pitch_family(&["No Such Family 7f1c", name, first], || false).as_deref(),
            Some(name)
        );
    }
    let installed = installed_families(|| false).unwrap();
    let proportional = installed
        .iter()
        .find(|name| !catalog.iter().any(|fixed| fixed.eq_ignore_ascii_case(name)))
        .expect("the test host has a proportional family");
    assert_eq!(fixed_pitch_family(&[proportional.as_str()], || false), None);
    assert_eq!(fixed_pitch_family(&["No Such Family 7f1c"], || false), None);
    assert_eq!(fixed_pitch_family(&[], || false), None);
    assert_eq!(fixed_pitch_family(&[first], || true), None);
}

/// Opt-in cost of the preferred-family check against the full catalog scan. Run
/// optimized in a fresh process (the catalog is cached process-wide):
/// `cargo test --release -p midnight-viewer-render --test integration preferred_family_check_timing -- --ignored --nocapture`
#[test]
#[ignore = "timing harness; run optimized with --ignored --nocapture"]
fn preferred_family_check_timing() {
    let time = |check: &dyn Fn() -> Option<String>| {
        let started = Instant::now();
        let family = check();
        (started.elapsed().as_secs_f64() * 1000.0, family)
    };
    let (cold, family) = time(&|| fixed_pitch_family(&["Cascadia Mono", "Consolas"], || false));
    let (warm, _) = time(&|| fixed_pitch_family(&["Cascadia Mono", "Consolas"], || false));
    let (missing, fallback) = time(&|| fixed_pitch_family(&["No Such Mono", "Consolas"], || false));
    let started = Instant::now();
    let catalog = monospace_families(|| false).unwrap();
    let scan = started.elapsed().as_secs_f64() * 1000.0;
    println!(
        "font_timing check_cold_ms={cold:.1} check_warm_ms={warm:.1} check_fallback_ms={missing:.1} \
         family={family:?} fallback={fallback:?} catalog_scan_ms={scan:.1} families={}",
        catalog.len()
    );
}

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

#[test]
fn installed_monospace_catalog_covers_basic_text_without_fallback() {
    let families = monospace_families(|| false).unwrap();
    assert!(!families.is_empty(), "the test host needs a code font");
    let mut fonts = parley::FontContext::default();
    for family_name in families.iter() {
        let family = fonts.collection.family_by_name(family_name).unwrap();
        let face = family.fonts().first().unwrap();
        let data = face.load(Some(&mut fonts.source_cache)).unwrap();
        let font = swash::FontRef::from_index(data.as_ref(), face.index() as usize).unwrap();
        let charmap = font.charmap();
        for character in ' '..='~' {
            assert_ne!(
                charmap.map(character),
                0,
                "{family_name} needs fallback for basic text {character:?}"
            );
        }
    }
    assert!(monospace_families(|| true).is_none());
}

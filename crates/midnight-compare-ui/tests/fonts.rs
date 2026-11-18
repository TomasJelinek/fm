use midnight_compare_ui::ComparatorWindows;
use midnight_core::{FileCompareLaunch, FileCompareSource, ViewerAppearance};
use slint::{
    ComponentHandle, Model,
    platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use std::{cell::RefCell, rc::Rc, sync::Arc};

#[path = "support/resident.rs"]
mod resident;
#[path = "support/mod.rs"]
mod support;
use support::{TestPlatform, wait_for};

#[test]
fn comparator_offers_fixed_pitch_fonts_persists_explicit_choice_and_fits_small_window() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let directory =
        std::env::temp_dir().join(format!("midnight-compare-fonts-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let paths = [directory.join("A.txt"), directory.join("B.txt")];
    std::fs::write(&paths[0], "column 1\tcolumn 2\nčeský 世界\nold\n").unwrap();
    std::fs::write(&paths[1], "column 1\tcolumn 2\nčeský 世界\nnew\n").unwrap();
    let runtime = resident::TestRuntime::new(tokio::runtime::Runtime::new().unwrap());
    let manager = ComparatorWindows::new(midnight_compare_engine::Service::new(
        runtime.handle().clone(),
        runtime.spawner(),
    ));
    let saved = Rc::new(RefCell::new(Vec::new()));
    let changes = saved.clone();
    manager.set_font_listener(Rc::new(move |name| changes.borrow_mut().push(name)));
    let comparator_snapshot = midnight_core::FileComparatorSnapshot::default();
    let window = manager
        .open(
            FileCompareLaunch {
                sources: paths.into_iter().map(FileCompareSource::Focused).collect(),
                disabled_plugins: Arc::from([]),
                comparators: comparator_snapshot.preferences.clone(),
                comparator_policy: comparator_snapshot.id,
            },
            ViewerAppearance {
                family: "Consolas".into(),
                font_size: 14.0,
                foreground: [240, 240, 240, 255],
                background: [20, 20, 20, 255],
                selection: [60, 80, 120, 255],
                accent: [100, 150, 220, 255],
            },
            slint::LogicalSize::new(800.0, 600.0),
            Rc::new(|_, _| None),
        )
        .unwrap()
        .upgrade()
        .unwrap();
    wait_for(&window, "finish font discovery", || {
        window.get_fonts_ready()
    });
    let families = window.get_font_families();
    assert_eq!(families.row_data(0).unwrap(), "System monospace");
    assert!(
        !families
            .iter()
            .any(|name| name.eq_ignore_ascii_case("Arial"))
    );
    if let Some(index) = families.iter().position(|name| name == "Consolas") {
        assert_eq!(window.get_font_index(), index as i32);
    } else {
        assert_eq!(window.get_font_index(), 0);
    }
    assert!(
        saved.borrow().is_empty(),
        "Default discovery must not overwrite saved preferences"
    );
    window.invoke_change_font(-1);
    window.invoke_change_font(20_000);
    assert!(saved.borrow().is_empty());
    let choices = families.row_count() > 1;
    if choices {
        // Rapid choices must retain the last explicit choice, even before the next frame.
        window.invoke_change_font(0);
        window.invoke_change_font(1);
        window.invoke_change_font(0);
        assert_eq!(window.get_font_index(), 0);
        assert_eq!(saved.borrow().last().unwrap(), "monospace");
    }
    let geometry = window.get_geometry_revision();
    native.set_size(slint::PhysicalSize::new(320, 200));
    window
        .window()
        .dispatch_event(slint::platform::WindowEvent::Resized {
            size: slint::LogicalSize::new(320.0, 200.0),
        });
    // The worker handles inputs in order and the resize follows the choices, so once
    // its frame is presented every frame painted before the last choice took effect has
    // been presented or replaced.
    wait_for(&window, "present the resized frame", || {
        window.get_geometry_revision() > geometry
            && window.get_frame_revision() == window.get_geometry_revision()
    });
    if choices {
        // Those frames must not have moved the selection back.
        assert_eq!(window.get_font_index(), 0);
    }
    window.set_compact(true);
    let mut pixels = vec![slint::Rgb8Pixel::default(); 320 * 200];
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 320);
    });
    assert_eq!(window.window().size(), slint::PhysicalSize::new(320, 200));
    assert!(window.get_content_width() > 250.0);
    assert!(window.get_content_height() > 30.0);
    manager.close_all();
    drop(runtime);
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn application_fonts_and_palette_updates_preserve_document_font_and_prime_later_opens() {
    use midnight_core::{PreferenceTheme, UiDensity};
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native))).unwrap();
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .join("_build/qa")
        .join(format!("compare-appearance-fonts-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let paths = [directory.join("left.txt"), directory.join("right.txt")];
    std::fs::write(&paths[0], "Shared text\nOld value\n").unwrap();
    std::fs::write(&paths[1], "Shared text\nNew value\n").unwrap();
    let runtime = resident::TestRuntime::new(tokio::runtime::Runtime::new().unwrap());
    let manager = ComparatorWindows::new(midnight_compare_engine::Service::new(
        runtime.handle().clone(),
        runtime.spawner(),
    ));
    let saved = Rc::new(RefCell::new(Vec::new()));
    let changes = saved.clone();
    manager.set_font_listener(Rc::new(move |family| changes.borrow_mut().push(family)));
    let a = ViewerAppearance {
        family: "Segoe UI".into(),
        font_size: 32.0,
        foreground: [48, 48, 48, 255],
        background: [255, 255, 255, 255],
        selection: [204, 204, 204, 255],
        accent: [69, 69, 69, 255],
    };
    let b = ViewerAppearance {
        family: "Arial".into(),
        font_size: 18.0,
        foreground: [255, 255, 255, 255],
        background: [0, 0, 0, 255],
        selection: [255, 255, 255, 255],
        accent: [191, 255, 255, 255],
    };
    let apply = |appearance: &ViewerAppearance, theme| {
        manager.apply_appearance(
            appearance,
            theme,
            UiDensity::Desktop,
            appearance.font_size as u16,
            36,
        );
    };
    let snapshot = midnight_core::FileComparatorSnapshot::default();
    let open = || {
        let mut document = a.clone();
        document.family = "monospace".into();
        document.font_size = 14.0;
        manager
            .open(
                FileCompareLaunch {
                    sources: paths
                        .iter()
                        .cloned()
                        .map(FileCompareSource::Focused)
                        .collect(),
                    disabled_plugins: Arc::from([]),
                    comparators: snapshot.preferences.clone(),
                    comparator_policy: snapshot.id,
                },
                document,
                slint::LogicalSize::new(1100.0, 740.0),
                Rc::new(|_, _| None),
            )
            .unwrap()
            .upgrade()
            .unwrap()
    };
    let recolored = |window: &midnight_compare_ui::ComparatorWindow, expected: [u8; 4]| {
        window.get_surface().to_rgba8().is_some_and(|pixels| {
            pixels
                .as_slice()
                .last()
                .is_some_and(|pixel| [pixel.r, pixel.g, pixel.b, pixel.a] == expected)
        })
    };
    apply(&a, PreferenceTheme::Light);
    let window = open();
    assert_eq!(window.get_ui_font_family(), "Segoe UI");
    assert_eq!(window.get_ui_font_size(), 32.0);
    wait_for(&window, "update comparison appearance", || {
        window.get_fonts_ready() && recolored(&window, a.background)
    });
    assert_eq!(
        window.get_font_index(),
        0,
        "Document font stays system monospace, not Segoe UI"
    );
    let text = window.get_accessible_text();
    apply(&b, PreferenceTheme::HighContrastDark);
    wait_for(&window, "update comparison appearance", || {
        recolored(&window, b.background)
    });
    assert_eq!(window.get_ui_font_family(), "Arial");
    assert_eq!(window.get_font_index(), 0);
    assert_eq!(window.get_accessible_text(), text);
    manager.close_all();

    // A window opened during the preview is primed before show and also rolls back.
    let window = open();
    assert_eq!(window.get_ui_font_family(), "Arial");
    assert_eq!(window.get_ui_font_size(), 18.0);
    wait_for(&window, "update comparison appearance", || {
        window.get_fonts_ready() && recolored(&window, b.background)
    });
    apply(&a, PreferenceTheme::Light);
    wait_for(&window, "update comparison appearance", || {
        recolored(&window, a.background)
    });
    assert_eq!(window.get_ui_font_family(), "Segoe UI");
    assert_eq!(window.get_font_index(), 0);
    assert!(
        saved.borrow().is_empty(),
        "Application appearance never invokes document-font persistence"
    );
    manager.close_all();
    drop(runtime);
    std::fs::remove_dir_all(directory).unwrap();
}

use midnight_compare_ui::ComparatorWindows;
use midnight_core::{FileCompareLaunch, FileCompareSource, ViewerAppearance};
use slint::{
    ComponentHandle, Model,
    platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

mod support;
use support::TestPlatform;

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
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let manager = ComparatorWindows::new(midnight_compare_engine::Service::new(
        runtime.handle().clone(),
    ));
    let saved = Rc::new(RefCell::new(Vec::new()));
    let changes = saved.clone();
    manager.set_font_listener(Rc::new(move |name| changes.borrow_mut().push(name)));
    let window = manager
        .open(
            FileCompareLaunch {
                sources: paths.into_iter().map(FileCompareSource::Focused).collect(),
                disabled_plugins: Arc::from([]),
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
    wait_for(|| window.get_fonts_ready());
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
    if families.row_count() > 1 {
        // Rapid choices must retain the last explicit choice, even before the next frame.
        window.invoke_change_font(0);
        window.invoke_change_font(1);
        window.invoke_change_font(0);
        wait_for(|| window.get_font_index() == 0);
        assert_eq!(saved.borrow().last().unwrap(), "monospace");
        for _ in 0..10 {
            std::thread::sleep(Duration::from_millis(20));
            slint::platform::update_timers_and_animations();
        }
        assert_eq!(window.get_font_index(), 0);
    }
    native.set_size(slint::PhysicalSize::new(320, 200));
    window
        .window()
        .dispatch_event(slint::platform::WindowEvent::Resized {
            size: slint::LogicalSize::new(320.0, 200.0),
        });
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

fn wait_for(done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !done() {
        assert!(
            Instant::now() < deadline,
            "Comparator did not finish font discovery/rendering"
        );
        std::thread::sleep(Duration::from_millis(10));
        slint::platform::update_timers_and_animations();
    }
}

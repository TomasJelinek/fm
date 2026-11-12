use midnight_compare_ui::{ComparatorWindow, ComparatorWindows};
use midnight_core::{
    FileCompareLaunch, FileCompareSource, ShortcutBinding, ShortcutContext, ShortcutMap,
    ViewerAppearance,
};
use slint::{
    ComponentHandle,
    platform::{
        Key, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
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
fn real_keys_jump_both_sides_without_wrapping_losing_repeats_or_bypassing_overrides() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let directory =
        std::env::temp_dir().join(format!("comparator-difference-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let paths = [directory.join("left.txt"), directory.join("right.txt")];
    for (side, path) in paths.iter().enumerate() {
        let text: String = (0..150)
            .map(|row| {
                if matches!(row, 0 | 50 | 100) {
                    format!("{}-{row}\n", if side == 0 { "OLD" } else { "NEW" })
                } else {
                    format!("shared-{row}\n")
                }
            })
            .collect();
        std::fs::write(path, text).unwrap();
    }
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let manager = ComparatorWindows::new(midnight_compare_engine::Service::new(
        runtime.handle().clone(),
    ));
    let map = Rc::new(RefCell::new(ShortcutMap::new(&[]).unwrap()));
    let keys = map.clone();
    let window = manager
        .open(
            FileCompareLaunch {
                sources: paths.into_iter().map(FileCompareSource::Focused).collect(),
                disabled_plugins: Arc::from([]),
            },
            ViewerAppearance {
                family: "monospace".into(),
                font_size: 14.0,
                foreground: [240, 240, 240, 255],
                background: [20, 20, 20, 255],
                selection: [60, 80, 120, 255],
                accent: [100, 150, 220, 255],
            },
            slint::LogicalSize::new(800.0, 600.0),
            Rc::new(move |chord, control| {
                keys.borrow()
                    .resolve(
                        chord,
                        if control {
                            ShortcutContext::ComparisonSearch
                        } else {
                            ShortcutContext::Comparison
                        },
                    )
                    .map(|action| action.id.clone())
            }),
        )
        .unwrap()
        .upgrade()
        .unwrap();
    native.set_size(slint::PhysicalSize::new(800, 600));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(800.0, 600.0),
    });
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let mut pixels = vec![slint::Rgb8Pixel::default(); 800 * 600];
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 800);
    });
    wait_for(|| window.get_status().contains("3 difference groups"));
    window.invoke_focus_content();
    assert_group(&window, 0);
    chord(&window, Some(Key::Alt), Key::DownArrow.into());
    wait_group(&window, 50);
    chord(&window, Some(Key::Alt), Key::End.into());
    wait_group(&window, 100);
    chord(&window, Some(Key::Alt), Key::DownArrow.into());
    chord(&window, Some(Key::Alt), Key::UpArrow.into());
    wait_group(&window, 50);
    chord(&window, Some(Key::Alt), Key::Home.into());
    wait_group(&window, 0);
    chord(&window, Some(Key::Alt), Key::UpArrow.into());
    chord(&window, None, Key::F7.into());
    wait_group(&window, 50);
    chord(&window, Some(Key::Alt), Key::Home.into());
    wait_group(&window, 0);
    // Two actions dispatched before a timer tick must remain two actions.
    chord(&window, None, Key::F7.into());
    chord(&window, None, Key::F7.into());
    wait_group(&window, 100);
    chord(&window, Some(Key::Shift), Key::F7.into());
    chord(&window, Some(Key::Shift), Key::F7.into());
    wait_group(&window, 0);
    chord(&window, None, Key::DownArrow.into());
    wait_for(|| {
        window
            .get_accessible_text()
            .lines()
            .next()
            .is_some_and(|line| line.contains("shared-1"))
    });
    chord(&window, Some(Key::Alt), Key::DownArrow.into());
    wait_group(&window, 50);
    *map.borrow_mut() = ShortcutMap::new(&[ShortcutBinding {
        action: "compare.next-difference".into(),
        keys: vec!["Ctrl+J".into()],
    }])
    .unwrap();
    chord(&window, Some(Key::Alt), Key::DownArrow.into());
    chord(&window, Some(Key::Alt), Key::UpArrow.into());
    wait_group(&window, 0);
    chord(&window, Some(Key::Control), "j".into());
    wait_group(&window, 50);
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 800);
    });
    assert!(pixels.iter().any(|pixel| pixel.r != 0));
    manager.close_all();
    drop(runtime);
    std::fs::remove_dir_all(directory).unwrap();
}

fn chord(window: &ComparatorWindow, modifier: Option<Key>, text: slint::SharedString) {
    if let Some(modifier) = modifier {
        window.window().dispatch_event(WindowEvent::KeyPressed {
            text: modifier.into(),
        });
    }
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
    if let Some(modifier) = modifier {
        window.window().dispatch_event(WindowEvent::KeyReleased {
            text: modifier.into(),
        });
    }
}
fn wait_group(window: &ComparatorWindow, row: usize) {
    wait_for(|| {
        window
            .get_accessible_text()
            .lines()
            .next()
            .is_some_and(|line| line.contains(&format!("OLD-{row}")))
    });
    assert_group(window, row);
}
fn assert_group(window: &ComparatorWindow, row: usize) {
    let text = window.get_accessible_text();
    let line = text.lines().next().unwrap();
    let (left, right) = line.split_once('\t').unwrap();
    assert!(
        left.ends_with(&format!("OLD-{row}")),
        "Left hunk is not visible: {left}"
    );
    assert!(
        right.ends_with(&format!("NEW-{row}")),
        "Right hunk is not visible: {right}"
    );
}
fn wait_for(done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !done() {
        assert!(
            Instant::now() < deadline,
            "Comparator did not present the requested difference"
        );
        std::thread::sleep(Duration::from_millis(10));
        slint::platform::update_timers_and_animations();
    }
}

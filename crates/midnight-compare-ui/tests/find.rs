//! Real comparison Find keys, result state and clear action through the Slint window.
#[path = "../../midnight-ui/tests/support/bitmap.rs"]
mod bitmap;
#[path = "support/resident.rs"]
mod resident;
#[path = "support/mod.rs"]
mod support;

use i_slint_core::window::PopupWindowLocation;
use midnight_compare_ui::{ComparatorWindow, ComparatorWindows};
use midnight_core::{
    FileCompareLaunch, FileCompareSource, ShortcutContext, ShortcutMap, ViewerAppearance,
};
use slint::{
    ComponentHandle,
    platform::{
        Key, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
    private_unstable_api::re_exports::{
        AccessibilityAction, AccessibleStringProperty, ItemRc, WindowInner, WindowItem, WindowKind,
    },
};
use std::{rc::Rc, sync::Arc};

fn item(root: ItemRc, label: &str) -> Option<ItemRc> {
    if root.is_visible()
        && root
            .accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|value| value == label)
    {
        return Some(root);
    }
    let mut child = root.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        if let Some(found) = item(current, label) {
            return Some(found);
        }
    }
    None
}

fn chord(window: &ComparatorWindow, modifier: Option<Key>, value: slint::SharedString) {
    if let Some(modifier) = modifier {
        window.window().dispatch_event(WindowEvent::KeyPressed {
            text: modifier.into(),
        });
    }
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: value.clone(),
    });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text: value });
    if let Some(modifier) = modifier {
        window.window().dispatch_event(WindowEvent::KeyReleased {
            text: modifier.into(),
        });
    }
}

#[test]
fn find_keys_search_both_files_center_hits_and_clear_stale_results_without_a_close_button() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(support::TestPlatform(native.clone()))).unwrap();
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .join("_build")
        .join("test-data")
        .join(format!("compare-find-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let fixture_directory = directory
        .join("long-directory-文档-first")
        .join("long-directory-second");
    std::fs::create_dir_all(&fixture_directory).unwrap();
    let paths = [
        fixture_directory.join("left.txt"),
        fixture_directory.join("right.txt"),
    ];
    let expected_paths = paths.each_ref().map(|path| path.display().to_string());
    let text: String = (0..120)
        .map(|line| {
            if line == 75 {
                "\tneedle ČESKÝ\n".to_owned()
            } else {
                format!("ordinary line {line}\n")
            }
        })
        .collect();
    for path in &paths {
        std::fs::write(path, &text).unwrap();
    }
    let runtime = resident::TestRuntime::new(tokio::runtime::Runtime::new().unwrap());
    let manager = ComparatorWindows::new(midnight_compare_engine::Service::new(
        runtime.handle().clone(),
        runtime.spawner(),
    ));
    let policy = midnight_core::FileComparatorSnapshot::default();
    let map = ShortcutMap::new(&[]).unwrap();
    let window = manager
        .open(
            FileCompareLaunch {
                sources: paths.into_iter().map(FileCompareSource::Focused).collect(),
                disabled_plugins: Arc::from([]),
                comparators: policy.preferences.clone(),
                comparator_policy: policy.id,
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
                map.resolve(
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
    support::wait_for(&window, "show compared text", || {
        window.get_status().contains("difference groups")
    });
    window.invoke_focus_content();
    chord(&window, Some(Key::Control), "f".into());
    assert!(window.get_search_visible());
    assert!(!window.get_content_focused());
    // Exercise the real field rather than only a Rust callback.
    for character in "needle".chars() {
        chord(&window, None, character.to_string().into());
    }
    assert_eq!(window.get_query(), "needle");
    chord(&window, None, Key::Return.into());
    support::wait_for(&window, "show first Find hit", || {
        window.get_search_summary().contains("Match 1 of 2")
    });
    assert!(window.get_search_summary().contains("File A · line 76"));
    let lines: Vec<_> = window
        .get_accessible_text()
        .lines()
        .map(str::to_owned)
        .collect();
    let hit = lines
        .iter()
        .position(|line| line.contains("needle"))
        .unwrap();
    assert!(
        hit > 2 && hit + 2 < lines.len(),
        "match needs context above and below"
    );
    chord(&window, None, Key::F3.into());
    chord(&window, None, Key::F3.into());
    support::wait_for(
        &window,
        "preserve two immediate relative Find steps",
        || window.get_search_summary().contains("Match 1 of 2"),
    );
    chord(&window, None, Key::F3.into());
    support::wait_for(&window, "show right-pane Find hit", || {
        window.get_search_summary().contains("Match 2 of 2")
    });
    chord(&window, Some(Key::Shift), Key::Return.into());
    support::wait_for(&window, "show previous hit with Shift+Enter", || {
        window.get_search_summary().contains("Match 1 of 2")
    });
    chord(&window, Some(Key::Shift), Key::F3.into());
    support::wait_for(&window, "wrap backwards with Shift+F3", || {
        window.get_search_summary().contains("Match 2 of 2")
    });
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 800);
    });
    let root = ItemRc::new_root(WindowInner::from_pub(window.window()).component());
    assert!(
        item(root.clone(), "Close").is_none(),
        "text comparison needs only its window close control"
    );
    assert!(item(root.clone(), "Whole words").is_some());
    for (width, height, label) in [
        (800, 600, "compare-find-normal"),
        (360, 320, "compare-find-narrow"),
    ] {
        native.set_size(slint::PhysicalSize::new(width, height));
        window.window().dispatch_event(WindowEvent::Resized {
            size: slint::LogicalSize::new(width as f32, height as f32),
        });
        window.set_compact(width < 500);
        window.window().request_redraw();
        let mut render = vec![slint::Rgb8Pixel::default(); (width * height) as usize];
        slint::platform::update_timers_and_animations();
        native.draw_if_needed(|renderer| {
            renderer.render(&mut render, width as usize);
        });
        support::wait_for(&window, "paint the resized Find viewport", || {
            window.get_frame_revision() == window.get_geometry_revision()
        });
        let visible: Vec<_> = window
            .get_accessible_text()
            .lines()
            .map(str::to_owned)
            .collect();
        let selected_row = visible
            .iter()
            .position(|line| line.contains("needle"))
            .expect("resizing must retain the currently visible selected Find hit");
        assert!(
            selected_row > 0 && selected_row + 1 < visible.len(),
            "resized Find hit must keep surrounding context"
        );
        window.window().request_redraw();
        native.draw_if_needed(|renderer| {
            renderer.render(&mut render, width as usize);
        });
        bitmap::snapshot(&render, width as usize, height as usize, label);
    }
    // Inspect real delayed hovers for both captured paths. Width alone is insufficient
    // when an unclamped embedded popup can start near the right edge of its host.
    let mut containment = Vec::new();
    for (side, path) in expected_paths.into_iter().enumerate() {
        window.window().dispatch_event(WindowEvent::PointerExited);
        window.window().request_redraw();
        let mut narrow = vec![slint::Rgb8Pixel::default(); 360 * 320];
        native.draw_if_needed(|renderer| {
            renderer.render(&mut narrow, 360);
        });
        let path_item = item(
            ItemRc::new_root(WindowInner::from_pub(window.window()).component()),
            &path,
        )
        .unwrap();
        let geometry = path_item.geometry();
        let origin = path_item.map_to_native_window(geometry.origin);
        let cursor = slint::LogicalPosition::new(
            origin.x + geometry.size.width / 2.0,
            origin.y + geometry.size.height / 2.0,
        );
        window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position: cursor });
        let before_tip = narrow.clone();
        std::thread::sleep(std::time::Duration::from_millis(650));
        slint::platform::update_timers_and_animations();
        native.draw_if_needed(|renderer| {
            renderer.render(&mut narrow, 360);
        });
        {
            let inner = WindowInner::from_pub(window.window());
            let popups = inner.active_popups();
            let popup = popups
                .iter()
                .find(|popup| matches!(popup.window_kind, WindowKind::ToolTip))
                .expect("actual delayed hover must create the path tooltip");
            let PopupWindowLocation::ChildWindow(origin) = popup.location else {
                panic!("expected embedded tooltip")
            };
            let root = ItemRc::new_root(popup.component.clone());
            let body = root.downcast::<WindowItem>().unwrap();
            let width = body.as_pin_ref().width().get();
            let height = body.as_pin_ref().height().get();
            let text =
                item(root.clone(), &path).expect("the popup must retain the full captured path");
            let text_geometry = text.geometry();
            let text_origin = text.map_to_native_window(text_geometry.origin);
            let window_origin = root.map_to_native_window(root.geometry().origin);
            let local_x = text_origin.x - window_origin.x;
            let local_y = text_origin.y - window_origin.y;
            let content_right = local_x + text_geometry.size.width;
            let content_bottom = local_y + text_geometry.size.height;
            eprintln!(
                "header {side} tooltip cursor={cursor:?} origin={origin:?} size={width}x{height} text-local=({local_x},{local_y}) text-size={}x{} text-host-right={} text-host-bottom={} host=360x320",
                text_geometry.size.width,
                text_geometry.size.height,
                origin.x + content_right,
                origin.y + content_bottom,
            );
            let fits = origin.x >= 7.9
                && origin.y >= 7.9
                && origin.x + width <= 352.1
                && origin.y + height <= 312.1
                && local_x >= -0.1
                && local_y >= -0.1
                && content_right <= width + 0.1
                && content_bottom <= height + 0.1
                && origin.x + content_right <= 352.1
                && origin.y + content_bottom <= 312.1;
            containment.push((side, origin.x, origin.y, width, height, fits));
        }
        assert!(
            before_tip
                .iter()
                .zip(&narrow)
                .filter(|(before, after)| before != after)
                .count()
                > 100,
            "the actual tooltip must paint on the narrow comparison"
        );
        bitmap::snapshot(
            &narrow,
            360,
            320,
            &format!("compare-path-tooltip-narrow-{side}"),
        );
    }
    assert!(
        containment.iter().all(|bounds| bounds.5),
        "header tooltip exceeds its 360x320 host: {containment:?}"
    );
    window.window().dispatch_event(WindowEvent::PointerExited);
    let root = ItemRc::new_root(WindowInner::from_pub(window.window()).component());
    item(root, "Clear Find")
        .unwrap()
        .accessible_action(&AccessibilityAction::Default);
    assert!(window.get_query().is_empty());
    assert!(window.get_search_summary().is_empty());
    // An unread frame from the old search must not bring the count back.
    for _ in 0..8 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        slint::platform::update_timers_and_animations();
    }
    assert!(window.get_search_summary().is_empty());
    chord(&window, None, Key::Escape.into());
    assert!(!window.window().is_visible());
    manager.close_all();
    drop(runtime);
    std::fs::remove_dir_all(directory).unwrap();
}

//! Byte comparison (hex view) in a real window: a binary pair, its differing regions
//! visited by keys and buttons, and the Hex button of a text pair.
use midnight_compare_ui::{ComparatorWindow, ComparatorWindows};
use midnight_core::{
    FileCompareLaunch, FileCompareSource, ShortcutContext, ShortcutMap, ViewerAppearance,
};
use slint::{
    ComponentHandle,
    platform::{
        Key, PointerEventButton, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
    private_unstable_api::re_exports::{AccessibleStringProperty, ItemRc, WindowInner},
};
use std::{path::PathBuf, rc::Rc, sync::Arc};
#[path = "support/resident.rs"]
mod resident;
#[path = "support/mod.rs"]
mod support;
use support::{TestPlatform, wait_for};

const HEX: &str = "Compare bytes (hex)";

struct Directory(PathBuf);

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn directory(name: &str) -> Directory {
    let path = std::env::temp_dir().join(format!("comparator-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir(&path).unwrap();
    Directory(path)
}

fn open(
    native: &Rc<MinimalSoftwareWindow>,
    manager: &ComparatorWindows,
    paths: [PathBuf; 2],
) -> ComparatorWindow {
    let map = ShortcutMap::new(&[]).unwrap();
    let snapshot = midnight_core::FileComparatorSnapshot::default();
    let window = manager
        .open(
            FileCompareLaunch {
                sources: paths.into_iter().map(FileCompareSource::Focused).collect(),
                disabled_plugins: Arc::from([]),
                comparators: snapshot.preferences.clone(),
                comparator_policy: snapshot.id,
            },
            ViewerAppearance {
                family: "monospace".into(),
                font_size: 14.0,
                foreground: [240, 240, 240, 255],
                background: [20, 20, 20, 255],
                selection: [60, 80, 120, 255],
                selection_text: [0; 4],
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
    draw(native);
    window
}

fn draw(native: &MinimalSoftwareWindow) {
    let mut pixels = vec![slint::Rgb8Pixel::default(); 800 * 600];
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 800);
    });
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

fn find(item: ItemRc, label: &str) -> Option<ItemRc> {
    if item
        .accessible_string_property(AccessibleStringProperty::Label)
        .is_some_and(|value| value == label)
        && item.is_visible()
    {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = find(item, label) {
            return Some(found);
        }
    }
    None
}

fn button(window: &ComparatorWindow, label: &str) -> ItemRc {
    find(
        ItemRc::new_root(WindowInner::from_pub(window.window()).component()),
        label,
    )
    .unwrap_or_else(|| panic!("no visible button {label:?}"))
}

fn click(native: &MinimalSoftwareWindow, window: &ComparatorWindow, label: &str) {
    draw(native);
    let item = button(window, label);
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    let position = slint::LogicalPosition::new(
        origin.x + rect.size.width / 2.0,
        origin.y + rect.size.height / 2.0,
    );
    for event in [
        WindowEvent::PointerMoved { position },
        WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        },
        WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        },
    ] {
        window.window().dispatch_event(event);
    }
}

fn first_row(window: &ComparatorWindow) -> String {
    window
        .get_accessible_text()
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned()
}

fn wait_row(window: &ComparatorWindow, offset: &str) {
    wait_for(window, &format!("show the row at {offset}"), || {
        first_row(window).starts_with(offset)
    });
    let row = first_row(window);
    let (left, right) = row.split_once('\t').unwrap();
    assert!(
        right.starts_with(offset),
        "both files show the same offsets: {left:?} {right:?}"
    );
}

#[test]
fn a_binary_pair_is_compared_as_bytes_and_keys_and_buttons_visit_each_region() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let directory = directory("hex-binary");
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    let left: Vec<u8> = (0..8192)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed as u8
        })
        .collect();
    let mut right = left.clone();
    for offset in [100, 4000, 8000] {
        right[offset] ^= 0xff;
    }
    let paths = [
        directory.0.join("binary-a.bin"),
        directory.0.join("binary-b.bin"),
    ];
    std::fs::write(&paths[0], &left).unwrap();
    std::fs::write(&paths[1], &right).unwrap();
    let runtime = resident::TestRuntime::new(tokio::runtime::Runtime::new().unwrap());
    let manager = ComparatorWindows::new(midnight_compare_engine::Service::new(
        runtime.handle().clone(),
        runtime.spawner(),
    ));
    let window = open(&native, &manager, paths);
    wait_for(&window, "summarize the byte comparison", || {
        window
            .get_status()
            .contains("3 differing bytes in 3 regions")
    });
    assert!(window.get_bytes_mode() && window.get_bytes_only());
    assert_eq!(window.get_identity_state(), 3, "Files differ");
    let status = window.get_status();
    assert!(
        status.contains("first difference at 0x0064 (100)") && status.contains("8,192 bytes each"),
        "{status}"
    );
    // An 800-pixel window shows eight bytes per row.
    window.invoke_focus_content();
    chord(&window, None, "n".into());
    wait_row(&window, "0060");
    chord(&window, None, Key::F7.into());
    wait_row(&window, "0FA0");
    chord(&window, Some(Key::Alt), Key::End.into());
    wait_row(&window, "1F40");
    chord(&window, Some(Key::Alt), Key::Home.into());
    wait_row(&window, "0060");
    click(&native, &window, "Next difference");
    wait_row(&window, "0FA0");
    chord(&window, None, "p".into());
    wait_row(&window, "0060");
    chord(&window, None, Key::PageDown.into());
    wait_for(&window, "page down", || {
        !first_row(&window).starts_with("0060")
    });
    // The pair has no text to return to, and no Find.
    let hex = button(&window, HEX);
    assert_eq!(
        hex.accessible_string_property(AccessibleStringProperty::Checked)
            .as_deref(),
        Some("true")
    );
    assert_eq!(
        hex.accessible_string_property(AccessibleStringProperty::Enabled)
            .as_deref(),
        Some("false")
    );
    chord(&window, Some(Key::Control), "f".into());
    assert!(!window.get_search_visible());
    manager.close_all();
    drop(runtime);
}

#[test]
fn the_hex_button_shows_the_bytes_of_a_text_pair_and_returns_to_its_text() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let directory = directory("hex-text");
    let paths = [directory.0.join("left.txt"), directory.0.join("right.txt")];
    std::fs::write(&paths[0], "same\nline\n").unwrap();
    std::fs::write(&paths[1], "same\nLINE\n").unwrap();
    let runtime = resident::TestRuntime::new(tokio::runtime::Runtime::new().unwrap());
    let manager = ComparatorWindows::new(midnight_compare_engine::Service::new(
        runtime.handle().clone(),
        runtime.spawner(),
    ));
    let window = open(&native, &manager, paths);
    wait_for(&window, "compare the text", || {
        window.get_status().contains("Difference 1 of 1")
    });
    assert!(!window.get_bytes_mode());
    click(&native, &window, HEX);
    wait_for(&window, "show the bytes", || {
        window.get_bytes_mode()
            && window
                .get_status()
                .contains("4 differing bytes in 1 region")
    });
    assert!(!window.get_bytes_only());
    assert!(first_row(&window).starts_with("0000  73 61 6D 65 0A"));
    assert!(window.get_content_focused(), "the content keeps the keys");
    click(&native, &window, HEX);
    wait_for(&window, "return to the text", || {
        !window.get_bytes_mode() && window.get_accessible_text().contains("LINE")
    });
    manager.close_all();
    drop(runtime);
}

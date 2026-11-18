//! Shortened comparison paths are marked visibly while their complete names survive.
#[path = "support/mod.rs"]
mod support;
use midnight_compare_ui::ComparatorWindow;
use slint::{
    ComponentHandle,
    platform::{
        WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
    private_unstable_api::re_exports::{
        AccessibleRole, AccessibleStringProperty, ComplexText, ItemRc, SimpleText, WindowInner,
    },
};

fn accessible_text(item: ItemRc, label: &str) -> Option<ItemRc> {
    if item.accessible_role() == AccessibleRole::Text
        && item
            .accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|value| value == label)
    {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = accessible_text(item, label) {
            return Some(found);
        }
    }
    None
}

fn visible_text(item: ItemRc, expected: &str) -> Option<ItemRc> {
    if item.is_visible()
        && (item
            .downcast::<SimpleText>()
            .is_some_and(|text| text.as_pin_ref().text() == expected)
            || item
                .downcast::<ComplexText>()
                .is_some_and(|text| text.as_pin_ref().text() == expected))
    {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = visible_text(item, expected) {
            return Some(found);
        }
    }
    None
}

#[test]
fn comparator_marks_both_shortened_paths_and_keeps_the_full_names() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(support::TestPlatform(native.clone()))).unwrap();
    let window = ComparatorWindow::new().unwrap();
    let paths = [
        format!("C:\\{}\\left.txt", "a-long-directory\\".repeat(20)),
        format!("C:\\{}\\right.txt", "文档-directory\\".repeat(20)),
    ];
    window.set_left_name(paths[0].as_str().into());
    window.set_right_name(paths[1].as_str().into());
    window.show().unwrap();
    let (width, height) = (500, 300);
    native.set_size(slint::PhysicalSize::new(width, height));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(width as f32, height as f32),
    });
    let mut pixels = vec![slint::Rgb8Pixel::default(); (width * height) as usize];
    native.window().request_redraw();
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, width as usize);
    });
    let root = ItemRc::new_root(WindowInner::from_pub(window.window()).component());
    for path in &paths {
        let label = accessible_text(root.clone(), path)
            .expect("comparison paths stay complete for accessibility");
        assert!(
            visible_text(label, "...").is_some(),
            "the shortened path must show its omission marker"
        );
    }
    assert_eq!(window.get_left_name(), paths[0]);
    assert_eq!(window.get_right_name(), paths[1]);
}

//! Find actions use normal control heights without clipping labels or artwork.
#[allow(dead_code)]
#[path = "support/modeless.rs"]
mod modeless;

use i_slint_core::{
    accessibility::AccessibilityAction,
    item_tree::ItemRc,
    items::{AccessibleRole, ComplexText, ImageItem, Orientation, SimpleText},
};
use midnight_core::PreferenceTheme;
use midnight_ui::FindWindow;
use slint::{
    ComponentHandle, SharedString, VecModel,
    platform::{WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{cell::RefCell, rc::Rc};

fn draw(native: &MinimalSoftwareWindow) {
    let (width, height) = (1250, 1000);
    native.set_size(slint::PhysicalSize::new(width, height));
    native.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(width as f32, height as f32),
    });
    slint::platform::update_timers_and_animations();
    let mut pixels = vec![slint::Rgb8Pixel::default(); (width * height) as usize];
    native.window().request_redraw();
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, width as usize);
    });
}

fn descendant(item: ItemRc, matches: &dyn Fn(&ItemRc) -> bool) -> Option<ItemRc> {
    if matches(&item) {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = descendant(item, matches) {
            return Some(found);
        }
    }
    None
}

fn assert_inside(button: &ItemRc, content: &ItemRc, label: &str) {
    let bounds = button.geometry();
    let at = button.map_to_native_window(bounds.origin);
    let content_bounds = content.geometry();
    let content_at = content.map_to_native_window(content_bounds.origin);
    assert!(
        content_at.x >= at.x - 0.5
            && content_at.y >= at.y - 0.5
            && content_at.x + content_bounds.width() <= at.x + bounds.width() + 0.5
            && content_at.y + content_bounds.height() <= at.y + bounds.height() + 0.5,
        "{label} content must fit its clickable bounds: {at:?} {bounds:?}, \
         content={content_at:?} {content_bounds:?}"
    );
}

fn assert_action(
    native: &Rc<MinimalSoftwareWindow>,
    label: &str,
    normal_height: f32,
    icon_size: f32,
    exact_height: bool,
    case: &str,
) {
    let button = modeless::accessible_item(native, AccessibleRole::Button, label)
        .unwrap_or_else(|| panic!("missing {label}: {case}"));
    assert!(button.is_visible(), "{label}: {case}");
    let height = button.geometry().height();
    assert!(
        height + 0.5 >= normal_height,
        "{label}: {case}, height={height}"
    );
    if exact_height {
        assert!(
            (height - normal_height).abs() <= 1.0,
            "{label} must use the normal control height: {case}, \
             height={height}, expected={normal_height}"
        );
    }
    let caption = descendant(button.clone(), &|item| {
        item.downcast::<SimpleText>()
            .is_some_and(|text| text.as_pin_ref().text() == label)
            || item
                .downcast::<ComplexText>()
                .is_some_and(|text| text.as_pin_ref().text() == label)
    })
    .unwrap_or_else(|| panic!("missing painted caption for {label}: {case}"));
    let adapter: Rc<dyn WindowAdapter> = native.clone();
    for orientation in [Orientation::Horizontal, Orientation::Vertical] {
        let required = caption.borrow().as_ref().layout_info(
            orientation,
            caption.geometry().width(),
            &adapter,
            &caption,
        );
        let available = match orientation {
            Orientation::Horizontal => caption.geometry().width(),
            Orientation::Vertical => caption.geometry().height(),
        };
        assert!(
            available + 0.5 >= required.preferred,
            "{label} caption must not be clipped: {case}, \
             available={available}, required={}",
            required.preferred
        );
    }
    assert_inside(&button, &caption, label);
    let icon = descendant(button.clone(), &|item| {
        item.is_visible() && item.downcast::<ImageItem>().is_some()
    })
    .unwrap_or_else(|| panic!("missing {label} icon: {case}"));
    assert!((icon.geometry().width() - icon_size).abs() <= 0.5);
    assert!((icon.geometry().height() - icon_size).abs() <= 0.5);
    assert_inside(&button, &icon, label);
}

#[test]
fn find_actions_match_normal_controls_and_preserve_scaled_content() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let window = FindWindow::new().unwrap();
    window.set_roots(
        Rc::new(VecModel::from(vec![SharedString::from(
            r"D:\fixtures\資料",
        )]))
        .into(),
    );
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    for theme in [PreferenceTheme::Dark, PreferenceTheme::Light] {
        for (density, desktop, touch, normal_height, icon_size, exact_height) in [
            (0, 12, 15, 24.0, 14.0, true),
            (1, 12, 15, 40.0, 20.0, true),
            (0, 24, 24, 34.0, 14.0, false),
            (1, 24, 24, 40.0, 20.0, false),
        ] {
            let case = format!("{theme:?}, density={density}, fonts={desktop}/{touch}");
            window.invoke_apply_preferences(theme.index(), density, "".into(), desktop, touch);
            window.set_show_details(false);
            draw(&native);
            let folders =
                modeless::accessible_item(&native, AccessibleRole::Combobox, "Search folders")
                    .unwrap();
            assert!((folders.geometry().height() - normal_height).abs() <= 0.5);
            let check = |label| {
                assert_action(
                    &native,
                    label,
                    normal_height,
                    icon_size,
                    exact_height,
                    &case,
                );
            };
            for label in [
                "Remove",
                "More options",
                "Find",
                "Go to file",
                "View",
                "Edit",
                "Open",
                "Details",
                "Copy details",
                "Close",
            ] {
                check(label);
            }
            modeless::accessible_item(&native, AccessibleRole::Button, "More options")
                .unwrap()
                .accessible_action(&AccessibilityAction::Default);
            draw(&native);
            check("Add");
            check("Fewer options");
            modeless::accessible_item(&native, AccessibleRole::Button, "Fewer options")
                .unwrap()
                .accessible_action(&AccessibilityAction::Default);
            window.set_running(true);
            draw(&native);
            check("Remove");
            check("Stop");
            window.set_running(false);
            window.set_show_details(true);
            draw(&native);
            for label in ["Copy", "Query", "Copy details", "Close"] {
                check(label);
            }
        }
    }
    window.hide().unwrap();
}

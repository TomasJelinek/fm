//! Filter-specific input and layout boundaries across supported presentation modes.
#[path = "support/window_input.rs"]
mod input;
#[path = "support/modeless.rs"]
#[allow(dead_code)]
mod modeless;

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, ComplexText, Flickable, SimpleText},
    window::WindowInner,
};
use midnight_ui::{
    HelpWindow, MainWindow, PanelFilterHistoryItem, PanelFilterView, PanelFilterWindow,
};
use slint::{
    ComponentHandle,
    platform::{Key, WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{cell::RefCell, rc::Rc};

type Windows = Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>;

fn platform() -> Windows {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    windows
}

fn model() -> PanelFilterView {
    PanelFilterView {
        open: true,
        enabled: true,
        expression: "*.old".into(),
        history: Rc::new(slint::VecModel::from(vec![
            PanelFilterHistoryItem {
                label: "Masks: *.txt".into(),
                mode: 0,
                expression: "*.txt".into(),
                case_sensitive: false,
            },
            PanelFilterHistoryItem {
                label: "Expression (case sensitive): name like \"文档*\"".into(),
                mode: 2,
                expression: "name like \"文档*\"".into(),
                case_sensitive: true,
            },
        ]))
        .into(),
        history_labels: Rc::new(slint::VecModel::from(vec![
            "Choose a recent filter…".into(),
            "Masks: *.txt".into(),
            "Expression (case sensitive): name like \"文档*\"".into(),
        ]))
        .into(),
        ..Default::default()
    }
}

fn form(
    windows: &Windows,
    scheme: i32,
    density: i32,
) -> (PanelFilterWindow, Rc<MinimalSoftwareWindow>) {
    let window = PanelFilterWindow::new().unwrap();
    window.invoke_apply_preferences(scheme, density, "Segoe UI".into(), 12, 15);
    window.set_model(model());
    let weak = window.as_weak();
    window.on_request_dismiss(move || weak.upgrade().unwrap().hide().unwrap());
    window.show().unwrap();
    let native = windows.borrow().last().unwrap().clone();
    modeless::draw(&native, 540, 360, "filter-qualification-initial");
    window.invoke_begin();
    native
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    (window, native)
}

fn inside(native: &MinimalSoftwareWindow, role: AccessibleRole, label: &str) -> ItemRc {
    let item = modeless::accessible_item(native, role, label)
        .unwrap_or_else(|| panic!("Missing filter control: {label}"));
    let rect = item.geometry();
    let at = item.map_to_native_window(rect.origin);
    let size = native.window().size();
    assert!(
        item.is_visible()
            && rect.width() > 0.0
            && rect.height() > 0.0
            && at.x >= -0.5
            && at.y >= -0.5
            && at.x + rect.width() <= size.width as f32 + 0.5
            && at.y + rect.height() <= size.height as f32 + 0.5,
        "{label} clipped in {size:?}: {at:?}, {rect:?}"
    );
    let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(ancestor) = parent {
        if ancestor.downcast::<Flickable>().is_some() {
            let clip = ancestor.geometry();
            let origin = ancestor.map_to_native_window(clip.origin);
            assert!(
                at.x >= origin.x - 0.5
                    && at.y >= origin.y - 0.5
                    && at.x + rect.width() <= origin.x + clip.width() + 0.5
                    && at.y + rect.height() <= origin.y + clip.height() + 0.5,
                "{label} is hidden by a scroll viewport"
            );
        }
        parent = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    item
}

fn property(
    native: &MinimalSoftwareWindow,
    role: AccessibleRole,
    label: &str,
    property: AccessibleStringProperty,
) -> String {
    modeless::accessible_item(native, role, label)
        .unwrap()
        .accessible_string_property(property)
        .unwrap()
        .to_string()
}

#[test]
fn filter_controls_and_footer_remain_reachable_at_desktop_12px_and_touch_in_both_themes() {
    let windows = platform();
    for scheme in [0, 17] {
        for density in [0, 1] {
            let (window, native) = form(&windows, scheme, density);
            for (width, height) in [(540, 360), (320, 480)] {
                modeless::draw(
                    &native,
                    width,
                    height,
                    &format!("filter-layout-{scheme}-{density}"),
                );
                for label in ["Filter history", "Filter mode"] {
                    inside(&native, AccessibleRole::Combobox, label);
                }
                let expression =
                    inside(&native, AccessibleRole::TextInput, "File filter expression");
                let clear = inside(&native, AccessibleRole::Button, "Clear filter expression");
                inside(&native, AccessibleRole::Checkbox, "Case sensitive");
                for label in ["Help", "Show all", "OK", "Cancel"] {
                    inside(&native, AccessibleRole::Button, label);
                }
                let center = |item: &ItemRc| {
                    item.map_to_native_window(item.geometry().origin).y
                        + item.geometry().height() / 2.0
                };
                assert!(
                    (center(&expression) - center(&clear)).abs() <= 1.0,
                    "draft and Clear centers differ"
                );
                if width == 540 {
                    let ok = inside(&native, AccessibleRole::Button, "OK");
                    let cancel = inside(&native, AccessibleRole::Button, "Cancel");
                    assert!(
                        (center(&ok) - center(&cancel)).abs() <= 1.0,
                        "normal footer is misaligned"
                    );
                }
            }
            modeless::draw(
                &native,
                320,
                200,
                &format!("filter-minimum-{scheme}-{density}"),
            );
            inside(&native, AccessibleRole::Button, "Cancel");
            input::key(&native, Key::Escape);
            assert!(
                !window.window().is_visible(),
                "minimum filter must accept Escape without pointer repair"
            );
        }
    }
}

#[test]
fn history_recall_and_draft_clear_preserve_applied_filter_and_busy_cancellation() {
    let windows = platform();
    for scheme in [0, 17] {
        for density in [0, 1] {
            let (window, native) = form(&windows, scheme, density);
            let applies = Rc::new(RefCell::new(Vec::new()));
            let out = applies.clone();
            window.on_apply(move |enabled, mode, expression, case| {
                out.borrow_mut()
                    .push((enabled, mode, expression.to_string(), case));
            });
            let clears = Rc::new(RefCell::new(0));
            let out = clears.clone();
            window.on_clear(move || *out.borrow_mut() += 1);
            modeless::click(&native, "Clear filter expression");
            assert_eq!(
                property(
                    &native,
                    AccessibleRole::TextInput,
                    "File filter expression",
                    AccessibleStringProperty::Value
                ),
                ""
            );
            assert_eq!(
                property(
                    &native,
                    AccessibleRole::Button,
                    "Clear filter expression",
                    AccessibleStringProperty::Enabled
                ),
                "false"
            );
            assert!(window.get_model().enabled);
            assert_eq!(
                *clears.borrow(),
                0,
                "draft Clear must not remove an applied filter"
            );
            input::key(&native, "*.文档");
            assert_eq!(
                property(
                    &native,
                    AccessibleRole::Button,
                    "Clear filter expression",
                    AccessibleStringProperty::Enabled
                ),
                "true"
            );
            let history = inside(&native, AccessibleRole::Combobox, "Filter history");
            input::click(&native, &history);
            input::key(&native, Key::DownArrow);
            input::key(&native, Key::DownArrow);
            input::key(&native, Key::Return);
            assert!(
                applies.borrow().is_empty(),
                "confirming history must only fill the draft"
            );
            assert_eq!(
                property(
                    &native,
                    AccessibleRole::TextInput,
                    "File filter expression",
                    AccessibleStringProperty::Value
                ),
                "name like \"文档*\""
            );
            let mut status = window.get_model();
            status.busy = true;
            window.set_model(status);
            modeless::draw(
                &native,
                540,
                360,
                &format!("filter-validating-{scheme}-{density}"),
            );
            for (role, label) in [
                (AccessibleRole::Combobox, "Filter history"),
                (AccessibleRole::Combobox, "Filter mode"),
                (AccessibleRole::TextInput, "File filter expression"),
                (AccessibleRole::Checkbox, "Case sensitive"),
                (AccessibleRole::Button, "Clear filter expression"),
                (AccessibleRole::Button, "OK"),
            ] {
                assert_eq!(
                    property(&native, role, label, AccessibleStringProperty::Enabled),
                    "false",
                    "{label} edits a validating draft"
                );
            }
            modeless::click(&native, "OK");
            assert!(applies.borrow().is_empty());
            modeless::click(&native, "Show all");
            assert_eq!(
                *clears.borrow(),
                1,
                "Show all remains available to cancel pending validation"
            );
            let mut status = window.get_model();
            status.busy = false;
            window.set_model(status);
            modeless::draw(
                &native,
                540,
                360,
                &format!("filter-recalled-{scheme}-{density}"),
            );
            modeless::click(&native, "OK");
            assert_eq!(
                applies.borrow().as_slice(),
                [(true, 2, "name like \"文档*\"".to_owned(), true)]
            );
            input::key(&native, Key::Escape);
            assert!(!window.window().is_visible());
        }
    }
}

#[test]
fn applied_filter_indicator_and_open_action_belong_to_each_panel_even_when_compact() {
    let windows = platform();
    let window = MainWindow::new().unwrap();
    window.invoke_apply_fonts("Segoe UI".into(), 12, 15);
    window.show().unwrap();
    let native = windows.borrow().last().unwrap().clone();
    let opened = Rc::new(RefCell::new(Vec::new()));
    let out = opened.clone();
    window.on_open_panel_filter(move |pane| out.borrow_mut().push(pane));
    for scheme in [0, 17] {
        for density in [0, 1] {
            window.invoke_apply_preferences(scheme, density);
            window.set_left_filter_label("Filter: *.rs".into());
            window.set_right_filter_label("".into());
            modeless::draw(
                &native,
                1280,
                720,
                &format!("filter-indicators-{scheme}-{density}"),
            );
            assert_eq!(
                property(
                    &native,
                    AccessibleRole::Button,
                    "Left panel filter",
                    AccessibleStringProperty::Checked
                ),
                "true"
            );
            assert_eq!(
                property(
                    &native,
                    AccessibleRole::Button,
                    "Right panel filter",
                    AccessibleStringProperty::Checked
                ),
                "false"
            );
            for (pane, label) in [(0, "Left panel filter"), (1, "Right panel filter")] {
                inside(&native, AccessibleRole::Button, label);
                modeless::click(&native, label);
                assert_eq!(opened.borrow().last(), Some(&pane));
                window.set_active_pane(pane);
                modeless::draw(
                    &native,
                    320,
                    200,
                    &format!("filter-indicator-compact-{scheme}-{density}-{pane}"),
                );
                inside(&native, AccessibleRole::Button, label);
                modeless::click(&native, label);
                assert_eq!(opened.borrow().last(), Some(&pane));
                modeless::draw(&native, 1280, 720, "filter-indicator-restored");
            }
            window.set_left_filter_label("".into());
            window.set_right_filter_label("Filter: expression kind = file".into());
            assert_eq!(
                property(
                    &native,
                    AccessibleRole::Button,
                    "Left panel filter",
                    AccessibleStringProperty::Checked
                ),
                "false"
            );
            assert_eq!(
                property(
                    &native,
                    AccessibleRole::Button,
                    "Right panel filter",
                    AccessibleStringProperty::Checked
                ),
                "true"
            );
        }
    }
    window.hide().unwrap();
}

fn help_text(item: ItemRc) -> Option<ItemRc> {
    let text = item
        .downcast::<SimpleText>()
        .map(|text| text.as_pin_ref().text())
        .or_else(|| {
            item.downcast::<ComplexText>()
                .map(|text| text.as_pin_ref().text())
        });
    if text.is_some_and(|text| text.contains("EXPRESSION")) {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = help_text(item) {
            return Some(found);
        }
    }
    None
}

#[test]
fn expression_help_wraps_and_scrolls_to_the_end_without_losing_close() {
    let windows = platform();
    for scheme in [0, 17] {
        for density in [0, 1] {
            for (width, height) in [(740, 600), (320, 200)] {
                let window = HelpWindow::new().unwrap();
                window.invoke_apply_preferences(scheme, density, "Segoe UI".into(), 12, 15);
                window.set_topic(1);
                let weak = window.as_weak();
                window.on_request_dismiss(move || weak.upgrade().unwrap().hide().unwrap());
                window.show().unwrap();
                let native = windows.borrow().last().unwrap().clone();
                modeless::draw(
                    &native,
                    width,
                    height,
                    &format!("filter-help-start-{scheme}-{density}"),
                );
                inside(&native, AccessibleRole::Button, "Close");
                let root = ItemRc::new_root(WindowInner::from_pub(window.window()).component());
                let text = help_text(root).unwrap();
                let start = text.map_to_native_window(text.geometry().origin);
                let mut parent = text.parent_item(ParentItemTraversalMode::StopAtPopups);
                let clip = loop {
                    let item = parent.expect("Filter help text must have a scroll viewport");
                    if item.downcast::<Flickable>().is_some() {
                        break item;
                    }
                    parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
                };
                let clip_at = clip.map_to_native_window(clip.geometry().origin);
                assert!(
                    text.geometry().width() <= clip.geometry().width(),
                    "Help text must wrap to its viewport"
                );
                native
                    .window()
                    .dispatch_event(WindowEvent::PointerScrolled {
                        position: slint::LogicalPosition::new(clip_at.x + 4.0, clip_at.y + 4.0),
                        delta_x: 0.0,
                        delta_y: -100_000.0,
                    });
                modeless::draw(
                    &native,
                    width,
                    height,
                    &format!("filter-help-end-{scheme}-{density}"),
                );
                let end = text.map_to_native_window(text.geometry().origin);
                assert!(
                    end.y < start.y,
                    "Expression help must scroll beyond its first page"
                );
                assert!(
                    end.y + text.geometry().height() <= clip_at.y + clip.geometry().height() + 1.0,
                    "The last help paragraph must be reachable"
                );
                inside(&native, AccessibleRole::Button, "Close");
                input::key(&native, Key::Escape);
                assert!(!window.window().is_visible());
            }
        }
    }
}

//! Form-only checks complement the installed native callback tests.
use super::{key, modeless, replace, value};
use i_slint_core::{accessibility::AccessibleStringProperty, items::AccessibleRole};
use midnight_ui::{HotPathsWindow, PanelFilterView, PanelFilterWindow};
use slint::{
    ComponentHandle,
    platform::{PointerEventButton, WindowAdapter, WindowEvent},
};
use std::{cell::RefCell, rc::Rc};
struct Form {
    window: PanelFilterWindow,
    native: Rc<slint::platform::software_renderer::MinimalSoftwareWindow>,
}
impl Form {
    fn new(model: PanelFilterView, width: u32, height: u32, density: i32, font: i32) -> Self {
        let windows = Rc::new(RefCell::new(Vec::new()));
        slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
        let window = PanelFilterWindow::new().unwrap();
        window.invoke_apply_preferences(0, density, "".into(), font, font);
        window.set_model(model);
        let weak = window.as_weak();
        window.on_request_dismiss(move || {
            if let Some(window) = weak.upgrade() {
                window.hide().unwrap();
            }
        });
        window.show().unwrap();
        let native = windows.borrow()[0].clone();
        modeless::draw(&native, width, height, "native-filter-form");
        window.invoke_begin();
        native
            .window()
            .dispatch_event(WindowEvent::WindowActiveChanged(true));
        Self { window, native }
    }
    fn feedback(&self) {
        let mut model = self.window.get_model();
        model.error = "Validation feedback preserves the typed draft.".into();
        self.window.set_model(model);
    }
}
pub(super) fn click_control(
    native: &slint::platform::software_renderer::MinimalSoftwareWindow,
    label: &str,
    role: AccessibleRole,
) {
    let item = modeless::accessible_item(native, role, label).unwrap();
    let bounds = item.geometry();
    let origin = item.map_to_native_window(bounds.origin);
    let position = slint::LogicalPosition::new(
        origin.x + bounds.width() / 2.,
        origin.y + bounds.height() / 2.,
    );
    native.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    native
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
}
fn action_visible(form: &Form, name: &str) {
    let item = modeless::accessible_item(&form.native, AccessibleRole::Button, name).unwrap();
    let rect = item.geometry();
    let pos = item.map_to_native_window(rect.origin);
    let size = form.native.window().size();
    assert!(
        pos.x >= -0.5
            && pos.y >= -0.5
            && pos.x + rect.width() <= size.width as f32 + 0.5
            && pos.y + rect.height() <= size.height as f32 + 0.5,
        "{name} outside native form"
    );
}
#[test]
fn native_filter_first_open_at_minimum_large_fonts_accepts_escape_without_pointer_repair() {
    let form = Form::new(
        PanelFilterView {
            open: true,
            pane: 1,
            expression: "*.český;*.世界".into(),
            case_sensitive: true,
            ..Default::default()
        },
        320,
        200,
        1,
        32,
    );
    assert_eq!(value(&form.native), "*.český;*.世界");
    assert_eq!(
        modeless::accessible_item(&form.native, AccessibleRole::Checkbox, "Case sensitive")
            .unwrap()
            .accessible_string_property(AccessibleStringProperty::Checked)
            .unwrap(),
        "true"
    );
    action_visible(&form, "Cancel");
    key(&form.native, slint::platform::Key::Escape);
    assert!(!form.native.window().is_visible());
}
#[test]
fn native_filter_ok_enables_draft_and_show_all_is_a_separate_action() {
    let form = Form::new(
        PanelFilterView {
            open: true,
            pane: 1,
            expression: "*.rs".into(),
            ..Default::default()
        },
        540,
        360,
        0,
        11,
    );
    let calls = Rc::new(RefCell::new(Vec::new()));
    let out = calls.clone();
    form.window.on_apply(move |enabled, mode, text, case| {
        out.borrow_mut().push((enabled, mode, text, case))
    });
    let clears = Rc::new(RefCell::new(0));
    let out = clears.clone();
    form.window.on_clear(move || *out.borrow_mut() += 1);
    modeless::click(&form.native, "OK");
    assert_eq!(calls.borrow().as_slice(), [(true, 0, "*.rs".into(), false)]);
    let mut model = form.window.get_model();
    model.enabled = true;
    form.window.set_model(model);
    modeless::draw(&form.native, 540, 360, "native-filter-clear");
    modeless::click(&form.native, "Show all");
    assert_eq!(*clears.borrow(), 1);
    assert_eq!(
        calls.borrow().len(),
        1,
        "Show all must not submit validation"
    );
}
#[test]
fn native_filter_feedback_and_resizing_preserve_expression_mode_and_edits() {
    let form = Form::new(
        PanelFilterView {
            open: true,
            pane: 1,
            enabled: true,
            expression: "kind = file and size >= 10MiB".into(),
            ..Default::default()
        },
        1040,
        820,
        0,
        11,
    );
    let calls = Rc::new(RefCell::new(Vec::new()));
    let out = calls.clone();
    form.window.on_apply(move |enabled, mode, text, case| {
        out.borrow_mut().push((enabled, mode, text, case))
    });
    click_control(&form.native, "Filter mode", AccessibleRole::Combobox);
    key(&form.native, slint::platform::Key::DownArrow);
    key(&form.native, slint::platform::Key::DownArrow);
    key(&form.native, slint::platform::Key::Return);
    assert!(calls.borrow().is_empty(), "selecting mode never applies");
    form.feedback();
    modeless::draw(&form.native, 320, 200, "native-expression-compact");
    action_visible(&form, "Cancel");
    modeless::click(&form.native, "OK");
    assert_eq!(calls.borrow().last().unwrap().1, 2);
    assert_eq!(
        calls.borrow().last().unwrap().2,
        "kind = file and size >= 10MiB"
    );
    for density in [0, 1] {
        for font in [11, 32] {
            form.window
                .invoke_apply_preferences(0, density, "".into(), font, font);
            modeless::draw(&form.native, 1040, 820, "native-filter-large");
            click_control(
                &form.native,
                "File filter expression",
                AccessibleRole::TextInput,
            );
            replace(&form.native, "*.rs;*.md");
            form.feedback();
            modeless::draw(&form.native, 1040, 820, "native-filter-feedback");
            for label in ["OK", "Cancel", "Show all"] {
                action_visible(&form, label);
            }
            assert_eq!(value(&form.native), "*.rs;*.md");
            modeless::click(&form.native, "OK");
            assert_eq!(calls.borrow().last().unwrap().2, "*.rs;*.md");
        }
    }
    key(&form.native, slint::platform::Key::Escape);
    assert!(!form.native.window().is_visible());
}
#[test]
fn native_hotpaths_first_open_at_minimum_large_fonts_accepts_escape() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let window = HotPathsWindow::new().unwrap();
    window.invoke_apply_preferences(0, 1, "".into(), 32, 32);
    let weak = window.as_weak();
    window.on_request_dismiss(move || weak.upgrade().unwrap().hide().unwrap());
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    modeless::draw(&native, 320, 200, "native-hotpaths-first-minimum");
    window.invoke_begin();
    key(&native, slint::platform::Key::Escape);
    assert!(!native.window().is_visible());
}

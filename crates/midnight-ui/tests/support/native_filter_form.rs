//! Form-only checks of the shipped embedded Filter and Hot Paths hosts, driven through
//! their presentation globals; the installed adapter tests cover callbacks and identity.
use super::{
    embedded_form::{self, Host},
    key, modeless, replace, value,
};
use i_slint_core::{accessibility::AccessibleStringProperty, items::AccessibleRole};
use midnight_ui::{HotPathsPresentation, PanelFilterPresentation, PanelFilterView};
use slint::{
    ComponentHandle,
    platform::{PointerEventButton, WindowAdapter, WindowEvent},
};
use std::{cell::RefCell, rc::Rc};
struct Form {
    host: Host,
}
impl Form {
    /// Open the embedded Filter on a `width`×`height` main window; Cancel and Escape
    /// close it as the adapter does.
    fn new(model: PanelFilterView, width: u32, height: u32, density: i32, font: i32) -> Self {
        let host = Host::new();
        host.apply_preferences(0, density, "", font, font);
        let presentation = host.main.global::<PanelFilterPresentation>();
        presentation.set_model(model);
        let weak = host.main.as_weak();
        presentation.on_dismiss(move |_| {
            if let Some(main) = weak.upgrade() {
                main.global::<PanelFilterPresentation>()
                    .set_token("".into());
            }
        });
        presentation.set_token("filter".into());
        host.draw(width, height, "embedded-filter-form");
        Self { host }
    }
    fn presentation(&self) -> PanelFilterPresentation<'_> {
        self.host.main.global::<PanelFilterPresentation>()
    }
    fn feedback(&self) {
        let mut model = self.presentation().get_model();
        model.error = "Validation feedback preserves the typed draft.".into();
        self.presentation().set_model(model);
    }
}
pub(super) fn click_control(
    native: &slint::platform::software_renderer::MinimalSoftwareWindow,
    label: &str,
    role: AccessibleRole,
) {
    let item = modeless::accessible_popup_item(native, role, label)
        .or_else(|| modeless::accessible_item(native, role, label))
        .unwrap();
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
    embedded_form::assert_inside(&form.host.item(AccessibleRole::Button, name), name);
}
#[test]
fn embedded_filter_first_open_at_minimum_large_fonts_accepts_escape_without_pointer_repair() {
    let form = Form::new(
        PanelFilterView {
            open: true,
            pane: 1,
            expression: "*.český;*.世界".into(),
            case_sensitive: true,
            ..Default::default()
        },
        // A 320×200 dialog body inside the minimum frame margin.
        328,
        208,
        1,
        32,
    );
    let native = &form.host.native;
    assert_eq!(value(native), "*.český;*.世界");
    assert_eq!(
        form.host
            .item(AccessibleRole::Checkbox, "Case sensitive")
            .accessible_string_property(AccessibleStringProperty::Checked)
            .unwrap(),
        "true"
    );
    action_visible(&form, "Cancel");
    key(native, slint::platform::Key::Escape);
    assert!(!form.host.is_open());
}
#[test]
fn embedded_filter_ok_enables_draft_and_show_all_is_a_separate_action() {
    // A 540 px dialog body inside the main window's 12 px frame margin.
    let form = Form::new(
        PanelFilterView {
            open: true,
            pane: 1,
            expression: "*.rs".into(),
            ..Default::default()
        },
        564,
        384,
        0,
        11,
    );
    let native = &form.host.native;
    let calls = Rc::new(RefCell::new(Vec::new()));
    let out = calls.clone();
    form.presentation()
        .on_apply(move |_, enabled, mode, text, case| {
            out.borrow_mut().push((enabled, mode, text, case))
        });
    let clears = Rc::new(RefCell::new(0));
    let out = clears.clone();
    form.presentation()
        .on_clear(move |_| *out.borrow_mut() += 1);
    modeless::click(native, "OK");
    assert_eq!(calls.borrow().as_slice(), [(true, 0, "*.rs".into(), false)]);
    let mut model = form.presentation().get_model();
    model.enabled = true;
    form.presentation().set_model(model);
    form.host.draw(564, 384, "embedded-filter-clear");
    modeless::click(native, "Show all");
    assert_eq!(*clears.borrow(), 1);
    assert_eq!(
        calls.borrow().len(),
        1,
        "Show all must not submit validation"
    );
}
#[test]
fn embedded_filter_feedback_and_resizing_preserve_expression_mode_and_edits() {
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
    let native = &form.host.native;
    let calls = Rc::new(RefCell::new(Vec::new()));
    let out = calls.clone();
    form.presentation()
        .on_apply(move |_, enabled, mode, text, case| {
            out.borrow_mut().push((enabled, mode, text, case))
        });
    click_control(native, "Filter mode", AccessibleRole::Combobox);
    key(native, slint::platform::Key::DownArrow);
    key(native, slint::platform::Key::DownArrow);
    key(native, slint::platform::Key::Return);
    assert!(calls.borrow().is_empty(), "selecting mode never applies");
    form.feedback();
    form.host.draw(328, 208, "embedded-expression-compact");
    action_visible(&form, "Cancel");
    modeless::click(native, "OK");
    assert_eq!(calls.borrow().last().unwrap().1, 2);
    assert_eq!(
        calls.borrow().last().unwrap().2,
        "kind = file and size >= 10MiB"
    );
    for density in [0, 1] {
        for font in [11, 32] {
            form.host.apply_preferences(0, density, "", font, font);
            form.host.draw(1040, 820, "embedded-filter-large");
            click_control(native, "File filter expression", AccessibleRole::TextInput);
            replace(native, "*.rs;*.md");
            form.feedback();
            form.host.draw(1040, 820, "embedded-filter-feedback");
            for label in ["OK", "Cancel", "Show all"] {
                action_visible(&form, label);
            }
            assert_eq!(value(native), "*.rs;*.md");
            modeless::click(native, "OK");
            assert_eq!(calls.borrow().last().unwrap().2, "*.rs;*.md");
        }
    }
    key(native, slint::platform::Key::Escape);
    assert!(!form.host.is_open());
}
#[test]
fn embedded_hotpaths_first_open_at_minimum_large_fonts_accepts_escape() {
    let host = Host::new();
    host.apply_preferences(0, 1, "", 32, 32);
    let presentation = host.main.global::<HotPathsPresentation>();
    let weak = host.main.as_weak();
    presentation.on_dismiss(move |_| {
        if let Some(main) = weak.upgrade() {
            main.global::<HotPathsPresentation>().set_token("".into());
        }
    });
    presentation.set_token("hot-paths".into());
    host.draw(328, 208, "embedded-hotpaths-first-minimum");
    key(&host.native, slint::platform::Key::Escape);
    assert!(!host.is_open());
}

#[test]
fn embedded_filter_history_restores_mode_case_and_text_without_applying_and_clear_edits_only() {
    use midnight_ui::PanelFilterHistoryItem;
    let item = PanelFilterHistoryItem {
        label: "Regex (case sensitive): \\.TXT$".into(),
        mode: 1,
        expression: r"\.TXT$".into(),
        case_sensitive: true,
    };
    let form = Form::new(
        PanelFilterView {
            open: true,
            expression: "*.old".into(),
            history: slint::ModelRc::new(slint::VecModel::from(vec![item.clone()])),
            history_labels: slint::ModelRc::new(slint::VecModel::from(vec![item.label])),
            ..Default::default()
        },
        564,
        384,
        0,
        11,
    );
    let native = &form.host.native;
    let calls = Rc::new(RefCell::new(Vec::new()));
    let out = calls.clone();
    form.presentation()
        .on_apply(move |_, enabled, mode, expression, case| {
            out.borrow_mut().push((enabled, mode, expression, case));
        });
    let clears = Rc::new(RefCell::new(0));
    let out = clears.clone();
    form.presentation()
        .on_clear(move |_| *out.borrow_mut() += 1);
    click_control(native, "Filter history", AccessibleRole::Combobox);
    key(native, slint::platform::Key::Return);
    assert_eq!(value(native), r"\.TXT$");
    assert!(calls.borrow().is_empty());
    form.feedback();
    form.host.draw(564, 384, "embedded-filter-history");
    for label in ["Help", "OK", "Cancel"] {
        action_visible(&form, label);
    }
    let ok = form.host.item(AccessibleRole::Button, "OK");
    let cancel = form.host.item(AccessibleRole::Button, "Cancel");
    assert!(
        (ok.map_to_native_window(ok.geometry().origin).y
            - cancel.map_to_native_window(cancel.geometry().origin).y)
            .abs()
            < 1.,
        "normal-width footer actions must share one row"
    );
    modeless::click(native, "OK");
    assert_eq!(
        calls.borrow().as_slice(),
        [(true, 1, r"\.TXT$".into(), true)]
    );
    modeless::click(native, "Clear filter expression");
    assert_eq!(value(native), "");
    assert_eq!(
        *clears.borrow(),
        0,
        "clearing text must not clear the applied panel filter"
    );
    assert_eq!(calls.borrow().len(), 1);
}

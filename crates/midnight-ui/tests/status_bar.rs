//! Filename priority and exact clipboard identities at real panel widths.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::ItemRc,
    items::AccessibleRole,
    window::{WindowInner, WindowKind},
};
use midnight_ui::DetailRow;
use slint::{
    ComponentHandle, ModelRc, VecModel,
    platform::{Key, WindowEvent},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

fn status_fields(name: &str) -> ModelRc<DetailRow> {
    ModelRc::new(VecModel::from(vec![
        DetailRow {
            identity: "captured-name".into(),
            label: "Name".into(),
            value: name.into(),
            ..Default::default()
        },
        DetailRow {
            identity: "captured-size".into(),
            label: "Bytes".into(),
            value: "32".into(),
            ..Default::default()
        },
    ]))
}

#[test]
fn control_click_status_filename_requests_full_path_and_other_fields_keep_their_value() {
    let mut test = ui::TestUi::new();
    test.window.set_left_details(status_fields("文档.txt"));
    test.draw();
    let copied = Rc::new(RefCell::new(Vec::new()));
    let received = copied.clone();
    test.window.on_detail_copy(move |pane, identity| {
        received.borrow_mut().push((pane, identity.to_string()));
    });
    let name = "Name: 文档.txt. Copy to clipboard";
    ui::click_root(&test.window, name, AccessibleRole::Button);
    test.window
        .window()
        .dispatch_event(WindowEvent::KeyPressed {
            text: Key::Control.into(),
        });
    ui::click_root(&test.window, name, AccessibleRole::Button);
    ui::click_root(
        &test.window,
        "Bytes: 32. Copy to clipboard",
        AccessibleRole::Button,
    );
    test.window
        .set_left_details(ModelRc::new(VecModel::from(vec![DetailRow {
            identity: "captured-selection".into(),
            label: "Selected".into(),
            value: "2 files, 0 folders".into(),
            ..Default::default()
        }])));
    test.draw();
    ui::click_root(
        &test.window,
        "Selected: 2 files, 0 folders. Copy to clipboard",
        AccessibleRole::Button,
    );
    test.window
        .window()
        .dispatch_event(WindowEvent::KeyReleased {
            text: Key::Control.into(),
        });
    assert_eq!(
        &*copied.borrow(),
        &[
            (0, "captured-name".into()),
            (0, "path:captured-name".into()),
            (0, "captured-size".into()),
            (0, "captured-selection".into()),
        ]
    );
}

#[test]
fn control_click_status_filename_captures_the_pressed_modifier_and_rejects_replaced_models() {
    use slint::platform::PointerEventButton;
    let mut test = ui::TestUi::new();
    test.window.set_left_details(status_fields("first.txt"));
    test.draw();
    let copied = Rc::new(RefCell::new(Vec::<String>::new()));
    let received = copied.clone();
    test.window
        .on_detail_copy(move |_, identity| received.borrow_mut().push(identity.to_string()));
    let position = ui::root_position(
        &test.window,
        "Name: first.txt. Copy to clipboard",
        AccessibleRole::Button,
    );
    test.window
        .window()
        .dispatch_event(WindowEvent::KeyPressed {
            text: Key::Control.into(),
        });
    ui::pointer_button(&test.window, position, PointerEventButton::Left, true);
    test.window
        .window()
        .dispatch_event(WindowEvent::KeyReleased {
            text: Key::Control.into(),
        });
    ui::pointer_button(&test.window, position, PointerEventButton::Left, false);
    assert_eq!(&*copied.borrow(), &["path:captured-name"]);
    copied.borrow_mut().clear();

    test.window
        .window()
        .dispatch_event(WindowEvent::KeyPressed {
            text: Key::Control.into(),
        });
    ui::pointer_button(&test.window, position, PointerEventButton::Left, true);
    // Even a reused row identity cannot make an old press refer to a new model.
    test.window.set_left_details(status_fields("other.txt"));
    test.draw();
    ui::pointer_button(&test.window, position, PointerEventButton::Left, false);
    test.window
        .window()
        .dispatch_event(WindowEvent::KeyReleased {
            text: Key::Control.into(),
        });
    assert!(
        copied.borrow().is_empty(),
        "a held click must not copy a replacement field"
    );
}

#[test]
fn status_filename_tooltip_and_accessibility_explain_control_click() {
    let time = Rc::new(Cell::new(Duration::ZERO));
    let mut test = ui::TestUi::with_clock(Some(time.clone()));
    test.window.set_left_details(status_fields("文档.txt"));
    test.draw();
    let label = "Name: 文档.txt. Copy to clipboard";
    let item = ui::root_item(&test.window, label, AccessibleRole::Button);
    let description = item
        .accessible_string_property(AccessibleStringProperty::Description)
        .unwrap_or_default();
    assert!(
        description.contains("Ctrl+click"),
        "accessible instructions must explain the modified click: {description}"
    );
    let position = ui::root_position(&test.window, label, AccessibleRole::Button);
    test.window
        .window()
        .dispatch_event(WindowEvent::PointerExited);
    test.draw();
    test.window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    test.draw();
    time.set(Duration::from_secs(2));
    test.draw();
    let inner = WindowInner::from_pub(test.window.window());
    let popups = inner.active_popups();
    let tooltip = popups
        .iter()
        .find(|popup| matches!(popup.window_kind, WindowKind::ToolTip))
        .expect("timed status hover shows a real tooltip");
    let text = ui::visible_text_values(ItemRc::new_root(tooltip.component.clone()));
    assert!(
        text.iter()
            .any(|text| text.contains("Ctrl+click") && text.contains("full path")),
        "actual tooltip must explain full-path copying: {text:?}"
    );
}

#[test]
fn filename_uses_available_width_before_metadata_and_copies_its_full_identity() {
    let mut ui = ui::TestUi::new();
    let name = "長い名前-source-configuration-release-candidate-2026-with-project-and-platform-details.txt";
    ui.window.set_left_details(ModelRc::new(VecModel::from(vec![
        DetailRow {
            identity: "captured-name".into(),
            label: "Name".into(),
            value: name.into(),
            ..Default::default()
        },
        DetailRow {
            identity: "captured-size".into(),
            label: "Bytes".into(),
            value: "123,456,789".into(),
            ..Default::default()
        },
    ])));
    ui.window.set_left_free_space("9.5 GB free".into());
    ui.draw();
    let label = format!("Name: {name}. Copy to clipboard");
    let item = ui::root_item(&ui.window, &label, AccessibleRole::Button);
    let wide = item.geometry().size.width;
    assert!(
        wide > 300.0,
        "filename must not retain the old 220px cap: {wide}"
    );
    render::snapshot(&ui, "status-filename-wide");

    let copied = Rc::new(RefCell::new(Vec::new()));
    let received = copied.clone();
    ui.window.on_detail_copy(move |pane, identity| {
        received.borrow_mut().push((pane, identity.to_string()));
    });
    ui.resize(760, 600);
    let item = ui::root_item(&ui.window, &label, AccessibleRole::Button);
    let geometry = item.geometry();
    let origin = item.map_to_native_window(geometry.origin);
    assert!(geometry.size.width > 100.0);
    assert!(geometry.size.width < wide);
    assert!(origin.x + geometry.size.width <= 382.0);
    render::snapshot(&ui, "status-filename-narrow");
    ui::click_root(&ui.window, &label, AccessibleRole::Button);
    assert_eq!(&*copied.borrow(), &[(0, "captured-name".into())]);
}

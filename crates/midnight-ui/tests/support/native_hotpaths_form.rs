//! The embedded Hot Paths manager keeps stable IDs, numbered edits and separate
//! management actions; it is driven through its presentation global.
use super::{embedded_form::Host, key};
use i_slint_core::{
    item_tree::ItemRc,
    items::{AccessibleRole, ComplexText, SimpleText},
};
use midnight_ui::{BookmarkRow, HotPathsPresentation};
use slint::{
    ComponentHandle, ModelRc, VecModel,
    platform::{Key, PointerEventButton, WindowAdapter, WindowEvent},
};
use std::{cell::RefCell, rc::Rc};
#[test]
fn embedded_hotpaths_manager_preserves_identity_selection_and_numbered_shortcuts() {
    let host = Host::new();
    let presentation = host.main.global::<HotPathsPresentation>();
    presentation.set_model(ModelRc::new(VecModel::from(vec![BookmarkRow {
        identity: "31".into(),
        label: "文档".into(),
        path: "D:\\文档\\work".into(),
        path_tail: "\\work".into(),
        slot: 1,
        note: Default::default(),
    }])));
    let events = Rc::new(RefCell::new(Vec::<String>::new()));
    let out = events.clone();
    presentation.on_add_bookmark(move |_| out.borrow_mut().push("add".into()));
    let out = events.clone();
    presentation.on_remove_bookmark(move |_, id| out.borrow_mut().push(format!("remove:{id}")));
    let out = events.clone();
    presentation.on_assign_bookmark(move |_, id, slot| {
        out.borrow_mut().push(format!("assign:{id}:{slot}"))
    });
    let weak = host.main.as_weak();
    presentation.on_dismiss(move |_| {
        if let Some(main) = weak.upgrade() {
            main.global::<HotPathsPresentation>().set_token("".into());
        }
    });
    presentation.set_token("manager".into());
    host.draw(700, 600, "embedded-hotpaths-manager");
    host.click("Add current");
    assert_eq!(events.borrow().as_slice(), ["add"]);
    host.click("Remove Hot Path: D:\\文档\\work\nThe folder is kept.");
    assert_eq!(events.borrow().last().unwrap(), "remove:31");
    assert!(host.is_open());
    host.click("D:\\文档\\work\nHot Path 1");
    assert_eq!(
        events.borrow().len(),
        2,
        "selecting a manager row does not navigate or commit"
    );
    let item = host.item(AccessibleRole::Combobox, "Hot Path shortcut");
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    let position =
        slint::LogicalPosition::new(origin.x + rect.width() / 2., origin.y + rect.height() / 2.);
    let native = &host.native;
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
    key(native, Key::DownArrow);
    key(native, Key::Return);
    host.draw(700, 600, "embedded-hotpaths-slot");
    host.click("Apply shortcut");
    assert_eq!(events.borrow().last().unwrap(), "assign:31:2");
    assert!(host.is_open());
    host.click("Close");
    assert!(!host.is_open());
}

/// Window rectangle of an item: x, y, width, height.
fn bounds(item: &ItemRc) -> (f32, f32, f32, f32) {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    (origin.x, origin.y, rect.size.width, rect.size.height)
}

/// Visible text items below `item` that show exactly `expected`.
fn drawn(item: &ItemRc, expected: &str) -> Vec<ItemRc> {
    let mut found = Vec::new();
    let mut pending = vec![item.clone()];
    while let Some(item) = pending.pop() {
        if !item.is_visible() {
            continue;
        }
        if item
            .downcast::<SimpleText>()
            .is_some_and(|text| text.as_pin_ref().text() == expected)
            || item
                .downcast::<ComplexText>()
                .is_some_and(|text| text.as_pin_ref().text() == expected)
        {
            found.push(item.clone());
        }
        let mut child = item.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            pending.push(item);
        }
    }
    found
}

#[test]
fn embedded_hotpaths_rows_with_a_common_parent_keep_their_last_folders_in_view() {
    let host = Host::new();
    let presentation = host.main.global::<HotPathsPresentation>();
    let parent = format!("/srv/{}", "shared-parent-folder/".repeat(8));
    let leaves = ["alpha-release", "beta-release"];
    presentation.set_model(ModelRc::new(VecModel::from(
        leaves
            .iter()
            .map(|leaf| BookmarkRow {
                identity: (*leaf).into(),
                label: (*leaf).into(),
                path: format!("{parent}{leaf}").into(),
                path_tail: format!("/{leaf}").into(),
                slot: 0,
                note: Default::default(),
            })
            .collect::<Vec<_>>(),
    )));
    presentation.set_path(format!("{parent}gamma-release").into());
    presentation.set_path_tail("/gamma-release".into());
    presentation.set_token("manager".into());
    host.draw(700, 600, "embedded-hotpaths-common-parent");
    let mut labels = vec![(
        format!("Current folder: {parent}gamma-release"),
        "/gamma-release".to_owned(),
    )];
    labels.extend(
        leaves
            .iter()
            .map(|leaf| (format!("{parent}{leaf}"), format!("/{leaf}"))),
    );
    for (text, tail) in labels {
        let label = host.item(AccessibleRole::Text, &text);
        let markers = drawn(&label, "...");
        let shown = drawn(&label, &tail);
        assert_eq!(
            (markers.len(), shown.len()),
            (1, 1),
            "{tail}: the middle is omitted and the last folder shown whole"
        );
        let (x, _, width, _) = bounds(&label);
        let (marker_x, _, marker_width, _) = bounds(&markers[0]);
        let (tail_x, _, tail_width, _) = bounds(&shown[0]);
        assert!(x < marker_x && marker_x + marker_width <= tail_x + 0.5);
        assert!(tail_x + tail_width <= x + width + 0.5);
    }
    // The row keeps the complete path for assistive technology.
    host.item(AccessibleRole::Button, &format!("{parent}alpha-release"));
}

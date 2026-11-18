//! The embedded Hot Paths manager keeps stable IDs, numbered edits and separate
//! management actions; it is driven through its presentation global.
use super::{embedded_form::Host, key};
use i_slint_core::items::AccessibleRole;
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
        slot: 1,
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

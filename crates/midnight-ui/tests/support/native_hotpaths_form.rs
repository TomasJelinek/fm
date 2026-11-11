//! Hot Paths manager keeps stable IDs, numbered edits and separate management actions.
use super::{key, modeless};
use i_slint_core::items::AccessibleRole;
use midnight_ui::{BookmarkRow, HotPathsWindow};
use slint::{
    ComponentHandle, ModelRc, VecModel,
    platform::{Key, PointerEventButton, WindowAdapter, WindowEvent},
};
use std::{cell::RefCell, rc::Rc};
#[test]
fn native_hotpaths_manager_preserves_identity_selection_and_numbered_shortcuts() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let window = HotPathsWindow::new().unwrap();
    window.set_model(ModelRc::new(VecModel::from(vec![BookmarkRow {
        identity: "31".into(),
        label: "文档".into(),
        path: "D:\\文档\\work".into(),
        slot: 1,
    }])));
    let events = Rc::new(RefCell::new(Vec::<String>::new()));
    let out = events.clone();
    window.on_add_bookmark(move || out.borrow_mut().push("add".into()));
    let out = events.clone();
    window.on_remove_bookmark(move |id| out.borrow_mut().push(format!("remove:{id}")));
    let out = events.clone();
    window.on_assign_bookmark(move |id, slot| out.borrow_mut().push(format!("assign:{id}:{slot}")));
    let weak = window.as_weak();
    window.on_request_dismiss(move || weak.upgrade().unwrap().hide().unwrap());
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    modeless::draw(&native, 700, 600, "native-hotpaths-manager");
    window.invoke_begin();
    modeless::click(&native, "Add current");
    assert_eq!(events.borrow().as_slice(), ["add"]);
    modeless::click(
        &native,
        "Remove Hot Path: D:\\文档\\work\nThe folder is kept.",
    );
    assert_eq!(events.borrow().last().unwrap(), "remove:31");
    assert!(native.window().is_visible());
    modeless::click(&native, "D:\\文档\\work\nHot Path 1");
    assert_eq!(
        events.borrow().len(),
        2,
        "selecting a manager row does not navigate or commit"
    );
    let item =
        modeless::accessible_item(&native, AccessibleRole::Combobox, "Hot Path shortcut").unwrap();
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    let position =
        slint::LogicalPosition::new(origin.x + rect.width() / 2., origin.y + rect.height() / 2.);
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
    key(&native, Key::DownArrow);
    key(&native, Key::Return);
    modeless::draw(&native, 700, 600, "native-hotpaths-slot");
    modeless::click(&native, "Apply shortcut");
    assert_eq!(events.borrow().last().unwrap(), "assign:31:2");
    assert!(native.window().is_visible());
    modeless::click(&native, "Close");
    assert!(!native.window().is_visible());
}

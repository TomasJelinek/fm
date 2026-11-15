#[path = "support/ui.rs"]
mod ui;

use midnight_ui::{Presenter, UnixPermissionsCodec};
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc};

#[test]
fn production_install_initializes_permission_parsing_and_projection() {
    let test = ui::TestUi::bare();
    let codec = test.window.global::<UnixPermissionsCodec>();
    assert!(!codec.invoke_parse("640".into()).valid);
    let presenter = Rc::new(RefCell::new(Presenter::new(
        "/left".into(),
        "/right".into(),
    )));
    midnight_ui::install(&test.window, presenter, Rc::new(|_| true));
    let parsed = codec.invoke_parse("640".into());
    assert!(parsed.valid);
    assert_eq!((parsed.mask, parsed.value), (0o777, 0o640));
    let projected = codec.invoke_project(0o600, 0o7777, 0o040, 0o040);
    assert_eq!(projected.octal.as_str(), "640");
    assert_eq!(projected.symbolic.as_str(), "rw-r-----");
}

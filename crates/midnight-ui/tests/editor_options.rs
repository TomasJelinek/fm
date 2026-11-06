#[path = "support/ui.rs"]
mod ui;
use i_slint_core::items::AccessibleRole;
use slint::platform::Key;
use std::{cell::RefCell, rc::Rc};
use ui::{TestUi, click_popup, key};

#[test]
fn editor_options_apply_literal_arguments_and_text_focus_consumes_f4() {
    let mut ui = TestUi::new();
    let changed = Rc::new(RefCell::new(Vec::new()));
    let captured = changed.clone();
    ui.window.on_editor_apply(move |executable, arguments| {
        captured.borrow_mut().push((executable, arguments))
    });
    let edits = Rc::new(RefCell::new(Vec::new()));
    let captured = edits.clone();
    ui.window
        .on_edit_file(move |pane| captured.borrow_mut().push(pane));
    ui.window.invoke_focus_files();
    key(&ui.window, Key::F4);
    assert_eq!(*edits.borrow(), [0]);
    edits.borrow_mut().clear();
    ui.window.set_editor_default_label("System editor".into());
    ui.window
        .set_editor_executable("D:/Editor 文档/editor.exe".into());
    ui.window
        .set_editor_arguments(r#"["{file}", "--literal=$HOME; &"]"#.into());
    ui.window.set_editor_revision("1".into());
    ui.window.invoke_show_options();
    ui.window.set_options_topic_index(4);
    ui.draw();
    click_popup(&ui.window, "Apply editor", AccessibleRole::Button);
    assert_eq!(changed.borrow().len(), 1);
    assert_eq!(changed.borrow()[0].0, "D:/Editor 文档/editor.exe");
    assert_eq!(changed.borrow()[0].1, r#"["{file}", "--literal=$HOME; &"]"#);
    click_popup(&ui.window, "Editor executable", AccessibleRole::TextInput);
    key(&ui.window, Key::F4);
    assert!(
        edits.borrow().is_empty(),
        "Typing focus must not dispatch F4"
    );
}

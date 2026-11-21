//! Windows folders ignore letter case: a file's path typed in other letter case lists
//! its folder with the cursor on the file under its listed name, through the real path
//! editor, the production Engine and native files.
#![cfg(windows)]
#[path = "support/operations_native.rs"]
mod native;
use crate::support::ui;

use midnight_core::PaneId;
use slint::{Model, platform::Key};

/// The name on the left panel's cursor row.
fn cursor_name(h: &native::Harness) -> Option<String> {
    let cursor = usize::try_from(h.cursor(PaneId::Left)).ok()?;
    let row = h.ui.window.get_left_rows().row_data(cursor)?;
    Some(row.name.to_string())
}

#[test]
fn a_file_typed_in_other_letter_case_gets_the_cursor_under_its_listed_name() {
    const LISTED: &str = "čti mě 文档.txt";
    let mut h = native::Harness::new();
    let folder = h.root.join("Složka");
    std::fs::create_dir(&folder).unwrap();
    for name in ["a.txt", LISTED, "z.txt"] {
        std::fs::write(folder.join(name), name).unwrap();
    }
    h.ui.window.invoke_activate(0);
    h.ui.window.invoke_focus_files();
    ui::chord(&h.ui.window, &[Key::Control], "g");
    h.settle();
    ui::chord(&h.ui.window, &[Key::Control], "a");
    let typed = folder.join("ČTI MĚ 文档.TXT");
    ui::key(&h.ui.window, typed.to_str().unwrap());
    ui::key(&h.ui.window, Key::Return);
    let shown = folder.to_str().unwrap().to_owned();
    // The listing of the folder commits with its cursor; a notice ends the wait at once.
    h.until(|h| {
        !h.ui.window.get_message_title().is_empty()
            || (h.ui.window.get_left_path().as_str() == shown && !h.ui.window.get_left_loading())
    });
    let notice = h.ui.window.get_message_title();
    assert!(
        notice.is_empty(),
        "{notice}: {}",
        h.ui.window.get_message_body()
    );
    assert_eq!(cursor_name(&h).as_deref(), Some(LISTED));
    assert!(h.ui.window.get_left_error().is_empty());
}

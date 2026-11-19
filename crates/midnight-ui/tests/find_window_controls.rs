//! Find's Search folders list, clickable error details, syntax help and resizable
//! result columns, driven through real pointer and keyboard input.
use crate::support::modeless;

use i_slint_core::{
    accessibility::AccessibleStringProperty, item_tree::ItemRc, items::AccessibleRole,
};
use midnight_ui::{FindRootRow, FindRow, FindWindow};
use slint::{
    ComponentHandle, Model, VecModel,
    platform::{
        Key, PointerEventButton, WindowAdapter as _, WindowEvent,
        software_renderer::MinimalSoftwareWindow,
    },
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

fn open() -> (FindWindow, Rc<MinimalSoftwareWindow>) {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let window = FindWindow::new().unwrap();
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    (window, native)
}

fn item(native: &MinimalSoftwareWindow, role: AccessibleRole, label: &str) -> ItemRc {
    modeless::accessible_item(native, role, label).unwrap_or_else(|| panic!("missing {label}"))
}

fn popup_item(native: &MinimalSoftwareWindow, role: AccessibleRole, label: &str) -> ItemRc {
    modeless::accessible_popup_item(native, role, label)
        .unwrap_or_else(|| panic!("missing popup {label}"))
}

fn center(item: &ItemRc) -> slint::LogicalPosition {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    slint::LogicalPosition::new(
        origin.x + rect.width() / 2.0,
        origin.y + rect.height() / 2.0,
    )
}

fn key(native: &MinimalSoftwareWindow, text: slint::SharedString) {
    native
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    native
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn value(item: &ItemRc) -> String {
    item.accessible_string_property(AccessibleStringProperty::Value)
        .unwrap_or_default()
        .to_string()
}

fn checked(item: &ItemRc) -> bool {
    item.accessible_string_property(AccessibleStringProperty::Checked)
        .is_some_and(|value| value == "true")
}

fn row(label: &str, searched: bool) -> FindRootRow {
    FindRootRow {
        label: label.into(),
        searched,
    }
}

#[test]
fn search_folders_add_tick_choose_and_remove_through_one_list() {
    let (window, native) = open();
    let paths = [r"C:\fixtures\první", "/home/user/日本語 folder"];
    let roots = Rc::new(VecModel::from(vec![
        row(paths[0], true),
        row(paths[1], false),
    ]));
    window.set_roots(roots.clone().into());
    window.set_searched_roots(1);
    let added = Rc::new(RefCell::new(Vec::<String>::new()));
    let toggles = Rc::new(RefCell::new(Vec::new()));
    let removed = Rc::new(RefCell::new(Vec::new()));
    let browsed = Rc::new(Cell::new(0));
    window.on_add_root({
        let added = added.clone();
        move |text| added.borrow_mut().push(text.to_string())
    });
    window.on_toggle_root({
        let (toggles, roots) = (toggles.clone(), roots.clone());
        // The host updates the listed row in place, as find_windows does.
        move |index, searched| {
            toggles.borrow_mut().push((index, searched));
            let mut data = roots.row_data(index as usize).unwrap();
            data.searched = searched;
            roots.set_row_data(index as usize, data);
        }
    });
    window.on_remove_root({
        let removed = removed.clone();
        move |index| removed.borrow_mut().push(index)
    });
    window.on_browse_root({
        let browsed = browsed.clone();
        move || browsed.set(browsed.get() + 1)
    });
    modeless::draw(&native, 860, 620, "find-folders-closed");
    item(&native, AccessibleRole::Text, "Search folders (1 of 2)");
    assert!(
        modeless::accessible_item(&native, AccessibleRole::TextInput, "Folder to add").is_none(),
        "no separate Add field until Add is pressed"
    );
    // The Add button precedes Remove on the Search folders line.
    let add = item(&native, AccessibleRole::Button, "Add");
    let remove = item(&native, AccessibleRole::Button, "Remove");
    let folders = item(&native, AccessibleRole::Combobox, "Search folders");
    assert!(center(&add).x < center(&remove).x);
    assert!((center(&add).y - center(&folders).y).abs() <= 1.0);

    modeless::click_item(&native, &folders, "Search folders");
    modeless::draw(&native, 860, 620, "find-folders-open");
    let second = popup_item(&native, AccessibleRole::Checkbox, paths[1]);
    assert!(!checked(&second));
    assert!(checked(&popup_item(
        &native,
        AccessibleRole::Checkbox,
        paths[0]
    )));
    modeless::click_item(&native, &second, paths[1]);
    modeless::draw(&native, 860, 620, "find-folders-ticked");
    assert_eq!(toggles.borrow().as_slice(), [(1, true)]);
    // Ticking keeps the list open, so further folders can be toggled.
    let first = popup_item(&native, AccessibleRole::Checkbox, paths[0]);
    assert!(checked(&popup_item(
        &native,
        AccessibleRole::Checkbox,
        paths[1]
    )));
    modeless::click_item(&native, &first, paths[0]);
    assert_eq!(toggles.borrow().as_slice(), [(1, true), (0, false)]);
    // Clicking a name chooses that folder and closes the list.
    modeless::click(&native, paths[1]);
    modeless::draw(&native, 860, 620, "find-folders-chosen");
    assert!(modeless::accessible_popup_item(&native, AccessibleRole::Checkbox, paths[1]).is_none());
    assert_eq!(
        value(&item(&native, AccessibleRole::Combobox, "Search folders")),
        paths[1]
    );
    modeless::click(&native, "Remove");
    assert_eq!(removed.borrow().as_slice(), [1]);

    // Add opens a one-line entry: typed text, Enter or OK, and Browse.
    modeless::click(&native, "Add");
    modeless::draw(&native, 860, 620, "find-folders-adding");
    let entry = item(&native, AccessibleRole::TextInput, "Folder to add");
    let entry_at = center(&entry);
    assert!(
        entry_at.y > center(&folders).y,
        "the entry opens under the list"
    );
    key(&native, "/srv/typed folder".into());
    modeless::draw(&native, 860, 620, "find-folders-typed");
    modeless::click(&native, "OK");
    key(&native, Key::Return.into());
    assert_eq!(
        added.borrow().as_slice(),
        ["/srv/typed folder", "/srv/typed folder"]
    );
    modeless::click(&native, "Browse…");
    assert_eq!(browsed.get(), 1);
    // A listed addition chooses the new folder and closes the entry.
    roots.push(row("/srv/typed folder", true));
    modeless::draw(&native, 860, 620, "find-folders-added");
    assert!(
        modeless::accessible_item(&native, AccessibleRole::TextInput, "Folder to add").is_none()
    );
    assert_eq!(
        value(&item(&native, AccessibleRole::Combobox, "Search folders")),
        "/srv/typed folder"
    );
    // A running search freezes the list and its buttons.
    window.set_running(true);
    modeless::draw(&native, 860, 620, "find-folders-running");
    for label in ["Add", "Remove"] {
        modeless::click(&native, label);
    }
    assert_eq!(removed.borrow().len(), 1);
    assert!(
        modeless::accessible_item(&native, AccessibleRole::TextInput, "Folder to add").is_none()
    );
    window.hide().unwrap();
}

#[test]
fn error_and_status_lines_open_details_without_a_details_button() {
    let (window, native) = open();
    let error = "Access denied\n\nFolder: C:\\Žluťoučký 日本語";
    let progress = "Incomplete · click for details · 3 found · 120 examined";
    window.set_error(error.into());
    window.set_progress(progress.into());
    window.set_attention(true);
    window.set_details("100 files · 12 folders\n1 errors".into());
    for (width, height) in [(860, 620), (320, 200)] {
        modeless::draw(&native, width, height, "find-error-lines");
        assert!(
            modeless::accessible_item(&native, AccessibleRole::Button, "Details").is_none(),
            "details open from the error itself"
        );
        // The minimum-size query scrolls its error line away; the status line stays.
        let labels: &[&str] = if width > 320 {
            &[error, progress]
        } else {
            &[progress]
        };
        for &label in labels {
            let line = item(&native, AccessibleRole::Button, label);
            assert_eq!(
                line.accessible_string_property(AccessibleStringProperty::Description)
                    .as_deref(),
                Some("Click for more details")
            );
            modeless::click_item(&native, &line, label);
            assert!(window.get_show_details(), "{label} at {width}x{height}");
            modeless::draw(&native, width, height, "find-error-details");
            modeless::click(&native, "Back");
            assert!(!window.get_show_details());
            modeless::draw(&native, width, height, "find-error-back");
        }
    }
    window.hide().unwrap();
}

#[test]
fn syntax_help_shows_examples_for_the_mode_and_uses_a_clicked_example() {
    let (window, native) = open();
    modeless::draw(&native, 860, 620, "find-help-closed");
    modeless::click(&native, "Help");
    assert!(window.get_show_help());
    modeless::draw(&native, 860, 620, "find-help-masks");
    item(&native, AccessibleRole::Text, "Masks syntax");
    modeless::click(&native, "*.rs;*.toml");
    modeless::draw(&native, 860, 620, "find-help-used");
    assert_eq!(
        value(&item(&native, AccessibleRole::TextInput, "File name masks")),
        "*.rs;*.toml"
    );
    assert!(window.get_show_help(), "using an example keeps help open");
    // F1 from the query editor toggles help.
    key(&native, Key::F1.into());
    modeless::draw(&native, 860, 620, "find-help-f1");
    assert!(!window.get_show_help());
    key(&native, Key::F1.into());
    assert!(window.get_show_help());
    // Each mode has its own examples: Tab to the mode chooser, then pick Expression.
    key(&native, Key::Tab.into());
    key(&native, Key::DownArrow.into());
    key(&native, Key::DownArrow.into());
    modeless::draw(&native, 860, 620, "find-help-expression");
    assert_eq!(
        value(&item(&native, AccessibleRole::Combobox, "Find match mode")),
        "Expression"
    );
    item(&native, AccessibleRole::Text, "Expression syntax");
    item(
        &native,
        AccessibleRole::Button,
        "kind = file and size >= 10MiB",
    );
    modeless::click(&native, "Close help");
    assert!(!window.get_show_help());
    window.set_show_help(true);
    // Starting a search closes help so the results are visible.
    let searched = Rc::new(Cell::new(0));
    window.on_search({
        let searched = searched.clone();
        move |_| searched.set(searched.get() + 1)
    });
    modeless::click(&native, "Find");
    assert_eq!(searched.get(), 1);
    assert!(!window.get_show_help());
    window.hide().unwrap();
}

fn drag(native: &MinimalSoftwareWindow, from: slint::LogicalPosition, dx: f32) {
    let window = native.window();
    window.dispatch_event(WindowEvent::PointerMoved { position: from });
    window.dispatch_event(WindowEvent::PointerPressed {
        position: from,
        button: PointerEventButton::Left,
    });
    let mut to = from;
    for step in 1..=4 {
        to = slint::LogicalPosition::new(from.x + dx * step as f32 / 4.0, from.y);
        window.dispatch_event(WindowEvent::PointerMoved { position: to });
    }
    window.dispatch_event(WindowEvent::PointerReleased {
        position: to,
        button: PointerEventButton::Left,
    });
}

fn right_edge(item: &ItemRc) -> f32 {
    let rect = item.geometry();
    item.map_to_native_window(rect.origin).x + rect.width()
}

#[test]
fn every_result_column_resizes_by_dragging_its_header_separator() {
    let (window, native) = open();
    let sorts = Rc::new(RefCell::new(Vec::new()));
    window.on_sort({
        let sorts = sorts.clone();
        move |column| sorts.borrow_mut().push(column)
    });
    let rows = || {
        Rc::new(VecModel::from(vec![FindRow {
            token: "1:1".into(),
            name: "report.txt".into(),
            extension: "txt".into(),
            path: "/home/user".into(),
            size: "10 B".into(),
            modified: "2026-11-19 08:30".into(),
            ..Default::default()
        }]))
    };
    window.set_rows(rows().into());
    // Narrow columns keep the whole table, and every separator, in view.
    let mut widths = window.get_column_widths();
    for width in [
        &mut widths.name,
        &mut widths.extension,
        &mut widths.path,
        &mut widths.size,
        &mut widths.modified,
    ] {
        *width = 80.0;
    }
    window.set_column_widths(widths);
    modeless::draw(&native, 860, 620, "find-columns");
    for (index, title) in ["Name", "Extension", "Path", "Size", "Modified"]
        .into_iter()
        .enumerate()
    {
        let header = item(&native, AccessibleRole::Button, &format!("Sort by {title}"));
        let next = ["Extension", "Path", "Size", "Modified"]
            .get(index)
            .map(|next| item(&native, AccessibleRole::Button, &format!("Sort by {next}")));
        let next_x = next.as_ref().map(|next| center(next).x);
        let before = right_edge(&header);
        let grip = slint::LogicalPosition::new(before + 5.0, center(&header).y);
        drag(&native, grip, 40.0);
        modeless::draw(&native, 860, 620, "find-columns-dragged");
        let header = item(&native, AccessibleRole::Button, &format!("Sort by {title}"));
        assert!(
            (right_edge(&header) - before - 40.0).abs() <= 1.0,
            "{title} widens with the drag"
        );
        if let Some(next_x) = next_x {
            let next = item(
                &native,
                AccessibleRole::Button,
                &format!(
                    "Sort by {}",
                    ["Extension", "Path", "Size", "Modified"][index]
                ),
            );
            assert!(
                (center(&next).x - next_x - 40.0).abs() <= 1.0,
                "later columns follow {title}"
            );
        }
    }
    let widths = window.get_column_widths();
    for width in [
        widths.name,
        widths.extension,
        widths.path,
        widths.size,
        widths.modified,
    ] {
        assert!((width - 120.0).abs() <= 1.0, "dragged width {width}");
    }
    assert!(sorts.borrow().is_empty(), "a separator drag never sorts");
    // Shrinking stops at a readable minimum.
    let header = item(&native, AccessibleRole::Button, "Sort by Name");
    drag(
        &native,
        slint::LogicalPosition::new(right_edge(&header) + 5.0, center(&header).y),
        -500.0,
    );
    let name = window.get_column_widths().name;
    assert!(name > 10.0 && name < 60.0, "minimum name width {name}");
    // New results keep the session's widths.
    window.set_rows(rows().into());
    modeless::draw(&native, 860, 620, "find-columns-new-results");
    assert_eq!(window.get_column_widths().name, name);
    window.hide().unwrap();
}

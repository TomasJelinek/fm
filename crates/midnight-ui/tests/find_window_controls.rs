//! Find's Search folders list, clickable error details, syntax help and resizable
//! result columns, driven through real pointer and keyboard input.
use crate::support::modeless;

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::ItemRc,
    items::{AccessibleRole, TouchArea},
    window::WindowInner,
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

#[test]
fn filename_history_dropdown_chooses_a_mask_without_starting_a_search() {
    let (window, native) = open();
    window.set_mask_history(slint::ModelRc::new(VecModel::from(vec![
        "*.rs".into(),
        "*.文档;*.txt".into(),
    ])));
    let searches = Rc::new(Cell::new(0));
    let count = searches.clone();
    window.on_search(move |_| count.set(count.get() + 1));
    modeless::layout(&native, 780, 560);
    window.invoke_begin();
    key(&native, Key::DownArrow.into());
    modeless::layout(&native, 780, 560);
    let choice = popup_item(&native, AccessibleRole::Button, "*.文档;*.txt");
    modeless::click_item(&native, &choice, "previous filename mask");
    modeless::layout(&native, 780, 560);
    assert_eq!(
        value(&item(&native, AccessibleRole::TextInput, "File name masks")),
        "*.文档;*.txt"
    );
    assert_eq!(searches.get(), 0);
    window.hide().unwrap();
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
    // The host answers an accepted Add with its folder's row: that folder is chosen
    // and the entry closes, with keyboard focus still in the window.
    roots.push(row("/srv/typed folder", true));
    window.invoke_root_added(2);
    modeless::draw(&native, 860, 620, "find-folders-added");
    assert!(
        modeless::accessible_item(&native, AccessibleRole::TextInput, "Folder to add").is_none()
    );
    assert_eq!(
        value(&item(&native, AccessibleRole::Combobox, "Search folders")),
        "/srv/typed folder"
    );
    key(&native, Key::F1.into());
    assert!(window.get_show_help(), "F1 still reaches the window");
    key(&native, Key::Escape.into());
    assert!(!window.get_show_help());
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
    // Each mode has its own examples: Tab to the mode chooser, then pick Expression.
    key(&native, Key::Tab.into());
    key(&native, Key::DownArrow.into());
    key(&native, Key::DownArrow.into());
    assert_eq!(
        value(&item(&native, AccessibleRole::Combobox, "Find match mode")),
        "Expression"
    );
    key(&native, Key::F1.into());
    assert!(window.get_show_help());
    modeless::draw(&native, 860, 620, "find-help-expression");
    item(&native, AccessibleRole::Text, "Expression syntax");
    item(
        &native,
        AccessibleRole::Button,
        "kind = file and size >= 10MiB",
    );
    // Linux reports no attributes: the help says so and offers the dot-name test.
    let attributes = item(
        &native,
        AccessibleRole::Button,
        "attributes has hidden or attributes has system",
    );
    assert!(
        attributes
            .accessible_string_property(AccessibleStringProperty::Description)
            .is_some_and(|meaning| meaning.contains("Windows only")),
        "attribute tests are marked Windows only"
    );
    item(&native, AccessibleRole::Button, "name like \".*\"");
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

fn left_edge(item: &ItemRc) -> f32 {
    item.map_to_native_window(item.geometry().origin).x
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
    let header = |title: &str| item(&native, AccessibleRole::Button, &format!("Sort by {title}"));
    // Path takes the spare width: the table ends at the list's edge, after Modified,
    // its separator and the row padding.
    let list_right = right_edge(&item(&native, AccessibleRole::List, "Find results"));
    assert!((right_edge(&header("Modified")) + 16.0 - list_right).abs() <= 1.0);
    assert!(header("Path").geometry().width() > 300.0);
    // Path last: while it fills, the other columns take their width from it.
    for title in ["Name", "Extension", "Size", "Modified", "Path"] {
        let before = header(title).geometry().width();
        let next_x = left_edge(&header("Extension"));
        let grip =
            slint::LogicalPosition::new(right_edge(&header(title)) + 5.0, center(&header(title)).y);
        drag(&native, grip, 40.0);
        modeless::draw(&native, 860, 620, "find-columns-dragged");
        assert!(
            (header(title).geometry().width() - before - 40.0).abs() <= 1.0,
            "{title} widens with the drag"
        );
        if title == "Name" {
            assert!(
                (left_edge(&header("Extension")) - next_x - 40.0).abs() <= 1.0,
                "later columns follow"
            );
        }
    }
    let widths = window.get_column_widths();
    for width in [widths.name, widths.extension, widths.size, widths.modified] {
        assert!((width - 120.0).abs() <= 1.0, "dragged width {width}");
    }
    // The Path drag started from its shown width, so the table now scrolls.
    assert!(widths.path > 300.0, "dragged Path width {}", widths.path);
    assert!(right_edge(&header("Modified")) + 16.0 > list_right + 39.0);
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

fn result_rows(count: usize) -> Rc<VecModel<FindRow>> {
    Rc::new(VecModel::from(
        (0..count)
            .map(|index| FindRow {
                token: format!("1:{index}").into(),
                name: format!("file {index}.txt").into(),
                extension: "txt".into(),
                path: "/home/user".into(),
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    ))
}

fn touch_areas(item: &ItemRc) -> usize {
    let mut count = usize::from(item.downcast::<TouchArea>().is_some());
    let mut child = item.first_child();
    while let Some(next) = child {
        child = next.next_sibling();
        count += touch_areas(&next);
    }
    count
}

fn first_list_item(item: ItemRc) -> Option<ItemRc> {
    if item.accessible_role() == AccessibleRole::ListItem {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(next) = child {
        child = next.next_sibling();
        if let Some(found) = first_list_item(next) {
            return Some(found);
        }
    }
    None
}

#[test]
fn only_the_header_has_column_separators() {
    let (window, native) = open();
    window.set_rows(result_rows(2).into());
    modeless::draw(&native, 860, 620, "find-row-items");
    let root = ItemRc::new_root(WindowInner::from_pub(native.window()).component());
    let row = first_list_item(root).expect("a result row");
    // The row's own selection area only: separators belong to the header.
    assert_eq!(touch_areas(&row), 1);
    // Rows keep the header's column positions with a plain gap.
    for (header, text) in [("Name", "file 0.txt"), ("Extension", "txt")] {
        let header = item(
            &native,
            AccessibleRole::Button,
            &format!("Sort by {header}"),
        );
        let text = item(&native, AccessibleRole::Text, text);
        assert!((left_edge(&text) - left_edge(&header)).abs() <= 1.0);
    }
    window.hide().unwrap();
}

#[test]
fn the_host_chooses_each_added_folder_and_counts_ticked_folders() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let main = midnight_ui::MainWindow::new().unwrap();
    let root = std::env::temp_dir();
    let state = Rc::new(RefCell::new(midnight_ui::Presenter::new(
        root.clone(),
        root.clone(),
    )));
    midnight_ui::install(&main, state.clone(), Rc::new(|_| true));
    main.show().unwrap();
    modeless::draw(&windows.borrow()[0], 1000, 700, "find-host-main");
    main.invoke_find_files(0);
    let find = windows.borrow()[1].clone();
    modeless::draw(&find, 860, 620, "find-host");
    let first = value(&item(&find, AccessibleRole::Combobox, "Search folders"));
    let typed = root.join("typed 文档").to_string_lossy().into_owned();
    for (path, label) in [
        (typed.as_str(), "Search folders (2)"),
        // Adding a listed folder again chooses it as well.
        (first.as_str(), "Search folders (2)"),
    ] {
        modeless::click(&find, "Add");
        modeless::draw(&find, 860, 620, "find-host-adding");
        key(&find, path.into());
        key(&find, Key::Return.into());
        modeless::draw(&find, 860, 620, "find-host-added");
        assert!(
            modeless::accessible_item(&find, AccessibleRole::TextInput, "Folder to add").is_none(),
            "an accepted Add closes the entry: {path}"
        );
        assert_eq!(
            value(&item(&find, AccessibleRole::Combobox, "Search folders")),
            path
        );
        item(&find, AccessibleRole::Text, label);
    }
    // An untick through the open list updates its row in place: the list stays open.
    let folders = item(&find, AccessibleRole::Combobox, "Search folders");
    modeless::click_item(&find, &folders, "Search folders");
    modeless::draw(&find, 860, 620, "find-host-list");
    let added = popup_item(&find, AccessibleRole::Checkbox, &typed);
    modeless::click_item(&find, &added, &typed);
    modeless::draw(&find, 860, 620, "find-host-unticked");
    assert!(!checked(&popup_item(
        &find,
        AccessibleRole::Checkbox,
        &typed
    )));
    item(&find, AccessibleRole::Text, "Search folders (1 of 2)");
    main.invoke_close_find_windows();
    state.borrow_mut().retire_all();
}

#[test]
fn escape_closes_an_open_overlay_first_and_focus_survives_it() {
    let (window, native) = open();
    let closed = Rc::new(Cell::new(0));
    window.on_close_session({
        let closed = closed.clone();
        move || closed.set(closed.get() + 1)
    });
    window.set_roots(Rc::new(VecModel::from(vec![row("/srv", true)])).into());
    window.set_rows(result_rows(3).into());
    window.invoke_begin();
    modeless::draw(&native, 860, 620, "find-overlays");
    // F1 from the query moves focus into the help: keys no longer reach the hidden rows.
    key(&native, Key::F1.into());
    modeless::draw(&native, 860, 620, "find-overlays-help");
    key(&native, Key::DownArrow.into());
    assert_eq!(
        window.get_selected_row(),
        -1,
        "the hidden results keep their row"
    );
    key(&native, Key::Escape.into());
    assert!(!window.get_show_help());
    assert_eq!(closed.get(), 0, "Escape closed only the help");
    // Focus went to the results that the help covered.
    modeless::draw(&native, 860, 620, "find-overlays-closed");
    key(&native, Key::DownArrow.into());
    assert_eq!(window.get_selected_row(), 0);
    // Close help returns focus to the results as well.
    modeless::click(&native, "Help");
    modeless::draw(&native, 860, 620, "find-overlays-help-again");
    modeless::click(&native, "Close help");
    modeless::draw(&native, 860, 620, "find-overlays-help-closed");
    key(&native, Key::DownArrow.into());
    assert_eq!(window.get_selected_row(), 1);
    // The Add entry also closes on Escape, and focus moves to Search folders.
    modeless::click(&native, "Add");
    modeless::draw(&native, 860, 620, "find-overlays-adding");
    key(&native, Key::Escape.into());
    modeless::draw(&native, 860, 620, "find-overlays-added");
    assert!(
        modeless::accessible_item(&native, AccessibleRole::TextInput, "Folder to add").is_none()
    );
    assert_eq!(closed.get(), 0, "Escape closed only the Add entry");
    key(&native, Key::F1.into());
    assert!(window.get_show_help(), "F1 still reaches the window");
    modeless::draw(&native, 860, 620, "find-overlays-help-third");
    key(&native, Key::Escape.into());
    modeless::draw(&native, 860, 620, "find-overlays-done");
    assert_eq!(closed.get(), 0);
    // Without an overlay, Escape closes the window.
    key(&native, Key::Escape.into());
    assert_eq!(closed.get(), 1);
    window.hide().unwrap();
}

#[test]
fn a_short_running_search_with_large_text_shows_its_status_under_the_results() {
    let (window, native) = open();
    window.invoke_apply_preferences(0, 0, "".into(), 24, 24);
    let progress = "Searching · 3 found · 120 examined";
    window.set_progress(progress.into());
    window.set_running(true);
    modeless::draw(&native, 700, 300, "find-compact-running");
    // The query area gives its height to the results; the status line stays usable.
    let line = item(&native, AccessibleRole::Button, progress);
    let top = line.map_to_native_window(line.geometry().origin).y;
    assert!(
        top >= 0.0 && top + line.geometry().height() <= 300.0,
        "status line at {top}"
    );
    modeless::click_item(&native, &line, progress);
    assert!(window.get_show_details());
    window.hide().unwrap();
}

#[test]
fn the_folder_list_fits_a_short_window_and_keeps_the_browsed_row_visible() {
    let (window, native) = open();
    let labels: Vec<String> = (0..16)
        .map(|index| format!("/srv/folder {index:02}"))
        .collect();
    window.set_roots(
        Rc::new(VecModel::from(
            labels
                .iter()
                .map(|label| row(label, true))
                .collect::<Vec<_>>(),
        ))
        .into(),
    );
    // The host keeps the popup surface at the window size.
    let placement = window.global::<midnight_ui::TooltipPlacement>();
    placement.set_surface_width(700.0);
    placement.set_surface_height(400.0);
    modeless::draw(&native, 700, 400, "find-folders-short");
    let folders = item(&native, AccessibleRole::Combobox, "Search folders");
    modeless::click_item(&native, &folders, "Search folders");
    modeless::draw(&native, 700, 400, "find-folders-short-open");
    key(&native, Key::End.into());
    modeless::draw(&native, 700, 400, "find-folders-short-end");
    // click_item asserts that the row is inside the window.
    let last = popup_item(&native, AccessibleRole::Checkbox, &labels[15]);
    let top = last.map_to_native_window(last.geometry().origin).y;
    assert!(top + last.geometry().height() <= 400.0, "last row at {top}");
    modeless::click_item(&native, &last, &labels[15]);
    window.hide().unwrap();
}

#[test]
fn a_narrow_touch_window_gives_the_folder_entry_its_own_line() {
    let (window, native) = open();
    window.invoke_apply_preferences(0, 1, "".into(), 13, 17);
    window.set_roots(Rc::new(VecModel::from(vec![row("/srv", true)])).into());
    modeless::draw(&native, 320, 620, "find-narrow-adding");
    modeless::click(&native, "Add");
    modeless::draw(&native, 320, 620, "find-narrow-adding-open");
    let entry = item(&native, AccessibleRole::TextInput, "Folder to add");
    let browse = item(&native, AccessibleRole::Button, "Browse…");
    assert!(
        entry.geometry().width() >= 120.0,
        "the path field is usable: {:?}",
        entry.geometry()
    );
    assert!(
        center(&browse).y > center(&entry).y,
        "Browse is under the path"
    );
    assert!(right_edge(&browse) <= 320.0);
    window.hide().unwrap();
}

//! A narrow Find window keeps its labels at their natural height: "Search folders"
//! stays above its picker instead of collapsing to nothing when the query area is short.
use crate::support::modeless;

use i_slint_core::{
    item_tree::ItemRc,
    items::{AccessibleRole, ComplexText, Orientation, SimpleText},
    window::WindowInner,
};
use midnight_core::PreferenceTheme;
use midnight_ui::{FindRootRow, FindWindow};
use slint::{ComponentHandle, ModelRc, VecModel, platform::WindowAdapter};
use std::{cell::RefCell, rc::Rc};

fn text_item(item: ItemRc, caption: &str) -> Option<ItemRc> {
    let text = item
        .downcast::<SimpleText>()
        .map(|text| text.as_pin_ref().text())
        .or_else(|| {
            item.downcast::<ComplexText>()
                .map(|text| text.as_pin_ref().text())
        });
    if item.is_visible() && text.is_some_and(|text| text == caption) {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = text_item(item, caption) {
            return Some(found);
        }
    }
    None
}

/// The label's drawn height and the height its text needs at its width.
fn heights(
    native: &Rc<slint::platform::software_renderer::MinimalSoftwareWindow>,
    item: &ItemRc,
) -> (f32, f32) {
    let adapter: Rc<dyn WindowAdapter> = native.clone();
    let rect = item.geometry();
    let needed = item
        .borrow()
        .as_ref()
        .layout_info(Orientation::Vertical, rect.width(), &adapter, item)
        .preferred;
    (rect.height(), needed)
}

#[test]
fn a_narrow_find_window_keeps_search_folders_above_its_picker() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let window = FindWindow::new().unwrap();
    window.set_roots(ModelRc::new(VecModel::from(vec![
        FindRootRow {
            label: "/home/user/資料".into(),
            searched: true,
        },
        FindRootRow {
            label: "/srv/shared".into(),
            searched: false,
        },
    ])));
    window.set_searched_roots(1);
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    // The UI review's sizes, below the narrow layout's 620 px, and a shorter one.
    for (theme, width, height) in [
        (PreferenceTheme::Blue, 560, 560),
        (PreferenceTheme::Dark, 560, 520),
        (PreferenceTheme::Blue, 560, 420),
        (PreferenceTheme::Dark, 480, 520),
    ] {
        let case = format!("{theme:?} {width}x{height}");
        window.invoke_apply_preferences(theme.index(), 0, "".into(), 12, 15);
        modeless::draw(
            &native,
            width,
            height,
            &format!("find-narrow-{theme:?}").to_lowercase(),
        );
        let root = ItemRc::new_root(WindowInner::from_pub(native.window()).component());
        for caption in ["Search folders (1 of 2)", "File name"] {
            let label = text_item(root.clone(), caption)
                .unwrap_or_else(|| panic!("{case}: {caption} is not shown"));
            let (drawn, needed) = heights(&native, &label);
            assert!(
                drawn > 0.0 && drawn + 0.5 >= needed,
                "{case}: {caption} is {}x{drawn} but needs {needed}",
                label.geometry().width()
            );
        }
        let label = text_item(root.clone(), "Search folders (1 of 2)").unwrap();
        let picker = modeless::accessible_item(&native, AccessibleRole::Combobox, "Search folders")
            .unwrap_or_else(|| panic!("{case}: no Search folders picker"));
        let label_bottom =
            label.map_to_native_window(label.geometry().origin).y + label.geometry().height();
        let picker_top = picker.map_to_native_window(picker.geometry().origin).y;
        assert!(
            label_bottom <= picker_top + 0.5,
            "{case}: the label must be above the picker ({label_bottom} > {picker_top})"
        );
    }
}

/// The label of the button that holds the keyboard focus.
fn focused_button(native: &slint::platform::software_renderer::MinimalSoftwareWindow) -> String {
    use i_slint_core::{
        accessibility::AccessibleStringProperty, item_tree::ParentItemTraversalMode,
    };
    let mut item = WindowInner::from_pub(native.window())
        .focus_item
        .borrow()
        .upgrade();
    while let Some(current) = item {
        if current.accessible_role() == AccessibleRole::Button {
            return current
                .accessible_string_property(AccessibleStringProperty::Label)
                .unwrap_or_default()
                .to_string();
        }
        item = current.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    String::new()
}

fn key(
    native: &slint::platform::software_renderer::MinimalSoftwareWindow,
    text: impl Into<slint::SharedString>,
) {
    let text = text.into();
    let window = native.window();
    window.dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
    window.dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
}

#[test]
fn find_footers_show_whole_evenly_spaced_actions_and_move_the_rest_into_more() {
    use midnight_ui::FindRow;
    use slint::platform::Key;
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let window = FindWindow::new().unwrap();
    window.set_rows(ModelRc::new(VecModel::from(vec![FindRow {
        token: "3:1".into(),
        name: "report.txt".into(),
        ..Default::default()
    }])));
    window.set_selected_row(0);
    let chosen = Rc::new(RefCell::new(Vec::new()));
    window.on_reveal({
        let chosen = chosen.clone();
        move |token| chosen.borrow_mut().push(format!("Go to file {token}"))
    });
    window.on_file_action({
        let chosen = chosen.clone();
        move |token, action| chosen.borrow_mut().push(format!("action {action} {token}"))
    });
    window.on_copy_details({
        let chosen = chosen.clone();
        move || chosen.borrow_mut().push("Copy details".into())
    });
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    let query = ["Find", "Go to file", "View", "Edit", "Open", "Copy details"];
    let details = ["Find", "Copy", "Back"];
    let mut overflowed = false;
    for (theme, density, font, width, height) in [
        (PreferenceTheme::Dark, 1, 15, 320, 200),
        (PreferenceTheme::Blue, 0, 12, 320, 200),
        (PreferenceTheme::Blue, 0, 12, 560, 420),
        (PreferenceTheme::Dark, 1, 15, 560, 420),
        (PreferenceTheme::Blue, 0, 12, 1000, 620),
        // The tool_reflow minimum: even Find and a captioned More… do not fit.
        (PreferenceTheme::Dark, 1, 32, 320, 200),
    ] {
        window.invoke_apply_preferences(theme.index(), density, "".into(), font, font);
        for (page, labels) in [(false, &query[..]), (true, &details[..])] {
            window.set_show_details(page);
            let case =
                format!("{theme:?} density {density} font {font} {width}x{height} details {page}");
            modeless::draw(&native, width, height, "find-footer-fit");
            // Every footer button whole, 8 px from the next one, Close last.
            let row = modeless::fitted_footer(&native, "Close", &case);
            let names: Vec<&str> = row.iter().map(String::as_str).collect();
            if !page && width == 320 {
                assert_eq!(names, ["Find", "More…", "Close"], "{case}");
            }
            if !page && width == 1000 {
                assert_eq!(names[..6], query, "{case}: {names:?}");
            }
            let more = names.contains(&"More…");
            assert_eq!(
                (
                    window.get_footer_buttons(),
                    window.get_footer_overflow() > 0
                ),
                (names.len() as i32 - 1 - i32::from(more), more),
                "{case}: {names:?}"
            );
            // The buttons keep the actions' order.
            let in_row: Vec<&str> = labels
                .iter()
                .copied()
                .filter(|label| names.contains(label))
                .collect();
            assert_eq!(
                in_row,
                names[..names.len() - 1 - usize::from(more)],
                "{case}"
            );
            if !more {
                assert_eq!(in_row.len(), labels.len(), "{case}: {names:?}");
                continue;
            }
            overflowed = true;
            // Every other action is in the More… menu, and only there.
            modeless::click(&native, "More…");
            for label in labels {
                let in_menu =
                    modeless::accessible_popup_item(&native, AccessibleRole::Button, label)
                        .is_some();
                assert_ne!(
                    in_menu,
                    names.contains(label),
                    "{case}: {label} must be a button or in More…, not both"
                );
            }
            key(&native, Key::Escape);
            let open = labels.iter().any(|label| {
                modeless::accessible_popup_item(&native, AccessibleRole::Button, label).is_some()
            });
            assert!(!open, "{case}: Escape closes More…");
        }
    }
    assert!(overflowed, "the narrow cases move actions into More…");

    // At 320×200 the menu's actions run, by pointer and from the keyboard.
    window.invoke_apply_preferences(PreferenceTheme::Blue.index(), 0, "".into(), 12, 15);
    window.set_show_details(false);
    modeless::draw(&native, 320, 200, "find-footer-fit-menu");
    chosen.borrow_mut().clear();
    modeless::click(&native, "More…");
    modeless::click(&native, "Copy details");
    assert_eq!(chosen.borrow().as_slice(), ["Copy details"]);
    // Tab reaches More…; Space opens it and Enter runs its first action.
    let mut tabs = 0;
    while focused_button(&native) != "More…" {
        key(&native, Key::Tab);
        tabs += 1;
        assert!(tabs < 80, "Tab never reaches More…");
    }
    key(&native, Key::Space);
    let first = *query
        .iter()
        .find(|label| {
            modeless::accessible_popup_item(&native, AccessibleRole::Button, label).is_some()
        })
        .expect("Space on More… opens its menu");
    chosen.borrow_mut().clear();
    key(&native, Key::Return);
    let expected = match first {
        "Go to file" => "Go to file 3:1",
        "View" => "action 0 3:1",
        "Edit" => "action 2 3:1",
        "Open" => "action 3 3:1",
        other => other,
    };
    assert_eq!(chosen.borrow().as_slice(), [expected], "Enter runs {first}");
    window.hide().unwrap();
}

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

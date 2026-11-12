//! Measure actual drive captions, not a duplicate of their Slint width expression.
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    item_tree::ItemRc,
    items::{AccessibleRole, ComplexText, SimpleText},
    layout::Orientation,
};
use midnight_ui::{LocationRole, LocationRow, NavigationChoiceRow, Theme};
use slint::{ComponentHandle, ModelRc, VecModel, platform::WindowAdapter};
use std::rc::Rc;

fn caption(item: ItemRc, label: &str) -> Option<ItemRc> {
    if item.is_visible()
        && (item
            .downcast::<SimpleText>()
            .is_some_and(|text| text.as_pin_ref().text() == label)
            || item
                .downcast::<ComplexText>()
                .is_some_and(|text| text.as_pin_ref().text() == label))
    {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = caption(item, label) {
            return Some(found);
        }
    }
    None
}

#[test]
fn drive_popup_expands_to_show_full_paths_while_button_stays_compact() {
    let mut ui = ui::TestUi::new();
    let adapter: Rc<dyn WindowAdapter> = ui.native.clone();
    ui.resize(1600, 900);
    let label =
        "7  C:\\Users\\Test User\\Documents\\Projects\\Midnight Explorer\\Unicode 文档\\Work files";
    ui.window
        .set_navigation_choices(ModelRc::new(VecModel::from(vec![NavigationChoiceRow {
            identity: "bookmark:7".into(),
            label: label.into(),
            tooltip: label.into(),
            ..Default::default()
        }])));
    for (density, font) in [(0, 13), (1, 15)] {
        ui.window.invoke_apply_preferences(0, density);
        ui.window.invoke_apply_fonts("".into(), font, font);
        ui.draw();
        let button = ui::root_item(&ui.window, "Drive or volume", AccessibleRole::Combobox);
        assert!(
            button.geometry().size.width <= 54.0,
            "trigger must stay compact"
        );
        ui::click_root(&ui.window, "Drive or volume", AccessibleRole::Combobox);
        ui.draw();
        let row = ui::popup_item(&ui.window, label, AccessibleRole::Button);
        let text = caption(row.clone(), label).expect("full path text node");
        let measured = text
            .borrow()
            .as_ref()
            .layout_info(
                Orientation::Horizontal,
                text.geometry().size.height,
                &adapter,
                &text,
            )
            .preferred;
        assert!(
            text.geometry().size.width + 1.0 >= measured,
            "path is clipped: available={}, needed={measured}",
            text.geometry().size.width
        );
        assert!(
            row.geometry().size.width > 540.0,
            "long path requires a wider popup"
        );
        ui::key(&ui.window, slint::platform::Key::Escape);
    }
}

#[test]
fn drive_buttons_fit_the_caption_without_fixed_empty_trailing_space() {
    let mut ui = ui::TestUi::new();
    let adapter: Rc<dyn WindowAdapter> = ui.native.clone();
    for (density, font) in [(0, 11), (1, 15), (0, 32), (1, 32)] {
        ui.window.invoke_apply_preferences(0, density);
        ui.window.invoke_apply_fonts("".into(), font, font);
        for label in ["I:", "D:", "Work 文档"] {
            ui.window
                .set_left_locations(ModelRc::new(VecModel::from(vec![LocationRow {
                    label: label.into(),
                    compact_label: label.into(),
                    tooltip: "Measured drive".into(),
                    identity: "raw-drive-identity".into(),
                    kind: LocationRole::Drive,
                    is_drive: true,
                    ..Default::default()
                }])));
            ui.draw();
            let button = ui::root_item(&ui.window, "Measured drive", AccessibleRole::Button);
            let text = caption(button.clone(), label).expect("visible drive caption");
            let width = text
                .borrow()
                .as_ref()
                .layout_info(
                    Orientation::Horizontal,
                    text.geometry().size.height,
                    &adapter,
                    &text,
                )
                .preferred;
            let button_left = button.map_to_native_window(button.geometry().origin).x;
            let text_left = text.map_to_native_window(text.geometry().origin).x;
            let actual = button.geometry().size.width;
            let minimum = ui.window.global::<Theme>().get_tool_width();
            let trailing = if density == 0 { 4.0 } else { 8.0 };
            let necessary = (text_left - button_left + width + trailing).max(minimum);
            assert!(
                (actual - necessary).abs() <= 1.0,
                "density={density}, font={font}, label={label}: width={actual}, needed={necessary}"
            );
            assert!(
                text.geometry().size.width + 1.0 >= width,
                "caption must remain legible"
            );
            assert!(
                actual >= minimum,
                "preserve the density's minimum pointer target"
            );
        }
    }
}

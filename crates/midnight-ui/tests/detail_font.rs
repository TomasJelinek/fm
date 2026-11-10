//! Detail text must fit its rendered viewport at every supported font size.
use crate::ui;

use i_slint_core::{
    item_tree::ItemRc,
    items::{AccessibleRole, ComplexText, Orientation, SimpleText},
};
use midnight_ui::DetailRow;
use slint::{ModelRc, VecModel, platform::WindowAdapter};
use std::rc::Rc;

fn text_item(item: ItemRc, expected: &str) -> Option<ItemRc> {
    let matches = item
        .downcast::<SimpleText>()
        .is_some_and(|text| text.as_pin_ref().text() == expected)
        || item
            .downcast::<ComplexText>()
            .is_some_and(|text| text.as_pin_ref().text() == expected);
    if matches {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = text_item(item, expected) {
            return Some(found);
        }
    }
    None
}

#[test]
fn detail_strip_contains_actual_text_metrics_at_maximum_desktop_font() {
    let mut ui = ui::TestUi::new();
    let value = "Žluťoučký gjpq 文档";
    ui.window
        .set_left_details(ModelRc::new(VecModel::from(vec![DetailRow {
            identity: "detail-font-fixture".into(),
            label: "Name".into(),
            value: value.into(),
        }])));
    let adapter: Rc<dyn WindowAdapter> = ui.native.clone();
    for (density, font) in [(0, 11), (1, 15), (0, 32), (1, 32)] {
        ui.window.invoke_apply_preferences(0, density);
        ui.window.invoke_apply_fonts("".into(), font, font);
        ui.draw();
        let field = ui::root_item(
            &ui.window,
            &format!("Name: {value}. Copy to clipboard"),
            AccessibleRole::Button,
        );
        let text = text_item(field.clone(), value).expect("the actual detail text");
        let geometry = text.geometry();
        let needed = text.borrow().as_ref().layout_info(
            Orientation::Vertical,
            geometry.size.width,
            &adapter,
            &text,
        );
        assert!(
            geometry.size.height + 1.0 >= needed.preferred,
            "detail text is vertically clipped: density={density}, font={font}, \
             viewport={}, preferred={}",
            geometry.size.height,
            needed.preferred,
        );
        let text_top = text.map_to_native_window(geometry.origin).y;
        let field_top = field.map_to_native_window(field.geometry().origin).y;
        assert!(text_top >= field_top - 1.0);
        assert!(
            text_top + needed.preferred <= field_top + field.geometry().size.height + 1.0,
            "detail glyph metrics must fit the field's rendered scroll viewport",
        );
    }
}

//! Exercise mode changes through the actual picker and virtualized file cells.
use crate::ui;

use i_slint_core::{
    item_tree::ItemRc,
    items::{AccessibleRole, ImageItem},
};
use midnight_ui::{FileRow, Theme};
use slint::{ComponentHandle, Model, ModelNotify};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

struct Rows {
    prefix: &'static str,
    reads: Cell<usize>,
    notify: ModelNotify,
    icon: slint::Image,
    revised: Cell<bool>,
}

impl Model for Rows {
    type Data = FileRow;
    fn row_count(&self) -> usize {
        10_000
    }
    fn row_data(&self, index: usize) -> Option<FileRow> {
        if index >= self.row_count() {
            return None;
        }
        self.reads.set(self.reads.get() + 1);
        Some(FileRow {
            identity: format!("{}-{index}", self.prefix).into(),
            name: format!(
                "{}-{index:05}{}",
                self.prefix,
                if index == 1 && self.revised.get() {
                    "-updated"
                } else {
                    ""
                }
            )
            .into(),
            size: "123 B".into(),
            modified: "2026-11-05".into(),
            icon: self.icon.clone(),
            selected: index == 1,
            cursor: index == 0,
            ..Default::default()
        })
    }
    fn model_tracker(&self) -> &dyn slint::ModelTracker {
        &self.notify
    }
}

fn origin(item: &ItemRc) -> (f32, f32) {
    let point = item.map_to_native_window(item.geometry().origin);
    (point.x, point.y)
}

fn largest_icon(item: ItemRc) -> f32 {
    let mut height = if item.downcast::<ImageItem>().is_some() {
        item.geometry().height()
    } else {
        0.0
    };
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        height = height.max(largest_icon(item));
    }
    height
}

fn choose(ui: &mut ui::TestUi, pane: &str, mode: &str) {
    ui::click_root(
        &ui.window,
        &format!("{pane} panel view mode"),
        AccessibleRole::Combobox,
    );
    ui.draw();
    ui::click_popup(&ui.window, mode, AccessibleRole::Button);
    ui.draw();
}

#[test]
fn independent_modes_preserve_file_identity_and_virtualize_column_and_icon_layouts() {
    let mut ui = ui::TestUi::new();
    let icon = slint::Image::from_rgb8(slint::SharedPixelBuffer::clone_from_slice(
        &[255, 0, 0],
        1,
        1,
    ));
    let left = Rc::new(Rows {
        prefix: "left",
        reads: Cell::new(0),
        notify: ModelNotify::default(),
        icon: icon.clone(),
        revised: Cell::new(false),
    });
    let right = Rc::new(Rows {
        prefix: "right",
        reads: Cell::new(0),
        notify: ModelNotify::default(),
        icon,
        revised: Cell::new(false),
    });
    ui.window.set_left_rows(left.clone().into());
    ui.window.set_right_rows(right.clone().into());
    ui.window.set_left_cursor(0);
    ui.window.set_left_path("C:\\kept\\文档".into());
    ui.window.set_left_sort(2);
    ui.window.set_left_descending(true);
    let clicked = Rc::new(RefCell::new(Vec::new()));
    let capture = clicked.clone();
    ui.window
        .on_row_click(move |pane, row, _, _| capture.borrow_mut().push((pane, row)));
    ui.draw();
    let row = ui::root_item(&ui.window, "left-00000", AccessibleRole::ListItem);
    assert!(ui::text_values(row).iter().any(|text| text == "123 B"));
    let right_before = origin(&ui::root_item(
        &ui.window,
        "right-00001",
        AccessibleRole::ListItem,
    ));
    left.reads.set(0);

    choose(&mut ui, "Left", "Brief");
    let first = ui::root_item(&ui.window, "left-00000", AccessibleRole::ListItem);
    let second = ui::root_item(&ui.window, "left-00001", AccessibleRole::ListItem);
    let (x0, y0) = origin(&first);
    let (x1, y1) = origin(&second);
    assert!(
        (x1 - x0).abs() < 1.0 && y1 > y0,
        "Brief fills each column downward"
    );
    let rows_per_column = (ui.window.get_left_list_height()
        / ui.window.global::<Theme>().get_row_height())
    .floor() as usize;
    let next_column = ui::root_item(
        &ui.window,
        &format!("left-{rows_per_column:05}"),
        AccessibleRole::ListItem,
    );
    let (x2, y2) = origin(&next_column);
    assert!(
        x2 > x0 + 100.0 && (y2 - y0).abs() < 1.0,
        "Brief starts the next column at its top"
    );
    assert!(!ui::text_values(first).iter().any(|text| text == "123 B"));
    assert!(
        left.reads.get() < 500,
        "Brief must only read visible cells, got {}",
        left.reads.get()
    );
    assert_eq!(
        origin(&ui::root_item(
            &ui.window,
            "right-00001",
            AccessibleRole::ListItem
        )),
        right_before
    );
    left.reads.set(0);

    choose(&mut ui, "Left", "Thumbnails");
    let first = ui::root_item(&ui.window, "left-00000", AccessibleRole::ListItem);
    let second = ui::root_item(&ui.window, "left-00001", AccessibleRole::ListItem);
    let (x0, y0) = origin(&first);
    let (x1, y1) = origin(&second);
    assert!(
        x1 > x0 + 60.0 && (y1 - y0).abs() < 1.0,
        "Thumbnails fill a row of icon cells"
    );
    assert!(
        largest_icon(first) >= 64.0,
        "Thumbnails use large host icons"
    );
    let selected = ui.window.global::<Theme>().get_selected_background();
    let pixel = ui.pixel(x1 + 8.0, y1 + 8.0);
    assert_eq!(
        (pixel.r, pixel.g, pixel.b),
        (selected.red(), selected.green(), selected.blue())
    );
    assert!(
        left.reads.get() < 500,
        "Thumbnails must only read visible cells, got {}",
        left.reads.get()
    );
    ui::click_root(&ui.window, "left-00001", AccessibleRole::ListItem);
    assert_eq!(
        clicked.borrow().last(),
        Some(&(0, 1)),
        "cell clicks retain source row identity"
    );
    assert_eq!(ui.window.get_left_path().as_str(), "C:\\kept\\文档");
    assert_eq!(ui.window.get_left_cursor(), 0);
    assert_eq!(ui.window.get_left_sort(), 2);
    assert!(ui.window.get_left_descending());
    choose(&mut ui, "Right", "Brief");
    assert_eq!(
        origin(&ui::root_item(
            &ui.window,
            "left-00001",
            AccessibleRole::ListItem
        )),
        (x1, y1)
    );
    left.revised.set(true);
    left.notify.row_changed(1);
    ui.draw();
    let _ = ui::root_item(&ui.window, "left-00001-updated", AccessibleRole::ListItem);
    choose(&mut ui, "Left", "Detailed");
    let first = ui::root_item(&ui.window, "left-00000", AccessibleRole::ListItem);
    assert!(ui::text_values(first).iter().any(|text| text == "123 B"));

    // A layout switch must reveal the same source identity far beyond the
    // first virtualized viewport, with bounded metadata refresh ranges.
    ui.window.set_left_cursor(1375);
    ui.window.invoke_reveal_left();
    ui.draw();
    for mode in ["Brief", "Thumbnails", "Detailed"] {
        choose(&mut ui, "Left", mode);
        let _ = ui::root_item(&ui.window, "left-01375", AccessibleRole::ListItem);
        assert!(ui.window.get_left_visible_row_start() <= 1375);
        assert!(ui.window.get_left_visible_row_end() > 1375);
        assert!(
            ui.window.get_left_visible_row_end() - ui.window.get_left_visible_row_start() < 500,
            "visible-source notifications stay bounded after a mode switch"
        );
        assert_eq!(ui.window.get_left_cursor(), 1375);
    }

    ui.window.set_density_index(1);
    ui.window.set_panel_ratio(0.0);
    ui.draw();
    let pane_left = ui.window.get_left_list_left();
    let pane_right = pane_left + ui.window.get_left_list_width();
    for (label, role) in [
        ("New empty file (Shift+F4)", AccessibleRole::Button),
        ("Left panel view mode", AccessibleRole::Combobox),
    ] {
        let item = ui::root_item(&ui.window, label, role);
        let (x, _) = origin(&item);
        assert!(
            x >= pane_left && x + item.geometry().width() <= pane_right + 1.0,
            "{label} must remain inside the narrow Touch pane: {x} + {} <= {pane_right}",
            item.geometry().width()
        );
    }
    let mutations = Rc::new(RefCell::new(Vec::new()));
    let capture = mutations.clone();
    ui.window
        .on_mutation(move |pane, action| capture.borrow_mut().push((pane, action)));
    ui::click_root(
        &ui.window,
        "New empty file (Shift+F4)",
        AccessibleRole::Button,
    );
    assert_eq!(mutations.borrow().last(), Some(&(0, 4)));
    choose(&mut ui, "Left", "Thumbnails");
    assert_eq!(ui.window.get_left_view_mode(), 2);
}

//! Render the archive entrance mark in every panel mode without changing row geometry.
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::items::AccessibleRole;
use midnight_ui::FileRow;
use slint::{Model, VecModel};
use std::rc::Rc;

#[test]
fn archive_entrance_badge_is_visible_in_all_modes_and_preserves_compression() {
    let mut ui = ui::TestUi::new();
    let model = Rc::new(VecModel::from(vec![FileRow {
        identity: "archive".into(),
        name: "package.whl".into(),
        compressed: true,
        ..Default::default()
    }]));
    ui.window.set_left_rows(model.clone().into());
    for theme in [0, 1, 2] {
        ui.window.set_theme_index(theme);
        for mode in [0, 1, 2] {
            ui.window.set_left_view_mode(mode);
            let mut row = model.row_data(0).unwrap();
            row.browsable_archive = false;
            model.set_row_data(0, row.clone());
            ui.draw();
            let before = ui.pixels.clone();
            let old = ui::root_item(
                &ui.window,
                "package.whl · Filesystem compressed",
                AccessibleRole::ListItem,
            );
            let geometry = old.geometry();
            row.browsable_archive = true;
            model.set_row_data(0, row);
            ui.draw();
            let item = ui::root_item(
                &ui.window,
                "package.whl · Filesystem compressed · Archive: open as folder",
                AccessibleRole::ListItem,
            );
            assert_eq!(item.geometry(), geometry, "badge must not resize rows");
            let changed = before
                .iter()
                .zip(&ui.pixels)
                .filter(|(old, new)| old.r != new.r || old.g != new.g || old.b != new.b)
                .count();
            assert!(
                changed >= 8,
                "archive badge must draw visibly: theme {theme}, mode {mode}"
            );
        }
    }
}

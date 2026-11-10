//! Render the archive entrance mark in every panel mode without changing row geometry.
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::items::AccessibleRole;
use midnight_ui::FileRow;
use slint::{Model, VecModel};
use std::rc::Rc;

#[test]
fn archive_vector_fallback_is_visible_and_associated_artwork_wins_in_every_view() {
    let mut ui = ui::TestUi::new();
    let model = Rc::new(VecModel::from(vec![FileRow {
        identity: "archive-art".into(),
        name: "package.whl".into(),
        browsable_archive: true,
        ..Default::default()
    }]));
    ui.window.set_left_rows(model.clone().into());
    for theme in midnight_core::PreferenceTheme::ALL.map(|theme| theme.index()) {
        ui.window.set_theme_index(theme);
        for mode in 0..3 {
            ui.window.set_left_view_mode(mode);
            let mut row = model.row_data(0).unwrap();
            row.icon = Default::default();
            model.set_row_data(0, row.clone());
            ui.draw();
            let item = ui::root_item(
                &ui.window,
                "package.whl · Archive: open as folder",
                AccessibleRole::ListItem,
            );
            let rectangle = item.geometry();
            let origin = item.map_to_native_window(rectangle.origin);
            let width = ui.width;
            let height = ui.height;
            let sample = |pixels: &[slint::Rgb8Pixel]| -> Vec<slint::Rgb8Pixel> {
                let mut result = Vec::new();
                for y in origin.y.max(0.0) as usize
                    ..((origin.y + rectangle.size.height) as usize).min(height)
                {
                    for x in origin.x.max(0.0) as usize
                        ..((origin.x + rectangle.size.width) as usize).min(width)
                    {
                        result.push(pixels[y * width + x]);
                    }
                }
                result
            };
            let fallback = sample(&ui.pixels);
            assert!(
                fallback
                    .iter()
                    .filter(|pixel| pixel.r > pixel.g.saturating_add(20)
                        && pixel.g > pixel.b.saturating_add(30))
                    .count()
                    > 5,
                "visible golden archive: {theme}/{mode}"
            );
            let pixels = [250_u8, 0, 200, 255].repeat(16 * 16);
            row.icon = slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
                &pixels, 16, 16,
            ));
            model.set_row_data(0, row);
            ui.draw();
            assert!(
                sample(&ui.pixels)
                    .iter()
                    .filter(|pixel| pixel.r > 240 && pixel.g < 5 && pixel.b > 190)
                    .count()
                    > 20,
                "associated application art remains visible: {theme}/{mode}"
            );
        }
    }
}

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

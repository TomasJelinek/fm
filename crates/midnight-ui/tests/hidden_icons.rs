//! Native attribute metadata reaches real file/folder artwork in all panel modes.
#![cfg(windows)]

#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, ComplexText, ImageItem, Opacity, Path, SimpleText},
};
use midnight_core::*;
use midnight_ui::{FileRow, Presenter, install, refresh};
use slint::{Image, Model, ModelRc, SharedPixelBuffer, VecModel};
use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc};

const INK: [u8; 3] = [248, 76, 38];

fn image(item: ItemRc) -> Option<ItemRc> {
    if item.is_visible() && item.downcast::<ImageItem>().is_some() {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = image(item) {
            return Some(found);
        }
    }
    None
}

fn channels(pixel: slint::Rgb8Pixel) -> [u8; 3] {
    [pixel.r, pixel.g, pixel.b]
}

fn effective_opacity(item: &ItemRc) -> f32 {
    let mut current = Some(item.clone());
    let mut opacity = 1.0;
    while let Some(item) = current {
        if let Some(group) = item.downcast::<Opacity>() {
            opacity *= group.as_pin_ref().opacity();
        }
        current = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    opacity
}

fn check_artwork_opacity(item: ItemRc, expected: f32) -> usize {
    if !item.is_visible() {
        return 0;
    }
    let compression_mark = item
        .downcast::<SimpleText>()
        .is_some_and(|text| text.as_pin_ref().text() == "C")
        || item
            .downcast::<ComplexText>()
            .is_some_and(|text| text.as_pin_ref().text() == "C");
    let artwork = item.downcast::<ImageItem>().is_some()
        || item.downcast::<Path>().is_some()
        || compression_mark;
    let mut count = usize::from(artwork);
    if artwork {
        assert!((effective_opacity(&item) - expected).abs() < 0.001);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        count += check_artwork_opacity(item, expected);
    }
    count
}

fn sample(test: &mut ui::TestUi, row: FileRow) -> ([u8; 3], [u8; 3]) {
    let label = row.name.clone();
    test.window
        .set_left_rows(ModelRc::new(VecModel::from(vec![row])));
    test.draw();
    let item = image(ui::root_item(
        &test.window,
        &label,
        AccessibleRole::ListItem,
    ))
    .unwrap();
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    let middle_y = origin.y + rect.height() / 2.0;
    (
        channels(test.pixel(origin.x + rect.width() / 2.0, middle_y)),
        channels(test.pixel(origin.x - 2.0, middle_y)),
    )
}

fn rendered_row(test: &mut ui::TestUi, row: FileRow) -> (ItemRc, ItemRc) {
    let label = format!(
        "{} · Filesystem compressed · Archive: open as folder",
        row.name,
    );
    test.window
        .set_left_rows(ModelRc::new(VecModel::from(vec![row])));
    test.draw();
    let row = ui::root_item(&test.window, &label, AccessibleRole::ListItem);
    let icon = image(row.clone()).unwrap();
    (row, icon)
}

#[test]
fn hidden_and_system_metadata_dims_only_artwork_in_every_mode_and_selection_state() {
    let mut test = ui::TestUi::new();
    let root = PathBuf::from("C:\\");
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    let mut entries = Vec::new();
    // H and S apply to both file and folder; neither filenames nor other flags count.
    for kind in [EntryKind::File, EntryKind::Directory] {
        for attributes in [Some(0x2), Some(0x4), Some(0x6), Some(0x20), None] {
            let mut entry = Entry::new(
                EntryId(entries.len() as u64 + 1),
                root.join(format!(".entry-{}", entries.len())),
                kind,
            );
            entry.metadata.native_attributes = attributes;
            entries.push(entry);
        }
    }
    let index = Arc::new(DirectoryIndex::from_entries(&entries));
    {
        let mut p = presenter.borrow_mut();
        p.apply(Event::PreferencesLoaded(Ok(Box::default())));
    }
    // Installation starts the initial native navigation through the real sender.
    install(&test.window, presenter.clone(), Rc::new(|_| true));
    {
        let mut p = presenter.borrow_mut();
        p.apply(Event::ListingFinished {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            entries: entries.into(),
            index,
        });
    }
    refresh(&test.window, &presenter.borrow());
    let projected = test.window.get_left_rows();
    assert_eq!(projected.row_count(), 10);
    let icon = Image::from_rgb8(SharedPixelBuffer::clone_from_slice(&INK, 1, 1));
    for mode in [0, 1, 2] {
        test.window.set_left_view_mode(mode);
        for selected in [false, true] {
            for index in 0..projected.row_count() {
                let mut row = projected.row_data(index).unwrap();
                row.icon = icon.clone();
                row.selected = selected;
                row.cursor = selected;
                // The non-attribute fields stay identical to the real model projection.
                let (actual, background) = sample(&mut test, row);
                let expected = if index % 5 < 3 {
                    std::array::from_fn(|channel| {
                        ((u16::from(INK[channel]) + u16::from(background[channel])) / 2) as u8
                    })
                } else {
                    INK
                };
                for channel in 0..3 {
                    assert!(
                        actual[channel].abs_diff(expected[channel]) <= 3,
                        "mode {mode}, selected {selected}, entry {index}: {actual:?} != {expected:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn compression_and_archive_overlays_dim_with_the_icon_while_the_filename_stays_opaque() {
    let mut test = ui::TestUi::new();
    let icon = Image::from_rgb8(SharedPixelBuffer::clone_from_slice(&INK, 1, 1));
    for mode in [0, 1, 2] {
        test.window.set_left_view_mode(mode);
        let mut row = FileRow {
            identity: "retained".into(),
            name: "Hidden archive 資料".into(),
            compressed: true,
            browsable_archive: true,
            selected: true,
            cursor: true,
            icon: icon.clone(),
            ..Default::default()
        };
        let (ordinary_row, artwork) = rendered_row(&mut test, row.clone());
        assert_eq!(check_artwork_opacity(ordinary_row.clone(), 1.0), 4);
        let rect = artwork.geometry();
        let origin = artwork.map_to_native_window(rect.origin);
        let background = channels(test.pixel(origin.x - 2.0, origin.y + rect.height() / 2.0));
        let before = test.pixels.clone();
        row.dim_icon = true;
        let (dimmed_row, _) = rendered_row(&mut test, row);
        // The software renderer applies inherited opacity to each primitive,
        // rather than flattening overlapping zipper/C layers before blending.
        // Check actual effective alpha for the image, both zipper paths and C.
        assert_eq!(check_artwork_opacity(dimmed_row.clone(), 0.5), 4);
        let mut overlay_pixels = 0;
        let mut changed_overlay_pixels = 0;
        for y in origin.y.ceil() as usize..(origin.y + rect.height()).floor() as usize {
            for x in origin.x.ceil() as usize..(origin.x + rect.width()).floor() as usize {
                let at = y * test.width + x;
                let original = channels(before[at]);
                if original != INK {
                    overlay_pixels += 1;
                    changed_overlay_pixels += usize::from(before[at] != test.pixels[at]);
                    continue;
                }
                let actual = channels(test.pixels[at]);
                for channel in 0..3 {
                    let expected =
                        ((u16::from(original[channel]) + u16::from(background[channel])) / 2) as u8;
                    assert!(
                        actual[channel].abs_diff(expected) <= 3,
                        "mode {mode} overlay pixel {x},{y}: {actual:?}, original {original:?}"
                    );
                }
            }
        }
        assert!(overlay_pixels > 4, "real overlays must be rendered");
        assert!(
            changed_overlay_pixels > 4,
            "overlay rendering must also change"
        );
        assert_eq!(ordinary_row.geometry(), dimmed_row.geometry());
        let row_rect = dimmed_row.geometry();
        let row_origin = dimmed_row.map_to_native_window(row_rect.origin);
        // Text is beside icons in list/brief mode and below them in thumbnail mode.
        let left = if mode == 2 {
            row_origin.x
        } else {
            origin.x + rect.width() + 5.0
        };
        let top = if mode == 2 {
            origin.y + rect.height() + 6.0
        } else {
            row_origin.y
        };
        for y in top.ceil() as usize..(row_origin.y + row_rect.height()).floor() as usize {
            for x in left.ceil() as usize..(row_origin.x + row_rect.width()).floor() as usize {
                let at = y * test.width + x;
                assert_eq!(
                    before[at], test.pixels[at],
                    "mode {mode}: filename region changed"
                );
            }
        }
    }
}

//! Disclosure gutters and partial-size indicators remain aligned in real row widgets.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    item_tree::ItemRc,
    items::{AccessibleRole, ComplexText, ImageItem, Orientation, SimpleText},
    window::WindowInner,
};
use midnight_ui::FileRow;
use slint::{ComponentHandle, ModelRc, VecModel, platform::WindowAdapter};
use std::rc::Rc;

fn localized_rows(test: &mut ui::TestUi) {
    use midnight_core::*;
    use midnight_ui::{Presenter, install, refresh};
    use std::{cell::RefCell, sync::Arc};
    let path = std::path::PathBuf::from(std::path::MAIN_SEPARATOR_STR);
    let state = Rc::new(RefCell::new(Presenter::new(path.clone(), path.clone())));
    let navigation = Rc::new(RefCell::new(None));
    let captured = navigation.clone();
    install(
        &test.window,
        state.clone(),
        Rc::new(move |command| {
            if let Command::Navigate {
                pane: PaneId::Left,
                navigation,
                ..
            } = command
            {
                *captured.borrow_mut() = Some(navigation);
            }
            true
        }),
    );
    state
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
            size_display: SizeDisplay::Bytes,
            ..Default::default()
        }))));
    let entries: Arc<[Entry]> = [
        ("first", "05.11.2025", "9:04:03"),
        ("second", "5. listopadu 2026", "12:34:56 PM"),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (name, date, time))| {
        let mut entry = Entry::new(EntryId(index as u64 + 1), path.join(name), EntryKind::File);
        entry.metadata.size = Some(index as u64 + 17499);
        entry.metadata.localized_modified = Some(LocalizedDateTime {
            date: date.into(),
            time: time.into(),
        });
        entry
    })
    .collect::<Vec<_>>()
    .into();
    state.borrow_mut().apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation: navigation.borrow().unwrap(),
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    });
    refresh(&test.window, &state.borrow());
    test.draw();
}

fn text_bounds(test: &ui::TestUi, caption: &str) -> (f32, f32) {
    let item = rendered_text(test, caption);
    let rect = item.geometry();
    let left = item.map_to_native_window(rect.origin).x;
    (left, left + rect.width())
}

#[test]
fn size_heading_and_values_share_the_same_right_edge() {
    let mut test = ui::TestUi::new();
    localized_rows(&mut test);
    for (width, font) in [(1340, 11), (2000, 18)] {
        test.resize(width, 820);
        test.window.invoke_apply_fonts("".into(), font, font);
        for column in [0, 1, 2, 3] {
            test.window.set_left_sort(column);
            test.draw();
            let heading = text_bounds(&test, "Size");
            let value = text_bounds(&test, "17499");
            assert!(
                (heading.1 - value.1).abs() < 1.0,
                "size column shifted at width {width}, font {font}, sort {column}: {heading:?} / {value:?}"
            );
        }
    }
}

fn visible_image(item: ItemRc) -> Option<ItemRc> {
    if item.is_visible() && item.downcast::<ImageItem>().is_some() {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = visible_image(item) {
            return Some(found);
        }
    }
    None
}

#[test]
fn sorted_size_heading_keeps_text_clear_of_its_direction_icon() {
    let mut test = ui::TestUi::new();
    localized_rows(&mut test);
    test.window.set_left_sort(1);
    for width in [640, 760] {
        test.resize(width, 820);
        for font in [24, 32] {
            test.window.invoke_apply_fonts("".into(), font, font);
            for descending in [false, true] {
                test.window.set_left_descending(descending);
                test.draw();
                let header = ui::root_item(&test.window, "Sort by Size", AccessibleRole::Button);
                let icon = visible_image(header).expect("size sort direction icon");
                let icon_bounds = icon.geometry();
                let icon_end =
                    icon.map_to_native_window(icon_bounds.origin).x + icon_bounds.width();
                let text = rendered_text(&test, "Size");
                let text_rect = text.geometry();
                let adapter: Rc<dyn WindowAdapter> = test.native.clone();
                let natural_width = text
                    .borrow()
                    .as_ref()
                    .layout_info(Orientation::Horizontal, text_rect.height(), &adapter, &text)
                    .preferred;
                // Right-aligned text starts after unused space; an elided title
                // may use its entire allocated width, but never the icon slot.
                let text_start = text.map_to_native_window(text_rect.origin).x
                    + (text_rect.width() - natural_width).max(0.0);
                assert!(
                    text_start >= icon_end + 2.0,
                    "Size text overlaps its arrow: width={width}, font={font}, \
                     descending={descending}, text_start={text_start}, icon_end={icon_end}"
                );
                let title_right = text_bounds(&test, "Size").1;
                let value_right = text_bounds(&test, "17499").1;
                assert!((title_right - value_right).abs() < 1.0);
            }
        }
    }
}

#[test]
fn modified_date_and_time_are_independent_aligned_locale_fields() {
    let mut test = ui::TestUi::new();
    localized_rows(&mut test);
    for (width, font) in [(1340, 11), (2000, 18)] {
        test.resize(width, 820);
        test.window.invoke_apply_fonts("".into(), font, font);
        test.draw();
        let date = text_bounds(&test, "05.11.2025");
        let other_date = text_bounds(&test, "5. listopadu 2026");
        let time = text_bounds(&test, "9:04:03");
        let other_time = text_bounds(&test, "12:34:56 PM");
        assert!((date.0 - other_date.0).abs() < 1.0);
        assert!((time.0 - other_time.0).abs() < 1.0);
        assert!(date.1 <= time.0, "date and time overlap");
        assert!((text_bounds(&test, "Modified").0 - date.0).abs() < 1.0);
        assert!((text_bounds(&test, "Time").0 - time.0).abs() < 1.0);
        render::snapshot(&test, &format!("panel-date-time-{width}-{font}"));
    }
    let sorts = Rc::new(std::cell::RefCell::new(Vec::new()));
    let captured = sorts.clone();
    test.window
        .on_sort(move |pane, column| captured.borrow_mut().push((pane, column)));
    ui::click_root(&test.window, "Sort by Modified", AccessibleRole::Button);
    ui::click_root(
        &test.window,
        "Sort by Modified time",
        AccessibleRole::Button,
    );
    assert_eq!(sorts.borrow().as_slice(), &[(0, 2), (0, 2)]);
}

fn text_item(item: ItemRc, caption: &str) -> Option<ItemRc> {
    if item.is_visible()
        && (item
            .downcast::<SimpleText>()
            .is_some_and(|text| text.as_pin_ref().text() == caption)
            || item
                .downcast::<ComplexText>()
                .is_some_and(|text| text.as_pin_ref().text() == caption))
    {
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

fn rendered_text(test: &ui::TestUi, caption: &str) -> ItemRc {
    text_item(
        ItemRc::new_root(WindowInner::from_pub(test.window.window()).component()),
        caption,
    )
    .unwrap_or_else(|| panic!("missing visible text: {caption}"))
}

#[test]
fn folder_links_files_and_folders_share_the_same_name_gutter() {
    let mut test = ui::TestUi::new();
    test.window.set_left_rows(ModelRc::new(VecModel::from(vec![
        FileRow {
            identity: "folder".into(),
            name: "Ordinary folder".into(),
            directory: true,
            tree_disclosure: 3,
            ..Default::default()
        },
        FileRow {
            identity: "junction".into(),
            name: "Documents and Settings".into(),
            kind: "Folder link".into(),
            directory: true,
            ..Default::default()
        },
        FileRow {
            identity: "file".into(),
            name: "regular.txt".into(),
            ..Default::default()
        },
    ])));
    for mode in [0, 1] {
        test.window.set_left_view_mode(mode);
        test.draw();
        let positions: Vec<_> = ["Ordinary folder", "Documents and Settings", "regular.txt"]
            .into_iter()
            .map(|name| {
                let item = rendered_text(&test, name);
                item.map_to_native_window(item.geometry().origin).x
            })
            .collect();
        assert!(
            (positions[0] - positions[1]).abs() < 1.0,
            "mode {mode}: {positions:?}"
        );
        assert!(
            (positions[0] - positions[2]).abs() < 1.0,
            "mode {mode}: {positions:?}"
        );
        render::snapshot(&test, &format!("panel-row-gutter-mode-{mode}"));
    }
}

#[test]
fn partial_size_error_stays_beside_the_number_in_a_wide_panel() {
    let mut test = ui::TestUi::new();
    test.resize(2200, 820);
    test.window
        .set_left_rows(ModelRc::new(VecModel::from(vec![FileRow {
            identity: "partial".into(),
            name: "Protected folder".into(),
            directory: true,
            tree_disclosure: 3,
            size: "≈ 129".into(),
            size_state: 3,
            ..Default::default()
        }])));
    test.draw();
    let glyph = ui::root_item(&test.window, "Folder size warning", AccessibleRole::Button);
    let value = rendered_text(&test, "≈ 129");
    let adapter: Rc<dyn WindowAdapter> = test.native.clone();
    let natural_width = value
        .borrow()
        .as_ref()
        .layout_info(
            Orientation::Horizontal,
            value.geometry().size.height,
            &adapter,
            &value,
        )
        .preferred;
    let value_start = value.map_to_native_window(value.geometry().origin).x
        + (value.geometry().size.width - natural_width).max(0.0);
    let glyph_end =
        glyph.map_to_native_window(glyph.geometry().origin).x + glyph.geometry().size.width;
    let gap = value_start - glyph_end;
    assert!(
        (0.0..=8.0).contains(&gap),
        "size warning is detached from its value: gap={gap}"
    );
    render::snapshot(&test, "panel-size-warning-adjacent");
}

#[test]
fn directory_names_use_empty_size_space_without_moving_metadata_or_file_names() {
    let mut test = ui::TestUi::new();
    test.window.set_sizes_in_bytes(true);
    test.window.set_left_rows(ModelRc::new(VecModel::from(vec![
        FileRow {
            identity: "provider".into(),
            name: "Microsoft Terminal Services".into(),
            directory: true,
            size: "<DIR>".into(),
            modified_date: "provider-date".into(),
            ..Default::default()
        },
        FileRow {
            identity: "folder".into(),
            name: "A long folder name with a numeric size".into(),
            directory: true,
            size: "≈ 129".into(),
            size_state: 3,
            modified_date: "folder-date".into(),
            ..Default::default()
        },
        FileRow {
            identity: "file".into(),
            name: "A regular file keeps its filename column.txt".into(),
            size: "17499".into(),
            modified_date: "file-date".into(),
            ..Default::default()
        },
    ])));
    for (width, font) in [(1340, 11), (2700, 24), (1800, 24)] {
        test.resize(width, 900);
        test.window.invoke_apply_fonts("".into(), font, font);
        test.draw();
        let file = text_bounds(&test, "A regular file keeps its filename column.txt");
        let folder = text_bounds(&test, "Microsoft Terminal Services");
        let directory_size = rendered_text(&test, "<DIR>");
        let rect = directory_size.geometry();
        let adapter: Rc<dyn WindowAdapter> = test.native.clone();
        let natural_width = directory_size
            .borrow()
            .as_ref()
            .layout_info(
                Orientation::Horizontal,
                rect.height(),
                &adapter,
                &directory_size,
            )
            .preferred;
        let directory_name = rendered_text(&test, "Microsoft Terminal Services");
        let name_rect = directory_name.geometry();
        let natural_name_width = directory_name
            .borrow()
            .as_ref()
            .layout_info(
                Orientation::Horizontal,
                name_rect.height(),
                &adapter,
                &directory_name,
            )
            .preferred;
        assert!(
            name_rect.width() >= natural_name_width,
            "the complete provider caption must fit at width {width}, font {font}"
        );
        let size_start = directory_size.map_to_native_window(rect.origin).x
            + (rect.width() - natural_width).max(0.0);
        assert!(
            folder.1 > file.1 + 30.0,
            "unused byte-size space must be available to directory names: {folder:?} / {file:?}"
        );
        assert!(
            (6.0..=12.0).contains(&(size_start - folder.1)),
            "directory label must stop before its actual size text: {folder:?} / {size_start}"
        );
        let warning = ui::root_item(&test.window, "Folder size warning", AccessibleRole::Button);
        let warning_start = warning.map_to_native_window(warning.geometry().origin).x;
        let highlighted = text_bounds(&test, "A long folder name with a numeric size");
        assert!(highlighted.1 <= warning_start - 4.0);
        let date = text_bounds(&test, "provider-date");
        assert!((date.0 - text_bounds(&test, "folder-date").0).abs() < 1.0);
        assert!((date.0 - text_bounds(&test, "file-date").0).abs() < 1.0);
        assert!((text_bounds(&test, "Size").1 - text_bounds(&test, "17499").1).abs() < 1.0);
    }
    render::snapshot(&test, "panel-directory-name-space");
}

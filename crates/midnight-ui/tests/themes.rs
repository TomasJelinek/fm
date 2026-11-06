//! Readability, real Options selection, and rendered panels for every theme.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::items::AccessibleRole;
use midnight_ui::{FileRow, Theme};
use slint::{Color, ComponentHandle, VecModel, platform::Key};
use std::{cell::Cell, collections::HashSet, rc::Rc};

fn luminance(color: Color) -> f64 {
    let linear = |channel: u8| {
        let channel = f64::from(channel) / 255.0;
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.red()) + 0.7152 * linear(color.green()) + 0.0722 * linear(color.blue())
}

fn contrast(foreground: Color, background: Color) -> f64 {
    let first = luminance(foreground);
    let second = luminance(background);
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

fn readable(theme: &Theme<'_>, name: &str) {
    for foreground in [
        theme.get_text(),
        theme.get_strong_text(),
        theme.get_secondary_text(),
        theme.get_directory_text(),
        theme.get_selected_text(),
        theme.get_search_match(),
        theme.get_pending_size(),
        theme.get_compression_mark(),
    ] {
        for background in [
            theme.get_pane_background(),
            theme.get_alternate_row(),
            theme.get_cursor_background(),
            theme.get_inactive_cursor(),
            theme.get_selected_background(),
        ] {
            assert!(
                contrast(foreground, background) >= 4.5,
                "{name}: row text {foreground:?} on {background:?}"
            );
        }
    }
    for foreground in [
        theme.get_text(),
        theme.get_strong_text(),
        theme.get_secondary_text(),
        theme.get_accent(),
    ] {
        for background in [
            theme.get_window_background(),
            theme.get_surface(),
            theme.get_secondary_surface(),
            theme.get_header(),
            theme.get_active_header(),
            theme.get_hover(),
        ] {
            assert!(
                contrast(foreground, background) >= 4.5,
                "{name}: control text {foreground:?} on {background:?}"
            );
        }
    }
    for (foreground, background, minimum) in [
        (theme.get_error_text(), theme.get_error_background(), 4.5),
        (
            theme.get_selection_text(),
            theme.get_selection_background(),
            4.5,
        ),
        (theme.get_cursor_edge(), theme.get_cursor_background(), 3.0),
        (
            theme.get_selected_edge(),
            theme.get_selected_background(),
            3.0,
        ),
        (theme.get_active_frame(), theme.get_active_header(), 3.0),
        (theme.get_frame(), theme.get_surface(), 3.0),
    ] {
        assert!(
            contrast(foreground, background) >= minimum,
            "{name}: semantic contrast {foreground:?} on {background:?}"
        );
    }
}

#[test]
fn eight_distinct_themes_are_selectable_and_new_palettes_keep_text_readable() {
    let mut test = ui::TestUi::new();
    let choices = [
        "Blue",
        "Dark",
        "Light",
        "Dracula",
        "Nord",
        "Gruvbox Dark",
        "Catppuccin Mocha",
        "Solarized Light",
    ];
    let accepted = Rc::new(Cell::new(0));
    let changed = accepted.clone();
    let weak = test.window.as_weak();
    test.window
        .on_preferences_changed(move |scheme, density, _| {
            changed.set(scheme);
            let window = weak.upgrade().unwrap();
            window.set_theme_index(scheme);
            window.set_density_index(density);
            window.invoke_apply_preferences(scheme, density);
        });
    test.window.set_left_rows(
        Rc::new(VecModel::from(vec![
            FileRow {
                identity: "1".into(),
                name: "Projects 文档".into(),
                directory: true,
                size: "12.4 GB".into(),
                size_state: 1,
                ..Default::default()
            },
            FileRow {
                identity: "2".into(),
                name: "README.md".into(),
                selected: true,
                cursor: true,
                prefix_end: 4,
                size: "24.5 KB".into(),
                modified: "2026-11-06 12:30".into(),
                ..Default::default()
            },
            FileRow {
                identity: "3".into(),
                name: "notes_é.txt".into(),
                compressed: true,
                size: "1.2 KB".into(),
                ..Default::default()
            },
        ]))
        .into(),
    );
    for density in [0, 1] {
        let mut unique = HashSet::new();
        accepted.set(0);
        test.window.set_theme_index(0);
        test.window.set_density_index(density);
        test.window.invoke_apply_preferences(0, density);
        test.window.invoke_apply_fonts("".into(), 11, 15);
        for (index, name) in choices.iter().enumerate() {
            test.window.invoke_show_options();
            test.draw();
            test.options_topic(1);
            ui::click_popup(&test.window, "Theme", AccessibleRole::Combobox);
            if index > 0 {
                ui::key(&test.window, Key::DownArrow);
            }
            ui::key(&test.window, Key::Return);
            test.draw();
            // Fluent animates foregrounds and borders for 150–200 ms. Capture
            // the settled palette, rather than an intermediate light/dark frame.
            std::thread::sleep(std::time::Duration::from_millis(250));
            slint::platform::update_timers_and_animations();
            test.draw();
            assert_eq!(accepted.get(), index as i32, "picker must offer {name}");
            let theme = test.window.global::<Theme>();
            assert_eq!(theme.get_light(), matches!(index, 2 | 7), "{name}");
            let color = theme.get_pane_background();
            assert!(
                unique.insert((color.red(), color.green(), color.blue())),
                "{name}"
            );
            readable(&theme, name);
            render::snapshot(&test, &format!("palette-{index}-density-{density}-options"));
            ui::click_popup(&test.window, "Done", AccessibleRole::Button);
            test.draw();
            render::snapshot(&test, &format!("palette-{index}-density-{density}-panel"));
        }
    }
    test.window.hide().unwrap();
}

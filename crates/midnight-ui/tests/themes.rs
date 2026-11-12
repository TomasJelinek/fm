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
    let minimum = match theme.get_scheme() {
        17..=20 => 7.0,
        21..=22 => 12.0,
        _ => 4.5,
    };
    let row_minimum = if matches!(theme.get_scheme(), 23..=25) {
        7.0
    } else {
        minimum
    };
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
                contrast(foreground, background) >= row_minimum,
                "{name}: row text {foreground:?} on {background:?}, minimum {row_minimum}"
            );
        }
    }
    for foreground in [
        theme.get_text(),
        theme.get_strong_text(),
        theme.get_secondary_text(),
        theme.get_muted_text(),
        theme.get_accent(),
    ] {
        for background in [
            theme.get_window_background(),
            theme.get_pane_background(),
            theme.get_surface(),
            theme.get_secondary_surface(),
            theme.get_header(),
            theme.get_active_header(),
            theme.get_hover(),
        ] {
            assert!(
                contrast(foreground, background) >= minimum,
                "{name}: control text {foreground:?} on {background:?}, minimum {minimum}"
            );
        }
    }
    for (foreground, background, minimum) in [
        (
            theme.get_error_text(),
            theme.get_error_background(),
            minimum,
        ),
        (
            theme.get_selection_text(),
            theme.get_selection_background(),
            minimum,
        ),
        (theme.get_cursor_edge(), theme.get_cursor_background(), 3.0),
        (
            theme.get_selected_edge(),
            theme.get_selected_background(),
            3.0,
        ),
        (
            theme.get_selected_edge(),
            theme.get_cursor_background(),
            3.0,
        ),
        (theme.get_selected_edge(), theme.get_inactive_cursor(), 3.0),
        (theme.get_active_frame(), theme.get_active_header(), 3.0),
        (theme.get_frame(), theme.get_surface(), 3.0),
    ] {
        assert!(
            contrast(foreground, background) >= minimum,
            "{name}: semantic contrast {foreground:?} on {background:?}"
        );
    }
}

fn high_contrast_selection_marks(test: &ui::TestUi) {
    let theme = test.window.global::<Theme>();
    for (label, focused) in [("README.md", true), ("Selected without focus.txt", false)] {
        let row = ui::root_item(&test.window, label, AccessibleRole::ListItem);
        let origin = row.map_to_native_window(row.geometry().origin);
        let stripe = test.pixel(origin.x + 1.0, origin.y + 5.0);
        let top = test.pixel(origin.x + 8.0, origin.y);
        let edge = theme.get_selected_edge();
        assert_eq!(
            (stripe.r, stripe.g, stripe.b),
            (edge.red(), edge.green(), edge.blue())
        );
        // A selected cursor adds an outline to the selected row's side stripe.
        // Focus and selection remain distinct without recognizing hue.
        let expected = if focused {
            theme.get_cursor_edge()
        } else {
            theme.get_selected_background()
        };
        assert_eq!(
            (top.r, top.g, top.b),
            (expected.red(), expected.green(), expected.blue())
        );
    }
}

#[test]
fn all_themes_are_selectable_and_keep_text_readable() {
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
        "Carbon Green",
        "Tokyo Night",
        "One Dark",
        "Rosé Pine",
        "Norton Commander",
        "Amber Terminal",
        "Verdant Light",
        "Catppuccin Latte",
        "Rosé Pine Dawn",
        "Paper Contrast",
        "Sandstone Contrast",
        "Deep Ocean",
        "Graphite Contrast",
        "High Contrast Dark",
        "High Contrast Light",
        "Slate Blue",
        "Slate Teal",
        "Slate Violet",
    ];
    assert_eq!(choices.len(), midnight_core::PreferenceTheme::ALL.len());
    let mut choices: Vec<_> = choices.into_iter().enumerate().collect();
    choices.sort_by_key(|(_, name)| name.to_lowercase());
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
                modified_date: "2026-11-06".into(),
                modified_time: "12:30".into(),
                ..Default::default()
            },
            FileRow {
                identity: "3".into(),
                name: "notes_é.txt".into(),
                compressed: true,
                size: "1.2 KB".into(),
                ..Default::default()
            },
            FileRow {
                identity: "4".into(),
                name: "Selected without focus.txt".into(),
                selected: true,
                size: "8 KB".into(),
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
        let mut current_position = 1usize; // Blue in the alphabetic picker.
        for (position, (index, name)) in choices.iter().enumerate() {
            test.window.invoke_show_options();
            test.draw();
            test.options_topic(1);
            ui::click_popup(&test.window, "Theme", AccessibleRole::Combobox);
            for _ in 0..position.abs_diff(current_position) {
                ui::key(
                    &test.window,
                    if position < current_position {
                        Key::UpArrow
                    } else {
                        Key::DownArrow
                    },
                );
            }
            current_position = position;
            ui::key(&test.window, Key::Return);
            test.draw();
            // Fluent animates foregrounds and borders for 150–200 ms. Capture
            // the settled palette, rather than an intermediate light/dark frame.
            std::thread::sleep(std::time::Duration::from_millis(250));
            slint::platform::update_timers_and_animations();
            test.draw();
            assert_eq!(accepted.get(), *index as i32, "picker must offer {name}");
            let theme = test.window.global::<Theme>();
            assert_eq!(
                theme.get_light(),
                matches!(index, 2 | 7 | 14..=18 | 22),
                "{name}"
            );
            let color = theme.get_pane_background();
            let accent = theme.get_accent();
            assert!(
                unique.insert((
                    color.red(),
                    color.green(),
                    color.blue(),
                    accent.red(),
                    accent.green(),
                    accent.blue(),
                )),
                "{name}"
            );
            readable(&theme, name);
            let background = theme.get_header();
            render::snapshot(&test, &format!("palette-{index}-density-{density}-options"));
            ui::key(&test.window, Key::Escape);
            test.draw();
            // The wide empty part of the bar must use the theme header rather
            // than an uncolored/light surface, in both density modes.
            let options = ui::root_position(&test.window, "Options", AccessibleRole::Button);
            let pixel = test.pixel(test.width as f32 * 0.75, options.y);
            assert_eq!(
                (pixel.r, pixel.g, pixel.b),
                (background.red(), background.green(), background.blue()),
                "{name}: top toolbar background must follow the theme",
            );
            if matches!(index, 21..=22) {
                high_contrast_selection_marks(&test);
            }
            render::snapshot(&test, &format!("palette-{index}-density-{density}-panel"));
        }
    }
    test.window.hide().unwrap();
}

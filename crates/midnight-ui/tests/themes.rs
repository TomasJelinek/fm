//! Readability, real Options selection, and rendered panels for every theme.
#[path = "support/render.rs"]
mod render;
use crate::support::ui;

use i_slint_core::{accessibility::AccessibleStringProperty, items::AccessibleRole};
use midnight_core::PreferenceTheme;
use midnight_ui::{FileRow, Theme};
use slint::{Color, ComponentHandle, VecModel, platform::Key};
use std::{cell::Cell, collections::HashSet, rc::Rc, time::Duration};

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

/// WCAG contrast ratio of two colours (shared with `action_button_states`).
pub(crate) fn contrast(foreground: Color, background: Color) -> f64 {
    let first = luminance(foreground);
    let second = luminance(background);
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

/// The text contrast a palette keeps: WCAG AA, 7:1 for the contrast palettes and 12:1
/// for the high-contrast ones (shared with `action_button_states`).
pub(crate) fn text_minimum(scheme: i32) -> f64 {
    match scheme {
        17..=20 => 7.0,
        21..=22 => 12.0,
        _ => 4.5,
    }
}

fn readable(theme: &Theme<'_>, name: &str) {
    let minimum = text_minimum(theme.get_scheme());
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
        // Error text on the dialog surface: a dangerous dialog's title and the warning
        // under a permanent Delete's question.
        (theme.get_error_text(), theme.get_surface(), minimum),
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
        let marker = test.pixel(origin.x + 5.0, origin.y + row.geometry().height() / 2.0);
        let top = test.pixel(origin.x + 8.0, origin.y);
        let edge = theme.get_selected_edge();
        assert_eq!(
            (marker.r, marker.g, marker.b),
            (edge.red(), edge.green(), edge.blue())
        );
        // The square's lower/upper gap is absent from an ordinary caret stroke.
        let gap = test.pixel(origin.x + 5.0, origin.y + 3.0);
        let fill = theme.get_selected_background();
        assert_eq!(
            (gap.r, gap.g, gap.b),
            (fill.red(), fill.green(), fill.blue())
        );
        // A selected cursor adds an outline around the marked row.
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
    let (mut test, clock) = ui::TestUi::with_virtual_clock();
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
        "Red Lacquer",
        "Overprint",
        "Faience",
        "Afterimage",
        "Last Train",
        "Drawing Office",
        "Letterhead",
        "Field Survey",
        "Mastering Room",
        "Titanium",
        "Banker’s Lamp",
        "Bindery",
        "Chronometer",
        "Iris",
        "Canary Copy",
        "Pencil Lead",
        "Carbon Black",
        "Warm Ash",
        "Pewter",
        "Plum Smoke",
    ];
    assert_eq!(choices.len(), PreferenceTheme::ALL.len());
    let mut choices: Vec<_> = PreferenceTheme::ALL.into_iter().zip(choices).collect();
    choices.sort_by_key(|(_, name)| name.to_lowercase());
    let accepted = Rc::new(Cell::new(None));
    let changed = accepted.clone();
    let host_light = Rc::new(Cell::new(false));
    let appearance = host_light.clone();
    let weak = test.window.as_weak();
    test.window
        .on_preferences_changed(move |scheme, density, field| {
            assert_eq!(field, 0);
            let picked = PreferenceTheme::from_index(scheme).unwrap();
            changed.set(Some(picked));
            let window = weak.upgrade().unwrap();
            if picked.is_light() {
                window.set_light_theme_index(scheme);
            } else {
                window.set_dark_theme_index(scheme);
            }
            if appearance.get() == picked.is_light() {
                window.invoke_apply_preferences(scheme, density);
            }
        });
    test.window.set_left_rows(
        Rc::new(VecModel::from(vec![
            FileRow {
                identity: "1".into(),
                name: "Projects 文档".into(),
                directory: true,
                size: "12.4 GB".into(),
                size_state: 1,
                size_running: true,
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
        test.window
            .set_dark_theme_index(PreferenceTheme::Blue.index());
        test.window
            .set_light_theme_index(PreferenceTheme::Light.index());
        test.window.invoke_apply_fonts("".into(), 11, 15);
        for light in [false, true] {
            host_light.set(light);
            let (label, other_label, initial) = if light {
                (
                    "Light theme",
                    "Dark theme",
                    test.window.get_light_theme_index(),
                )
            } else {
                (
                    "Dark theme",
                    "Light theme",
                    test.window.get_dark_theme_index(),
                )
            };
            test.window.invoke_apply_preferences(initial, density);
            let group: Vec<_> = choices
                .iter()
                .filter(|(theme, _)| theme.is_light() == light)
                .collect();
            let mut current_position = group
                .iter()
                .position(|(theme, _)| theme.index() == initial)
                .unwrap();
            for (position, (choice, name)) in group.into_iter().enumerate() {
                let index = choice.index();
                accepted.set(None);
                // Only the panel frame below is a pixel assertion; the dialog itself is
                // rendered for the first theme of each density (and for artifacts).
                let render_dialog = ui::render_case(position == 0);
                test.window.invoke_show_options();
                if render_dialog {
                    test.draw();
                } else {
                    test.layout();
                }
                test.options_topic(1);
                let inactive_choice =
                    ui::popup_item(&test.window, other_label, AccessibleRole::Combobox)
                        .accessible_string_property(AccessibleStringProperty::Value);
                ui::click_popup(&test.window, label, AccessibleRole::Combobox);
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
                test.layout();
                // Fluent animates foregrounds and borders for 150–200 ms. Capture
                // the settled palette, rather than an intermediate light/dark frame.
                // The virtual clock also keeps the next iteration's clicks single clicks.
                ui::advance(&clock, Duration::from_millis(600));
                if render_dialog {
                    test.draw();
                } else {
                    test.layout();
                }
                assert_eq!(accepted.get(), Some(*choice), "picker must offer {name}");
                assert_eq!(
                    ui::popup_item(&test.window, other_label, AccessibleRole::Combobox)
                        .accessible_string_property(AccessibleStringProperty::Value),
                    inactive_choice,
                    "{other_label} must keep its own choice"
                );
                let picker = ui::popup_item(&test.window, label, AccessibleRole::Combobox);
                assert_eq!(
                    picker
                        .accessible_string_property(AccessibleStringProperty::Value)
                        .as_deref(),
                    Some(*name),
                    "picker label for scheme {index}"
                );
                let theme = test.window.global::<Theme>();
                assert_eq!(theme.get_light(), light, "{name}");
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
    }
    test.window.hide().unwrap();
}

#[test]
fn marked_rows_keep_each_theme_hue_and_distinct_cursor_outline() {
    // Accepted marked-row fills in persisted theme order.
    let selected_rgb = [
        0x375775, 0x4b4b4b, 0xc8c8c8, 0x4d425e, 0x3d5660, 0x435038, 0x414767, 0xccd9ba, 0x3d4824,
        0x393451, 0x434354, 0x3b344c, 0x003e75, 0x49402e, 0xe2ebcc, 0xe6dcf4, 0xedded8, 0xd4e1f0,
        0xe9dbc1, 0x243b49, 0x3a313b, 0x181818, 0xeeeeee, 0x293946, 0x283d38, 0x352f42, 0x643337,
        0xf5aac2, 0x8dcdbc, 0x563b72, 0xdca771, 0x4d2d40, 0xcedace, 0xa5b8cd, 0x33271f, 0xbfb0a1,
        0x19344e, 0x25473d, 0x714345, 0xaca4d6, 0xf4c5d8, 0x252525, 0x383838, 0x514a45, 0x393f42,
        0x343436,
    ];
    assert_eq!(
        selected_rgb.len(),
        midnight_core::PreferenceTheme::ALL.len()
    );
    let mut test = ui::TestUi::new();
    test.resize(900, 600);
    test.window.set_active_pane(0);
    test.window.set_left_rows(
        Rc::new(VecModel::from(vec![
            FileRow {
                identity: "marked-cursor".into(),
                name: "README.md".into(),
                selected: true,
                cursor: true,
                ..Default::default()
            },
            FileRow {
                identity: "marked".into(),
                name: "Selected without focus.txt".into(),
                selected: true,
                ..Default::default()
            },
        ]))
        .into(),
    );
    for density in [0, 1] {
        for (scheme, expected) in selected_rgb.into_iter().enumerate() {
            test.window.invoke_apply_preferences(scheme as i32, density);
            test.draw();
            let theme = test.window.global::<Theme>();
            let actual = theme.get_selected_background();
            let actual = (u32::from(actual.red()) << 16)
                | (u32::from(actual.green()) << 8)
                | u32::from(actual.blue());
            assert_eq!(actual, expected, "theme {scheme}, density {density}");
            assert_ne!(
                theme.get_selected_background(),
                theme.get_cursor_background(),
                "theme {scheme}: selection and cursor must remain distinguishable"
            );
            if (23..=25).contains(&scheme) {
                assert_eq!(theme.get_selected_edge(), theme.get_accent());
            }
            readable(&theme, &format!("theme {scheme}, density {density}"));
            high_contrast_selection_marks(&test);
        }
    }
}

/// Selected text in every palette: the selection text color reads on its fill at the
/// palette's text minimum (4.5:1; 7:1 for the contrast palettes and 12:1 for high
/// contrast), active and while the window is inactive. The fill shows against the page
/// without glaring, except in high contrast, where it stands out strongly; the inactive
/// fill stays visible but is dimmer than the active one.
#[test]
fn text_selection_reads_shows_calmly_and_dims_while_inactive_in_every_palette() {
    let test = ui::TestUi::new();
    let theme = test.window.global::<Theme>();
    let mut findings = Vec::new();
    for scheme in 0..midnight_core::PreferenceTheme::ALL.len() as i32 {
        theme.set_scheme(scheme);
        let page = theme.get_pane_background();
        let minimum = text_minimum(scheme);
        let high_contrast = matches!(scheme, 21 | 22);
        let fill = theme.get_selection_background();
        let inactive_fill = theme.get_inactive_selection_background();
        for (state, text, background) in [
            ("selected", theme.get_selection_text(), fill),
            (
                "inactive",
                theme.get_inactive_selection_text(),
                inactive_fill,
            ),
        ] {
            let ratio = contrast(text, background);
            if ratio < minimum {
                findings.push(format!(
                    "palette {scheme}: {state} text {text:?} on {background:?} is {ratio:.2}:1, minimum {minimum}"
                ));
            }
        }
        let shown = contrast(fill, page);
        let floor = if high_contrast { 7.0 } else { 1.4 };
        if shown < floor {
            findings.push(format!(
                "palette {scheme}: selection {fill:?} on the page {page:?} is {shown:.2}:1, below {floor}"
            ));
        }
        if !high_contrast && shown > 3.0 {
            findings.push(format!(
                "palette {scheme}: selection {fill:?} glares on the page {page:?} at {shown:.2}:1"
            ));
        }
        let dimmed = contrast(inactive_fill, page);
        if dimmed < 1.15 || dimmed >= shown {
            findings.push(format!(
                "palette {scheme}: inactive selection {inactive_fill:?} is {dimmed:.2}:1 on the page, the active one {shown:.2}:1"
            ));
        }
    }
    test.window.hide().unwrap();
    assert!(findings.is_empty(), "{findings:#?}");
}

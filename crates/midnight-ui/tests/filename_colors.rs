#[path = "support/popup_scroll.rs"]
mod popup_scroll;
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::items::AccessibleRole;
use midnight_core::{Command, Event, UserPreferences};
use midnight_ui::{ColorOptionsView, FileRow, FilenameColorPresentation, Presenter, Theme};
use slint::{Color, ComponentHandle, Model, VecModel};
use std::{cell::RefCell, rc::Rc};

fn contrast(first: Color, second: Color) -> f64 {
    let luminance = |color: Color| {
        let channels = [color.red(), color.green(), color.blue()];
        channels
            .into_iter()
            .zip([0.2126, 0.7152, 0.0722])
            .map(|(channel, weight)| {
                let value = f64::from(channel) / 255.0;
                weight
                    * if value <= 0.04045 {
                        value / 12.92
                    } else {
                        ((value + 0.055) / 1.055).powf(2.4)
                    }
            })
            .sum::<f64>()
    };
    let (a, b) = (luminance(first), luminance(second));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

// At 11px the font's strokes need not cover any pixel completely. Recognize
// the expected ink composited over its actual background, rather than requiring
// a fully opaque glyph pixel. The old bright highlight follows a different RGB
// segment and cannot satisfy this test on the light custom background.
fn has_ink(pixel: slint::Rgb8Pixel, foreground: Color, background: Color) -> bool {
    let pixel = [pixel.r, pixel.g, pixel.b].map(f64::from);
    let foreground = [foreground.red(), foreground.green(), foreground.blue()].map(f64::from);
    let background = [background.red(), background.green(), background.blue()].map(f64::from);
    let direction = std::array::from_fn::<_, 3, _>(|index| foreground[index] - background[index]);
    let squared: f64 = direction.iter().map(|value| value * value).sum();
    if squared < 1.0 {
        return false;
    }
    let coverage: f64 = (0..3)
        .map(|index| (pixel[index] - background[index]) * direction[index])
        .sum::<f64>()
        / squared;
    (0.65..=1.01).contains(&coverage)
        && (0..3).all(|index| {
            (pixel[index] - background[index] - coverage * direction[index]).abs() <= 3.0
        })
}

#[test]
fn default_palette_renders_every_category_with_contrast_in_every_theme_and_selection_state() {
    let mut test = ui::TestUi::new();
    let rules = midnight_core::default_filename_colors();
    let names = [
        "Source main.rs",
        "Video movie.mp4",
        "PDF report.pdf",
        "Document letter.docx",
        "Image photo.jpg",
        "Executable app.exe",
        "Spreadsheet table.csv",
        "Audio music.flac",
        "Archive backup.zip",
        "Presentation slides.pptx",
        "Plain text README.md",
        "Font type.ttf",
    ];
    assert_eq!(rules.len(), names.len());
    let window = test.window.clone_strong();
    let presentation = window.global::<FilenameColorPresentation>();
    for theme in midnight_core::PreferenceTheme::ALL.map(|theme| theme.index()) {
        window.invoke_apply_preferences(theme, 0);
        for active in [true, false] {
            window.set_active_pane(if active { 0 } else { 1 });
            for state in 0..4 {
                let cursor = state == 1 || state == 3;
                let selected = state >= 2;
                let rows: Vec<_> = rules
                    .iter()
                    .zip(names)
                    .enumerate()
                    .map(|(index, (rule, name))| {
                        let rgb = rule.style.foreground.unwrap();
                        FileRow {
                            identity: index.to_string().into(),
                            name: name.into(),
                            color_rule: true,
                            custom_foreground: true,
                            foreground: Color::from_rgb_u8(rgb.0, rgb.1, rgb.2),
                            cursor,
                            selected,
                            ..Default::default()
                        }
                    })
                    .collect();
                window.set_left_rows(Rc::new(VecModel::from(rows)).into());
                test.draw();
                for (index, (rule, name)) in rules.iter().zip(names).enumerate() {
                    let rgb = rule.style.foreground.unwrap();
                    let base = Color::from_rgb_u8(rgb.0, rgb.1, rgb.2);
                    for alternate in [false, true] {
                        let background = presentation.invoke_background(
                            false,
                            Color::default(),
                            cursor,
                            active,
                            selected,
                            alternate,
                        );
                        let ink =
                            presentation.invoke_foreground(true, base, background, selected, false);
                        assert!(
                            contrast(ink, background) >= 4.5,
                            "{name}: theme {theme}, state {state}, active {active}, alternate {alternate}"
                        );
                        if matches!(index, 3 | 10)
                            && state == 0
                            && !window.global::<Theme>().get_light()
                        {
                            assert!(
                                contrast(ink, background) >= 7.0,
                                "ordinary document names must stay bright in dark theme {theme}"
                            );
                        }
                        // Check every category against both stripes, then inspect
                        // the ink on the stripe used by its rendered fixture row.
                        if alternate != (index % 2 != 0) {
                            continue;
                        }
                        let row = ui::root_item(&window, name, AccessibleRole::ListItem);
                        let rect = row.geometry();
                        let origin = row.map_to_native_window(rect.origin);
                        // Inspect the filename region, excluding artwork and focus edges.
                        let found = ((origin.y + 2.0) as usize
                            ..(origin.y + rect.size.height - 2.0) as usize)
                            .any(|y| {
                                ((origin.x + 64.0) as usize..(origin.x + 230.0) as usize).any(|x| {
                                    has_ink(test.pixels[y * test.width + x], ink, background)
                                })
                            });
                        assert!(
                            found,
                            "actual filename ink: {name}, theme {theme}, state {state}, active {active}"
                        );
                    }
                }
                if (active && state == 0 && matches!(theme, 0 | 2 | 17 | 21..=25))
                    || (matches!(theme, 0 | 23..=25) && !active && state == 3)
                {
                    render::snapshot(
                        &test,
                        &format!("default-palette-theme-{theme}-state-{state}-active-{active}"),
                    );
                }
            }
        }
    }
}

#[test]
fn all_themes_states_and_views_render_readable_filename_colors_with_preserved_cues() {
    let mut test = ui::TestUi::new();
    let foreground = Color::from_rgb_u8(170, 90, 230);
    let background = Color::from_rgb_u8(230, 220, 235);
    let rows: Vec<_> = (0..4)
        .map(|state| FileRow {
            identity: state.to_string().into(),
            name: format!("Color file{state}.txt").into(),
            color_rule: true,
            custom_foreground: true,
            foreground,
            custom_background: true,
            background,
            cursor: state == 1 || state == 3,
            selected: state >= 2,
            prefix_end: if state == 0 { 4 } else { 0 },
            size: "12.4 GB".into(),
            size_state: 1,
            ..Default::default()
        })
        .collect();
    test.window
        .set_left_rows(Rc::new(VecModel::from(rows)).into());
    let window = test.window.clone_strong();
    for theme in midnight_core::PreferenceTheme::ALL.map(|theme| theme.index()) {
        test.window.invoke_apply_preferences(theme, 0);
        let presentation = window.global::<FilenameColorPresentation>();
        for base in [
            foreground,
            Color::from_rgb_u8(128, 128, 128),
            Color::from_rgb_u8(0, 0, 0),
            Color::from_rgb_u8(255, 255, 255),
        ] {
            for state in 0..4 {
                for active in [true, false] {
                    let bg = presentation.invoke_background(
                        true,
                        background,
                        state == 1 || state == 3,
                        active,
                        state >= 2,
                        false,
                    );
                    let fg = presentation.invoke_foreground(true, base, bg, state >= 2, false);
                    assert!(
                        contrast(fg, bg) >= 4.5,
                        "theme {theme}, state {state}: {fg:?} on {bg:?}"
                    );
                    if state > 0 {
                        assert_ne!(bg, background, "theme state must retain its background cue");
                    }
                }
            }
        }
        for mode in 0..3 {
            test.window.set_left_view_mode(mode);
            test.draw();
            let row = ui::root_item(&test.window, "Color file0.txt", AccessibleRole::ListItem);
            let rect = row.geometry();
            let origin = row.map_to_native_window(rect.origin);
            let count = |color: Color| {
                (origin.y.max(0.0) as usize
                    ..(origin.y + rect.size.height).min(test.height as f32) as usize)
                    .flat_map(|y| {
                        (origin.x.max(0.0) as usize
                            ..(origin.x + rect.size.width).min(test.width as f32) as usize)
                            .map(move |x| (x, y))
                    })
                    .filter(|&(x, y)| {
                        let pixel = test.pixels[y * test.width + x];
                        (pixel.r, pixel.g, pixel.b) == (color.red(), color.green(), color.blue())
                    })
                    .count()
            };
            assert!(
                count(background) > 20,
                "theme {theme}, mode {mode}: custom filename background must paint"
            );
            let search_color = presentation
                .invoke_readable(test.window.global::<Theme>().get_search_match(), background);
            render::snapshot(&test, &format!("filename-colors-theme-{theme}-mode-{mode}"));
            let search_pixels = (origin.y as usize..(origin.y + rect.size.height) as usize)
                .flat_map(|y| {
                    (origin.x as usize..(origin.x + rect.size.width) as usize).map(move |x| (x, y))
                })
                .filter(|&(x, y)| {
                    has_ink(test.pixels[y * test.width + x], search_color, background)
                })
                .count();
            assert!(
                search_pixels > 0,
                "theme {theme}, mode {mode}: prefix highlight must remain readable on the custom background; expected={search_color:?}, unadjusted={}, text={}",
                count(test.window.global::<Theme>().get_search_match()),
                count(foreground)
            );
            if mode == 0 {
                assert!(
                    count(test.window.global::<Theme>().get_pending_size()) > 0,
                    "pending size retains its semantic color"
                );
                let ordinary =
                    ui::root_item(&test.window, "Color file1.txt", AccessibleRole::ListItem);
                let geometry = ordinary.geometry();
                let point = ordinary.map_to_native_window(geometry.origin);
                let bg = presentation.invoke_background(true, background, true, true, false, false);
                let fg = presentation.invoke_foreground(true, foreground, bg, false, false);
                let first_glyph = (point.x as usize..(point.x + geometry.size.width) as usize)
                    .find(|&x| {
                        (point.y as usize..(point.y + geometry.size.height) as usize).any(|y| {
                            let pixel = test.pixels[y * test.width + x];
                            has_ink(pixel, fg, bg)
                        })
                    })
                    .expect("colored filename must render");
                assert!(
                    first_glyph < point.x as usize + 64,
                    "detailed filenames must remain aligned to the left"
                );
            }
            let combined = ui::root_item(&test.window, "Color file3.txt", AccessibleRole::ListItem);
            let geometry = combined.geometry();
            let point = combined.map_to_native_window(geometry.origin);
            let edge = test.window.global::<Theme>().get_cursor_edge();
            let pixel = test.pixel(point.x + geometry.size.width / 2.0, point.y + 0.5);
            assert_eq!(
                (pixel.r, pixel.g, pixel.b),
                (edge.red(), edge.green(), edge.blue()),
                "combined focus/selection cue must paint above filename backgrounds"
            );
            render::snapshot(&test, &format!("filename-colors-theme-{theme}-mode-{mode}"));
        }
    }
}

#[test]
fn colors_topic_edits_reorders_and_waits_for_backend_validation_before_saving() {
    let mut test = ui::TestUi::new();
    let presenter = Rc::new(RefCell::new(Presenter::new("left".into(), "right".into())));
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
            // Exercise creation and reordering from an explicitly cleared preset list.
            filename_colors: Vec::new().into(),
            ..UserPreferences::default()
        }))));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let captured = commands.clone();
    let send: Rc<dyn Fn(Command) -> bool> = Rc::new(move |command| {
        captured.borrow_mut().push(command);
        true
    });
    midnight_ui::install(&test.window, presenter.clone(), send.clone());
    commands.borrow_mut().clear();
    test.window.invoke_show_options();
    test.draw();
    popup_scroll::click(&mut test, "Colors", AccessibleRole::Button);
    test.draw();
    assert_eq!(test.window.get_options_topic_index(), 8);
    popup_scroll::click(&mut test, "Add rule", AccessibleRole::Button);
    test.draw();
    let window = test.window.clone_strong();
    let view = window.global::<ColorOptionsView>();
    let mut draft = view.get_draft();
    draft.expression = "*.rs".into();
    draft.foreground = "#b749de".into();
    view.invoke_edit(draft);
    popup_scroll::click(&mut test, "Add rule", AccessibleRole::Button);
    test.draw();
    let mut draft = view.get_draft();
    draft.expression = "文档*".into();
    draft.background = "#ece5cd".into();
    view.invoke_edit(draft);
    popup_scroll::click(&mut test, "Move up", AccessibleRole::Button);
    test.draw();
    assert!(view.get_labels().row_data(0).unwrap().contains("文档"));
    assert_eq!(view.get_selected(), 0);
    popup_scroll::click(&mut test, "Apply colors", AccessibleRole::Button);
    test.draw();
    assert!(view.get_busy());
    assert!(
        !commands
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::SavePreferences { .. }))
    );
    let request = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::ValidateFilenameColors(request) => Some(request.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(request.rules[0].expression, "文档*");
    presenter.borrow_mut().apply_with_sender(
        Event::FilenameColorsValidated {
            request,
            result: Err("Invalid expression".into()),
        },
        send.as_ref(),
    );
    midnight_ui::refresh(&test.window, &presenter.borrow());
    test.draw();
    assert!(view.get_error().contains("Invalid expression"));
    assert!(!view.get_busy(), "a failed validation must allow retry");
    assert!(
        !commands
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::SavePreferences { .. }))
    );
    commands.borrow_mut().clear();
    popup_scroll::click(&mut test, "Apply colors", AccessibleRole::Button);
    let request = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::ValidateFilenameColors(request) => Some(request.clone()),
            _ => None,
        })
        .unwrap();
    presenter.borrow_mut().apply_with_sender(
        Event::FilenameColorsValidated {
            request,
            result: Ok(()),
        },
        send.as_ref(),
    );
    midnight_ui::refresh(&test.window, &presenter.borrow());
    test.draw();
    assert!(!view.get_busy());
    assert!(
        !commands
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::SavePreferences { .. }))
    );
    assert_eq!(view.get_labels().row_count(), 2);
    assert!(view.get_labels().row_data(0).unwrap().contains("文档*"));
    render::snapshot(&test, "filename-colors-options");
    popup_scroll::click(&mut test, "Remove rule", AccessibleRole::Button);
    test.draw();
    assert_eq!(view.get_labels().row_count(), 1);
    popup_scroll::click(&mut test, "Discard changes", AccessibleRole::Button);
    test.draw();
    assert_eq!(view.get_labels().row_count(), 2);
    popup_scroll::click(&mut test, "OK", AccessibleRole::Button);
    let saved = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::SavePreferences { preferences, .. } => Some(preferences.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(saved.filename_colors.len(), 2);
    assert_eq!(saved.filename_colors[0].expression, "文档*");
}

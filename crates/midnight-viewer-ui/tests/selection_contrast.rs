//! Selected text reads well in every theme, as users see it. Each theme's document
//! palette is taken from the viewer's own Theme as the application captures it, every
//! syntax role is colored by the backend's role colors, and the selected glyphs are
//! painted by the backend's renderer. The painted fill must be the theme's selection,
//! every selected glyph must be drawn in the theme's selection text color, and that
//! pair must reach the theme's text minimum: WCAG AA 4.5:1, 7:1 for the contrast
//! themes and 12:1 for high contrast. The same holds for the dimmer pair that an
//! inactive viewer window paints. Unselected syntax colors keep 5:1 against the page,
//! a margin over AA, and 7:1 in the Contrast and High Contrast themes.
//!
//! The earlier form of this test measured whatever ink the renderer drew against the
//! fill. The renderer lifts any glyph to exactly 4.5:1, so that passed glaring fills,
//! washed-out "just readable" text and high-contrast themes below their own minimum.
use midnight_core::{PreferenceTheme, ViewerAppearance};
use midnight_plugin_host::api::viewer;
use midnight_viewer_render::{ColorSpan, Raster, Rect, TextOptions, TextRenderer, Viewport};
use midnight_viewer_ui::ViewerWindow;
use slint::platform::{
    Platform, PlatformError, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use std::{collections::HashMap, rc::Rc};

struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

fn rgba(color: slint::Color) -> [u8; 4] {
    let color = color.to_argb_u8();
    [color.red, color.green, color.blue, color.alpha]
}

/// WCAG 2 relative luminance of an opaque sRGB color.
fn luminance(color: [u8; 4]) -> f64 {
    color[..3]
        .iter()
        .zip([0.2126, 0.7152, 0.0722])
        .map(|(&channel, weight)| {
            let value = f64::from(channel) / 255.0;
            weight
                * if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
        })
        .sum()
}

fn contrast(a: [u8; 4], b: [u8; 4]) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// The text contrast that each theme keeps (as `midnight-ui` tests/themes.rs).
fn minimum(theme: PreferenceTheme) -> f64 {
    match theme as i32 {
        17..=20 => 7.0,
        21..=22 => 12.0,
        _ => 4.5,
    }
}

/// The document palette of `theme`, from the same Theme properties that the
/// application copies to content windows (midnight-explorer content_appearance.rs),
/// with the inactive pair instead while the window is inactive (src/appearance.rs).
fn appearance(window: &ViewerWindow, theme: PreferenceTheme, active: bool) -> ViewerAppearance {
    window.invoke_apply_tool_theme(theme as i32, 0, "".into(), 12, 15);
    let (selection, selection_text) = if active {
        (
            window.get_selection_background(),
            window.get_selection_text(),
        )
    } else {
        (
            window.get_inactive_selection_background(),
            window.get_inactive_selection_text(),
        )
    };
    ViewerAppearance {
        family: "monospace".into(),
        font_size: 14.0,
        foreground: rgba(window.get_foreground()),
        background: rgba(window.get_background_color()),
        selection: rgba(selection),
        selection_text: rgba(selection_text),
        accent: rgba(window.get_accent()),
    }
}

fn pixel(raster: &Raster, x: usize, y: usize) -> [u8; 4] {
    let at = (y * raster.width as usize + x) * 4;
    raster.rgba[at..at + 4].try_into().unwrap()
}

/// Whole pixels inside `rect`.
fn pixels(raster: &Raster, rect: Rect) -> Vec<[u8; 4]> {
    let columns = rect.x0.ceil() as usize..(rect.x1.floor() as usize).min(raster.width as usize);
    (rect.y0.ceil() as usize..(rect.y1.floor() as usize).min(raster.height as usize))
        .flat_map(|y| columns.clone().map(move |x| pixel(raster, x, y)))
        .collect()
}

#[test]
fn selected_text_of_every_syntax_role_is_the_theme_selection_text_in_every_theme() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native))).unwrap();
    let window = ViewerWindow::new().unwrap();
    let mut renderer = TextRenderer::default();
    // Large monospace stems cover whole pixels, which then show the selected ink itself
    // rather than its antialiased blend with the selection.
    let options = TextOptions {
        size: 36.0,
        ..TextOptions::default()
    };
    // One run of wide glyphs per role, plain to invalid, separated by spaces.
    let mut text = String::new();
    let mut runs = Vec::new();
    for role in viewer::STYLE_PLAIN..=viewer::STYLE_INVALID {
        if !text.is_empty() {
            text.push(' ');
        }
        let start = text.len();
        text.push_str("MMM");
        runs.push((role, start..text.len()));
    }
    let mut failures = Vec::new();
    let mut checked = 0;
    for theme in PreferenceTheme::ALL {
        for active in [true, false] {
            let state = if active { "active" } else { "inactive" };
            let appearance = appearance(&window, theme, active);
            let colors: Vec<_> = runs
                .iter()
                .map(|(role, range)| ColorSpan {
                    range: range.clone(),
                    color: midnight_viewer_engine::role_color(*role, &appearance),
                })
                .collect();
            let page = renderer
                .layout_with_options(&text, &options, &colors)
                .unwrap();
            let raster = renderer
                .rasterize(
                    &page,
                    Viewport {
                        width: page.width().ceil() as u32 + 8,
                        height: page.height().ceil() as u32 + 8,
                        origin: [0.0; 2],
                        background: appearance.background,
                        selection_color: appearance.selection,
                        selection_text: appearance.selection_text,
                    },
                    Some(0..text.len()),
                )
                .unwrap();
            // The selection as painted behind the selected glyphs: the most frequent
            // pixel of the selected line.
            let [line] = page.selection_geometry(0..text.len()).unwrap()[..] else {
                panic!("{theme:?}: the sample is one line");
            };
            let mut counts = HashMap::new();
            for color in pixels(&raster, line) {
                *counts.entry(color).or_insert(0) += 1;
            }
            let fill = counts
                .into_iter()
                .max_by_key(|&(_, count)| count)
                .map(|(color, _)| color)
                .unwrap();
            if fill != appearance.selection {
                failures.push(format!(
                    "{theme:?} {state}: the selection is painted as {fill:?}, the theme's is {:?}",
                    appearance.selection
                ));
            }
            let ratio = contrast(appearance.selection_text, fill);
            if ratio < minimum(theme) {
                failures.push(format!(
                    "{theme:?} {state}: selection text {:?} on {fill:?} is {ratio:.2}:1",
                    appearance.selection_text
                ));
            }
            for (role, range) in &runs {
                let start = page.caret_geometry(range.start).unwrap();
                let end = page.caret_geometry(range.end).unwrap();
                // The glyph cells, inset by a pixel so that a neighbor's edge stays outside.
                let cells = Rect {
                    x0: start.x0.min(end.x0) + 1.0,
                    x1: start.x0.max(end.x0) - 1.0,
                    y0: start.y0,
                    y1: start.y1,
                };
                let ink = pixels(&raster, cells)
                    .into_iter()
                    .max_by(|a, b| contrast(*a, fill).total_cmp(&contrast(*b, fill)))
                    .unwrap();
                if ink != appearance.selection_text {
                    failures.push(format!(
                        "{theme:?} {state} role {role}: selected text is painted {ink:?}, the theme's selection text is {:?}",
                        appearance.selection_text
                    ));
                }
                checked += 1;
            }
        }
    }
    assert_eq!(checked, PreferenceTheme::ALL.len() * 2 * 12);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn unselected_syntax_colors_of_every_theme_keep_their_contrast_on_the_page() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native))).unwrap();
    let window = ViewerWindow::new().unwrap();
    let mut failures = Vec::new();
    for theme in PreferenceTheme::ALL {
        let appearance = appearance(&window, theme, true);
        // A margin over WCAG AA's 4.5:1, and AAA in the Contrast and High Contrast themes.
        let required = if matches!(theme as i32, 17..=22) {
            7.0
        } else {
            5.0
        };
        for role in viewer::STYLE_PLAIN..=viewer::STYLE_INVALID {
            let color = midnight_viewer_engine::role_color(role, &appearance);
            let ratio = contrast(color, appearance.background);
            if ratio < required {
                failures.push(format!(
                    "{theme:?} role {role}: {color:?} on {:?} is {ratio:.2}:1, below {required}:1",
                    appearance.background
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

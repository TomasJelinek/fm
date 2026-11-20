//! Action glyphs share one style in every palette: a monochrome glyph tinted with the
//! theme's dimmed foreground (muted when disabled), whatever artwork the host supplies.
#[path = "support/toolbar.rs"]
mod toolbar;
use crate::support::ui;

use i_slint_core::{
    item_tree::ItemRc,
    items::{AccessibleRole, ImageItem},
};
use midnight_core::PreferenceTheme;
use midnight_ui::{ActionIcons, Theme};
use slint::{Color, ComponentHandle};

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

fn icon_pixels(test: &ui::TestUi, label: &str) -> Vec<slint::Rgb8Pixel> {
    let control = ui::root_item(&test.window, label, AccessibleRole::Button);
    let icon = visible_image(control).expect("command icon");
    let rect = icon.geometry();
    let origin = icon.map_to_native_window(rect.origin);
    let mut pixels = Vec::new();
    for y in origin.y.ceil() as usize..(origin.y + rect.height()).floor() as usize {
        for x in origin.x.ceil() as usize..(origin.x + rect.width()).floor() as usize {
            pixels.push(test.pixels[y * test.width + x]);
        }
    }
    pixels
}

fn channels(pixel: slint::Rgb8Pixel) -> [f32; 3] {
    [pixel.r, pixel.g, pixel.b].map(f32::from)
}

/// Every pixel is a blend of the background and the tint (anti-aliased edges included),
/// and the glyph's strokes reach the tint itself: no accent color survives.
fn assert_tinted(pixels: &[slint::Rgb8Pixel], tint: Color, case: &str) {
    let mut counts = std::collections::HashMap::new();
    for pixel in pixels {
        *counts.entry((pixel.r, pixel.g, pixel.b)).or_insert(0) += 1;
    }
    let (&(r, g, b), _) = counts.iter().max_by_key(|(_, count)| **count).unwrap();
    let background = [r, g, b].map(f32::from);
    let tint = [tint.red(), tint.green(), tint.blue()].map(f32::from);
    let span: [f32; 3] = std::array::from_fn(|i| tint[i] - background[i]);
    let length = span.iter().map(|value| value * value).sum::<f32>();
    let mut at_tint = 0;
    for pixel in pixels {
        let pixel = channels(*pixel);
        let t = if length > 0.0 {
            ((0..3)
                .map(|i| (pixel[i] - background[i]) * span[i])
                .sum::<f32>()
                / length)
                .clamp(0.0, 1.0)
        } else {
            0.0
        };
        let off = (0..3)
            .map(|i| (pixel[i] - (background[i] + t * span[i])).abs())
            .fold(0.0, f32::max);
        assert!(
            off <= 40.0,
            "{case}: {pixel:?} is neither the background {background:?} nor the tint {tint:?}"
        );
        if (0..3).all(|i| (pixel[i] - tint[i]).abs() <= 32.0) {
            at_tint += 1;
        }
    }
    assert!(at_tint >= 2, "{case}: no stroke reaches the tint {tint:?}");
}

#[test]
fn action_glyphs_share_the_dimmed_theme_tint_and_ignore_native_artwork() {
    let mut test = ui::TestUi::new();
    test.window.set_left_native_filesystem(true);
    for theme in PreferenceTheme::ALL {
        test.window.set_theme_index(theme.index());
        toolbar::reveal(&mut test, "Checksums", AccessibleRole::Button);
        test.draw();
        let palette = test.window.global::<Theme>();
        let (enabled, disabled) = (palette.get_secondary_text(), palette.get_muted_text());
        let pixels = icon_pixels(&test, "Checksums");
        assert_tinted(&pixels, enabled, &format!("Checksums, {theme:?}"));
        // Checksums also verifies a manifest without a selection; its availability
        // follows the active filesystem, not whether a file is selected.
        test.window.set_left_native_filesystem(false);
        test.draw();
        let muted = icon_pixels(&test, "Checksums");
        if disabled != enabled {
            assert_ne!(muted, pixels, "disabled artwork changes: {theme:?}");
        }
        assert_tinted(&muted, disabled, &format!("disabled Checksums, {theme:?}"));
        test.window.set_left_native_filesystem(true);
    }
    // Host artwork, colored here, never replaces the shared glyph of an action.
    test.window.set_action_icons(ActionIcons {
        folder_sizes: slint::Image::from_rgb8(slint::SharedPixelBuffer::clone_from_slice(
            &[251, 7, 13],
            1,
            1,
        )),
        ..Default::default()
    });
    toolbar::reveal(&mut test, "Folder sizes", AccessibleRole::Button);
    test.draw();
    let pixels = icon_pixels(&test, "Folder sizes");
    assert!(
        !pixels.iter().any(|p| p.r > 200 && p.g < 60 && p.b < 60),
        "native artwork stays out of the action glyph"
    );
    let tint = test.window.global::<Theme>().get_secondary_text();
    assert_tinted(&pixels, tint, "Folder sizes with native artwork");
}

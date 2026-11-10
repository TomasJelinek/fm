//! Render command artwork through the real toolbar across every palette.
#[path = "support/toolbar.rs"]
mod toolbar;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    item_tree::ItemRc,
    items::{AccessibleRole, ImageItem},
};
use midnight_core::PreferenceTheme;
use midnight_ui::ActionIcons;

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

#[test]
fn command_accents_survive_theme_masks_and_native_rgb_remains_intact() {
    let mut test = ui::TestUi::new();
    test.window.set_left_native_filesystem(true);
    for theme in PreferenceTheme::ALL {
        test.window.set_theme_index(theme.index());
        toolbar::reveal(&mut test, "Checksums", AccessibleRole::Button);
        test.draw();
        let pixels = icon_pixels(&test, "Checksums");
        assert!(
            pixels
                .iter()
                .filter(|p| p.g > p.r.saturating_add(15) && p.b > p.r.saturating_add(5))
                .count()
                >= 2,
            "the checksum accent must survive the fallback mask: {theme:?}"
        );
        // Checksums also verifies a manifest without a selection; its availability
        // follows the active filesystem, not whether a file is selected.
        test.window.set_left_native_filesystem(false);
        test.draw();
        assert_ne!(
            icon_pixels(&test, "Checksums"),
            pixels,
            "disabled artwork changes: {theme:?}"
        );
        test.window.set_left_native_filesystem(true);
    }
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
    assert!(
        icon_pixels(&test, "Folder sizes")
            .iter()
            .filter(|p| (p.r, p.g, p.b) == (251, 7, 13))
            .count()
            > 20,
        "native artwork keeps its supplied RGB values"
    );
}

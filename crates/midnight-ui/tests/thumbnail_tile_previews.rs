//! A Thumbnails tile fits an image or video preview to its preview box: the tile's
//! width less the name's 10 px insets and the thumbnail size plus 6 px of height,
//! aspect ratio kept, standing on the icons' baseline 6 px above the name. Icons keep
//! their square of the thumbnail size. The window reports the longest side a preview
//! is drawn at, which the thumbnail requests follow, at every size, density and font.
#[path = "support/render.rs"]
mod render;
use crate::support::ui;
use i_slint_core::items::AccessibleRole;
use midnight_core::{FONT_SIZE_MAX, FONT_SIZE_MIN, THUMBNAIL_SIZES};
use midnight_ui::FileRow;
use slint::{Image, Rgba8Pixel, SharedPixelBuffer, VecModel};
use std::rc::Rc;

/// A tile's name, its bitmap's size and colour, and whether it is a preview.
const TILES: [(&str, u32, u32, [u8; 3], bool); 4] = [
    ("wide.mp4", 320, 180, [230, 20, 20], true),
    ("tall.jpg", 120, 180, [20, 190, 40], true),
    ("panorama.png", 400, 100, [220, 20, 220], true),
    ("type-icon.txt", 64, 64, [20, 40, 230], false),
];

fn solid(width: u32, height: u32, [r, g, b]: [u8; 3]) -> Image {
    let pixels = [r, g, b, 255].repeat((width * height) as usize);
    Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
        &pixels, width, height,
    ))
}

fn show_tiles(ui: &mut ui::TestUi) {
    let rows: Vec<FileRow> = TILES
        .iter()
        .map(|(name, width, height, colour, preview)| FileRow {
            identity: (*name).into(),
            name: (*name).into(),
            icon: solid(*width, *height, *colour),
            preview: *preview,
            ..Default::default()
        })
        .collect();
    ui.window
        .set_left_rows(Rc::new(VecModel::from(rows)).into());
    ui.window.set_left_view_mode(2);
}

/// The drawn tile and the bounding box of `colour` inside it, in window pixels.
fn drawn(ui: &ui::TestUi, name: &str, colour: [u8; 3]) -> ([f32; 4], Option<[usize; 4]>) {
    let item = ui::root_item(&ui.window, name, AccessibleRole::ListItem);
    let rectangle = item.geometry();
    let origin = item.map_to_native_window(rectangle.origin);
    let tile = [
        origin.x,
        origin.y,
        rectangle.size.width,
        rectangle.size.height,
    ];
    let near = |pixel: slint::Rgb8Pixel| {
        pixel.r.abs_diff(colour[0]) <= 6
            && pixel.g.abs_diff(colour[1]) <= 6
            && pixel.b.abs_diff(colour[2]) <= 6
    };
    let mut bounds: Option<[usize; 4]> = None;
    for y in
        origin.y.max(0.0) as usize..((origin.y + rectangle.size.height) as usize).min(ui.height)
    {
        for x in
            origin.x.max(0.0) as usize..((origin.x + rectangle.size.width) as usize).min(ui.width)
        {
            if near(ui.pixels[y * ui.width + x]) {
                let [left, top, right, bottom] = bounds.get_or_insert([x, y, x + 1, y + 1]);
                *left = (*left).min(x);
                *top = (*top).min(y);
                *right = (*right).max(x + 1);
                *bottom = (*bottom).max(y + 1);
            }
        }
    }
    (tile, bounds)
}

/// A tile can start at a fractional pixel (the grid width shared by its columns), so
/// the picture's first and last columns may be blended with the background.
fn close(actual: f32, expected: f32, what: &str) {
    assert!(
        (actual - expected).abs() <= 2.0,
        "{what}: drawn {actual}, expected {expected}"
    );
}

#[test]
fn previews_fill_their_tiles_and_icons_keep_their_square_at_every_size_and_density() {
    let mut ui = ui::TestUi::new();
    ui.resize(1900, 1400);
    show_tiles(&mut ui);
    for density in [0, 1] {
        ui.window.set_density_index(density);
        for font in [FONT_SIZE_MIN, 12, FONT_SIZE_MAX] {
            ui.window.set_desktop_font_size(i32::from(font));
            ui.window.set_touch_font_size(i32::from(font));
            for size in THUMBNAIL_SIZES {
                let size = f32::from(size);
                ui.window.set_thumbnail_size(size as i32);
                ui.draw();
                let case = format!("density {density}, font {font}, size {size}");
                let mut longest = 0.0_f32;
                for (name, width, height, colour, preview) in TILES {
                    let (tile, bounds) = drawn(&ui, name, colour);
                    let [left, top, right, bottom] =
                        bounds.unwrap_or_else(|| panic!("{name} is not drawn ({case})"));
                    let [x, y, tile_width, _] = tile;
                    let (drawn_width, drawn_height) =
                        ((right - left) as f32, (bottom - top) as f32);
                    let what = |part: &str| format!("{name} {part} ({case})");
                    if preview {
                        let (box_width, box_height) = (tile_width - 20.0, size + 6.0);
                        let scale = (box_width / width as f32).min(box_height / height as f32);
                        close(drawn_width, (width as f32 * scale).round(), &what("width"));
                        close(
                            drawn_height,
                            (height as f32 * scale).round(),
                            &what("height"),
                        );
                        // As large as the box allows: one side fills it.
                        assert!(
                            (drawn_width - box_width).abs() <= 2.0
                                || (drawn_height - box_height).abs() <= 2.0,
                            "{}",
                            what("fills its box")
                        );
                        close(bottom as f32, y + size + 12.0, &what("baseline"));
                        close(
                            left as f32 - x,
                            x + tile_width - right as f32,
                            &what("centring"),
                        );
                        assert!(left as f32 >= x + 9.0, "{}", what("left inset"));
                        if width <= 2 * height {
                            longest = longest.max(drawn_width).max(drawn_height);
                        }
                    } else {
                        // The type icon's square is unchanged.
                        let edge = size.min(tile_width - 24.0);
                        close(drawn_width, edge, &what("icon width"));
                        close(drawn_height, edge, &what("icon height"));
                        close(top as f32, y + 12.0, &what("icon top"));
                    }
                }
                // Requests cover the longest side a preview of at most 2:1 is drawn at.
                let [edge, right] = midnight_ui::tile_preview_edges(&ui.window);
                assert_eq!(right, 0, "the right pane shows no tiles ({case})");
                assert!(
                    edge as f32 >= longest - 1.0 && edge as f32 <= longest + 2.0 * size,
                    "request edge {edge} for a drawn {longest} ({case})"
                );
                assert!(
                    edge as f32 >= size,
                    "never below the thumbnail size ({case})"
                );
            }
        }
    }
}

#[test]
fn preview_edges_follow_the_tiles_and_vanish_without_them() {
    let mut ui = ui::TestUi::new();
    show_tiles(&mut ui);
    ui.window.set_thumbnail_size(96);
    ui.draw();
    let columns = ui.window.get_left_view_columns() as f32;
    let (tile, _) = drawn(&ui, "wide.mp4", TILES[0].3);
    let expected = (tile[2] - 20.0).clamp(102.0, 2.0 * 102.0).ceil() as u32;
    assert!(columns >= 1.0);
    let [edge, _] = midnight_ui::tile_preview_edges(&ui.window);
    assert!(
        edge.abs_diff(expected) <= 1,
        "{edge} for a {} px tile",
        tile[2]
    );
    // A narrower panel draws narrower tiles; a wider one can draw a 2:1 preview.
    ui.resize(900, 820);
    let [narrow, _] = midnight_ui::tile_preview_edges(&ui.window);
    assert!((102..=204).contains(&narrow), "{narrow}");
    ui.window.set_left_view_mode(0);
    ui.draw();
    assert_eq!(midnight_ui::tile_preview_edges(&ui.window), [0, 0]);
}

/// Software-rendered views of the tiles for review (`MIDNIGHT_UI_RENDER_DIR`).
#[test]
fn thumbnail_tile_snapshots() {
    let mut ui = ui::TestUi::new();
    show_tiles(&mut ui);
    for (size, density) in [(96, 0), (192, 0), (96, 1)] {
        ui.window.set_density_index(density);
        ui.window.set_thumbnail_size(size);
        ui.draw();
        render::snapshot(&ui, &format!("tiles-{size}-density{density}"));
    }
}

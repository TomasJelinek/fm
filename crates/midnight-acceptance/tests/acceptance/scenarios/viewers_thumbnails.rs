//! The thumbnail view (Alt+5) through the installed thumbnailers: thumbnails of the
//! visible files appear, show the images' own colours, never disappear and come back
//! while the view is still (no flicker), follow keyboard and wheel scrolling, and are
//! shown again at once after scrolling back. Images of every format and broken images
//! are visited too: the JPEG, GIF, WebP and SVG tiles show their images' own colours, a
//! PDF must show its first page (a white page with dark text), a password-protected PDF
//! keeps its placeholder, a two-frame GIF and WebP show their first frame, an AVIF from
//! the image decoder's licensed fixtures looks like its PNG twin, and on Windows a
//! program's tile shows the program's own icon. Without a thumbnailers package in the
//! stage the scenario is skipped; with one, no thumbnail at all fails it, with the
//! reason the thumbnailers report, and on a disk whose storage the platform reports as
//! Unknown it names THUMB-UNKNOWN-STORAGE.
use crate::{
    Context,
    app::{self, App, Config},
    encode, files_support as files, input, inspect, panel, viewers_stage,
    viewers_support::{self as support, Canvas, Failures, generated},
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{PaneId, StorageKind, StorageService};
use slint::{
    LogicalPosition,
    platform::{Key, WindowEvent},
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub const TIMEOUT: Duration = Duration::from_secs(10 * 60);
const LOAD_LIMIT: Duration = Duration::from_secs(45);

pub fn thumbnails(context: &mut Context) -> Result<(), String> {
    viewers_stage::run(context, TIMEOUT, run)
}

/// The colour of `thumbs/tile-NN.png` (see `fixtures::viewer_extras`).
fn tile_colour(index: u32) -> [u8; 3] {
    [
        (index * 37 % 256) as u8,
        (index * 91 % 256) as u8,
        ((index * 53 + 80) % 256) as u8,
    ]
}

fn tile_index(name: &str) -> Option<u32> {
    name.strip_prefix("tile-")?
        .strip_suffix(".png")?
        .parse()
        .ok()
}

/// The left panel's list, rendered.
fn left_list(app: &App) -> Canvas {
    let (width, _, pixels) = app::render(&app.main);
    let window = &app.window;
    let (left, top) = (
        window.get_left_list_left().max(0.0) as usize,
        window.get_left_list_top().max(0.0) as usize,
    );
    let (list_width, list_height) = (
        window.get_left_list_width().max(0.0) as usize,
        window.get_left_list_height().max(0.0) as usize,
    );
    Canvas::crop(&pixels, width, left, top, list_width, list_height)
}

/// Visible tiles whose own colour is on screen.
fn shown_tiles(canvas: &Canvas, tiles: &[u32]) -> Vec<u32> {
    tiles
        .iter()
        .copied()
        .filter(|index| canvas.count_near(tile_colour(*index), 10) >= 150)
        .collect()
}

/// Indices of the tiles whose grid cells are drawn entirely inside the list. Rows cut
/// at the list's edges are drawn too (and requested), but only a whole cell shows its
/// whole thumbnail, so the cells come from the window's item tree.
fn visible_tiles(app: &App) -> Vec<u32> {
    let window = &app.window;
    let (left, top) = (window.get_left_list_left(), window.get_left_list_top());
    let (right, bottom) = (
        left + window.get_left_list_width(),
        top + window.get_left_list_height(),
    );
    let mut tiles: Vec<u32> = inspect::controls(app.slint(), AccessibleRole::ListItem)
        .into_iter()
        .filter_map(|(label, item)| {
            let index = tile_index(&label)?;
            let (x, y, width, height) = inspect::rect(&item);
            (x >= left - 1.0
                && x + width <= right + 1.0
                && y >= top - 1.0
                && y + height <= bottom + 1.0)
                .then_some(index)
        })
        .collect();
    tiles.sort_unstable();
    tiles.dedup();
    tiles
}

fn presented(app: &App) -> Vec<PathBuf> {
    support::plugins(app)
        .map(|plugins| plugins.thumbnail_log.borrow().paths.clone())
        .unwrap_or_default()
}

fn notices(app: &App) -> Vec<String> {
    support::plugins(app)
        .map(|plugins| plugins.thumbnail_log.borrow().notices.clone())
        .unwrap_or_default()
}

/// A notice shown while thumbnails load fails the check named by `during`; it is then
/// closed, since it takes the keyboard from the panel, so the remaining checks still run.
fn close_notice(app: &mut App, failures: &Failures, during: &str) {
    if let Some((title, body)) = app.notice() {
        failures.check(
            &format!("no notice during {during}"),
            Err(format!("{title}: {body}")),
        );
        files::dismiss_all(app);
    }
}

/// Watch the visible tiles frame by frame until every one shows its colour (or the
/// limit passes); a tile that showed and then vanished while nothing scrolled is
/// flicker. Returns the time until all were shown.
fn watch_tiles(
    app: &mut App,
    limit: Duration,
    flicker: &mut Vec<String>,
) -> (Duration, Vec<u32>, Vec<u32>) {
    let started = Instant::now();
    let mut seen: BTreeMap<u32, bool> = BTreeMap::new();
    loop {
        app.settle();
        let tiles = visible_tiles(app);
        let canvas = left_list(app);
        let shown = shown_tiles(&canvas, &tiles);
        for index in &tiles {
            let now = shown.contains(index);
            if let Some(before) = seen.insert(*index, now)
                && before
                && !now
            {
                flicker.push(format!(
                    "tile-{index:02}.png disappeared after {} ms while the view was still",
                    started.elapsed().as_millis()
                ));
            }
        }
        if (!tiles.is_empty() && shown.len() == tiles.len()) || started.elapsed() >= limit {
            return (started.elapsed(), tiles, shown);
        }
        app.idle(Duration::from_millis(40));
    }
}

fn run(context: &mut Context) -> Result<(), String> {
    if viewers_stage::installed_packages("thumbnailers") == 0 {
        return Err(
            "SKIP: the staged installation has no thumbnailers package; build them first \
             (./build.sh) or name a folder holding thumbnailers/ in MIDNIGHT_ACCEPTANCE_PLUGINS"
                .into(),
        );
    }
    let root = context.fixture.root.clone();
    // BMP, ICO and TIFF, which the fixture does not hold, in the right panel.
    let extra = context.work.join("extra");
    generated::write(
        &extra,
        "picture.bmp",
        generated::bmp(96, 72, generated::picture),
    )?;
    generated::write(
        &extra,
        "picture.ico",
        generated::ico(48, generated::picture),
    )?;
    generated::write(
        &extra,
        "picture.tif",
        generated::tiff(96, 72, generated::picture),
    )?;
    generated::write(
        &extra,
        "picture.tiff",
        generated::tiff(80, 60, generated::picture),
    )?;
    let failures = Failures::default();
    let extras = extra_files(&extra, &failures)?;
    // Program tiles show their own icon, which the desktop service reads (Windows; Linux
    // tiles keep the icon of a file's type).
    if cfg!(windows) {
        support::read_file_icons();
    }
    let mut config = Config::new(&root, &extra);
    config.plugins = Some(context.work.join("plugins"));
    let mut app = App::start(config);
    panel::enter(&mut app, PaneId::Left, "thumbs")?;
    panel::focus_list(&mut app, PaneId::Left)?;
    let detailed = app.window.get_left_view_mode();
    input::chord(app.slint(), &[Key::Alt], "5");
    app.settle();
    let mode = app.window.get_left_view_mode();
    if mode == detailed {
        return Err(format!(
            "Alt+5 did not change the panel view mode (still {mode})"
        ));
    }
    context.note(format!(
        "Alt+5: panel view mode {detailed} → {mode}; {} columns × {} rows visible",
        app.window.get_left_view_columns(),
        app.window.get_left_view_rows()
    ));
    let mut flicker = Vec::new();
    let (elapsed, tiles, shown) = watch_tiles(&mut app, LOAD_LIMIT, &mut flicker);
    app.screenshot(
        context,
        "thumbnail-grid",
        &format!(
            "Alt+5 in thumbs/: {} of {} visible tiles show their thumbnails after {} ms",
            shown.len(),
            tiles.len(),
            elapsed.as_millis()
        ),
    );
    let notes = notices(&app);
    context.note(format!(
        "Thumbnails: {} of {} visible tiles shown after {} ms; {} frames presented; notices {:?}",
        shown.len(),
        tiles.len(),
        elapsed.as_millis(),
        presented(&app).len(),
        notes
    ));
    if shown.is_empty() {
        // The stage has thumbnailers (checked above): no thumbnail at all is a failure.
        let reason = if notes.is_empty() {
            "no thumbnail and no notice".to_owned()
        } else {
            notes.join("; ")
        };
        app.shutdown()?;
        // Thumbnails are off where the storage cannot be proven local (finding
        // THUMB-UNKNOWN-STORAGE: a virtual machine's disk stays Unknown).
        let storage = midnight_platform::LocalFileSystem::new(None)
            .storage_kind(&context.fixture.path("thumbs"));
        let finding = if matches!(storage, Ok(StorageKind::Unknown)) {
            "THUMB-UNKNOWN-STORAGE: "
        } else {
            ""
        };
        return Err(format!(
            "{finding}no thumbnail appeared within {LOAD_LIMIT:?} although the stage has {} \
             thumbnailers package(s): {reason}; the folder's storage is {storage:?}",
            viewers_stage::installed_packages("thumbnailers")
        ));
    }
    failures.check(
        "every visible tile",
        if shown.len() == tiles.len() {
            Ok(())
        } else {
            Err(format!(
                "only {} of {} visible tiles show their thumbnails: missing {:?}",
                shown.len(),
                tiles.len(),
                tiles
                    .iter()
                    .filter(|index| !shown.contains(index))
                    .collect::<Vec<_>>()
            ))
        },
    );
    // A still view keeps every frame identical.
    let still: Vec<u64> = (0..10)
        .map(|_| {
            app.idle(Duration::from_millis(60));
            left_list(&app).fingerprint()
        })
        .collect();
    failures.check(
        "still view",
        if still.windows(2).all(|pair| pair[0] == pair[1]) {
            Ok(())
        } else {
            Err("the thumbnail grid changed between frames while nothing moved".into())
        },
    );

    // Keyboard: End scrolls to the last tiles, which load; Home returns at once.
    input::key(app.slint(), Key::End);
    let scrolled = support_wait(&mut app, "End to scroll the grid", |app| {
        app.window.get_left_thumbnail_row_start() > 0
    });
    failures.check("End scrolls", scrolled);
    let (elapsed, tiles, shown) = watch_tiles(&mut app, LOAD_LIMIT, &mut flicker);
    app.screenshot(
        context,
        "thumbnail-grid-end",
        &format!(
            "End: the last tiles ({} of {} shown after {} ms)",
            shown.len(),
            tiles.len(),
            elapsed.as_millis()
        ),
    );
    failures.check(
        "tiles after End",
        if !tiles.is_empty() && shown.len() == tiles.len() {
            Ok(())
        } else {
            Err(format!(
                "after End {} of {} tiles show thumbnails",
                shown.len(),
                tiles.len()
            ))
        },
    );
    input::key(app.slint(), Key::Home);
    app.settle();
    let tiles = visible_tiles(&app);
    let shown = shown_tiles(&left_list(&app), &tiles);
    context.note(format!(
        "Home: {} of {} tiles shown in the first frame after scrolling back",
        shown.len(),
        tiles.len()
    ));
    failures.check(
        "cached thumbnails after Home",
        if shown.len() == tiles.len() {
            Ok(())
        } else {
            let (elapsed, tiles, shown) = watch_tiles(&mut app, LOAD_LIMIT, &mut flicker);
            Err(format!(
                "scrolling back showed placeholders: {} of {} tiles after {} ms",
                shown.len(),
                tiles.len(),
                elapsed.as_millis()
            ))
        },
    );

    // The wheel over the grid scrolls it as well.
    let position = LogicalPosition::new(
        app.window.get_left_list_left() + app.window.get_left_list_width() / 2.0,
        app.window.get_left_list_top() + app.window.get_left_list_height() / 2.0,
    );
    let start = app.window.get_left_thumbnail_row_start();
    for _ in 0..6 {
        app.slint()
            .dispatch_event(WindowEvent::PointerMoved { position });
        app.slint().dispatch_event(WindowEvent::PointerScrolled {
            position,
            delta_x: 0.0,
            delta_y: -120.0,
        });
        app.idle(Duration::from_millis(30));
    }
    failures.check(
        "wheel scrolls",
        support_wait(&mut app, "the wheel to scroll the grid", |app| {
            app.window.get_left_thumbnail_row_start() > start
        }),
    );
    let (_, tiles, shown) = watch_tiles(&mut app, LOAD_LIMIT, &mut flicker);
    failures.check(
        "tiles after the wheel",
        if shown.len() == tiles.len() {
            Ok(())
        } else {
            Err(format!(
                "after the wheel {} of {} tiles show thumbnails",
                shown.len(),
                tiles.len()
            ))
        },
    );
    failures.check(
        "no flicker",
        if flicker.is_empty() {
            Ok(())
        } else {
            Err(flicker.join("; "))
        },
    );

    close_notice(&mut app, &failures, "the thumbs/ grid");

    // Every image format, broken images, the documents folder.
    for folder in ["images", "docs"] {
        // Thumbnails can arrive while the folder is entered: count from before.
        let before = presented(&app).len();
        panel::leave(&mut app, PaneId::Left)?;
        panel::enter(&mut app, PaneId::Left, folder)?;
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut count = before;
        let mut quiet = Instant::now();
        while Instant::now() < deadline && quiet.elapsed() < Duration::from_secs(3) {
            app.idle(Duration::from_millis(100));
            let now = presented(&app).len();
            if now != count {
                count = now;
                quiet = Instant::now();
            }
        }
        let files: Vec<String> = presented(&app)[before..]
            .iter()
            .filter_map(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .collect();
        app.screenshot(
            context,
            &format!("thumbnail-{folder}"),
            &format!("Thumbnail view of {folder}/: thumbnails for {files:?}"),
        );
        context.note(format!(
            "Thumbnails in {folder}/: {files:?}; notices {:?}",
            notices(&app)
        ));
        if folder == "images" {
            let missing: Vec<&str> = [
                "photo.png",
                "banner.jpg",
                "icon.gif",
                "pattern.webp",
                "vector.svg",
            ]
            .into_iter()
            .filter(|name| !files.iter().any(|file| file == name))
            .collect();
            failures.check(
                "image formats",
                if missing.is_empty() {
                    Ok(())
                } else {
                    Err(format!("no thumbnails for {missing:?}"))
                },
            );
            for (name, check) in IMAGE_COLOURS {
                failures.check(
                    &format!("the {name} tile's pixels"),
                    artwork(&app, name)
                        .ok_or_else(|| format!("{name} is not visible in the thumbnail view"))
                        .and_then(|tile| check(&tile)),
                );
            }
        } else {
            let page = if files.iter().any(|file| file == PDF) {
                pdf_page(&app)
            } else {
                Err(format!("no thumbnail for {PDF}"))
            };
            if let Ok(detail) = &page {
                context.note(format!("Thumbnail of docs/{PDF}: {detail}"));
            }
            failures.check("PDF first page", page.map(|_| ()));
        }
        close_notice(&mut app, &failures, &format!("{folder}/"));
    }
    input::chord(app.slint(), &[Key::Alt], "3");
    app.settle();
    panel::leave(&mut app, PaneId::Left)?;
    extra_formats(&mut app, context, &failures, &extras)?;
    close_notice(&mut app, &failures, "the right panel's thumbnails");
    app.shutdown()?;
    failures.finish(context)
}

/// Pixel checks of the `images/` tiles: each finds its image's own colours (see
/// `fixtures::images`) in the tile's artwork, which a placeholder icon never has.
type ColourCheck = fn(&Canvas) -> Result<(), String>;
const IMAGE_COLOURS: [(&str, ColourCheck); 4] = [
    ("banner.jpg", greys),
    ("icon.gif", gif_palette),
    ("pattern.webp", webp_colours),
    ("vector.svg", svg_colours),
];

/// The greyscale JPEG's flat 8 x 8 blocks of 128 levels: at least six distinct greys
/// spread over 150 levels or more.
fn greys(tile: &Canvas) -> Result<(), String> {
    let mut levels = std::collections::BTreeMap::<u8, usize>::new();
    for pixel in &tile.pixels {
        let (high, low) = (
            pixel.r.max(pixel.g).max(pixel.b),
            pixel.r.min(pixel.g).min(pixel.b),
        );
        if high - low <= 4 {
            *levels.entry(pixel.g / 8).or_default() += 1;
        }
    }
    let shown: Vec<u8> = levels
        .into_iter()
        .filter(|(_, count)| *count >= 6)
        .map(|(level, _)| level)
        .collect();
    let spread = shown
        .last()
        .zip(shown.first())
        .map_or(0, |(high, low)| u32::from(high - low) * 8);
    if shown.len() >= 6 && spread >= 150 {
        Ok(())
    } else {
        Err(format!(
            "the tile shows {} grey levels spread over {spread}, not the JPEG's blocks",
            shown.len()
        ))
    }
}

/// The GIF's eight palette colours ([i, 255 - i, 7i] for i = 0, 32, ..., 224): five or
/// more of them.
fn gif_palette(tile: &Canvas) -> Result<(), String> {
    let shown = (0..8_u32)
        .map(|step| {
            let index = step * 32;
            [index as u8, (255 - index) as u8, (index * 7 % 256) as u8]
        })
        .filter(|colour| tile.count_near(*colour, 16) >= 20)
        .count();
    if shown >= 5 {
        Ok(())
    } else {
        Err(format!(
            "the tile shows {shown} of the GIF's 8 palette colours"
        ))
    }
}

/// The WebP checkerboard's two colours.
fn webp_colours(tile: &Canvas) -> Result<(), String> {
    colours_shown(tile, &[([40, 60, 90], 200), ([230, 200, 120], 200)])
}

/// The SVG's background rectangle (#203040) and circle (#3a7bd5).
fn svg_colours(tile: &Canvas) -> Result<(), String> {
    colours_shown(tile, &[([32, 48, 64], 1000), ([58, 123, 213], 300)])
}

/// Each colour covers at least its number of pixels (within 12 per channel).
fn colours_shown(tile: &Canvas, wanted: &[([u8; 3], usize)]) -> Result<(), String> {
    let missing: Vec<String> = wanted
        .iter()
        .map(|(colour, least)| (colour, least, tile.count_near(*colour, 12)))
        .filter(|(_, least, count)| *count < **least)
        .map(|(colour, least, count)| format!("{colour:?} on {count} pixels (wanted {least})"))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("the tile lacks {}", missing.join(", ")))
    }
}

/// The artwork band of `name`'s cell in the thumbnail view (its name below excluded),
/// rendered.
fn artwork(app: &App, name: &str) -> Option<Canvas> {
    let (x, y, width, _) = inspect::controls(app.slint(), AccessibleRole::ListItem)
        .into_iter()
        .find(|(label, _)| panel::is_row_label(label, name))
        .map(|(_, item)| inspect::rect(&item))?;
    let edge = THUMBNAIL_EDGE.min(width - 24.0).max(1.0);
    let (render_width, _, pixels) = app::render(&app.main);
    // The artwork starts 12 px below the cell's top; its name starts 6 px below it.
    Some(Canvas::crop(
        &pixels,
        render_width,
        x.max(0.0) as usize,
        (y + 8.0).max(0.0) as usize,
        width.max(0.0) as usize,
        (edge + 8.0) as usize,
    ))
}

/// What the right panel's folder holds besides BMP, ICO and TIFF ([`extra_files`]).
struct Extras {
    /// The AVIF and its PNG twin were found in the checkout.
    avif: bool,
    /// A program with an icon of its own and a file named like a program without one.
    programs: bool,
}

const TWO_GIF: &str = "two-frames.gif";
const TWO_WEBP: &str = "two-frames.webp";
const LOCKED_PDF: &str = "locked.pdf";
const AVIF: &str = "draw-points.avif";
const AVIF_TWIN: &str = "draw-points.png";
const PROGRAM: &str = "program.exe";
const GENERIC: &str = "generic.exe";
/// The two frames' colours.
const RED: [u8; 3] = [200, 40, 40];
const BLUE: [u8; 3] = [40, 40, 200];

/// The right panel's files beyond BMP, ICO and TIFF; what this machine cannot provide is
/// reported as skipped.
fn extra_files(folder: &Path, failures: &Failures) -> Result<Extras, String> {
    let mut palette = [[0_u8; 3]; 256];
    palette[1] = RED;
    palette[2] = BLUE;
    generated::write(
        folder,
        TWO_GIF,
        encode::gif_animated(48, 36, &palette, 50, &[&|_, _| 1, &|_, _| 2]),
    )?;
    generated::write(
        folder,
        TWO_WEBP,
        encode::webp_animated(
            48,
            36,
            &[
                encode::WebpFrame {
                    low: RED,
                    high: RED,
                    is_high: &|_, _| true,
                    milliseconds: 500,
                },
                encode::WebpFrame {
                    low: BLUE,
                    high: BLUE,
                    is_high: &|_, _| true,
                    milliseconds: 500,
                },
            ],
        ),
    )?;
    generated::write(
        folder,
        LOCKED_PDF,
        encode::pdf_encrypted(&["Locked thumbnail fixture"], "pdf-password"),
    )?;
    // libavif's own test image (BSD-2-Clause, see the folder's README) and its PNG twin.
    let fixtures =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/image-decoder/tests/fixtures");
    let avif = match (
        std::fs::read(fixtures.join("draw_points_idat.avif")),
        std::fs::read(fixtures.join("draw_points.png")),
    ) {
        (Ok(avif), Ok(png)) => {
            generated::write(folder, AVIF, avif)?;
            generated::write(folder, AVIF_TWIN, png)?;
            true
        }
        (avif, png) => {
            failures.skip(
                "the AVIF thumbnail",
                &format!(
                    "the image decoder's fixtures are not in {}: {:?} {:?}",
                    fixtures.display(),
                    avif.err(),
                    png.err()
                ),
            );
            false
        }
    };
    let programs = program_files(folder, failures)?;
    Ok(Extras { avif, programs })
}

/// A copy of a system program, whose icon resources its tile shows, and a file that
/// is named like a program but holds none.
#[cfg(windows)]
fn program_files(folder: &Path, failures: &Failures) -> Result<bool, String> {
    let system = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"));
    let source = system.join("explorer.exe");
    if let Err(error) = std::fs::copy(&source, folder.join(PROGRAM)) {
        failures.skip(
            "a program's own icon",
            &format!("{} cannot be copied here: {error}", source.display()),
        );
        return Ok(false);
    }
    generated::write(folder, GENERIC, b"MZ: named like a program, with no icon")?;
    Ok(true)
}

#[cfg(not(windows))]
fn program_files(_: &Path, failures: &Failures) -> Result<bool, String> {
    failures.skip(
        "a program's own icon",
        "Linux tiles show a file's type icon, never artwork from its content",
    );
    Ok(false)
}

/// The names in `wanted` that got no thumbnail since `before`, after waiting up to
/// `limit` for them.
fn wait_presented(app: &mut App, before: usize, wanted: &[&str], limit: Duration) -> Vec<String> {
    let deadline = Instant::now() + limit;
    loop {
        app.idle(Duration::from_millis(100));
        let files: Vec<String> = presented(app)[before..]
            .iter()
            .filter_map(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .collect();
        let missing: Vec<String> = wanted
            .iter()
            .filter(|name| !files.iter().any(|file| file == *name))
            .map(|name| (*name).to_owned())
            .collect();
        if missing.is_empty() || Instant::now() >= deadline {
            return missing;
        }
    }
}

/// Alt+5 in the right panel: BMP, ICO, both TIFF files, the two-frame GIF and WebP and
/// the AVIF and its PNG twin get thumbnails; the animations show their first frame, the
/// AVIF looks like its twin; the password-protected PDF keeps its placeholder; and a
/// program shows its own icon, unlike a file named like one without any.
fn extra_formats(
    app: &mut App,
    context: &mut Context,
    failures: &Failures,
    extras: &Extras,
) -> Result<(), String> {
    let before = presented(app).len();
    panel::focus_list(app, PaneId::Right)?;
    input::chord(app.slint(), &[Key::Alt], "5");
    let mut wanted = vec![
        "picture.bmp",
        "picture.ico",
        "picture.tif",
        "picture.tiff",
        TWO_GIF,
        TWO_WEBP,
    ];
    if extras.avif {
        wanted.extend([AVIF, AVIF_TWIN]);
    }
    let missing = wait_presented(app, before, &wanted, LOAD_LIMIT);
    app.screenshot(
        context,
        "thumbnail-extra-formats",
        "Thumbnail view of BMP, ICO, TIFF, a two-frame GIF and WebP, AVIF, a locked PDF and \
         programs",
    );
    let pictures: Vec<&String> = missing
        .iter()
        .filter(|name| name.starts_with("picture."))
        .collect();
    failures.check(
        "BMP, ICO and TIFF thumbnails",
        if pictures.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "no thumbnails for {pictures:?} within {LOAD_LIMIT:?}; notices {:?}",
                notices(app)
            ))
        },
    );
    for name in [TWO_GIF, TWO_WEBP] {
        failures.check(
            &format!("{name} shows its first frame"),
            if missing.iter().any(|missed| missed == name) {
                Err(format!("no thumbnail within {LOAD_LIMIT:?}"))
            } else {
                artwork(app, name)
                    .ok_or_else(|| format!("{name} is not visible"))
                    .and_then(|tile| {
                        let (red, blue) = (tile.count_near(RED, 16), tile.count_near(BLUE, 16));
                        if red >= 500 && blue < 50 {
                            Ok(())
                        } else {
                            Err(format!(
                                "the tile shows {red} pixels of the first frame's red and \
                                 {blue} of the second's blue"
                            ))
                        }
                    })
            },
        );
    }
    if extras.avif {
        failures.check(
            "the AVIF thumbnail matches its PNG twin",
            if missing
                .iter()
                .any(|missed| missed == AVIF || missed == AVIF_TWIN)
            {
                Err(format!("no thumbnails for {missing:?}"))
            } else {
                match (artwork(app, AVIF), artwork(app, AVIF_TWIN)) {
                    (Some(avif), Some(png)) => same_picture(&avif, &png),
                    _ => Err("the AVIF or its twin is not visible".into()),
                }
            },
        );
    }
    // The locked PDF: no thumbnail by the time the others have theirs, and no page.
    let locked = presented(app)[before..]
        .iter()
        .any(|path| path.file_name().is_some_and(|name| name == LOCKED_PDF));
    failures.check(
        "a password-protected PDF keeps its placeholder",
        if locked {
            Err("a thumbnail of the password-protected PDF was presented".into())
        } else {
            match page_shown(app, LOCKED_PDF) {
                Ok(detail) => Err(format!("its tile shows a page: {detail}")),
                Err(_) => Ok(()),
            }
        },
    );
    if extras.programs {
        failures.check("a program's own icon", program_icon(app));
    }
    input::chord(app.slint(), &[Key::Alt], "3");
    app.settle();
    Ok(())
}

/// Two tiles show the same picture: at least 95 % of their pixels within 24 per channel.
fn same_picture(one: &Canvas, other: &Canvas) -> Result<(), String> {
    if one.width != other.width || one.height != other.height {
        return Err(format!(
            "the tiles differ in size: {}x{} and {}x{}",
            one.width, one.height, other.width, other.height
        ));
    }
    let close = one
        .pixels
        .iter()
        .zip(&other.pixels)
        .filter(|(a, b)| {
            a.r.abs_diff(b.r) <= 24 && a.g.abs_diff(b.g) <= 24 && a.b.abs_diff(b.b) <= 24
        })
        .count();
    if close * 100 >= one.pixels.len() * 95 {
        Ok(())
    } else {
        Err(format!("only {close} of {} pixels agree", one.pixels.len()))
    }
}

/// The program's tile shows its own icon: within the load limit it differs from the
/// tile of the file named like a program without one.
fn program_icon(app: &mut App) -> Result<(), String> {
    let deadline = Instant::now() + LOAD_LIMIT;
    loop {
        app.settle();
        if let (Some(program), Some(generic)) = (artwork(app, PROGRAM), artwork(app, GENERIC))
            && same_picture(&program, &generic).is_err()
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "after {LOAD_LIMIT:?} {PROGRAM}'s tile looks like that of {GENERIC}, which has \
                 no icon of its own"
            ));
        }
        app.idle(Duration::from_millis(100));
    }
}

/// The fixture document: one 300 x 200 pt landscape page with two lines of Helvetica.
const PDF: &str = "report.pdf";
/// The panel's default thumbnail edge.
const THUMBNAIL_EDGE: f32 = 96.0;

/// Whether the fixture PDF's cell shows its first page ([`page_shown`]).
fn pdf_page(app: &App) -> Result<String, String> {
    page_shown(app, PDF)
}

/// Whether the cell of the PDF `name` shows its first page: a white page as wide as the
/// thumbnail (a landscape page fills the width) with dark text on it. A placeholder
/// icon has no white rows that wide. Only the artwork band of the cell is read, not its
/// name.
fn page_shown(app: &App, name: &str) -> Result<String, String> {
    let (x, y, width, _) = inspect::controls(app.slint(), AccessibleRole::ListItem)
        .into_iter()
        .find(|(label, _)| panel::is_row_label(label, name))
        .map(|(_, item)| inspect::rect(&item))
        .ok_or_else(|| format!("{name} is not visible in the thumbnail view"))?;
    let edge = THUMBNAIL_EDGE.min(width - 24.0).max(1.0);
    let (canvas_width, canvas_height, pixels) = app::render(&app.main);
    let horizontal = |value: f32| (value.max(0.0) as usize).min(canvas_width);
    let vertical = |value: f32| (value.max(0.0) as usize).min(canvas_height);
    let (left, right) = (horizontal(x), horizontal(x + width));
    // The artwork starts 12 px below the cell's top; its name starts 6 px below it.
    let (top, bottom) = (vertical(y + 8.0), vertical(y + 12.0 + edge + 4.0));
    let wide = (edge * 0.85) as usize;
    let white = |line: usize, column: usize| {
        let pixel = pixels[line * canvas_width + column];
        pixel.r >= 235 && pixel.g >= 235 && pixel.b >= 235
    };
    let mut page_rows = Vec::new();
    let (mut page_left, mut page_right) = (usize::MAX, 0);
    for line in top..bottom {
        // The longest white run of this row, as a column range.
        let (mut start, mut longest) = (left, (left, left));
        for column in left..=right {
            if column < right && white(line, column) {
                continue;
            }
            if column - start > longest.1 - longest.0 {
                longest = (start, column);
            }
            start = column + 1;
        }
        if longest.1 - longest.0 >= wide {
            page_rows.push(line);
            page_left = page_left.min(longest.0);
            page_right = page_right.max(longest.1);
        }
    }
    let (first, last) = match (page_rows.first(), page_rows.last()) {
        (Some(first), Some(last)) if page_rows.len() as f32 >= edge * 0.4 => (*first, *last),
        _ => {
            return Err(format!(
                "{name} shows no page: {} rows of its {edge} px artwork have a white run of \
                 {wide} px or more",
                page_rows.len()
            ));
        }
    };
    let ink = (first..=last)
        .flat_map(|line| (page_left..page_right).map(move |column| line * canvas_width + column))
        .filter(|index| {
            let pixel = pixels[*index];
            pixel.r.max(pixel.g).max(pixel.b) < 140
        })
        .count();
    if ink < 10 {
        return Err(format!(
            "{name} shows a white page without its text: {ink} dark pixels"
        ));
    }
    Ok(format!(
        "a white page of {} x {} px with {ink} dark text pixels",
        page_right - page_left,
        last + 1 - first
    ))
}

fn support_wait(app: &mut App, what: &str, condition: impl Fn(&App) -> bool) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.settle();
        if condition(app) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("timed out waiting for {what}"));
        }
        app.idle(Duration::from_millis(20));
    }
}

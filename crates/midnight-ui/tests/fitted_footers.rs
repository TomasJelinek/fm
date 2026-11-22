//! Fitted dialog footers (`DialogFooter`'s `footer-actions`, laid out by `FooterActions`)
//! in the windows that use them, Find and Checksums. At any width, font and density every
//! offered action is a whole button inside the footer (and, from the windows' minimum
//! width on, inside the window), 8 px from the next, or in More…, never both. The steps
//! that narrow the row are taken in order, each only where the row does not fit without
//! it, judged here from the buttons' real widths: More… as its glyph (1); every action of
//! rank 1 and up in More…, the rank 0 actions as their glyphs (2); the dismiss button as
//! its glyph (3); every action in More… (4). Widths below the 320 px minimum reach the last
//! steps with any text metrics, so every step runs on every platform.
use crate::support::modeless;

use i_slint_core::{
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::AccessibleRole,
};
use midnight_core::PreferenceTheme;
use midnight_ui::{ChecksumWindow, FindRow, FindWindow};
use slint::{
    ComponentHandle, ModelRc, VecModel,
    platform::{Key, WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

/// The sweep's window widths, widest first; the first shows every action with its caption.
const WIDTHS: [u32; 22] = [
    2400, 1400, 1000, 800, 640, 560, 480, 440, 400, 360, 340, 320, 300, 280, 260, 240, 220, 200,
    180, 160, 140, 120,
];
/// Where the More… menu is opened and checked.
const MENU_WIDTHS: [u32; 3] = [320, 200, 120];
const FONTS: [i32; 4] = [12, 15, 32, 48];
/// The windows' minimum width (`ToolWindow`).
const MINIMUM_WIDTH: u32 = 320;
/// Rounding of the software layout; the step checks allow 1 px.
const EPSILON: f32 = 0.5;

/// One state of a footer: its offered actions of rank 0, and the others at a window width
/// (some captions follow the width).
struct Offer<'a> {
    name: &'a str,
    rank0: &'a [&'a str],
    others: &'a dyn Fn(u32) -> Vec<&'static str>,
}

/// A button showing its glyph alone: 24 px of padding and `Theme.icon-size` (14 px, 20 px
/// in touch density).
fn glyph(density: i32) -> f32 {
    if density == 1 { 44.0 } else { 38.0 }
}

fn key(native: &MinimalSoftwareWindow, key: Key) {
    let text: slint::SharedString = key.into();
    native
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    native
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

/// A footer button: its label, and its left and right edges.
type Button = (String, f32, f32);
/// The left and right edges of the footer.
type Bar = (f32, f32);

/// The footer row's buttons, Close last, after the shared check that each is whole and
/// 8 px from the next; and the footer's edges.
fn footer(native: &MinimalSoftwareWindow, case: &str) -> (Vec<Button>, Bar) {
    let labels = modeless::fitted_footer(native, "Close", case);
    let row: Vec<Button> = modeless::footer_row(native, "Close")
        .into_iter()
        .map(|(label, item)| {
            let (left, _, right, _) = modeless::edges(&item);
            (label, left, right)
        })
        .collect();
    assert_eq!(
        row.iter()
            .map(|button| button.0.as_str())
            .collect::<Vec<_>>(),
        labels,
        "{case}"
    );
    let close = modeless::accessible_item(native, AccessibleRole::Button, "Close")
        .expect("the footer's Close");
    let bar: ItemRc = close
        .parent_item(ParentItemTraversalMode::StopAtPopups)
        .expect("the footer");
    let (left, _, right, _) = modeless::edges(&bar);
    (row, (left, right))
}

/// Sweeps one footer state over `WIDTHS`; returns the steps it saw.
fn sweep(
    native: &MinimalSoftwareWindow,
    offer: &Offer,
    label: &str,
    glyph: f32,
    height: u32,
) -> [bool; 5] {
    let mut seen = [false; 5];
    let mut captioned: HashMap<String, f32> = HashMap::new();
    let mut previous = 0;
    for (index, &width) in WIDTHS.iter().enumerate() {
        modeless::layout(native, width, height);
        let case = format!("{label} {} {width}x{height}", offer.name);
        let (row, bar) = footer(native, &case);
        let others = (offer.others)(width);
        let shown = |name: &str| {
            row.iter()
                .find(|button| button.0 == name)
                .map(|b| b.2 - b.1)
        };
        let glyph_only = |name: &str| shown(name).is_some_and(|width| width <= glyph + EPSILON);
        if index == 0 {
            // Every action with its caption: the real widths the checks below use.
            for name in offer.rank0.iter().chain(&others).chain(&["Close"]) {
                let width = shown(name)
                    .unwrap_or_else(|| panic!("{case}: {name} is not a button: {row:?}"));
                assert!(
                    width > glyph + EPSILON,
                    "{case}: {name} shows its caption: {row:?}"
                );
                captioned.insert((*name).to_owned(), width);
            }
        }
        // Inside the footer, and from the minimum width on inside the window.
        for (name, left, right) in &row {
            assert!(
                *left >= bar.0 - EPSILON && *right <= bar.1 + EPSILON,
                "{case}: {name} at {left}..{right} leaves the footer at {bar:?}"
            );
            if width >= MINIMUM_WIDTH {
                assert!(
                    *left >= -EPSILON && *right <= width as f32 + EPSILON,
                    "{case}: {name} at {left}..{right} leaves the window"
                );
            }
        }
        // The step the row took, read from the buttons.
        let more = shown("More…").is_some();
        let rank0: Vec<&str> = offer
            .rank0
            .iter()
            .copied()
            .filter(|name| shown(name).is_some())
            .collect();
        let level = if rank0.is_empty() {
            4
        } else if glyph_only("Close") {
            3
        } else if rank0.iter().any(|name| glyph_only(name)) {
            2
        } else if more && glyph_only("More…") {
            1
        } else {
            0
        };
        seen[level] = true;
        assert!(
            level >= previous,
            "{case}: step {level} after step {previous} in a wider window"
        );
        previous = level;
        if level < 4 {
            assert_eq!(
                rank0.len(),
                offer.rank0.len(),
                "{case}: rank 0 stays before step 4: {row:?}"
            );
        }
        if level < 3 {
            assert!(
                !glyph_only("Close"),
                "{case}: Close keeps its caption before step 3"
            );
        }
        if level < 2 {
            for name in &rank0 {
                assert!(
                    !glyph_only(name),
                    "{case}: {name} keeps its caption before step 2"
                );
            }
        }
        if level >= 1 && more {
            assert!(
                glyph_only("More…"),
                "{case}: More… shows its glyph alone from step 1"
            );
        }
        if level >= 2 {
            for (name, _, _) in &row {
                let name = name.as_str();
                assert!(
                    name == "More…"
                        || name == "Close"
                        || (offer.rank0.contains(&name) && glyph_only(name)),
                    "{case}: step {level} shows only rank 0 actions, as glyphs: {row:?}"
                );
            }
        }
        if level == 4 {
            let names: Vec<&str> = row.iter().map(|button| button.0.as_str()).collect();
            assert_eq!(names, ["More…", "Close"], "{case}");
            assert!(glyph_only("Close"), "{case}: {row:?}");
        }
        // Each step only where the row does not fit without it, and taken where it does
        // not: the real widths of the captioned buttons and of the footer.
        let room = (bar.1 - bar.0) - captioned["Close"] - 8.0;
        let compact_room = (bar.1 - bar.0) - glyph - 8.0;
        let menu = if others.is_empty() { 0.0 } else { glyph + 8.0 };
        let captions: f32 = offer
            .rank0
            .iter()
            .map(|name| captioned[*name] + 8.0)
            .sum::<f32>()
            + menu;
        let glyphs = offer.rank0.len() as f32 * (glyph + 8.0) + menu;
        let fits = |row: f32, room: f32| row <= room + 8.0;
        let check = |needed: bool, what: &str| {
            assert!(
                needed,
                "{case}: step {level}, {what} (room {room}, compact {compact_room}, captions {captions}, glyphs {glyphs})"
            );
        };
        if level >= 2 {
            check(
                !fits(captions, room - 1.0),
                "though the captions and the More… glyph fit",
            );
        } else {
            check(
                fits(captions, room + 1.0),
                "though the captions and the More… glyph do not fit",
            );
        }
        if level >= 3 {
            check(
                !fits(glyphs, room - 1.0),
                "though the glyphs fit beside the captioned Close",
            );
        } else if level == 2 {
            check(
                fits(glyphs, room + 1.0),
                "though the glyphs do not fit beside the captioned Close",
            );
        }
        if level == 4 {
            check(
                !fits(glyphs, compact_room - 1.0),
                "though the glyphs fit beside the Close glyph",
            );
        } else if level == 3 {
            check(
                fits(glyphs, compact_room + 1.0),
                "though the glyphs do not fit beside the Close glyph",
            );
        }
        // Each offered action is a button or in More…, never both.
        if MENU_WIDTHS.contains(&width) && more && height <= 200 {
            modeless::click(native, "More…");
            for name in offer.rank0.iter().chain(&others) {
                let in_menu =
                    modeless::accessible_popup_item(native, AccessibleRole::Button, name).is_some();
                assert_ne!(
                    in_menu,
                    shown(name).is_some(),
                    "{case}: {name} must be a button or in More…, not both"
                );
            }
            key(native, Key::Escape);
            for name in offer.rank0.iter().chain(&others) {
                assert!(
                    modeless::accessible_popup_item(native, AccessibleRole::Button, name).is_none(),
                    "{case}: Escape closes More…"
                );
            }
        }
    }
    seen
}

/// The fonts with large ones in a taller window, so that the footer stays in view.
fn height(font: i32) -> u32 {
    if font > 32 { 400 } else { 200 }
}

#[test]
fn find_footers_fit_every_width_font_and_density_in_order() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let window = FindWindow::new().unwrap();
    window.set_rows(ModelRc::new(VecModel::from(vec![FindRow {
        token: "3:1".into(),
        name: "report.txt".into(),
        ..Default::default()
    }])));
    window.set_selected_row(0);
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    let query = |_: u32| vec!["Go to file", "View", "Edit", "Open", "Copy details"];
    let details = |_: u32| vec!["Find", "Copy"];
    let offers = [
        (
            false,
            Offer {
                name: "query",
                rank0: &["Find"],
                others: &query,
            },
        ),
        (
            true,
            Offer {
                name: "details",
                rank0: &["Back"],
                others: &details,
            },
        ),
    ];
    let mut seen = [false; 5];
    for (density, theme) in [(0, PreferenceTheme::Blue), (1, PreferenceTheme::Dark)] {
        for font in FONTS {
            window.invoke_apply_preferences(theme.index(), density, "".into(), font, font);
            for (page, offer) in &offers {
                window.set_show_details(*page);
                let label = format!("Find density {density} font {font}");
                let steps = sweep(&native, offer, &label, glyph(density), height(font));
                for (step, reached) in steps.iter().enumerate() {
                    seen[step] |= reached;
                }
            }
        }
    }
    assert_eq!(
        [seen[0], seen[2], seen[3], seen[4]],
        [true; 4],
        "the sweep reaches steps 0, 2, 3 and 4: {seen:?}"
    );
    window.hide().unwrap();
}

#[test]
fn checksum_footers_fit_every_width_font_and_density_in_order() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let window = ChecksumWindow::new().unwrap();
    window.set_has_output(true);
    window.set_output_available(Rc::new(VecModel::from(vec![true; 5])).into());
    window.set_copy_lines(1);
    window.set_copy_lines_text("1".into());
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    let calculate = |width: u32| {
        if width < 460 {
            vec!["Copy", "Save…"]
        } else {
            vec!["Copy SHA-256", "Save manifest…"]
        }
    };
    let verify = |_: u32| Vec::<&'static str>::new();
    let details = |_: u32| vec!["Copy details"];
    // Page, details shown, running: two rank 0 actions while a run's details show.
    let offers = [
        (
            0,
            false,
            false,
            Offer {
                name: "calculate",
                rank0: &["Calculate"],
                others: &calculate,
            },
        ),
        (
            1,
            false,
            false,
            Offer {
                name: "verify",
                rank0: &["Verify"],
                others: &verify,
            },
        ),
        (
            0,
            true,
            false,
            Offer {
                name: "details",
                rank0: &["Back"],
                others: &details,
            },
        ),
        (
            0,
            true,
            true,
            Offer {
                name: "running details",
                rank0: &["Back", "Stop"],
                others: &details,
            },
        ),
    ];
    let mut seen = [false; 5];
    for (density, theme) in [(0, PreferenceTheme::Blue), (1, PreferenceTheme::Dark)] {
        for font in FONTS {
            window.invoke_apply_preferences(theme.index(), density, "".into(), font, font);
            for (page, details, running, offer) in &offers {
                window.set_page(*page);
                // A page change closes the details: let that run before they open.
                modeless::layout(&native, WIDTHS[0], height(font));
                window.set_show_details(*details);
                window.set_running(*running);
                let label = format!("Checksums density {density} font {font}");
                let steps = sweep(&native, offer, &label, glyph(density), height(font));
                for (step, reached) in steps.iter().enumerate() {
                    seen[step] |= reached;
                }
            }
        }
    }
    assert_eq!(
        [seen[0], seen[2], seen[3], seen[4]],
        [true; 4],
        "the sweep reaches steps 0, 2, 3 and 4: {seen:?}"
    );
    window.hide().unwrap();
}

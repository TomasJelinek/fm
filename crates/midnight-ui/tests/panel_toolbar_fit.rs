//! The panel toolbar's View and Sort pickers give way to the drive and folder buttons
//! (the owner's request of 24 November): while the buttons fit beside the full pickers
//! ("Detailed", "Name ↑") the pickers stay whole; once they need the room, the pickers
//! turn compact (the view's glyph, the sort key's initial and its direction), decided
//! from the buttons' measured widths and the full pickers' widths measured from their
//! longest captions, never from the pickers' own current size, so the row cannot flip
//! between the two. The buttons are never narrowed for the pickers; only when the
//! compact pickers leave too little room do they scroll behind the strip's arrows. The
//! compact pickers keep their accessible names, tooltips, menus and keys.
#[path = "support/window_polish.rs"]
mod polish;
use crate::support::ui;

use i_slint_core::{
    accessibility::AccessibleStringProperty, item_tree::ItemRc, items::AccessibleRole,
};
use midnight_ui::{LocationRole, LocationRow};
use slint::{ComponentHandle, VecModel};
use std::rc::Rc;

const VIEW: &str = "Left panel view mode";
const SORT: &str = "Left panel sort";
const FILTER: &str = "Left panel filter";

fn edges(item: &ItemRc) -> (f32, f32, f32, f32) {
    let rect = item.geometry();
    let at = item.map_to_native_window(rect.origin);
    (at.x, at.y, at.x + rect.width(), at.y + rect.height())
}

/// `count` drive buttons: C:, D:, …, each named by its tooltip.
fn drives(count: usize) -> Vec<LocationRow> {
    (0..count)
        .map(|index| {
            let letter = char::from(b'C' + index as u8);
            LocationRow {
                identity: format!("drive-{letter}").into(),
                kind: LocationRole::Drive,
                label: format!("{letter}:").into(),
                compact_label: format!("{letter}:").into(),
                is_drive: true,
                path: format!("{letter}:\\").into(),
                tooltip: format!("Local Disk ({letter}:)").into(),
                selected: index == 0,
                ..Default::default()
            }
        })
        .collect()
}

/// What the left panel's toolbar shows.
struct Row {
    /// The drive buttons' rectangles, in order.
    buttons: Vec<(f32, f32, f32, f32)>,
    /// Where the strip of buttons starts: right of the drive menu.
    strip: f32,
    /// The filter button's left edge: where the panel's own controls start.
    controls: f32,
    view: (f32, f32, f32, f32),
    sort: (f32, f32, f32, f32),
}

impl Row {
    fn compact(&self) -> bool {
        self.view.2 - self.view.0 < 60.0
    }

    /// The buttons' laid-out width (the strip's content).
    fn content(&self) -> f32 {
        match (self.buttons.first(), self.buttons.last()) {
            (Some(first), Some(last)) => last.2 - first.0,
            _ => 0.0,
        }
    }
}

fn row(ui: &mut ui::TestUi, count: usize) -> Row {
    ui.window
        .set_left_locations(Rc::new(VecModel::from(drives(count))).into());
    ui.draw();
    let menu = ui::root_item(&ui.window, "Drive or volume", AccessibleRole::Combobox);
    let mut buttons: Vec<_> = (0..count)
        .map(|index| {
            let letter = char::from(b'C' + index as u8);
            edges(&ui::root_item(
                &ui.window,
                &format!("Local Disk ({letter}:)"),
                AccessibleRole::Button,
            ))
        })
        .collect();
    buttons.sort_by(|a, b| a.0.total_cmp(&b.0));
    Row {
        buttons,
        strip: edges(&menu).2,
        controls: edges(&ui::root_item(&ui.window, FILTER, AccessibleRole::Button)).0,
        view: edges(&ui::root_item(&ui.window, VIEW, AccessibleRole::Combobox)),
        sort: edges(&ui::root_item(&ui.window, SORT, AccessibleRole::Combobox)),
    }
}

fn value(ui: &ui::TestUi, label: &str) -> String {
    ui::root_item(&ui.window, label, AccessibleRole::Combobox)
        .accessible_string_property(AccessibleStringProperty::Value)
        .unwrap_or_default()
        .to_string()
}

#[test]
fn pickers_turn_compact_only_when_the_drive_buttons_need_their_room() {
    let mut ui = ui::TestUi::new();
    let mut findings = Vec::new();
    for (density, font) in [(0, 8), (0, 12), (0, 32), (1, 8), (1, 15), (1, 32)] {
        ui.window.set_density_index(density);
        ui.window.set_desktop_font_size(font);
        ui.window.set_touch_font_size(font);
        for width in [1600, 1340, 1100, 900, 700] {
            ui.resize(width, 700);
            let case = format!("density {density}, font {font}, window {width}");
            // The full pickers' room: what the strip has beside them with no buttons.
            let empty = row(&mut ui, 0);
            if empty.compact() {
                // Too narrow for the full pickers even without buttons: compact always.
                continue;
            }
            let room = empty.controls - empty.strip;
            let full = (empty.view, empty.sort);
            for count in [1, 2, 4, 8, 14] {
                let shown = row(&mut ui, count);
                let content = shown.content();
                let needs_room = content > room - 2.0 * if density == 1 { 6.0 } else { 4.0 };
                if shown.compact() != needs_room {
                    findings.push(format!(
                        "{case}, {count} drives: compact {} with buttons {content:.0}px wide and {room:.0}px beside the full pickers",
                        shown.compact()
                    ));
                }
                if !shown.compact() {
                    // Whole pickers keep their full width, and every button shows.
                    if (shown.view.2 - shown.view.0 - (full.0.2 - full.0.0)).abs() > 0.5
                        || (shown.sort.2 - shown.sort.0 - (full.1.2 - full.1.0)).abs() > 0.5
                    {
                        findings.push(format!("{case}, {count} drives: full pickers resized"));
                    }
                    if let Some(last) = shown.buttons.last()
                        && last.2 > shown.controls + 0.5
                    {
                        findings.push(format!(
                            "{case}, {count} drives: the last button ends at {:.0}, the controls start at {:.0}",
                            last.2, shown.controls
                        ));
                    }
                }
                // Buttons are never squeezed: each as wide as in a window wide enough
                // for all of them.
                ui.resize(4000, 700);
                let natural = row(&mut ui, count);
                ui.resize(width, 700);
                let shown = row(&mut ui, count);
                for (button, wide) in shown.buttons.iter().zip(&natural.buttons) {
                    if (button.2 - button.0) + 0.5 < wide.2 - wide.0 {
                        findings.push(format!(
                            "{case}, {count} drives: a button is {:.0}px wide, {:.0}px where all fit",
                            button.2 - button.0,
                            wide.2 - wide.0
                        ));
                    }
                }
                // The same layout again: no flip between the variants.
                let again = row(&mut ui, count);
                if again.compact() != shown.compact() {
                    findings.push(format!("{case}, {count} drives: the pickers flipped"));
                }
                // The row lines up: pickers, filter and buttons share one centre.
                let centre = |rect: (f32, f32, f32, f32)| (rect.1 + rect.3) / 2.0;
                for (name, rect) in [("view", shown.view), ("sort", shown.sort)] {
                    if let Some(button) = shown.buttons.first()
                        && (centre(rect) - centre(*button)).abs() > 1.0
                    {
                        findings.push(format!(
                            "{case}, {count} drives: the {name} picker is centred at {:.1}, the buttons at {:.1}",
                            centre(rect),
                            centre(*button)
                        ));
                    }
                }
            }
        }
    }
    ui.window.hide().unwrap();
    assert!(findings.is_empty(), "{findings:#?}");
}

#[test]
fn compact_pickers_keep_their_names_menus_and_keys() {
    let mut ui = ui::TestUi::new();
    let sorts = Rc::new(std::cell::RefCell::new(Vec::new()));
    let capture = sorts.clone();
    ui.window
        .on_sort(move |pane, column| capture.borrow_mut().push((pane, column)));
    ui.resize(1100, 700);
    let shown = row(&mut ui, 14);
    assert!(
        shown.compact(),
        "14 drives in a 1100 px window need the room"
    );
    assert_eq!(value(&ui, VIEW), "Detailed");
    assert_eq!(value(&ui, SORT), "Name");
    // The compact sort picker names its key and direction.
    let sort = ui::root_item(&ui.window, SORT, AccessibleRole::Combobox);
    let captions = ui::visible_text_values(sort);
    assert!(
        captions.iter().any(|caption| caption.as_str() == "N ↑"),
        "the compact sort caption: {captions:?}"
    );
    // The same menu: Size sorts by size.
    ui::click_root(&ui.window, SORT, AccessibleRole::Combobox);
    ui.draw();
    ui::click_popup(&ui.window, "Size", AccessibleRole::Button);
    ui.draw();
    assert_eq!(sorts.borrow().last(), Some(&(0, 1)));
    // The keyboard opens it as before.
    ui::click_root(&ui.window, VIEW, AccessibleRole::Combobox);
    ui.draw();
    assert!(
        ui::has_popup(&ui.window),
        "the compact view picker opens its menu"
    );
    assert!(
        ui::try_popup_item(&ui.window, "Thumbnails", AccessibleRole::Button).is_some(),
        "the same view choices"
    );
    ui::key(&ui.window, slint::platform::Key::Escape);
    ui.draw();
    // The toolbar row passes the polish gate's alignment checks in both variants.
    let mut findings = Vec::new();
    for count in [0, 14] {
        row(&mut ui, count);
        let pixels: Vec<slint::Rgb8Pixel> = Vec::new();
        findings.extend(
            polish::alignment_findings(ui.window.window(), &pixels)
                .into_iter()
                .filter(|finding| {
                    finding.contains("Left panel")
                        || finding.contains("Local Disk")
                        || finding.contains("Drive or volume")
                })
                .map(|finding| format!("{count} drives: {finding}")),
        );
    }
    ui.window.hide().unwrap();
    assert!(findings.is_empty(), "{findings:#?}");
}

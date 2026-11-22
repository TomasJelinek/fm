//! The About box, driven with the pointer and keys: opened from the toolbar's
//! application icon; the About tab (Version, a Copyright row with the year and the owner's
//! name, License naming the GNU GPL version 3 or later, Built with, Libraries, in one label
//! column; no Author row) laid out with room between its lines; the free-software notice
//! and its View license showing the GPL's full text, with Back returning to the About tab;
//! Saturn drawn in the sky clear of the title and the icon, no
//! crescent moon; the Licenses tab: the count,
//! the filter ("K of N", no match,
//! cleared), a click selecting a library (selected state, the "›" open mark, View
//! license enabled), hover marking only the hovered row, the pointer cursor over rows,
//! the mark's click, View license, a double-click, Enter in the list and in the filter,
//! Up, Down, Page Down, Home and End, Down from the filter into the list, Left and Right
//! between the tabs; the detail view (title, License, Used by, authors, text) and Back
//! and Escape returning to the list with the filter and row kept, also after reopening
//! About; Escape, Close, a click outside and dragging the sky.
//!
//! The harness does not set the application version; the scenario sets it as
//! `crates/midnight-explorer/src/main.rs` does, from the workspace version.
use crate::{
    Context,
    app::{App, Config},
    input, inspect, panel,
    tools_support::{self as tools},
};
use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, Clip, ClippedImage, ImageItem, TouchArea},
};
use midnight_core::PaneId;
use midnight_ui::AboutView;
use slint::{ComponentHandle, LogicalPosition, Model, platform::Key};
use std::time::{Duration, Instant};

const MARKER: &str = ". Shows its license and authors.";
const SUMMARY: &str = "libraries and components";

type Rect = inspect::Rect;

/// One library row as the list shows it.
#[derive(Clone)]
struct Library {
    label: String,
    license: String,
    rect: Rect,
    selected: bool,
    item: ItemRc,
}

fn wait(app: &mut App, what: &str, condition: impl Fn(&App) -> bool) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.settle();
        if condition(app) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "{what} did not happen; visible text: {:?}",
                inspect::visible_texts(app.slint())
            ));
        }
        app.idle(Duration::from_millis(10));
    }
}

fn about_open(app: &App) -> bool {
    app.window.get_about_open()
}

fn detail_open(app: &App) -> bool {
    app.window.global::<AboutView>().get_detail_open()
}

/// The About dialog's popup (the topmost one while About is open).
fn dialog(app: &App) -> Result<ItemRc, String> {
    if !about_open(app) {
        return Err("About is not open".into());
    }
    inspect::roots(app.slint())
        .pop()
        .ok_or_else(|| "About has no popup".into())
}

/// Whether no ancestor hides the item through `visible: false`.
fn shown(item: &ItemRc) -> bool {
    let mut current = Some(item.clone());
    while let Some(node) = current {
        if let Some(clip) = node.downcast::<Clip>() {
            let clip = clip.as_pin_ref();
            if clip.is_visibility_clip() && clip.clip() {
                return false;
            }
        }
        current = node.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    true
}

fn property(item: &ItemRc, which: AccessibleStringProperty) -> String {
    item.accessible_string_property(which)
        .map(|value| value.to_string())
        .unwrap_or_default()
}

/// The library rows a user can see, top to bottom.
fn libraries(app: &App) -> Vec<Library> {
    let Ok(root) = dialog(app) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    inspect::walk(&root, &mut |item| {
        if item.accessible_role() != AccessibleRole::Button {
            return;
        }
        let description = property(item, AccessibleStringProperty::Description);
        let Some(license) = description.strip_suffix(MARKER) else {
            return;
        };
        let Some(rect) = inspect::visible_rect(item) else {
            return;
        };
        // Rows the list shows only in part are left to scrolling.
        if rect.3 < inspect::rect(item).3 * 0.9 {
            return;
        }
        rows.push(Library {
            label: inspect::label(item).unwrap_or_default(),
            license: license.to_owned(),
            rect,
            selected: property(item, AccessibleStringProperty::ItemSelected) == "true",
            item: item.clone(),
        });
    });
    rows.sort_by(|a, b| a.rect.1.total_cmp(&b.rect.1));
    rows
}

/// Whether the row of `label` is fully in the list's view: None when it is, otherwise
/// what a user sees of it.
fn in_view(app: &App, label: &str) -> Option<String> {
    let root = dialog(app).ok()?;
    let mut found = None;
    inspect::walk(&root, &mut |item| {
        if found.is_none()
            && item.accessible_role() == AccessibleRole::Button
            && inspect::label(item).as_deref() == Some(label)
            && property(item, AccessibleStringProperty::Description).ends_with(MARKER)
        {
            found = Some(item.clone());
        }
    });
    let Some(row) = found else {
        return Some(format!("the selected {label:?} is not in the list's view"));
    };
    let full = inspect::rect(&row);
    let shown = inspect::visible_rect(&row).map_or(0.0, |rect| rect.3);
    (shown < full.3 - 0.5).then(|| {
        format!(
            "the selected {label:?} is only partly in view ({shown:.0} of {:.0} px shown)",
            full.3
        )
    })
}

fn selected(app: &App) -> Option<String> {
    libraries(app)
        .into_iter()
        .find(|row| row.selected)
        .map(|row| row.label)
}

/// Whether the row shows its open mark (the "›" image).
fn mark_shown(row: &ItemRc) -> bool {
    let mut found = false;
    inspect::walk(row, &mut |item| {
        if (item.downcast::<ImageItem>().is_some() || item.downcast::<ClippedImage>().is_some())
            && shown(item)
        {
            found = true;
        }
    });
    found
}

/// Where the row's shown open mark is.
fn mark_rect(row: &ItemRc) -> Option<Rect> {
    let mut found = None;
    inspect::walk(row, &mut |item| {
        if found.is_none()
            && (item.downcast::<ImageItem>().is_some() || item.downcast::<ClippedImage>().is_some())
            && shown(item)
        {
            found = inspect::visible_rect(item);
        }
    });
    found
}

/// The mouse cursors the row's pointer areas set.
fn cursors(row: &ItemRc) -> Vec<String> {
    let mut found = Vec::new();
    inspect::walk(row, &mut |item| {
        if let Some(area) = item.downcast::<TouchArea>() {
            found.push(format!("{:?}", area.as_pin_ref().mouse_cursor()));
        }
    });
    found
}

/// Visible texts of the dialog with their window rectangles.
fn texts(app: &App) -> Vec<(String, Rect)> {
    let Ok(root) = dialog(app) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    inspect::walk(&root, &mut |item| {
        if let Some(text) = tools::text_of(item)
            && !text.trim().is_empty()
            && let Some(rect) = inspect::visible_rect(item)
        {
            found.push((text, rect));
        }
    });
    found
}

fn summary(app: &App) -> Option<String> {
    texts(app)
        .into_iter()
        .map(|(text, _)| text)
        .find(|text| text.ends_with(SUMMARY))
}

/// "N libraries and components" or "K of N libraries and components".
fn counts(text: &str) -> Option<(u64, u64)> {
    let numbers: Vec<u64> = text.split_whitespace().filter_map(tools::number).collect();
    match numbers.as_slice() {
        [all] => Some((*all, *all)),
        [shown, all] if text.contains(" of ") => Some((*shown, *all)),
        _ => None,
    }
}

/// The value shown to the right of a label on the same line (an InfoRow).
fn row_value(texts: &[(String, Rect)], label: &str) -> Option<String> {
    let (_, at) = texts.iter().find(|(text, _)| text == label)?;
    texts
        .iter()
        .filter(|(text, rect)| text != label && rect.0 > at.0 + at.2 && (rect.1 - at.1).abs() < 3.0)
        .min_by(|a, b| a.1.0.total_cmp(&b.1.0))
        .map(|(text, _)| text.clone())
}

/// The vertical gaps between consecutive lines of `wanted` texts (texts that start on
/// the same line form one line); an error names the tightest pair below `minimum`.
fn spacing(texts: &[(String, Rect)], wanted: &[&str], minimum: f32) -> Result<f32, String> {
    let mut lines: Vec<(f32, f32, String)> = Vec::new();
    for want in wanted {
        let (text, rect) = texts
            .iter()
            .find(|(text, _)| text == want)
            .ok_or_else(|| format!("{want:?} is not shown"))?;
        if let Some(line) = lines.iter_mut().find(|line| (line.0 - rect.1).abs() < 2.0) {
            line.1 = line.1.max(rect.1 + rect.3);
            line.2.push_str(&format!(" | {text}"));
        } else {
            lines.push((rect.1, rect.1 + rect.3, text.clone()));
        }
    }
    lines.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut tightest = f32::MAX;
    for pair in lines.windows(2) {
        let gap = pair[1].0 - pair[0].1;
        if gap < minimum {
            return Err(format!(
                "{:?} ends {gap:.1}px above {:?} (at least {minimum}px expected)",
                pair[0].2, pair[1].2
            ));
        }
        tightest = tightest.min(gap);
    }
    Ok(tightest)
}

const LICENSE_VALUE: &str = "GNU General Public License, version 3 or later";
const FREE_SOFTWARE: &str = "Midnight Explorer is free software: you can redistribute it and/or modify it under the terms of the GNU General Public License as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version. It comes with ABSOLUTELY NO WARRANTY.";

/// The About tab names the program's license, states the free-software notice, and its
/// View license shows the GPL's full text in the detail view; Back returns to the tab.
fn own_license(app: &mut App, shown: &[(String, Rect)]) -> Result<(), String> {
    if row_value(shown, "License").as_deref() != Some(LICENSE_VALUE) {
        return Err(format!(
            "the About tab's License row shows {:?}, not {LICENSE_VALUE:?}",
            row_value(shown, "License")
        ));
    }
    if !shown.iter().any(|(text, _)| text == FREE_SOFTWARE) {
        return Err("the About tab shows no free-software notice".into());
    }
    input::click_button(app.slint(), "View license")?;
    wait(
        app,
        "View license on the About tab to open the GPL",
        detail_open,
    )?;
    let view = app.window.global::<AboutView>();
    let (title, license) = (view.get_detail_title(), view.get_detail_license());
    if title != "Midnight Explorer" || license != "GPL-3.0-or-later" {
        return Err(format!(
            "View license on the About tab shows {title:?} under {license:?}"
        ));
    }
    let detail = texts(app);
    if row_value(&detail, "License").as_deref() != Some("GPL-3.0-or-later") {
        return Err(format!(
            "the program's license detail shows License {:?}",
            row_value(&detail, "License")
        ));
    }
    if !detail.iter().any(|(text, _)| text == COPYRIGHT) {
        return Err("the program's license detail shows no copyright line".into());
    }
    if !detail.iter().any(|(text, _)| {
        text.contains("GNU GENERAL PUBLIC LICENSE") && text.contains("Version 3, 29 June 2007")
    }) {
        return Err("the program's license detail shows no GPL text".into());
    }
    input::click_button(app.slint(), "Back")?;
    wait(app, "Back to the About tab", |app| {
        !detail_open(app)
            && inspect::visible_texts(app.slint())
                .iter()
                .any(|text| text == "Built with")
    })
}

const COPYRIGHT_LABEL: &str = "Copyright";
const COPYRIGHT: &str = "© 2026 Tomas Jelinek";
/// The owner's name alone, as the removed Author row showed it.
const NAME: &str = "Tomas Jelinek";

/// The Copyright row reads exactly the copyright line, its value in the Version row's
/// column (one label column for every row); there is no Author row, and the name never
/// stands alone.
fn copyright(shown: &[(String, Rect)]) -> Result<(), String> {
    if row_value(shown, COPYRIGHT_LABEL).as_deref() != Some(COPYRIGHT) {
        return Err(format!(
            "the About tab's Copyright row shows {:?}, not {COPYRIGHT:?}",
            row_value(shown, COPYRIGHT_LABEL)
        ));
    }
    if let Some((text, _)) = shown
        .iter()
        .find(|(text, _)| text == "Author" || text == NAME)
    {
        return Err(format!(
            "the About tab still shows {text:?}: the Author row is gone"
        ));
    }
    let value_x = |label: &str| {
        let value = row_value(shown, label)?;
        shown
            .iter()
            .find(|(text, _)| *text == value)
            .map(|(_, rect)| rect.0)
    };
    match (value_x("Version"), value_x(COPYRIGHT_LABEL)) {
        (Some(version), Some(copyright)) if (version - copyright).abs() <= 1.0 => Ok(()),
        (version, copyright) => Err(format!(
            "the Copyright row's value starts at x {copyright:?}, the Version row's at {version:?}: not one column"
        )),
    }
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
}

/// Saturn is in the sky beside the application's icon, drawn (its gold planet and rings
/// in the rendered pixels), and covers neither the title nor the icon; the crescent moon
/// it replaced is gone.
fn saturn(app: &mut App, shown: &[(String, Rect)]) -> Result<(), String> {
    let planet = inspect::find(app.slint(), AccessibleRole::Image, "Saturn")
        .and_then(|item| inspect::visible_rect(&item))
        .ok_or("the sky shows no Saturn")?;
    let icon = inspect::find(app.slint(), AccessibleRole::Image, "Midnight Explorer icon")
        .and_then(|item| inspect::visible_rect(&item))
        .ok_or("the sky shows no application icon")?;
    let title = shown
        .iter()
        .find(|(text, _)| text == "Midnight Explorer")
        .map(|(_, rect)| *rect)
        .ok_or("the sky shows no title")?;
    if overlaps(planet, icon) || overlaps(planet, title) {
        return Err(format!(
            "Saturn at {planet:?} overlaps the icon at {icon:?} or the title at {title:?}"
        ));
    }
    let moons: Vec<String> = inspect::labels(app.slint(), AccessibleRole::Image)
        .into_iter()
        .filter(|label| {
            let label = label.to_lowercase();
            label.contains("moon") || label.contains("crescent")
        })
        .collect();
    if !moons.is_empty() {
        return Err(format!("the sky still shows {moons:?}"));
    }
    // Drawn: warm, bright pixels (the planet's gold, the rings) inside its square, which
    // the dark blue sky around it does not have.
    let (width, height, pixels) = crate::app::render(&app.main);
    let (x0, y0) = (planet.0.max(0.0) as usize, planet.1.max(0.0) as usize);
    let (x1, y1) = (
        ((planet.0 + planet.2) as usize).min(width),
        ((planet.1 + planet.3) as usize).min(height),
    );
    let warm = (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|(x, y)| {
            let pixel = pixels[y * width + x];
            pixel.r >= 140 && pixel.r as i32 - pixel.b as i32 >= 50
        })
        .count();
    // The sky around is dark blue (no warm pixel); a few tiny warm stars may fall inside.
    if warm < 20 {
        return Err(format!(
            "Saturn's square at {planet:?} holds {warm} warm pixels: it is not drawn"
        ));
    }
    Ok(())
}

fn click_point(app: &mut App, x: f32, y: f32) {
    input::click_at(app.slint(), LogicalPosition::new(x, y));
    app.settle();
}

fn click_rect(app: &mut App, rect: Rect) {
    click_point(app, rect.0 + rect.2 / 3.0, rect.1 + rect.3 / 2.0);
}

/// Wait out the double-click interval before clicking near the same spot again.
fn pause_for_click(app: &mut App) {
    app.idle(Duration::from_millis(650));
}

fn open_about(app: &mut App) -> Result<(), String> {
    panel::wait_input(app);
    input::click_button(app.slint(), "About Midnight Explorer")?;
    wait(app, "About to open", about_open)?;
    app.idle(Duration::from_millis(150));
    Ok(())
}

fn show_tab(app: &mut App, name: &str) -> Result<(), String> {
    input::click(app.slint(), AccessibleRole::Tab, name)?;
    app.settle();
    let wanted = name.to_owned();
    wait(app, &format!("the {name} tab"), move |app| {
        inspect::find(app.slint(), AccessibleRole::Tab, &wanted)
            .is_some_and(|tab| property(&tab, AccessibleStringProperty::ItemSelected) == "true")
    })
}

fn filter(app: &mut App, text: &str) -> Result<(), String> {
    input::fill(app.slint(), "Filter libraries", text)?;
    app.idle(Duration::from_millis(80));
    Ok(())
}

/// Open the detail of `label` by `how` and check what it shows; returns to the list.
fn check_detail(app: &mut App, label: &str, license: &str, how: &str) -> Result<(), String> {
    wait(app, &format!("{how} to open {label:?}"), detail_open)?;
    wait(app, &format!("the notices of {label:?}"), |app| {
        let view = app.window.global::<AboutView>();
        !view.get_detail_loading() && !view.get_detail_authors_heading().is_empty()
    })?;
    let shown = texts(app);
    if !shown.iter().any(|(text, _)| text == label) {
        return Err(format!(
            "{how}: the detail of {label:?} has no such title; it shows {:?}",
            shown.iter().map(|(text, _)| text).collect::<Vec<_>>()
        ));
    }
    let declared = row_value(&shown, "License");
    if declared.as_deref() != Some(license) {
        return Err(format!(
            "{how}: the detail of {label:?} shows License {declared:?}, the list said {license:?}"
        ));
    }
    if row_value(&shown, "Used by").is_none_or(|used| used.trim().is_empty()) {
        return Err(format!("{how}: the detail of {label:?} names no component"));
    }
    if !shown.iter().any(|(text, _)| text.starts_with("Authors")) {
        return Err(format!(
            "{how}: the detail of {label:?} has no authors heading"
        ));
    }
    if !shown.iter().any(|(text, _)| text.len() > 60) {
        return Err(format!(
            "{how}: the detail of {label:?} shows no license text"
        ));
    }
    if inspect::find(app.slint(), AccessibleRole::Tab, "Licenses").is_some() {
        return Err(format!(
            "{how}: the tabs stay visible over the detail of {label:?}"
        ));
    }
    Ok(())
}

pub fn about(context: &mut Context) -> Result<(), String> {
    let root = context.fixture.root.clone();
    let mut app = App::start(Config::new(&root, &root));
    let package = env!("CARGO_PKG_VERSION");
    let version = package.strip_suffix(".0").unwrap_or(package).to_owned();
    app.window.set_application_version(version.as_str().into());
    let mut failures: Vec<String> = Vec::new();
    if let Err(error) = checks(&mut app, context, &version, &mut failures) {
        failures.push(format!("stopped: {error}"));
    }
    app.shutdown()?;
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

fn checks(
    app: &mut App,
    context: &mut Context,
    version: &str,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    // The About tab: the version in the title strip and below, with room between lines.
    open_about(app)?;
    let shown = texts(app);
    let pill = format!("Version {version}");
    for wanted in [
        "Midnight Explorer",
        pill.as_str(),
        "A focused, two-pane file manager.",
        "Rust, Slint and FemtoVG",
    ] {
        if !shown.iter().any(|(text, _)| text == wanted) {
            failures.push(format!("the About tab does not show {wanted:?}"));
        }
    }
    if row_value(&shown, "Version").as_deref() != Some(version) {
        failures.push(format!(
            "the About tab's Version row shows {:?}, not {version:?}",
            row_value(&shown, "Version")
        ));
    }
    // The Copyright row, and no Author row.
    if let Err(error) = copyright(&shown) {
        failures.push(error);
    }
    match spacing(
        &shown,
        &[
            "A focused, two-pane file manager.",
            "Built for keyboard and mouse alike: fast panels, tabs, previews, archives as folders and a command line within reach.",
            "Version",
            COPYRIGHT_LABEL,
            COPYRIGHT,
            "License",
            LICENSE_VALUE,
            "Built with",
            "Libraries",
        ],
        6.0,
    ) {
        Ok(gap) => context.note(format!(
            "About tab: the tightest gap between its lines is {gap:.0}px"
        )),
        Err(error) => failures.push(format!("the About tab is cramped: {error}")),
    }
    if let Err(error) = saturn(app, &shown) {
        failures.push(error);
    }
    app.screenshot(context, "about-tab", "About, the About tab");
    if let Err(error) = own_license(app, &shown) {
        failures.push(error);
    }

    // Left and Right move between the tabs.
    show_tab(app, "About")?;
    input::key(app.slint(), Key::RightArrow);
    if wait(app, "Right to show the Licenses tab", |app| {
        inspect::find(app.slint(), AccessibleRole::TextInput, "Filter libraries").is_some()
    })
    .is_err()
    {
        failures.push("Right on the About tab does not show the Licenses tab".into());
    }
    input::key(app.slint(), Key::LeftArrow);
    if wait(app, "Left to show the About tab", |app| {
        inspect::find(app.slint(), AccessibleRole::TextInput, "Filter libraries").is_none()
            && inspect::visible_texts(app.slint())
                .iter()
                .any(|text| text == "Built with")
    })
    .is_err()
    {
        failures.push("Left on the Licenses tab does not show the About tab".into());
    }

    // The Licenses tab: the count, every row a library with its license.
    show_tab(app, "Licenses")?;
    let all = summary(app).and_then(|text| counts(&text)).ok_or_else(|| {
        format!(
            "the Licenses tab shows no count: {:?}",
            inspect::visible_texts(app.slint())
        )
    })?;
    if all.0 != all.1 || all.1 < 50 {
        failures.push(format!("the unfiltered list counts {all:?} libraries"));
    }
    let model = app.window.global::<AboutView>().get_rows().row_count() as u64;
    if model != all.1 {
        failures.push(format!(
            "the list says {} libraries and components but holds {model}",
            all.1
        ));
    }
    let rows = libraries(app);
    if rows.len() < 5 {
        return Err(format!(
            "the Licenses tab shows {} library rows",
            rows.len()
        ));
    }
    if rows
        .iter()
        .any(|row| row.license.is_empty() || row.label.is_empty())
    {
        failures.push("a library row has no name or license".into());
    }
    if tools::has(app.slint(), AccessibleRole::Button, "View license")
        && inspect::find(app.slint(), AccessibleRole::Button, "View license")
            .is_some_and(|button| tools::enabled(&button))
    {
        failures.push("View license is enabled before a library is selected".into());
    }

    // The rows look and act selectable: the pointer cursor, hover marks one row, a click
    // selects it.
    let first = rows[0].clone();
    let second = rows[1].clone();
    if !cursors(&first.item)
        .iter()
        .any(|cursor| cursor.contains("Pointer"))
    {
        failures.push(format!(
            "a library row sets no pointer cursor ({:?})",
            cursors(&first.item)
        ));
    }
    if rows.iter().any(|row| mark_shown(&row.item)) {
        failures.push("an open mark shows before any row is hovered or selected".into());
    }
    let plain = {
        let frame = crate::options_support::Frame::of(app);
        frame.digest(second.rect)
    };
    input::move_to(
        app.slint(),
        LogicalPosition::new(
            second.rect.0 + second.rect.2 / 3.0,
            second.rect.1 + second.rect.3 / 2.0,
        ),
    );
    app.settle();
    let hovered = libraries(app);
    let marked: Vec<&String> = hovered
        .iter()
        .filter(|row| mark_shown(&row.item))
        .map(|row| &row.label)
        .collect();
    if marked != vec![&second.label] {
        failures.push(format!(
            "hovering {:?} shows the open mark on {marked:?}",
            second.label
        ));
    }
    let lit = crate::options_support::Frame::of(app).digest(second.rect);
    if lit == plain {
        failures.push(format!(
            "hovering {:?} does not change its row",
            second.label
        ));
    }
    app.screenshot(
        context,
        "licenses-hover",
        "Licenses: a hovered row with its open mark",
    );
    click_rect(app, first.rect);
    if selected(app).as_deref() != Some(first.label.as_str()) {
        failures.push(format!(
            "clicking {:?} selects {:?}",
            first.label,
            selected(app)
        ));
    }
    // Away from the list, the selected row keeps its mark; others show none.
    input::move_to(app.slint(), LogicalPosition::new(4.0, 4.0));
    app.settle();
    let marked: Vec<String> = libraries(app)
        .into_iter()
        .filter(|row| mark_shown(&row.item))
        .map(|row| row.label)
        .collect();
    if marked != vec![first.label.clone()] {
        failures.push(format!(
            "with {:?} selected the open mark shows on {marked:?}",
            first.label
        ));
    }
    if !inspect::find(app.slint(), AccessibleRole::Button, "View license")
        .is_some_and(|button| tools::enabled(&button))
    {
        failures.push("View license stays disabled with a library selected".into());
    }
    app.screenshot(
        context,
        "licenses-selected",
        "Licenses: the first library selected",
    );

    // View license, then Back: the list returns with the row still selected.
    input::click_button(app.slint(), "View license")?;
    if let Err(error) = check_detail(app, &first.label, &first.license, "View license") {
        failures.push(error);
    }
    let shown = texts(app);
    if let Err(error) = spacing(&shown, &[first.label.as_str(), "License", "Used by"], 3.0) {
        failures.push(format!("the library detail is cramped: {error}"));
    }
    app.screenshot(
        context,
        "licenses-detail",
        "Licenses: a library's license and authors",
    );
    input::click_button(app.slint(), "Back")?;
    wait(app, "Back to the list", |app| !detail_open(app))?;
    if selected(app).as_deref() != Some(first.label.as_str()) {
        failures.push(format!(
            "after Back the selected library is {:?}, not {:?}",
            selected(app),
            first.label
        ));
    }

    // The keyboard: Down, Page Down, End, Home and Up move the selection.
    let start = libraries(app);
    click_rect(app, start[0].rect);
    let order = |app: &App| -> Vec<String> {
        let model = app.window.global::<AboutView>().get_rows();
        model
            .iter()
            .map(|row| {
                if row.version.is_empty() {
                    row.name.to_string()
                } else {
                    format!("{} {}", row.name, row.version)
                }
            })
            .collect()
    };
    let names = order(app);
    // The selection is AboutView's row; whether a user sees it is checked separately.
    let position = |app: &App| {
        usize::try_from(app.window.global::<AboutView>().get_current())
            .ok()
            .filter(|index| *index < names.len())
    };
    for (key, check) in [
        (Key::DownArrow, "Down"),
        (Key::PageDown, "Page Down"),
        (Key::End, "End"),
        (Key::UpArrow, "Up"),
        (Key::Home, "Home"),
    ] {
        let before = position(app);
        input::key(app.slint(), key);
        // The list creates the rows of its new viewport while it is drawn.
        for _ in 0..3 {
            app.idle(Duration::from_millis(40));
        }
        let after = position(app);
        if let Some(index) = after
            && let Some(error) = in_view(app, &names[index])
        {
            failures.push(format!(
                "finding: after {check} in the library list {error}"
            ));
        }
        let ok = match (check, before, after) {
            ("Down", Some(b), Some(a)) => a == b + 1,
            ("Page Down", Some(b), Some(a)) => a > b + 3,
            ("End", _, Some(a)) => a + 1 == names.len(),
            ("Up", Some(b), Some(a)) => a + 1 == b,
            ("Home", _, Some(a)) => a == 0,
            _ => false,
        };
        if !ok {
            failures.push(format!(
                "{check} in the library list moved the selection from {before:?} to {after:?} of {}",
                names.len()
            ));
        }
    }
    // Enter opens the selected library; Escape returns to the list, not out of About.
    input::key(app.slint(), Key::DownArrow);
    app.idle(Duration::from_millis(80));
    if let Some(row) = libraries(app).into_iter().find(|row| row.selected) {
        input::key(app.slint(), Key::Return);
        if let Err(error) = check_detail(app, &row.label, &row.license, "Enter") {
            failures.push(error);
        }
        input::key(app.slint(), Key::Escape);
        if wait(app, "Escape to return to the list", |app| {
            !detail_open(app) && about_open(app)
        })
        .is_err()
        {
            failures.push("Escape in a library's detail does not return to the list".into());
        }
    } else {
        failures.push("no library is selected after Down".into());
    }

    // A double-click opens a row; so does a click on its open mark.
    let rows = libraries(app);
    let third = rows
        .get(2)
        .cloned()
        .ok_or("fewer than three library rows")?;
    pause_for_click(app);
    input::double_click_at(
        app.slint(),
        LogicalPosition::new(
            third.rect.0 + third.rect.2 / 3.0,
            third.rect.1 + third.rect.3 / 2.0,
        ),
    );
    if let Err(error) = check_detail(app, &third.label, &third.license, "a double-click") {
        failures.push(error);
    }
    input::click_button(app.slint(), "Back")?;
    wait(app, "Back to the list", |app| !detail_open(app))?;
    let fourth = libraries(app)
        .get(3)
        .cloned()
        .ok_or("fewer than four library rows")?;
    // Hover shows the row's "›" mark; a click on the mark opens the library.
    pause_for_click(app);
    input::move_to(
        app.slint(),
        LogicalPosition::new(
            fourth.rect.0 + fourth.rect.2 / 3.0,
            fourth.rect.1 + fourth.rect.3 / 2.0,
        ),
    );
    app.settle();
    match mark_rect(&fourth.item) {
        Some(mark) => {
            click_point(app, mark.0 + mark.2 / 2.0, mark.1 + mark.3 / 2.0);
            app.idle(Duration::from_millis(300));
            if !detail_open(app) {
                // Where the click went: the list's overlay scrollbar covers the last
                // 14 px of every row. Does the mark's left edge still open it?
                let row_end = fourth.rect.0 + fourth.rect.2;
                pause_for_click(app);
                click_point(app, mark.0 + 1.0, mark.1 + mark.3 / 2.0);
                app.idle(Duration::from_millis(300));
                let edge = detail_open(app);
                failures.push(format!(
                    "finding: a click in the middle of {:?}'s \"›\" mark does not open it; the mark spans x {:.0}–{:.0} of a row ending at {row_end:.0}, under the list's 14 px overlay scrollbar{}",
                    fourth.label,
                    mark.0,
                    mark.0 + mark.2,
                    if edge {
                        "; a click on its left edge does open it"
                    } else {
                        "; its left edge does not open it either"
                    }
                ));
            } else if let Err(error) = check_detail(
                app,
                &fourth.label,
                &fourth.license,
                "a click on the open mark",
            ) {
                failures.push(error);
            }
        }
        None => failures.push(format!(
            "hovering {:?} shows no open mark to click",
            fourth.label
        )),
    }
    if detail_open(app) {
        input::key(app.slint(), Key::Escape);
        wait(app, "Escape to return to the list", |app| !detail_open(app))?;
    }

    // The filter: K of N, every row matching, no match, Down into the list, Enter.
    filter(app, "slint")?;
    let narrowed = summary(app).and_then(|text| counts(&text));
    let rows = libraries(app);
    match narrowed {
        Some((count, total)) if count > 0 && count < total && total == all.1 => {}
        other => failures.push(format!("filtering by \"slint\" counts {other:?}")),
    }
    if let Some(row) = rows.iter().find(|row| {
        !row.label.to_lowercase().contains("slint") && !row.license.to_lowercase().contains("slint")
    }) {
        failures.push(format!(
            "filtering by \"slint\" shows {:?} ({})",
            row.label, row.license
        ));
    }
    if selected(app).is_some() {
        failures.push("a changed filter keeps a selection".into());
    }
    input::key(app.slint(), Key::DownArrow);
    app.idle(Duration::from_millis(80));
    let first_match = libraries(app).first().cloned();
    if selected(app) != first_match.as_ref().map(|row| row.label.clone()) {
        failures.push(format!(
            "Down in the filter selects {:?}, not the first match",
            selected(app)
        ));
    }
    input::key(app.slint(), Key::DownArrow);
    app.idle(Duration::from_millis(80));
    if selected(app) != libraries(app).get(1).map(|row| row.label.clone()) {
        failures.push("after Down into the list, Down does not move to the second match".into());
    }
    filter(app, "zz-no-such-library")?;
    if summary(app).and_then(|text| counts(&text)) != Some((0, all.1)) {
        failures.push(format!(
            "a filter matching nothing shows {:?}",
            summary(app)
        ));
    }
    if !libraries(app).is_empty()
        || inspect::find(app.slint(), AccessibleRole::Button, "View license")
            .is_some_and(|button| tools::enabled(&button))
    {
        failures.push("a filter matching nothing still lists rows or enables View license".into());
    }
    // Enter in the filter opens the first match when none is selected.
    filter(app, "slint")?;
    let first_match = libraries(app)
        .first()
        .cloned()
        .ok_or("no match for slint")?;
    input::key(app.slint(), Key::Return);
    if let Err(error) = check_detail(
        app,
        &first_match.label,
        &first_match.license,
        "Enter in the filter",
    ) {
        failures.push(error);
    }
    input::click_button(app.slint(), "Back")?;
    wait(app, "Back to the list", |app| !detail_open(app))?;
    if inspect::find(app.slint(), AccessibleRole::TextInput, "Filter libraries")
        .map(|field| property(&field, AccessibleStringProperty::Value))
        .as_deref()
        != Some("slint")
    {
        failures.push("Back does not keep the filter".into());
    }

    // Escape in the list closes About; reopening keeps the filter and the selection.
    let before_close = selected(app);
    input::key(app.slint(), Key::Escape);
    if wait(app, "Escape to close About", |app| !about_open(app)).is_err() {
        failures.push("Escape in the library list does not close About".into());
        input::click_button(app.slint(), "Close")?;
    }
    open_about(app)?;
    show_tab(app, "Licenses")?;
    let field = inspect::find(app.slint(), AccessibleRole::TextInput, "Filter libraries")
        .map(|field| property(&field, AccessibleStringProperty::Value));
    if field.as_deref() != Some("slint") || selected(app) != before_close {
        failures.push(format!(
            "reopened About shows the filter {field:?} and selection {:?}, not \"slint\" and {before_close:?}",
            selected(app)
        ));
    }
    filter(app, "")?;
    if summary(app).and_then(|text| counts(&text)) != Some(all) {
        failures.push(format!("clearing the filter shows {:?}", summary(app)));
    }

    // Dragging the sky moves the dialog; a click outside it closes About.
    let close = inspect::find(app.slint(), AccessibleRole::Button, "Close")
        .map(|button| inspect::rect(&button))
        .ok_or("About has no Close button")?;
    let sky = texts(app)
        .into_iter()
        .find(|(text, _)| text == "Midnight Explorer")
        .map(|(_, rect)| rect)
        .ok_or("the sky shows no title")?;
    let from = LogicalPosition::new(sky.0 - 40.0, sky.1 + sky.3 / 2.0);
    input::drag(
        app.slint(),
        from,
        LogicalPosition::new(from.x + 40.0, from.y + 30.0),
        8,
    );
    app.settle();
    let moved = inspect::find(app.slint(), AccessibleRole::Button, "Close")
        .map(|button| inspect::rect(&button))
        .ok_or("About lost its Close button")?;
    if (moved.0 - close.0 - 40.0).abs() > 2.0 || (moved.1 - close.1 - 30.0).abs() > 2.0 {
        failures.push(format!(
            "dragging the sky by (40, 30) moved the dialog by ({:.0}, {:.0})",
            moved.0 - close.0,
            moved.1 - close.1
        ));
    }
    pause_for_click(app);
    let bottom = app.height as f32 - 6.0;
    click_point(app, 6.0, bottom);
    if wait(app, "a click outside to close About", |app| {
        !about_open(app)
    })
    .is_err()
    {
        failures.push("a click outside the About box does not close it".into());
        input::click_button(app.slint(), "Close")?;
    }

    // Close closes it too, and the panels take input again.
    open_about(app)?;
    input::click_button(app.slint(), "Close")?;
    wait(app, "Close to close About", |app| !about_open(app))?;
    panel::focus_list(app, PaneId::Left)?;
    context.note(format!(
        "About: version {version}; {} libraries and components; every check ran",
        all.1
    ));
    Ok(())
}

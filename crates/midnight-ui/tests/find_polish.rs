//! The UI polish gate's Find finding of 20 November 2026 (U-D16): the result columns'
//! sort headings are Tab stops that show where the focus is and answer hovering;
//! Return or Space sorts, a click sorts without taking the keyboard from the results,
//! and a heading that takes the focus scrolls into view.
use crate::support::modeless;
#[path = "support/window_input.rs"]
mod input;

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::AccessibleRole,
    window::WindowInner,
};
use midnight_ui::{FindRow, FindWindow};
use slint::{
    ComponentHandle, Rgb8Pixel, VecModel,
    platform::{Key, WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{cell::RefCell, rc::Rc};

const TITLES: [&str; 5] = ["Name", "Extension", "Path", "Size", "Modified"];

/// Window rectangle: left, top, width, height.
type Rect = (f32, f32, f32, f32);

struct Find {
    window: FindWindow,
    native: Rc<MinimalSoftwareWindow>,
    sorts: Rc<RefCell<Vec<i32>>>,
    size: (u32, u32),
}

impl Find {
    /// A Find window with three results, its query focused as when it opens.
    fn open(width: u32, height: u32) -> Self {
        let windows = Rc::new(RefCell::new(Vec::new()));
        slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
        let window = FindWindow::new().unwrap();
        let rows: Vec<FindRow> = (0..3)
            .map(|index| FindRow {
                token: format!("1:{index}").into(),
                name: format!("report {index} 文档.txt").into(),
                extension: "txt".into(),
                path: "/home/user/projects/midnight".into(),
                path_tail: "/midnight".into(),
                size: "10 B".into(),
                modified: "2026-11-19 08:30".into(),
                ..Default::default()
            })
            .collect();
        window.set_rows(Rc::new(VecModel::from(rows)).into());
        let sorts = Rc::new(RefCell::new(Vec::new()));
        window.on_sort({
            let sorts = sorts.clone();
            move |column| sorts.borrow_mut().push(column)
        });
        window.show().unwrap();
        let native = windows.borrow()[0].clone();
        modeless::layout(&native, width, height);
        window.invoke_begin();
        Self {
            window,
            native,
            sorts,
            size: (width, height),
        }
    }

    fn heading(&self, title: &str) -> ItemRc {
        modeless::accessible_item(
            &self.native,
            AccessibleRole::Button,
            &format!("Sort by {title}"),
        )
        .unwrap_or_else(|| panic!("Find shows no Sort by {title}"))
    }

    fn list(&self) -> ItemRc {
        modeless::accessible_item(&self.native, AccessibleRole::List, "Find results")
            .expect("the result list")
    }

    fn render(&self) -> Vec<Rgb8Pixel> {
        slint::platform::update_timers_and_animations();
        modeless::render(&self.native, self.size.0, self.size.1, "find-polish")
    }

    fn tab(&self, backwards: bool) {
        if backwards {
            input::chord(&self.native, &[Key::Shift], Key::Tab);
        } else {
            input::key(&self.native, Key::Tab);
        }
        slint::platform::update_timers_and_animations();
    }

    fn focus(&self) -> ItemRc {
        WindowInner::from_pub(self.native.window())
            .focus_item
            .borrow()
            .upgrade()
            .expect("the window keeps a keyboard focus")
    }

    /// The Tab stops from the opening focus until Tab returns to it.
    fn tab_cycle(&self) -> Vec<ItemRc> {
        let first = self.focus();
        let mut cycle = vec![first.clone()];
        for _ in 0..80 {
            self.tab(false);
            let item = self.focus();
            if item == first {
                return cycle;
            }
            assert!(
                !cycle.contains(&item),
                "Tab revisits {} before the cycle closes",
                describe(&item)
            );
            cycle.push(item);
        }
        panic!("Tab never returns to the opening focus: {}", cycle.len());
    }
}

fn label(item: &ItemRc) -> String {
    item.accessible_string_property(AccessibleStringProperty::Label)
        .map(|label| label.to_string())
        .unwrap_or_default()
}

/// The control a focus stop belongs to, as the gate finds it: the item or its nearest
/// ancestor with an interactive role.
fn control(item: &ItemRc) -> Option<ItemRc> {
    let mut current = Some(item.clone());
    while let Some(node) = current {
        if matches!(
            node.accessible_role(),
            AccessibleRole::Button
                | AccessibleRole::Checkbox
                | AccessibleRole::Combobox
                | AccessibleRole::Slider
                | AccessibleRole::Spinbox
                | AccessibleRole::Tab
                | AccessibleRole::TextInput
                | AccessibleRole::Switch
                | AccessibleRole::RadioButton
        ) {
            return Some(node);
        }
        current = node.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    None
}

fn describe(item: &ItemRc) -> String {
    control(item).map_or_else(
        || format!("an unnamed item at {:?}", bounds(item)),
        |control| format!("{:?} {:?}", control.accessible_role(), label(&control)),
    )
}

/// The sort heading a focus stop belongs to, if any.
fn heading_of(item: &ItemRc) -> Option<String> {
    let control = control(item)?;
    if control.accessible_role() != AccessibleRole::Button {
        return None;
    }
    label(&control).strip_prefix("Sort by ").map(str::to_owned)
}

fn bounds(item: &ItemRc) -> Rect {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    (origin.x, origin.y, rect.width(), rect.height())
}

fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let (left, top) = (a.0.max(b.0), a.1.max(b.1));
    let (right, bottom) = ((a.0 + a.2).min(b.0 + b.2), (a.1 + a.3).min(b.1 + b.3));
    (right > left && bottom > top).then_some((left, top, right - left, bottom - top))
}

fn inflate((x, y, w, h): Rect, by: f32) -> Rect {
    (x - by, y - by, w + 2.0 * by, h + 2.0 * by)
}

/// The pixels of `area`, clipped to the window.
fn region(pixels: &[Rgb8Pixel], size: (u32, u32), (x, y, w, h): Rect) -> Vec<Rgb8Pixel> {
    let (width, height) = (size.0 as usize, size.1 as usize);
    let left = x.floor().max(0.0) as usize;
    let top = y.floor().max(0.0) as usize;
    let right = ((x + w).ceil().max(0.0) as usize).min(width);
    let bottom = ((y + h).ceil().max(0.0) as usize).min(height);
    (top..bottom)
        .flat_map(|row| (left..right).map(move |column| row * width + column))
        .map(|index| pixels[index])
        .collect()
}

fn luminance(pixel: Rgb8Pixel) -> f64 {
    let linear = |channel: u8| {
        let channel = f64::from(channel) / 255.0;
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(pixel.r) + 0.7152 * linear(pixel.g) + 0.0722 * linear(pixel.b)
}

fn contrast(first: Rgb8Pixel, second: Rgb8Pixel) -> f64 {
    let (first, second) = (luminance(first), luminance(second));
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

/// Pixel bounds of `area` inside the window: left, top, right, bottom.
fn pixel_bounds(size: (u32, u32), (x, y, w, h): Rect) -> (usize, usize, usize, usize) {
    (
        x.floor().max(0.0) as usize,
        y.floor().max(0.0) as usize,
        ((x + w).ceil().max(0.0) as usize).min(size.0 as usize),
        ((y + h).ceil().max(0.0) as usize).min(size.1 as usize),
    )
}

/// The most common colour of a thin ring just outside `area`.
fn ring_colour(pixels: &[Rgb8Pixel], size: (u32, u32), area: Rect) -> Option<Rgb8Pixel> {
    let (left, top, right, bottom) = pixel_bounds(size, inflate(area, 3.0));
    let inner = pixel_bounds(size, area);
    let mut counts = std::collections::HashMap::<(u8, u8, u8), usize>::new();
    for row in top..bottom {
        for column in left..right {
            if column >= inner.0 && column < inner.2 && row >= inner.1 && row < inner.3 {
                continue;
            }
            let pixel = pixels[row * size.0 as usize + column];
            *counts.entry((pixel.r, pixel.g, pixel.b)).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .max_by_key(|(colour, count)| (*count, *colour))
        .map(|((r, g, b), _)| Rgb8Pixel { r, g, b })
}

/// How strongly `area` changes from `before` to `after` (0 when nothing changed), as the
/// gate measures a focus indicator: each changed pixel against its old colour and
/// against the colour around the area (WCAG 1.4.11 asks for 3:1).
fn strongest_change(
    before: &[Rgb8Pixel],
    after: &[Rgb8Pixel],
    size: (u32, u32),
    area: Rect,
) -> f64 {
    let ring = ring_colour(after, size, area);
    region(before, size, area)
        .into_iter()
        .zip(region(after, size, area))
        .filter(|(before, after)| before != after)
        .map(|(before, after)| {
            contrast(before, after).max(ring.map_or(0.0, |ring| contrast(after, ring)))
        })
        .fold(0.0, f64::max)
}

/// The part of `item` the result list's viewport shows: the header row scrolls with it.
fn shown(find: &Find, item: &ItemRc) -> Option<Rect> {
    let list = bounds(&find.list());
    // The header row sits right above the list and spans its width.
    let viewport = (list.0, 0.0, list.2, find.size.1 as f32);
    intersect(bounds(item), viewport)
}

#[test]
fn tab_and_shift_tab_reach_every_sort_heading_and_each_shows_its_focus() {
    let find = Find::open(860, 620);
    for (scheme, density) in [(0, 0), (1, 1)] {
        find.window
            .invoke_apply_preferences(scheme, density, "".into(), 12, 15);
        modeless::layout(&find.native, 860, 620);
        let what = format!("theme {scheme}, density {density}");
        let cycle = find.tab_cycle();
        let headings: Vec<String> = cycle.iter().filter_map(heading_of).collect();
        assert_eq!(
            headings,
            TITLES,
            "{what}: Tab reaches every sort heading, in column order: {:#?}",
            cycle.iter().map(describe).collect::<Vec<_>>()
        );
        // Shift+Tab visits the same stops backwards.
        for (step, expected) in cycle.iter().enumerate().rev() {
            find.tab(true);
            assert_eq!(
                &find.focus(),
                expected,
                "{what}: Shift+Tab to stop {step} ({})",
                describe(expected)
            );
        }
        // Every stop looks focused: something of at least 3:1 appears around it, as
        // the gate measures it (the headings scroll with the result list).
        let mut before = find.render();
        let mut faint = Vec::new();
        for _ in 0..cycle.len() {
            find.tab(false);
            let after = find.render();
            let focus = find.focus();
            let area = match heading_of(&focus) {
                Some(title) => shown(&find, &find.heading(&title))
                    .unwrap_or_else(|| panic!("{what}: focused Sort by {title} is out of view")),
                None => bounds(&control(&focus).unwrap_or_else(|| focus.clone())),
            };
            let change = strongest_change(&before, &after, find.size, inflate(area, 4.0));
            if change < 3.0 {
                faint.push(format!("{} ({change:.2}:1)", describe(&focus)));
            }
            before = after;
        }
        assert!(
            faint.is_empty(),
            "{what}: stops whose focus does not show: {faint:#?}"
        );
    }
    find.window.hide().unwrap();
}

#[test]
fn sort_headings_answer_hovering() {
    let find = Find::open(1100, 620);
    for (scheme, density) in [(0, 0), (1, 1), (2, 0)] {
        find.window
            .invoke_apply_preferences(scheme, density, "".into(), 12, 15);
        let what = format!("theme {scheme}, density {density}");
        for title in TITLES {
            find.native
                .window()
                .dispatch_event(WindowEvent::PointerExited);
            let before = find.render();
            let heading = find.heading(title);
            let area = shown(&find, &heading)
                .unwrap_or_else(|| panic!("{what}: Sort by {title} is out of view"));
            find.native
                .window()
                .dispatch_event(WindowEvent::PointerMoved {
                    position: slint::LogicalPosition::new(
                        area.0 + area.2 / 2.0,
                        area.1 + area.3 / 2.0,
                    ),
                });
            let after = find.render();
            let area = inflate(area, 2.0);
            assert!(
                region(&before, find.size, area) != region(&after, find.size, area),
                "{what}: Sort by {title} does not change when hovered"
            );
        }
    }
    find.window.hide().unwrap();
}

#[test]
fn return_and_space_sort_by_the_focused_heading_and_a_click_keeps_the_focus() {
    let find = Find::open(1100, 620);
    // A click on a heading sorts, and the keyboard stays with the results.
    let row = modeless::accessible_items(
        &find.native,
        AccessibleRole::ListItem,
        "report 0 文档.txt · txt · /home/user/projects/midnight · 10 B · 2026-11-19 08:30",
    )
    .pop()
    .expect("the first result row");
    input::click(&find.native, &row);
    drop(row);
    assert_eq!(find.window.get_selected_row(), 0);
    let results = find.focus();
    input::click(&find.native, &find.heading("Extension"));
    assert_eq!(*find.sorts.borrow(), [1], "a click sorts by Extension");
    assert_eq!(
        find.focus(),
        results,
        "a click leaves the focus in the results"
    );
    // Tab from the results reaches Name, Extension and Size.
    let mut reached = Vec::new();
    for _ in 0..8 {
        find.tab(false);
        let Some(title) = heading_of(&find.focus()) else {
            continue;
        };
        reached.push(title.clone());
        if title == "Size" {
            break;
        }
    }
    assert_eq!(reached, ["Name", "Extension", "Path", "Size"]);
    input::key(&find.native, Key::Return);
    input::key(&find.native, Key::Space);
    assert_eq!(
        *find.sorts.borrow(),
        [1, 3, 3],
        "Return and Space sort by Size"
    );
    // Other keys still move through the results.
    find.tab(true);
    find.tab(true);
    find.tab(true);
    find.tab(true);
    assert_eq!(find.focus(), results, "Shift+Tab returns to the results");
    input::key(&find.native, Key::DownArrow);
    assert_eq!(find.window.get_selected_row(), 1);
    find.window.hide().unwrap();
}

#[test]
fn a_focused_heading_outside_the_list_view_scrolls_into_it() {
    // The five columns need more than this window's width: Size and Modified start
    // out of view.
    let find = Find::open(640, 480);
    let list = bounds(&find.list());
    let inside = |find: &Find, title: &str| {
        let (x, _, w, _) = bounds(&find.heading(title));
        x >= list.0 - 0.5 && x + w <= list.0 + list.2 + 0.5
    };
    assert!(
        !inside(&find, "Modified"),
        "the test needs Modified out of view"
    );
    let mut seen = Vec::new();
    for _ in 0..40 {
        find.tab(false);
        modeless::layout(&find.native, 640, 480);
        if let Some(title) = heading_of(&find.focus()) {
            assert!(
                inside(&find, &title),
                "focused Sort by {title} is out of view"
            );
            seen.push(title);
            if seen.len() == TITLES.len() {
                break;
            }
        }
    }
    assert_eq!(seen, TITLES);
    // Shift+Tab back to Name scrolls the table back to its start.
    for _ in 0..4 {
        find.tab(true);
        modeless::layout(&find.native, 640, 480);
    }
    assert_eq!(heading_of(&find.focus()).as_deref(), Some("Name"));
    assert!(inside(&find, "Name"), "focused Sort by Name is out of view");
    find.window.hide().unwrap();
}

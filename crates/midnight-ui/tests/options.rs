//! The topic tabs and nested pickers remain usable through actual pointer input.
#[path = "support/popup_scroll.rs"]
mod popup_scroll;
use crate::support::ui;

use i_slint_core::{accessibility::AccessibleStringProperty, items::AccessibleRole};
use midnight_ui::{NavigationChoiceRow, Theme};
use slint::{ComponentHandle, ModelRc, VecModel, platform::Key};
use std::{cell::Cell, rc::Rc};
use ui::{TestUi, click, click_popup, key};

/// The Options pages in their listed order with their topic identities.
const PAGES: [(&str, i32); 10] = [
    ("General", 0),
    ("Appearance", 1),
    ("Colors", 8),
    ("Previews", 6),
    ("Archives", 7),
    ("Editors", 4),
    ("Comparators", 10),
    ("Custom Commands", 9),
    ("Keyboard", 3),
    ("Plugins", 5),
];

/// A control that only the page of this topic shows, with its role.
fn page_control(topic: i32) -> (&'static str, AccessibleRole) {
    match topic {
        0 => ("Show hidden files", AccessibleRole::Checkbox),
        1 => ("Theme", AccessibleRole::Combobox),
        3 => ("Search commands", AccessibleRole::TextInput),
        4 => ("Editor rules", AccessibleRole::List),
        5 => ("Rescan all plugins", AccessibleRole::Button),
        6 => ("Show image and video previews", AccessibleRole::Checkbox),
        7 => ("Search archive associations", AccessibleRole::TextInput),
        8 => ("Filename color rules", AccessibleRole::List),
        9 => ("Configured Custom Commands", AccessibleRole::List),
        10 => ("Comparator rules", AccessibleRole::List),
        _ => unreachable!("no Options topic {topic}"),
    }
}

#[test]
fn options_pages_list_in_reading_order_and_keep_their_topic_identities() {
    assert_eq!(PAGES.map(|(_, topic)| topic), ui::OPTIONS_TOPIC_ORDER);
    let mut ui = TestUi::new();
    ui.window.invoke_show_options();
    ui.draw();
    let tops = PAGES.map(|(title, _)| {
        let button = ui::popup_item(&ui.window, title, AccessibleRole::Button);
        button.map_to_native_window(button.geometry().origin).y
    });
    assert!(
        tops.windows(2).all(|pair| pair[0] < pair[1]),
        "the list shows the pages top to bottom in their order: {tops:?}"
    );
    // A click shows the page's own content under its own topic identity.
    for (title, topic) in PAGES {
        ui.options_topic(topic);
        let (control, role) = page_control(topic);
        assert!(
            ui::try_popup_item(&ui.window, control, role).is_some(),
            "{title} shows {control}"
        );
    }
    // Arrows walk the list in the same order and stop at its ends.
    ui.options_topic(0);
    for (title, topic) in PAGES.iter().skip(1).chain(PAGES.last()) {
        key(&ui.window, Key::DownArrow);
        ui.draw();
        let shown = ui.window.get_options_topic_index();
        assert_eq!(shown, *topic, "Down to {title}");
    }
    for (title, topic) in PAGES.iter().rev().skip(1).chain(PAGES.first()) {
        key(&ui.window, Key::UpArrow);
        ui.draw();
        let shown = ui.window.get_options_topic_index();
        assert_eq!(shown, *topic, "Up to {title}");
    }
    // A page opened by its identity (the Custom Commands shortcut, a page's validation
    // error, the page shown when Options closed) is the one marked in the list.
    for (title, topic) in PAGES {
        ui.window.set_options_topic_index(topic);
        ui.draw();
        let marked: Vec<&str> = PAGES
            .iter()
            .map(|(other, _)| *other)
            .filter(|other| {
                ui::popup_item(&ui.window, other, AccessibleRole::Button)
                    .accessible_string_property(AccessibleStringProperty::Checked)
                    .is_some_and(|checked| checked == "true")
            })
            .collect();
        assert_eq!(marked, [title], "topic {topic}");
    }
    // A narrow window's topic picker lists the pages in the same order.
    ui.resize(600, 700);
    for (title, topic) in PAGES {
        ui.options_topic(topic);
        let picker = ui::popup_item(&ui.window, "Options topic", AccessibleRole::Combobox);
        assert_eq!(
            picker
                .accessible_string_property(AccessibleStringProperty::Value)
                .as_deref(),
            Some(title)
        );
    }
}

#[test]
fn options_topics_keep_panel_preferences_and_appearance_readable() {
    let mut ui = TestUi::new();
    ui.window.set_preferences_ready(true);
    let accepted = Rc::new(Cell::new((-1, -1)));
    let changed = accepted.clone();
    let weak = ui.window.as_weak();
    ui.window.on_preferences_changed(move |theme, density, _| {
        changed.set((theme, density));
        weak.upgrade()
            .unwrap()
            .invoke_apply_preferences(theme, density);
    });
    let weak = ui.window.as_weak();
    ui.window.on_search_filter_changed(move |enabled| {
        weak.upgrade().unwrap().set_quick_search_filter(enabled);
    });
    let weak = ui.window.as_weak();
    ui.window.on_sizes_format_changed(move |enabled| {
        weak.upgrade().unwrap().set_sizes_in_bytes(enabled);
    });
    let weak = ui.window.as_weak();
    ui.window
        .on_remember_command_history_changed(move |enabled| {
            weak.upgrade()
                .unwrap()
                .set_remember_command_history(enabled);
        });
    ui.window
        .set_command_history(ModelRc::new(VecModel::from(vec![NavigationChoiceRow {
            identity: "history".into(),
            label: "echo text".into(),
            ..Default::default()
        }])));
    let cleared = Rc::new(Cell::new(0));
    let captured = cleared.clone();
    ui.window
        .on_command_history_clear(move || captured.set(captured.get() + 1));
    ui.window.invoke_show_options();
    ui.draw();
    assert_eq!(ui.window.get_options_topic_index(), 0);
    click(
        &ui.window,
        ui.window.get_options_left() + 35.0,
        ui.window.get_options_top() + 25.0,
    );
    click_popup(
        &ui.window,
        "Show only filename search matches",
        AccessibleRole::Checkbox,
    );
    assert!(
        ui.window.get_quick_search_filter(),
        "Panel search checkbox must work"
    );
    click_popup(
        &ui.window,
        "Always show file sizes in bytes",
        AccessibleRole::Checkbox,
    );
    assert!(
        ui.window.get_sizes_in_bytes(),
        "Byte-size checkbox must work"
    );
    ui.options_topic(1);
    click_popup(&ui.window, "Theme", AccessibleRole::Combobox);
    // Alphabetic display order, independent of the persisted Dark theme ID.
    for _ in 0..4 {
        key(&ui.window, Key::DownArrow);
    }
    key(&ui.window, Key::Return);
    ui.draw();
    assert_eq!(
        accepted.get(),
        (1, 0),
        "The nested theme picker selects Dark"
    );
    click_popup(&ui.window, "Spacing", AccessibleRole::Combobox);
    key(&ui.window, Key::DownArrow);
    key(&ui.window, Key::Return);
    ui.draw();
    assert_eq!(
        accepted.get(),
        (1, 1),
        "Touch applies without closing Options"
    );
    ui.options_topic(0);
    assert!(ui.window.get_quick_search_filter());
    assert!(ui.window.get_sizes_in_bytes());
    // Command history belongs to General; reveal its controls in the touch layout.
    popup_scroll::click(
        &mut ui,
        "Remember command history after restart",
        AccessibleRole::Checkbox,
    );
    assert!(!ui.window.get_remember_command_history());
    popup_scroll::click(&mut ui, "Clear command history", AccessibleRole::Button);
    assert_eq!(cleared.get(), 1);
    ui.options_topic(0);
    key(&ui.window, Key::RightArrow);
    key(&ui.window, Key::RightArrow);
    assert_eq!(
        ui.window.get_options_topic_index(),
        8,
        "repeated arrows reach Colors, the third page, without a Command page"
    );
    ui.draw();
    key(&ui.window, Key::LeftArrow);
    key(&ui.window, Key::LeftArrow);
    assert_eq!(
        ui.window.get_options_topic_index(),
        0,
        "repeated arrows return to General"
    );
    ui.draw();
    ui.options_topic(1);
    let theme = ui.window.global::<Theme>();
    let interior = (
        ui.window.get_options_left() + 5.0,
        ui.window.get_options_top() + 80.0,
    );
    let surface = theme.get_surface();
    let expected = slint::Rgb8Pixel {
        r: surface.red(),
        g: surface.green(),
        b: surface.blue(),
    };
    assert_eq!(ui.pixel(interior.0, interior.1), expected);
    click_popup(&ui.window, "Cancel", AccessibleRole::Button);
    ui.draw();
    assert!(
        !ui::has_popup(&ui.window),
        "Cancel closes Options after changing topics and density"
    );
    ui.window.hide().unwrap();
}

/// Tab through Options as a keyboard user does: the cycles behind U-D25 (Archives) and
/// U-D26 (Keyboard), and the same promise on every page. Each Tab keeps the focus inside
/// Options on something that was there before the walk, never on a hidden or disabled
/// control, scrolls it into view and shows it with a change of at least 3:1 (the
/// acceptance gate's measure). The cycle reaches every control a user can scroll to, in
/// visual order, wraps, runs backwards on Shift+Tab and changes nothing.
mod tab_walk {
    use super::*;
    use i_slint_core::{
        item_tree::{ItemRc, ParentItemTraversalMode},
        items::{Clip, Flickable, TextInput},
        window::{WindowInner, WindowKind},
    };
    use midnight_core::Event;
    use midnight_ui::{KeyboardEditorView, Presenter, install, refresh};
    use slint::{Rgb8Pixel, platform::WindowEvent};
    use std::{cell::RefCell, collections::HashMap, time::Duration};

    type Bounds = (f32, f32, f32, f32);

    /// The roles of the controls a keyboard user operates, as the acceptance gate counts them.
    fn interactive(role: AccessibleRole) -> bool {
        matches!(
            role,
            AccessibleRole::Button
                | AccessibleRole::Checkbox
                | AccessibleRole::Combobox
                | AccessibleRole::Slider
                | AccessibleRole::Spinbox
                | AccessibleRole::Tab
                | AccessibleRole::TextInput
                | AccessibleRole::Switch
                | AccessibleRole::RadioButton
        )
    }

    fn property(item: &ItemRc, what: AccessibleStringProperty) -> String {
        item.accessible_string_property(what)
            .unwrap_or_default()
            .to_string()
    }

    /// The stop's name: its label or, for a focusable area that is not a control (a list
    /// the arrows move in), the first label inside it.
    fn name(item: &ItemRc) -> String {
        let own = property(item, AccessibleStringProperty::Label);
        if !own.is_empty() {
            return own;
        }
        let mut child = item.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            let inner = name(&current);
            if !inner.is_empty() {
                return inner;
            }
        }
        String::new()
    }

    fn bounds(item: &ItemRc) -> Bounds {
        let rect = item.geometry();
        let origin = item.map_to_native_window(rect.origin);
        (origin.x, origin.y, rect.size.width, rect.size.height)
    }

    fn meet((ax, ay, aw, ah): Bounds, (bx, by, bw, bh): Bounds) -> bool {
        ax < bx + bw && bx < ax + aw && ay < by + bh && by < ay + ah
    }

    fn grow((x, y, width, height): Bounds, by: f32) -> Bounds {
        (x - by, y - by, width + 2.0 * by, height + 2.0 * by)
    }

    /// Why a keyboard user cannot use the item: hidden (`visible: false`, or clipped away
    /// where no scrolling brings it back) or disabled, by itself or by what holds it.
    /// Scrolled out of view does not count: Tab scrolls a control into view. Nor does
    /// transparency: a check box drawn by its own indicator keeps the native control
    /// transparent on top of it, and whether a stop shows is what the focus check measures.
    fn unusable(item: &ItemRc) -> Option<&'static str> {
        let (_, _, width, height) = bounds(item);
        if width < 2.0 || height < 2.0 {
            return Some("hidden");
        }
        let mut subject = bounds(item);
        let mut current = Some(item.clone());
        while let Some(at) = current {
            if at.downcast::<Clip>().is_some_and(|clip| {
                let clip = clip.as_pin_ref();
                clip.is_visibility_clip() && clip.clip()
            }) {
                return Some("hidden");
            }
            if property(&at, AccessibleStringProperty::Enabled) == "false"
                || at
                    .downcast::<TextInput>()
                    .is_some_and(|input| !input.as_pin_ref().enabled())
            {
                return Some("disabled");
            }
            if at != *item {
                if at.downcast::<Flickable>().is_some() {
                    // Scrolling brings the item into the Flickable's view; what counts
                    // further out is whether the Flickable itself shows.
                    subject = bounds(&at);
                } else if at.borrow().as_ref().clips_children() && !meet(bounds(&at), subject) {
                    return Some("clipped away");
                }
            }
            current = at.parent_item(ParentItemTraversalMode::StopAtPopups);
        }
        None
    }

    /// The nearest control: the item itself or its closest interactive ancestor, else the
    /// item.
    fn control(item: &ItemRc) -> ItemRc {
        let mut current = Some(item.clone());
        while let Some(at) = current {
            if interactive(at.accessible_role()) {
                return at;
            }
            current = at.parent_item(ParentItemTraversalMode::StopAtPopups);
        }
        item.clone()
    }

    fn all_items(item: &ItemRc, found: &mut Vec<ItemRc>) {
        found.push(item.clone());
        let mut child = item.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            all_items(&current, found);
        }
    }

    /// The controls a keyboard user can operate, not counting those inside another
    /// control: enabled and shown, or scrolled out of view.
    fn operable(items: &[ItemRc]) -> Vec<ItemRc> {
        items
            .iter()
            .filter(|item| {
                interactive(item.accessible_role())
                    && item
                        .parent_item(ParentItemTraversalMode::StopAtPopups)
                        .is_none_or(|parent| !interactive(control(&parent).accessible_role()))
                    && unusable(item).is_none()
            })
            .cloned()
            .collect()
    }

    /// What every control of the dialog says about itself: a walk must change none of it.
    fn states(root: &ItemRc) -> Vec<String> {
        let mut items = Vec::new();
        all_items(root, &mut items);
        items
            .iter()
            .filter(|item| interactive(item.accessible_role()))
            .map(|item| {
                format!(
                    "{:?} {:?} value {:?} checked {:?} enabled {:?}",
                    item.accessible_role(),
                    property(item, AccessibleStringProperty::Label),
                    property(item, AccessibleStringProperty::Value),
                    property(item, AccessibleStringProperty::Checked),
                    property(item, AccessibleStringProperty::Enabled),
                )
            })
            .collect()
    }

    /// The topmost dialog: Options.
    fn dialog(ui: &TestUi) -> ItemRc {
        let inner = WindowInner::from_pub(ui.window.window());
        let popups = inner.active_popups();
        let popup = popups
            .iter()
            .rev()
            .find(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
            .expect("Options is open");
        ItemRc::new_root(popup.component.clone())
    }

    /// The focused item and the root of the tree that holds it.
    fn focus(ui: &TestUi) -> Option<(ItemRc, ItemRc)> {
        let item = WindowInner::from_pub(ui.window.window())
            .focus_item
            .borrow()
            .upgrade()?;
        let mut root = item.clone();
        while let Some(parent) = root.parent_item(ParentItemTraversalMode::StopAtPopups) {
            root = parent;
        }
        Some((item, root))
    }

    /// The frame's pixel columns and rows that `area` covers: left, top, right, bottom.
    fn span(ui: &TestUi, (x, y, width, height): Bounds) -> (usize, usize, usize, usize) {
        let column = |at: f32| (at.max(0.0) as usize).min(ui.width);
        let row = |at: f32| (at.max(0.0) as usize).min(ui.height);
        (column(x), row(y), column(x + width), row(y + height))
    }

    fn luminance(pixel: Rgb8Pixel) -> f64 {
        let channel = |value: u8| {
            let value = f64::from(value) / 255.0;
            if value <= 0.040_45 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(pixel.r) + 0.7152 * channel(pixel.g) + 0.0722 * channel(pixel.b)
    }

    fn contrast(a: Rgb8Pixel, b: Rgb8Pixel) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// How strongly `area` changed since `before`, as the acceptance gate measures a focus
    /// indicator: the best contrast of a changed pixel with what it was, or with the
    /// colour around the area. `None` when nothing in the area changed.
    fn change(ui: &TestUi, before: &[Rgb8Pixel], area: Bounds) -> Option<f64> {
        let at = |frame: &[Rgb8Pixel], x: usize, y: usize| frame[y * ui.width + x];
        let (left, top, right, bottom) = span(ui, area);
        let (outer_left, outer_top, outer_right, outer_bottom) = span(ui, grow(area, 3.0));
        let mut around: HashMap<(u8, u8, u8), usize> = HashMap::new();
        for y in outer_top..outer_bottom {
            for x in outer_left..outer_right {
                if !(left..right).contains(&x) || !(top..bottom).contains(&y) {
                    let pixel = at(&ui.pixels, x, y);
                    *around.entry((pixel.r, pixel.g, pixel.b)).or_default() += 1;
                }
            }
        }
        let ring = around
            .into_iter()
            .max_by_key(|(colour, count)| (*count, *colour))
            .map(|((r, g, b), _)| Rgb8Pixel { r, g, b });
        let mut best: Option<f64> = None;
        for y in top..bottom {
            for x in left..right {
                let (old, new) = (at(before, x, y), at(&ui.pixels, x, y));
                if old != new {
                    let ratio =
                        contrast(old, new).max(ring.map_or(0.0, |ring| contrast(new, ring)));
                    best = Some(best.map_or(ratio, |best| best.max(ratio)));
                }
            }
        }
        best
    }

    /// Timers and animations run their course, and the frame shows the result.
    fn settle(ui: &mut TestUi, clock: &Cell<Duration>) {
        for _ in 0..3 {
            ui::advance(clock, Duration::from_millis(100));
            ui.draw();
        }
    }

    /// Options open over a presenter with default settings, on a virtual clock.
    fn options() -> (TestUi, Rc<Cell<Duration>>, Rc<RefCell<Presenter>>) {
        let (mut ui, clock) = TestUi::with_virtual_clock();
        let presenter = Rc::new(RefCell::new(Presenter::new("left".into(), "right".into())));
        install(&ui.window, presenter.clone(), Rc::new(|_| true));
        presenter
            .borrow_mut()
            .apply(Event::PreferencesLoaded(Ok(Box::default())));
        refresh(&ui.window, &presenter.borrow());
        ui.window.invoke_show_options();
        settle(&mut ui, &clock);
        (ui, clock, presenter)
    }

    /// What one walk found.
    struct Walk {
        /// The names of the stops in Tab order, from the focus the page opened with.
        stops: Vec<String>,
        /// Every broken promise, in words.
        issues: Vec<String>,
    }

    /// Open the page `topic` and walk its Tab cycle from the focus the page opened with,
    /// then back with Shift+Tab.
    fn walk(ui: &mut TestUi, clock: &Cell<Duration>, topic: i32) -> Walk {
        let (title, _) = PAGES.into_iter().find(|(_, id)| *id == topic).unwrap();
        ui.options_topic(topic);
        // No tooltip may show up during the walk.
        ui.window
            .window()
            .dispatch_event(WindowEvent::PointerExited);
        settle(ui, clock);
        let root = dialog(ui);
        let mut items = Vec::new();
        all_items(&root, &mut items);
        let wanted = operable(&items);
        // Where everything was before the walk scrolled anything, and whether it showed.
        let places: Vec<(Bounds, bool)> = items
            .iter()
            .map(|item| (bounds(item), item.is_visible()))
            .collect();
        let place = |item: &ItemRc| {
            items
                .iter()
                .position(|known| known == item)
                .map(|index| places[index])
        };
        let before_walk = states(&root);
        let mut issues = Vec::new();
        if focus(ui).is_none() {
            key(&ui.window, Key::Tab);
            settle(ui, clock);
        }
        let first = control(&focus(ui).expect("the page has the keyboard focus").0);
        let mut cycle = vec![first.clone()];
        let mut closed = false;
        for _ in 0..4 * wanted.len() + 40 {
            let before = ui.pixels.clone();
            key(&ui.window, Key::Tab);
            settle(ui, clock);
            let Some((item, top)) = focus(ui) else {
                issues.push(format!("{title}: Tab dropped the focus"));
                break;
            };
            let stop = control(&item);
            let what = format!("{title}: {:?} {:?}", stop.accessible_role(), name(&stop));
            if top != root {
                issues.push(format!("{what}: Tab moved the focus out of Options"));
                break;
            }
            if place(&stop).is_none() {
                issues.push(format!(
                    "{what}: Tab reached something that was not there before the walk"
                ));
            }
            if let Some(why) = unusable(&stop) {
                issues.push(format!("{what}: Tab stopped on a {why} control"));
            }
            if !item.is_visible() {
                issues.push(format!("{what}: Tab left the focus out of view"));
            }
            if stop != *cycle.last().unwrap() {
                let shown = change(ui, &before, grow(bounds(&stop), 4.0));
                if !shown.is_some_and(|ratio| ratio >= 3.0) {
                    issues.push(format!(
                        "{what}: the focus shows with a contrast of {shown:?}, 3:1 wanted"
                    ));
                }
            }
            if stop == first {
                closed = true;
                break;
            }
            cycle.push(stop);
        }
        let stops: Vec<String> = cycle.iter().map(name).collect();
        if !closed {
            issues.push(format!(
                "{title}: Tab never comes back to where it began: {stops:?}"
            ));
            return Walk { stops, issues };
        }
        // Every control is reached; one member of a group of tabs or radio buttons reaches
        // the group, inside which the arrows move.
        let missing: Vec<String> = wanted
            .iter()
            .filter(|item| !cycle.contains(item))
            .filter(|item| {
                !matches!(
                    item.accessible_role(),
                    AccessibleRole::Tab | AccessibleRole::RadioButton
                ) || !cycle.iter().any(|stop| {
                    stop.accessible_role() == item.accessible_role()
                        && stop.parent_item(ParentItemTraversalMode::StopAtPopups)
                            == item.parent_item(ParentItemTraversalMode::StopAtPopups)
                })
            })
            .map(name)
            .collect();
        if !missing.is_empty() {
            issues.push(format!(
                "{title}: Tab never reaches {missing:?}; it visits {stops:?}"
            ));
        }
        // Visual order, with the places from before the walk: the header's close, the page
        // list from the top, the page row by row (or column by column) and the footer from
        // the left, wrapping from the footer to the header.
        let button = |label: &str| {
            items.iter().find(|item| {
                item.accessible_role() == AccessibleRole::Button
                    && property(item, AccessibleStringProperty::Label) == label
            })
        };
        let at = |item: &ItemRc| place(item).unwrap();
        let list_top = at(button(PAGES[0].0).unwrap()).0.1;
        let list_right = PAGES
            .iter()
            .filter_map(|(page, _)| button(page))
            .map(|item| at(item).0)
            .fold(0.0_f32, |right, (x, _, width, _)| right.max(x + width));
        let footer_top = at(button("OK").unwrap()).0.1 - 2.0;
        let part = |item: &ItemRc| {
            let ((x, y, width, height), shown) = at(item);
            if shown && y >= footer_top {
                3
            } else if shown && y + height <= list_top + 1.0 {
                0
            } else if x + width <= list_right + 1.0 {
                1
            } else {
                2
            }
        };
        let mut known: Vec<ItemRc> = cycle
            .iter()
            .filter(|stop| place(stop).is_some())
            .cloned()
            .collect();
        known.dedup();
        let count = known.len();
        let parts: Vec<u8> = known.iter().map(part).collect();
        let order: Vec<String> = known
            .iter()
            .zip(&parts)
            .map(|(stop, part)| format!("{part} {}", name(stop)))
            .collect();
        let wraps = (0..count)
            .filter(|&index| parts[(index + 1) % count] < parts[index])
            .count();
        if wraps != 1 {
            issues.push(format!(
                "{title}: Tab goes back and forth between the header (0), the page list (1), \
                 the page (2) and the footer (3): {order:?}"
            ));
        }
        let against: Vec<String> = (0..count)
            .filter(|&index| parts[index] == parts[(index + 1) % count])
            .filter(|&index| {
                let (a, b) = (at(&known[index]).0, at(&known[(index + 1) % count]).0);
                if a.1 < b.1 + b.3 && b.1 < a.1 + a.3 {
                    // The same row: on to the right.
                    b.0 <= a.0
                } else {
                    // The next row, or the top of the next column to the right.
                    b.1 < a.1 + a.3 - 1.0 && b.0 < a.0 + a.2 - 1.0
                }
            })
            .map(|index| {
                format!(
                    "{} → {}",
                    name(&known[index]),
                    name(&known[(index + 1) % count])
                )
            })
            .collect();
        if !against.is_empty() {
            issues.push(format!(
                "{title}: Tab goes against the visual order at {against:?}: {order:?}"
            ));
        }
        // Shift+Tab from the first stop retraces the cycle.
        let expected: Vec<&ItemRc> = cycle.iter().skip(1).rev().chain([&first]).collect();
        let mut from = first.clone();
        for next in expected {
            ui::chord(&ui.window, &[Key::Shift], Key::Tab);
            ui::advance(clock, Duration::from_millis(50));
            let reached = focus(ui).map(|(item, _)| control(&item));
            if reached.as_ref() != Some(next) {
                issues.push(format!(
                    "{title}: Shift+Tab from {:?} goes to {:?} instead of {:?}",
                    name(&from),
                    reached.as_ref().map(name),
                    name(next)
                ));
                break;
            }
            from = next.clone();
        }
        // The walk changed nothing.
        settle(ui, clock);
        if ui.window.get_options_topic_index() != topic {
            issues.push(format!("{title}: the walk changed the page"));
        }
        let keyboard = ui.window.global::<KeyboardEditorView>();
        if !keyboard.get_candidate().is_empty() {
            issues.push(format!(
                "{title}: the walk recorded the shortcut {:?}",
                keyboard.get_candidate()
            ));
        }
        if keyboard.get_capturing() {
            issues.push(format!("{title}: the walk left a shortcut capture on"));
        }
        let after_walk = states(&dialog(ui));
        if after_walk != before_walk {
            let gone: Vec<&String> = before_walk
                .iter()
                .filter(|state| !after_walk.contains(state))
                .collect();
            let new: Vec<&String> = after_walk
                .iter()
                .filter(|state| !before_walk.contains(state))
                .collect();
            issues.push(format!(
                "{title}: the walk changed controls: {gone:?} became {new:?}"
            ));
        }
        Walk { stops, issues }
    }

    /// U-D25: on Archives, Tab reaches every association, then Reset to defaults, Export,
    /// Import, OK and Cancel, and wraps through the header's close to the page list.
    /// Shift+Tab retraces it, also into the search field scrolled out of view above the
    /// first file type.
    #[test]
    fn archives_tab_cycle_reaches_the_footer_and_wraps_in_visual_order() {
        let (mut ui, clock, _presenter) = options();
        let Walk { stops, issues } = walk(&mut ui, &clock, 7);
        assert!(issues.is_empty(), "{issues:#?}");
        let path = [
            "Archive format",
            "Search archive associations",
            ".zip · ZIP archive",
            "Add custom archive extension",
            "Reset to defaults",
            "Export configuration…",
            "Import configuration…",
            "OK",
            "Cancel",
            "Cancel dialog",
            "General",
        ];
        let positions: Vec<Option<usize>> = path
            .iter()
            .map(|wanted| stops.iter().position(|stop| stop == wanted))
            .collect();
        assert!(
            positions.iter().all(Option::is_some)
                && positions.windows(2).all(|pair| pair[0] < pair[1]),
            "Archives: Tab visits {stops:?}; {path:?} wanted in this order"
        );
    }

    /// U-D26: on Keyboard, the command list shows when it has the keyboard, and Tab and
    /// Shift+Tab pass the shortcut field without recording a shortcut (no error notice
    /// appears and nothing moves). Clicked, the field still records Tab like any key.
    #[test]
    fn keyboard_tab_cycle_shows_the_command_list_and_records_no_shortcut() {
        let (mut ui, clock, _presenter) = options();
        let Walk { stops, issues } = walk(&mut ui, &clock, 3);
        assert!(issues.is_empty(), "{issues:#?}");
        for wanted in [
            "Search commands",
            "Commands",
            "Press shortcut keys",
            "Restore all defaults",
        ] {
            assert!(
                stops.iter().any(|stop| stop == wanted),
                "Keyboard: Tab never reaches {wanted}: {stops:?}"
            );
        }
        click_popup(&ui.window, "Press shortcut keys", AccessibleRole::TextInput);
        key(&ui.window, Key::Tab);
        settle(&mut ui, &clock);
        let keyboard = ui.window.global::<KeyboardEditorView>();
        assert_eq!(
            keyboard.get_candidate(),
            "Tab",
            "a clicked field records Tab"
        );
    }

    /// Every other page keeps the same promise, the focus never landing on a hidden or
    /// disabled control. Archives and Keyboard have their own tests above.
    #[test]
    fn every_options_page_tab_cycle_keeps_to_shown_enabled_controls() {
        let (mut ui, clock, _presenter) = options();
        let issues: Vec<String> = PAGES
            .into_iter()
            .filter(|(_, topic)| ![3, 7].contains(topic))
            .flat_map(|(_, topic)| walk(&mut ui, &clock, topic).issues)
            .collect();
        assert!(issues.is_empty(), "{issues:#?}");
    }

    /// U-D29: in the page list the arrows, Home and End move from the focused entry, as in
    /// any list, also when Tab or Shift+Tab left the focus on another entry than the page
    /// shown; after each key the focus and the page shown agree. The order and the
    /// clamps at both ends stay the list's.
    #[test]
    fn page_list_keys_move_from_the_focused_entry_and_keep_focus_and_page_together() {
        let (mut ui, clock, _presenter) = options();
        let state = |ui: &TestUi| {
            let topic = ui.window.get_options_topic_index();
            let shown = PAGES.iter().find(|(_, id)| *id == topic).unwrap().0;
            let focused = focus(ui)
                .map(|(item, _)| name(&control(&item)))
                .unwrap_or_default();
            format!("focus {focused}, page {shown}")
        };
        let press = |ui: &mut TestUi, shift: bool, pressed: Key| {
            if shift {
                ui::chord(&ui.window, &[Key::Shift], pressed);
            } else {
                key(&ui.window, pressed);
            }
            settle(ui, &clock);
            state(&*ui)
        };
        ui.options_topic(7);
        settle(&mut ui, &clock);
        assert_eq!(state(&ui), "focus Archives, page Archives");
        // Tab leaves the page shown behind; Down goes on from the focused entry.
        assert_eq!(
            press(&mut ui, false, Key::Tab),
            "focus Editors, page Archives"
        );
        assert_eq!(
            press(&mut ui, false, Key::DownArrow),
            "focus Comparators, page Comparators",
            "Down from the focused Editors"
        );
        assert_eq!(
            press(&mut ui, true, Key::Tab),
            "focus Editors, page Comparators"
        );
        assert_eq!(
            press(&mut ui, false, Key::UpArrow),
            "focus Archives, page Archives",
            "Up from the focused Editors"
        );
        // End and Home go to the last and the first page, and the ends clamp.
        press(&mut ui, false, Key::Tab);
        assert_eq!(
            press(&mut ui, false, Key::End),
            "focus Plugins, page Plugins"
        );
        assert_eq!(
            press(&mut ui, false, Key::DownArrow),
            "focus Plugins, page Plugins"
        );
        assert_eq!(
            press(&mut ui, true, Key::Tab),
            "focus Keyboard, page Plugins"
        );
        assert_eq!(
            press(&mut ui, false, Key::Home),
            "focus General, page General"
        );
        assert_eq!(
            press(&mut ui, false, Key::UpArrow),
            "focus General, page General"
        );
        // Keys that come before the focus has moved each still move one entry.
        key(&ui.window, Key::DownArrow);
        assert_eq!(
            press(&mut ui, false, Key::DownArrow),
            "focus Colors, page Colors"
        );
    }
}

/// Appearance › Scrolling glide: a slider with six named stops. The arrows, Home and End
/// move one stop or to an end, Page Up and Page Down two stops; each choice is reported
/// once, and the name beside the slider follows the host's staged value.
#[test]
fn scrolling_glide_slider_moves_by_keys_and_names_its_stop() {
    let mut ui = TestUi::new();
    ui.window.set_preferences_ready(true);
    let chosen = Rc::new(std::cell::RefCell::new(Vec::new()));
    let captured = chosen.clone();
    let weak = ui.window.as_weak();
    ui.window.on_scroll_glide_changed(move |index| {
        captured.borrow_mut().push(index);
        weak.upgrade().unwrap().set_scroll_glide_index(index);
    });
    ui.window.invoke_show_options();
    ui.draw();
    ui.options_topic(1);
    assert_eq!(ui.window.get_scroll_glide_index(), 3);
    let slider = ui::popup_item(&ui.window, "Scrolling glide", AccessibleRole::Slider);
    assert_eq!(
        slider
            .accessible_string_property(AccessibleStringProperty::Value)
            .unwrap_or_default()
            .as_str(),
        "Default"
    );
    click_popup(&ui.window, "Scrolling glide", AccessibleRole::Slider);
    chosen.borrow_mut().clear();
    ui.window.set_scroll_glide_index(3);
    ui.draw();
    for (step, (pressed, expected)) in [
        (Key::End, 5),
        (Key::Home, 0),
        (Key::PageUp, 2),
        (Key::PageUp, 4),
        (Key::RightArrow, 5),
        (Key::RightArrow, 5),
        (Key::PageDown, 3),
        (Key::LeftArrow, 2),
    ]
    .into_iter()
    .enumerate()
    {
        key(&ui.window, pressed);
        ui.draw();
        assert_eq!(
            ui.window.get_scroll_glide_index(),
            expected,
            "after key {step}"
        );
    }
    // The last stop takes no second choice.
    assert_eq!(*chosen.borrow(), [5, 0, 2, 4, 5, 3, 2]);
    let slider = ui::popup_item(&ui.window, "Scrolling glide", AccessibleRole::Slider);
    assert_eq!(
        slider
            .accessible_string_property(AccessibleStringProperty::Value)
            .unwrap_or_default()
            .as_str(),
        "Short"
    );
}

/// Appearance › Scrolling speed: the same named-stop slider with five stops, Medium (the
/// system's notch) by default.
#[test]
fn scrolling_speed_slider_moves_by_keys_and_names_its_stop() {
    let mut ui = TestUi::new();
    ui.window.set_preferences_ready(true);
    let chosen = Rc::new(std::cell::RefCell::new(Vec::new()));
    let captured = chosen.clone();
    let weak = ui.window.as_weak();
    ui.window.on_wheel_speed_changed(move |index| {
        captured.borrow_mut().push(index);
        weak.upgrade().unwrap().set_wheel_speed_index(index);
    });
    ui.window.invoke_show_options();
    ui.draw();
    ui.options_topic(1);
    assert_eq!(ui.window.get_wheel_speed_index(), 2);
    let named = |ui: &TestUi| {
        ui::popup_item(&ui.window, "Scrolling speed", AccessibleRole::Slider)
            .accessible_string_property(AccessibleStringProperty::Value)
            .unwrap_or_default()
            .to_string()
    };
    assert_eq!(named(&ui), "Medium");
    click_popup(&ui.window, "Scrolling speed", AccessibleRole::Slider);
    chosen.borrow_mut().clear();
    ui.window.set_wheel_speed_index(2);
    ui.draw();
    for (step, (pressed, expected)) in [
        (Key::End, 4),
        (Key::Home, 0),
        (Key::PageUp, 2),
        (Key::PageUp, 4),
        (Key::RightArrow, 4),
        (Key::PageDown, 2),
        (Key::LeftArrow, 1),
    ]
    .into_iter()
    .enumerate()
    {
        key(&ui.window, pressed);
        ui.draw();
        assert_eq!(
            ui.window.get_wheel_speed_index(),
            expected,
            "after key {step}"
        );
    }
    assert_eq!(*chosen.borrow(), [4, 0, 2, 4, 2, 1]);
    assert_eq!(named(&ui), "Slow");
}

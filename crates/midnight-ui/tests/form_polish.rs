//! The UI polish gate's second batch of dialog findings (20 November 2026) and the
//! first batch's leftovers: icon-only buttons in Attributes › Times and Options ›
//! Colors name their action in a tooltip (U-D19), the command line shortens a long
//! working folder with an ellipsis instead of cutting it (U-D20), Create dir and New
//! file say why Enter with an empty name does nothing (U-D21), the colour picker,
//! shortcut replacement, Select by mask and the main menu route keys without a Tab
//! stop of their own (U-D04), Make File List selects with the theme's colours (U-D07),
//! Options pages keep their fields whole at 320×200 (U-D08) and its topics answer
//! hovering (U-D05).
#[path = "support/embedded_form.rs"]
mod form;
#[path = "support/window_input.rs"]
mod input;
#[path = "support/render.rs"]
mod render;
use crate::support::{modeless, ui};

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{
        AccessibleRole, Clip, ComplexText, Flickable, FocusScope, Opacity, SimpleText, TextInput,
        TooltipArea,
    },
    layout::Orientation,
    window::{WindowInner, WindowKind},
};
use midnight_core::Event;
use midnight_ui::{
    ColorOptionsView, FileListPresentation, KeyboardEditorView, OperationPresentation,
    PanelChoices, Presenter, Theme, install, refresh,
};
use slint::{
    Color, ComponentHandle, ModelRc, SharedString, VecModel,
    platform::{Key, WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

/// Window rectangle: left, top, width, height.
type Rect = (f32, f32, f32, f32);

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

fn contains((x, y, w, h): Rect, px: f32, py: f32) -> bool {
    px >= x && px <= x + w && py >= y && py <= y + h
}

fn label(item: &ItemRc) -> String {
    item.accessible_string_property(AccessibleStringProperty::Label)
        .map(|label| label.to_string())
        .unwrap_or_default()
}

fn text_of(item: &ItemRc) -> Option<String> {
    if let Some(text) = item.downcast::<SimpleText>() {
        return Some(text.as_pin_ref().text().to_string());
    }
    item.downcast::<ComplexText>()
        .map(|text| text.as_pin_ref().text().to_string())
}

/// A `visible: false` element: Slint lowers it to a clip that hides its subtree.
fn hides(item: &ItemRc) -> bool {
    item.downcast::<Clip>().is_some_and(|clip| {
        let clip = clip.as_pin_ref();
        clip.is_visibility_clip() && clip.clip()
    })
}

/// Every item below `root` that no `visible: false` element hides, in tree order.
fn shown_items(root: &ItemRc) -> Vec<ItemRc> {
    fn walk(item: &ItemRc, found: &mut Vec<ItemRc>) {
        if hides(item) {
            return;
        }
        found.push(item.clone());
        let mut child = item.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            walk(&current, found);
        }
    }
    let mut found = Vec::new();
    walk(root, &mut found);
    found
}

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

fn inside_control(item: &ItemRc) -> bool {
    let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(ancestor) = parent {
        if interactive(ancestor.accessible_role()) {
            return true;
        }
        parent = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    false
}

fn preferred(item: &ItemRc, orientation: Orientation) -> f32 {
    let adapter = item.window_adapter().expect("a shown item has a window");
    item.borrow()
        .as_ref()
        .layout_info(orientation, -1.0, &adapter, item)
        .preferred
}

/// Texts (by their glyph area) and controls that reach more than 1.5 px outside a
/// clipping container that does not scroll, as the gate's U-K03 measures them: a scroll
/// view's content is cut by the view, which scrolls, so only containers between it and
/// the item count. Only items whose centre lies in `band` (all when None) are judged.
fn cut_without_scrolling(root: &ItemRc, band: Option<Rect>) -> Vec<String> {
    fn walk(item: &ItemRc, clip: Option<Rect>, band: Option<Rect>, found: &mut Vec<String>) {
        if hides(item) {
            return;
        }
        let rect = bounds(item);
        let area = match text_of(item) {
            Some(text) if text.chars().any(char::is_alphanumeric) => {
                let width = preferred(item, Orientation::Horizontal).min(rect.2);
                Some((rect.0, rect.1, width.max(0.0), rect.3))
            }
            _ if interactive(item.accessible_role()) && !inside_control(item) => Some(rect),
            _ => None,
        };
        if let (Some(area), Some(clip)) = (area, clip)
            && area.2 > 0.5
            && area.3 > 0.5
            && band.is_none_or(|band| contains(band, area.0 + area.2 / 2.0, area.1 + area.3 / 2.0))
        {
            let outside = (clip.0 - area.0).max(0.0)
                + (area.0 + area.2 - clip.0 - clip.2).max(0.0)
                + (clip.1 - area.1).max(0.0)
                + (area.1 + area.3 - clip.1 - clip.3).max(0.0);
            if outside > 1.5 {
                found.push(format!(
                    "{:?} {:?} reaches {outside:.0}px outside {clip:?}",
                    item.accessible_role(),
                    text_of(item).unwrap_or_else(|| label(item))
                ));
            }
        }
        let child_clip = if item.downcast::<Flickable>().is_some() {
            None
        } else if item.borrow().as_ref().clips_children() {
            Some(match clip {
                Some(clip) => intersect(clip, rect).unwrap_or((rect.0, rect.1, 0.0, 0.0)),
                None => rect,
            })
        } else {
            clip
        };
        let mut child = item.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            walk(&current, child_clip, band, found);
        }
    }
    let mut found = Vec::new();
    walk(root, None, band, &mut found);
    found
}

/// The root of the main window's topmost dialog (tooltips are not dialogs).
fn dialog_root(window: &slint::Window) -> ItemRc {
    let inner = WindowInner::from_pub(window);
    let popups = inner.active_popups();
    let popup = popups
        .iter()
        .rev()
        .find(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
        .expect("a dialog is open");
    ItemRc::new_root(popup.component.clone())
}

fn main_root(window: &slint::Window) -> ItemRc {
    ItemRc::new_root(WindowInner::from_pub(window).component())
}

/// Shown buttons that draw no caption and that no tooltip area covers (the gate's
/// U-K08).
fn icon_only_without_tooltip(root: &ItemRc) -> Vec<String> {
    let items = shown_items(root);
    let tooltips: Vec<Rect> = items
        .iter()
        .filter(|item| item.downcast::<TooltipArea>().is_some())
        .map(|item| {
            let rect = bounds(item);
            match item.parent_item(ParentItemTraversalMode::StopAtPopups) {
                Some(parent) if rect.2 <= 0.0 || rect.3 <= 0.0 => bounds(&parent),
                _ => rect,
            }
        })
        .collect();
    items
        .iter()
        .filter(|item| item.accessible_role() == AccessibleRole::Button)
        .filter(|item| {
            let rect = bounds(item);
            rect.2 > 1.0 && rect.3 > 1.0
        })
        .filter(|item| {
            !shown_items(item).iter().any(|child| {
                text_of(child).is_some_and(|text| text.chars().any(char::is_alphanumeric))
                    && bounds(child).2 > 1.0
            })
        })
        .filter(|item| {
            let (x, y, w, h) = bounds(item);
            !tooltips
                .iter()
                .any(|area| contains(*area, x + w / 2.0, y + h / 2.0))
        })
        .map(|item| format!("{:?} at {:?}", label(item), bounds(item)))
        .collect()
}

/// The rendered pixels of `area` and a two-pixel margin around it.
fn region(test: &ui::TestUi, (x, y, w, h): Rect) -> Vec<slint::Rgb8Pixel> {
    let left = (x - 2.0).floor().max(0.0) as usize;
    let top = (y - 2.0).floor().max(0.0) as usize;
    let right = ((x + w + 2.0).ceil() as usize).min(test.width);
    let bottom = ((y + h + 2.0).ceil() as usize).min(test.height);
    (top..bottom)
        .flat_map(|row| (left..right).map(move |column| row * test.width + column))
        .map(|index| test.pixels[index])
        .collect()
}

fn pointer_out(test: &mut ui::TestUi, clock: &Cell<Duration>) {
    test.window
        .window()
        .dispatch_event(WindowEvent::PointerExited);
    ui::advance(clock, Duration::from_millis(200));
    test.draw();
}

fn point_at(test: &ui::TestUi, (x, y, w, h): Rect) {
    test.window
        .window()
        .dispatch_event(WindowEvent::PointerMoved {
            position: slint::LogicalPosition::new(x + w / 2.0, y + h / 2.0),
        });
}

/// Whether hovering the centre of `item` changes how it looks.
fn answers_hovering(test: &mut ui::TestUi, clock: &Cell<Duration>, item: &ItemRc) -> bool {
    let area = bounds(item);
    pointer_out(test, clock);
    let before = region(test, area);
    point_at(test, area);
    ui::advance(clock, Duration::from_millis(200));
    test.draw();
    let after = region(test, area);
    pointer_out(test, clock);
    before != after
}

/// The texts of the tooltip that resting the pointer on `item` shows.
fn tooltip_of(test: &mut ui::TestUi, clock: &Cell<Duration>, item: &ItemRc) -> Vec<String> {
    let area = bounds(item);
    pointer_out(test, clock);
    point_at(test, area);
    ui::advance(clock, Duration::from_millis(1200));
    test.draw();
    let texts = WindowInner::from_pub(test.window.window())
        .active_popups()
        .iter()
        .filter(|popup| matches!(popup.window_kind, WindowKind::ToolTip))
        .flat_map(|popup| ui::visible_text_values(ItemRc::new_root(popup.component.clone())))
        .map(|text| text.to_string())
        .filter(|text| !text.trim().is_empty())
        .collect();
    pointer_out(test, clock);
    texts
}

/// Scroll the scroll views around the dialog control `label` until it is inside them.
fn reveal(test: &mut ui::TestUi, label: &str, role: AccessibleRole) -> ItemRc {
    for _ in 0..40 {
        let item = ui::popup_item(&test.window, label, role);
        let area = bounds(&item);
        let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
        let mut scroll = None;
        while let Some(ancestor) = parent {
            if ancestor.downcast::<Flickable>().is_some() {
                let view = bounds(&ancestor);
                if area.1 < view.1 - 0.5 || area.1 + area.3 > view.1 + view.3 + 0.5 {
                    scroll = Some((view, area.1 < view.1));
                    break;
                }
            }
            parent = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
        }
        let Some((view, up)) = scroll else {
            return item;
        };
        // Over the view's scroll bar strip, which no page control covers.
        test.window
            .window()
            .dispatch_event(WindowEvent::PointerScrolled {
                position: slint::LogicalPosition::new(view.0 + view.2 - 6.0, view.1 + view.3 / 2.0),
                delta_x: 0.0,
                delta_y: if up { 40.0 } else { -40.0 },
            });
        test.draw();
    }
    panic!("{label} never scrolls into view");
}

fn presenter_ui(test: &ui::TestUi) -> Rc<RefCell<Presenter>> {
    let state = Rc::new(RefCell::new(Presenter::new("left".into(), "right".into())));
    install(&test.window, state.clone(), Rc::new(|_| true));
    state
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::default())));
    refresh(&test.window, &state.borrow());
    state
}

// ---------------------------------------------------------------------------------
// U-D19: icon-only buttons name their action in a tooltip.

#[test]
fn attributes_time_pickers_name_their_action_in_a_tooltip() {
    let (mut test, clock) = ui::TestUi::with_virtual_clock();
    test.window.set_metadata_can_windows(true);
    test.window.set_metadata_can_created(true);
    test.window.set_metadata_can_modified(true);
    test.window.set_metadata_can_accessed(true);
    test.window.set_metadata_ready(true);
    test.window.set_metadata_token("attributes:1".into());
    test.draw();
    ui::click_popup(&test.window, "Times", AccessibleRole::Button);
    test.draw();
    let missing = icon_only_without_tooltip(&dialog_root(test.window.window()));
    assert!(
        missing.is_empty(),
        "Times: icon-only buttons without a tooltip: {missing:#?}"
    );
    for (label, words) in [
        ("Modified: choose date", "calendar"),
        ("Modified: choose time", "hour"),
    ] {
        let item = ui::popup_item(&test.window, label, AccessibleRole::Button);
        let tip = tooltip_of(&mut test, &clock, &item);
        assert!(
            tip.iter().any(|text| text.contains(words)),
            "{label}: tooltip {tip:?}"
        );
    }
}

#[test]
fn color_swatches_and_discard_changes_name_their_action_in_a_tooltip() {
    let (mut test, clock) = ui::TestUi::with_virtual_clock();
    let _state = presenter_ui(&test);
    test.window.invoke_show_options();
    test.window.set_options_topic_index(8);
    test.draw();
    assert!(
        test.window.global::<ColorOptionsView>().get_selected() >= 0,
        "a filename color rule is selected"
    );
    let missing = icon_only_without_tooltip(&dialog_root(test.window.window()));
    assert!(
        missing.is_empty(),
        "Colors: icon-only buttons without a tooltip: {missing:#?}"
    );
    for (label, words) in [
        ("Choose Foreground", "foreground"),
        ("Choose Background", "background"),
        ("Discard changes", "saved"),
    ] {
        let item = reveal(&mut test, label, AccessibleRole::Button);
        let tip = tooltip_of(&mut test, &clock, &item);
        assert!(
            tip.iter().any(|text| text.to_lowercase().contains(words)),
            "{label}: tooltip {tip:?}"
        );
    }
}

// ---------------------------------------------------------------------------------
// U-D23: the colour picker's swatches name their colour and show hover and focus.

fn tooltips_shown(test: &ui::TestUi) -> usize {
    WindowInner::from_pub(test.window.window())
        .active_popups()
        .iter()
        .filter(|popup| matches!(popup.window_kind, WindowKind::ToolTip))
        .count()
}

#[test]
fn colour_picker_swatches_name_their_colour_and_show_hover_and_focus() {
    let (mut test, clock) = ui::TestUi::with_virtual_clock();
    let _state = presenter_ui(&test);
    test.window.invoke_show_options();
    test.window.set_options_topic_index(8);
    test.draw();
    let choose = reveal(&mut test, "Choose Foreground", AccessibleRole::Button);
    point_at(&test, bounds(&choose));
    ui::click_popup(&test.window, "Choose Foreground", AccessibleRole::Button);
    test.draw();
    assert_eq!(
        test.window.global::<ColorOptionsView>().get_picker_field(),
        0
    );
    let missing = icon_only_without_tooltip(&dialog_root(test.window.window()));
    assert!(
        missing.is_empty(),
        "colour picker: icon-only buttons without a tooltip: {missing:#?}"
    );
    for (label, name) in [
        ("RGB 0, 0, 0", "Black #000000"),
        ("RGB 255, 255, 255", "White #FFFFFF"),
        ("RGB 239, 68, 68", "Red #EF4444"),
        ("RGB 245, 158, 11", "Amber #F59E0B"),
        ("RGB 34, 197, 94", "Green #22C55E"),
        ("RGB 6, 182, 212", "Cyan #06B6D4"),
        ("RGB 59, 130, 246", "Blue #3B82F6"),
        ("RGB 168, 85, 247", "Purple #A855F7"),
    ] {
        let swatch = ui::popup_item(&test.window, label, AccessibleRole::Button);
        assert_eq!(
            swatch
                .accessible_string_property(AccessibleStringProperty::Description)
                .as_deref(),
            Some(name),
            "{label}"
        );
        let tip = tooltip_of(&mut test, &clock, &swatch);
        assert!(
            tip.iter().any(|text| text.contains(name)),
            "{label}: tooltip {tip:?}"
        );
        assert!(
            answers_hovering(&mut test, &clock, &swatch),
            "{label} does not change when hovered"
        );
        // Keyboard focus changes how the swatch looks, as the Tab check sees it.
        let area = bounds(&swatch);
        test.draw();
        let unfocused = region(&test, area);
        let mut scope = None;
        let mut child = swatch.first_child();
        while let Some(item) = child {
            if item.downcast::<FocusScope>().is_some() {
                scope = Some(item.clone());
            }
            child = item.next_sibling();
        }
        WindowInner::from_pub(test.window.window()).set_focus_item(
            &scope.expect("the swatch's focus scope"),
            true,
            i_slint_core::items::FocusReason::TabNavigation,
        );
        test.draw();
        assert_ne!(region(&test, area), unfocused, "{label} shows no focus");
    }
    // Closing the picker while a swatch's tooltip shows keeps Options open and leaves
    // no tooltip behind (Slint 1.18.1 must not destroy a shown tip inside a popup).
    for close in ["Escape", "Use color"] {
        if test.window.global::<ColorOptionsView>().get_picker_field() < 0 {
            ui::click_popup(&test.window, "Choose Foreground", AccessibleRole::Button);
            test.draw();
        }
        let swatch = ui::popup_item(&test.window, "RGB 239, 68, 68", AccessibleRole::Button);
        pointer_out(&mut test, &clock);
        point_at(&test, bounds(&swatch));
        ui::advance(&clock, Duration::from_millis(1200));
        test.draw();
        assert_eq!(tooltips_shown(&test), 1, "the swatch's tooltip shows");
        if close == "Escape" {
            ui::key(&test.window, Key::Escape);
        } else {
            ui::popup_item(&test.window, close, AccessibleRole::Button)
                .accessible_action(&i_slint_core::accessibility::AccessibilityAction::Default);
        }
        for _ in 0..3 {
            ui::advance(&clock, Duration::from_millis(50));
            test.draw();
        }
        assert_eq!(
            test.window.global::<ColorOptionsView>().get_picker_field(),
            -1
        );
        assert!(
            test.window.get_options_open(),
            "{close} closes only the picker"
        );
        assert_eq!(tooltips_shown(&test), 0, "{close} leaves no tooltip behind");
        pointer_out(&mut test, &clock);
    }
}

// ---------------------------------------------------------------------------------
// U-D05: the Options topics answer hovering.

fn flag(item: &ItemRc, property: AccessibleStringProperty) -> bool {
    item.accessible_string_property(property)
        .is_some_and(|value| value == "true")
}

/// Drawn with no opacity, as the input layer of a check box with its own indicator.
fn transparent(item: &ItemRc) -> bool {
    let mut current = Some(item.clone());
    while let Some(node) = current {
        if node
            .downcast::<Opacity>()
            .is_some_and(|layer| layer.as_pin_ref().opacity() < 0.01)
        {
            return true;
        }
        current = node.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    false
}

/// The controls of the open dialog a user can hover, as the gate's U-K30 sweep picks
/// them: shown and enabled buttons, tabs, check boxes, radio buttons, combo boxes and
/// switches that are drawn and not inside another control, leaving out the active page
/// or tab.
fn hoverable(root: &ItemRc) -> Vec<ItemRc> {
    shown_items(root)
        .into_iter()
        .filter(|item| {
            matches!(
                item.accessible_role(),
                AccessibleRole::Button
                    | AccessibleRole::Tab
                    | AccessibleRole::Checkbox
                    | AccessibleRole::RadioButton
                    | AccessibleRole::Combobox
                    | AccessibleRole::Switch
            ) && !inside_control(item)
                && !transparent(item)
                && !item
                    .accessible_string_property(AccessibleStringProperty::Enabled)
                    .is_some_and(|value| value == "false")
                && !(flag(item, AccessibleStringProperty::Checkable)
                    && flag(item, AccessibleStringProperty::Checked))
                && !flag(item, AccessibleStringProperty::ItemSelected)
        })
        .filter(|item| {
            let (x, y, w, h) = bounds(item);
            // Its centre shows: no scroll view around it has scrolled it out of view.
            let (cx, cy) = (x + w / 2.0, y + h / 2.0);
            let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
            let mut shown = w >= 2.0 && h >= 2.0;
            while let Some(ancestor) = parent {
                if ancestor.borrow().as_ref().clips_children() {
                    shown &= contains(bounds(&ancestor), cx, cy);
                }
                parent = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
            }
            shown
        })
        .collect()
}

#[test]
fn options_topics_and_controls_answer_hovering() {
    // No Presenter: its refresh would apply its own saved theme again.
    let (mut test, clock) = ui::TestUi::with_virtual_clock();
    for (scheme, density) in [(0, 0), (1, 1), (2, 0)] {
        test.window.invoke_apply_preferences(scheme, density);
        test.window.invoke_show_options();
        test.draw();
        let controls = hoverable(&dialog_root(test.window.window()));
        let topics = controls
            .iter()
            .filter(|item| {
                [
                    "Appearance",
                    "Keyboard",
                    "Editors",
                    "Comparators",
                    "Plugins",
                    "Previews",
                    "Archives",
                    "Colors",
                    "Custom Commands",
                ]
                .contains(&label(item).as_str())
            })
            .count();
        assert_eq!(topics, 9, "every topic but the shown one is hovered");
        let silent: Vec<String> = controls
            .iter()
            .filter(|item| !answers_hovering(&mut test, &clock, item))
            .map(|item| format!("{:?} {:?}", item.accessible_role(), label(item)))
            .collect();
        assert!(
            silent.is_empty(),
            "theme {scheme}, density {density}: no hover feedback: {silent:#?}"
        );
        ui::key(&test.window, Key::Escape);
        test.draw();
    }
}

// ---------------------------------------------------------------------------------
// U-D08: Options pages keep their fields whole at the minimum window size.

/// Single-line texts that need more width than they have and neither elide nor end at
/// an omission marker (the gate's U-K04), and texts squeezed to less than about one
/// character (U-K18, U-K19).
fn texts_cut_short(root: &ItemRc) -> Vec<String> {
    let items = shown_items(root);
    let mut found = Vec::new();
    for item in &items {
        let Some(text) = text_of(item) else { continue };
        if !text.chars().any(char::is_alphanumeric) {
            continue;
        }
        let (elides, wraps, size) = match item.downcast::<ComplexText>() {
            Some(complex) => {
                let complex = complex.as_pin_ref();
                (
                    matches!(complex.overflow(), i_slint_core::items::TextOverflow::Elide),
                    !matches!(complex.wrap(), i_slint_core::items::TextWrap::NoWrap),
                    complex.font_size().get(),
                )
            }
            None => (
                false,
                false,
                item.downcast::<SimpleText>()
                    .map_or(12.0, |simple| simple.as_pin_ref().font_size().get()),
            ),
        };
        let (x, y, w, h) = bounds(item);
        let short: String = text.chars().take(40).collect();
        if w <= 0.0 || h <= 0.0 {
            found.push(format!("{short:?} is {w:.0}×{h:.0}"));
            continue;
        }
        let needed = preferred(item, Orientation::Horizontal);
        if wraps || needed <= w + 1.0 {
            continue;
        }
        if w < size.max(8.0) * 1.2 {
            found.push(format!("{short:?} shows {w:.0} of {needed:.0}px"));
            continue;
        }
        let marked = items.iter().any(|other| {
            text_of(other).is_some_and(|text| matches!(text.trim(), "…" | "..."))
                && (bounds(other).0 - (x + w)).abs() <= 4.0
                && bounds(other).1 < y + h
                && bounds(other).1 + bounds(other).3 > y
        });
        if !elides && !marked {
            found.push(format!(
                "{short:?} needs {needed:.0}px, has {w:.0}px and no ellipsis"
            ));
        }
    }
    found
}

#[test]
fn options_pages_squeeze_no_field_or_list_at_the_minimum_size() {
    // No Presenter: its refresh would apply its own saved theme and fonts again. A
    // filename colour rule is chosen, so that the Colors page shows its editor.
    let mut test = ui::TestUi::new();
    let colors = test.window.global::<ColorOptionsView>();
    colors.set_labels(ModelRc::new(VecModel::from(vec![SharedString::from(
        "*.rs;*.toml · 文档",
    )])));
    colors.set_selected(0);
    let mut findings = Vec::new();
    // The gate's Blue Desktop and Dark Touch at their default fonts, where every check
    // applies, and larger fonts, where no list may be squeezed either. (At 320 px some
    // single-line captions of the pages need a wrap at these fonts.)
    for (scheme, density, font, whole) in [
        (0, 0, 12, true),
        (1, 1, 15, true),
        (0, 0, 20, false),
        (1, 1, 24, false),
    ] {
        test.window.invoke_apply_fonts("".into(), font, font);
        test.window.invoke_apply_preferences(scheme, density);
        test.resize(320, 200);
        test.window.invoke_show_options();
        for topic in [0, 1, 3, 4, 10, 5, 6, 7, 8, 9] {
            test.window.set_options_topic_index(topic);
            test.draw();
            let root = dialog_root(test.window.window());
            let case = format!("theme {scheme}, density {density}, font {font}, topic {topic}");
            if whole {
                for finding in cut_without_scrolling(&root, None)
                    .into_iter()
                    .chain(texts_cut_short(&root))
                {
                    findings.push(format!("{case}: {finding}"));
                }
            }
            // A field squeezed to a sliver, or a scroll view that shows less than one
            // control of what it scrolls.
            let control = test.window.global::<Theme>().get_control_height();
            for item in shown_items(&root) {
                let (_, _, w, h) = bounds(&item);
                if let Some(view) = item.downcast::<Flickable>() {
                    let content = view.as_pin_ref().content_height().get();
                    if content > h + 1.0 && h < content.min(control) - 0.5 {
                        findings.push(format!(
                            "{case}: a scroll view shows {h:.0} of {content:.0}px"
                        ));
                    }
                    continue;
                }
                if !interactive(item.accessible_role()) || inside_control(&item) {
                    continue;
                }
                if w > 0.0 && h > 0.0 && (w < 16.0 || h < 12.0) {
                    findings.push(format!(
                        "{case}: {:?} {:?} is {w:.0}×{h:.0}",
                        item.accessible_role(),
                        label(&item)
                    ));
                }
            }
            render::snapshot(&test, &format!("options-{font}-{density}-{topic}"));
        }
        ui::key(&test.window, Key::Escape);
        test.draw();
    }
    assert!(findings.is_empty(), "cut off at 320×200: {findings:#?}");
}

// ---------------------------------------------------------------------------------
// U-D20: the command line shortens a long working folder with an ellipsis.

#[test]
fn the_command_line_shortens_a_long_working_folder_instead_of_cutting_it() {
    let mut test = ui::TestUi::new();
    let path = format!("C:\\Users\\{}文档 folder", "a-long-folder-name\\".repeat(8));
    test.window.set_left_path(path.as_str().into());
    test.window
        .set_terminal_choices(ModelRc::new(VecModel::from(vec![
            SharedString::from("PowerShell"),
            "cmd".into(),
        ])));
    test.window.set_windows_shells(true);
    test.window.set_terminal_available(true);
    test.window.set_terminal_ready(true);
    for (scheme, density, width, height) in [(1, 1, 960, 600), (0, 0, 320, 200), (0, 0, 1340, 820)]
    {
        let what = format!("theme {scheme}, density {density}, {width}×{height}");
        test.window.invoke_apply_preferences(scheme, density);
        test.resize(width, height);
        let band = (
            0.0,
            test.window.get_command_bar_top(),
            width as f32,
            test.window.get_command_bar_height(),
        );
        let cut = cut_without_scrolling(&main_root(test.window.window()), Some(band));
        assert!(cut.is_empty(), "{what}: the command line cuts {cut:#?}");
        // A shortened prompt shows where it is shortened.
        assert!(
            test.window.get_command_path_content_width()
                > test.window.get_command_path_width() + 1.0,
            "{what}: the test needs a shortened working folder"
        );
        let marker = ui::root_item(
            &test.window,
            "Working folder path shortened",
            AccessibleRole::Text,
        );
        assert!(
            ui::visible_text_values(marker)
                .iter()
                .any(|text| text == "..."),
            "{what}: no ellipsis marks the shortened working folder"
        );
        // The whole folder stays available to assistive technology.
        ui::root_item(
            &test.window,
            &format!("Working folder: {path}"),
            AccessibleRole::Text,
        );
    }
}

// ---------------------------------------------------------------------------------
// U-D21: Enter with an empty name keeps Create dir and New file open with the reason.

/// Whether the open dialog shows a text containing `needle`.
fn dialog_shows(host: &form::Host, needle: &str) -> bool {
    host.popup().is_some_and(|root| {
        shown_items(&root).iter().any(|item| {
            text_of(item).is_some_and(|text| text.contains(needle)) && bounds(item).2 > 1.0
        })
    })
}

fn type_text(host: &form::Host, text: &str) {
    for character in text.chars() {
        host.key(SharedString::from(character.to_string()));
    }
}

#[test]
fn enter_with_an_empty_name_keeps_create_dialogs_open_and_says_why() {
    let host = form::Host::new();
    let form = host.main.global::<OperationPresentation>();
    let confirmed = Rc::new(RefCell::new(Vec::new()));
    form.on_confirm({
        let confirmed = confirmed.clone();
        move |_, value| confirmed.borrow_mut().push(value.to_string())
    });
    form.set_token("operation".into());
    for (kind, title, input_label, reason) in [
        (
            3,
            "Create directory",
            "Folder name or path",
            "Type the folder name or path first.",
        ),
        (4, "New file", "File name", "Type the file name first."),
    ] {
        confirmed.borrow_mut().clear();
        form.set_kind(kind);
        form.set_title_text(title.into());
        form.set_confirm_text(title.into());
        form.set_has_name(true);
        form.set_input_label(input_label.into());
        form.set_initial_name("".into());
        form.set_selection_end(0);
        form.invoke_begin();
        host.layout(640, 420);
        assert!(
            !dialog_shows(&host, reason),
            "{title}: the reason waits for Enter"
        );
        host.key(Key::Return);
        host.layout(640, 420);
        assert!(confirmed.borrow().is_empty(), "{title}: nothing to confirm");
        assert!(host.is_open(), "{title}: the dialog stays open");
        assert!(
            dialog_shows(&host, reason),
            "{title}: Enter with an empty name says why nothing happens"
        );
        // Typing a name removes the reason, and Enter confirms it.
        type_text(&host, "new 文档");
        host.layout(640, 420);
        assert!(
            !dialog_shows(&host, reason),
            "{title}: a typed name clears the reason"
        );
        host.key(Key::Return);
        assert_eq!(*confirmed.borrow(), ["new 文档"], "{title}");
    }
}

// ---------------------------------------------------------------------------------
// U-D04: the remaining dialog-wide key routers are no Tab stops.

/// The FocusScopes around `control`: the dialog-wide key routers that hold it.
fn routers(control: &ItemRc) -> Vec<ItemRc> {
    let mut found = Vec::new();
    let mut parent = control.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(ancestor) = parent {
        if ancestor.downcast::<FocusScope>().is_some() {
            found.push(ancestor.clone());
        }
        parent = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    found
}

fn tab(native: &MinimalSoftwareWindow, backwards: bool) {
    if backwards {
        input::chord(native, &[Key::Shift], Key::Tab);
    } else {
        input::key(native, Key::Tab);
    }
    slint::platform::update_timers_and_animations();
}

/// Tab and then Shift+Tab `presses` times each: the focus never rests on a key router
/// around the dialog control `control`, which would show no focus.
fn assert_tab_skips_routers(host: &form::Host, control: &str, what: &str, presses: usize) {
    host.layout(1000, 700);
    let control = host.item(AccessibleRole::Button, control);
    let routers = routers(&control);
    drop(control);
    assert!(
        !routers.is_empty(),
        "{what} routes its keys through a FocusScope"
    );
    for backwards in [false, true] {
        let key = if backwards { "Shift+Tab" } else { "Tab" };
        for press in 1..=presses {
            tab(&host.native, backwards);
            let focus = WindowInner::from_pub(host.native.window())
                .focus_item
                .borrow()
                .upgrade()
                .unwrap_or_else(|| panic!("{what}: {key} {press} lost the keyboard focus"));
            assert!(
                !routers.contains(&focus),
                "{what}: {key} {press} rests on a key router, which shows no focus"
            );
        }
    }
}

fn popup_count(host: &form::Host) -> usize {
    host.settle();
    WindowInner::from_pub(host.main.window())
        .active_popups()
        .iter()
        .filter(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
        .count()
}

#[test]
fn select_by_mask_routes_keys_without_a_tab_stop() {
    let host = form::Host::new();
    let choices = host.main.global::<PanelChoices>();
    let weak = host.main.as_weak();
    choices.on_mask_dismiss(move |_| {
        if let Some(main) = weak.upgrade() {
            main.global::<PanelChoices>().set_mask_token("".into());
        }
    });
    choices.set_mask_token("mask".into());
    assert_tab_skips_routers(&host, "OK", "Select by mask", 12);
    host.key(Key::Escape);
    assert!(!host.is_open(), "Select by mask: Escape still closes it");
}

#[test]
fn the_main_menu_routes_keys_without_a_tab_stop() {
    let host = form::Host::new();
    host.main.invoke_show_app_menu(2);
    // Enough presses to wrap around all of its actions both ways.
    assert_tab_skips_routers(&host, "Close menu", "Main menu", 100);
    host.key(Key::Escape);
    assert!(!host.is_open(), "Main menu: Escape still closes it");
}

#[test]
fn the_colour_picker_routes_keys_without_a_tab_stop() {
    let host = form::Host::new();
    host.main.set_options_open(true);
    host.layout(1000, 700);
    let colors = host.main.global::<ColorOptionsView>();
    colors.set_picker_field(0);
    assert_eq!(
        popup_count(&host),
        2,
        "the colour picker opens over Options"
    );
    assert_tab_skips_routers(&host, "Use color", "Colour picker", 16);
    host.key(Key::Escape);
    assert_eq!(
        colors.get_picker_field(),
        -1,
        "Escape still closes the picker"
    );
    assert_eq!(popup_count(&host), 1);
}

#[test]
fn the_shortcut_replacement_question_routes_keys_without_a_tab_stop() {
    let host = form::Host::new();
    host.main.set_options_open(true);
    host.layout(1000, 700);
    let keyboard = host.main.global::<KeyboardEditorView>();
    let decided = Rc::new(RefCell::new(Vec::new()));
    let weak = host.main.as_weak();
    keyboard.on_decide_replacement({
        let decided = decided.clone();
        move |token, replace| {
            decided.borrow_mut().push((token.to_string(), replace));
            if let Some(main) = weak.upgrade() {
                main.global::<KeyboardEditorView>()
                    .set_replacement_token("".into());
            }
        }
    });
    keyboard.set_replacement_body("Ctrl+K is assigned to Find files.".into());
    keyboard.set_replacement_token("replace-1".into());
    assert_eq!(popup_count(&host), 2, "the replacement question opens");
    assert_tab_skips_routers(&host, "Replace", "Shortcut replacement", 8);
    host.key(Key::Escape);
    assert_eq!(
        *decided.borrow(),
        [("replace-1".to_owned(), false)],
        "Escape still keeps the current shortcuts"
    );
    assert_eq!(popup_count(&host), 1);
}

// ---------------------------------------------------------------------------------
// U-D07: Make File List's fields select with the theme's selection colours.

fn luminance(color: Color) -> f64 {
    let linear = |channel: u8| {
        let channel = f64::from(channel) / 255.0;
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.red()) + 0.7152 * linear(color.green()) + 0.0722 * linear(color.blue())
}

fn contrast(first: Color, second: Color) -> f64 {
    let (first, second) = (luminance(first), luminance(second));
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

/// `top` drawn with `alpha` over `under`.
fn blend(top: Color, alpha: f64, under: Color) -> Color {
    let mix = |top: u8, under: u8| {
        (f64::from(top) * alpha + f64::from(under) * (1.0 - alpha)).round() as u8
    };
    Color::from_rgb_u8(
        mix(top.red(), under.red()),
        mix(top.green(), under.green()),
        mix(top.blue(), under.blue()),
    )
}

/// The text input inside the field `label`.
fn field_input(host: &form::Host, label: &str) -> ItemRc {
    let field = host.item(AccessibleRole::TextInput, label);
    shown_items(&field)
        .into_iter()
        .find(|item| item.downcast::<TextInput>().is_some())
        .unwrap_or_else(|| panic!("{label} has no text input"))
}

#[test]
fn make_file_list_fields_select_with_theme_colours_in_every_theme() {
    let host = form::Host::new();
    let list = host.main.global::<FileListPresentation>();
    list.set_destination(2);
    list.set_destination_path("C:\\out\\list 文档.txt".into());
    list.set_preview_text("文档.txt\nnotes.txt\n".into());
    list.set_token("file-list".into());
    list.invoke_begin();
    for scheme in 0..26 {
        host.apply_preferences(scheme, 0, "", 12, 15);
        host.layout(1000, 700);
        let theme = host.main.global::<Theme>();
        for label in [
            "File list line template",
            "File list destination path",
            "File list preview",
        ] {
            let item = field_input(&host, label);

            let input = item.downcast::<TextInput>().expect("a text input");
            let input = input.as_pin_ref();
            let (foreground, background) = (
                input.selection_foreground_color(),
                input.selection_background_color(),
            );
            assert_eq!(
                (foreground, background),
                (theme.get_selection_text(), theme.get_selection_background()),
                "{label}, theme {scheme}: the selection uses the theme's colours"
            );
            let alpha = f64::from(input.color().color().alpha()) / 255.0;
            let shown = blend(foreground, alpha, background);
            let ratio = contrast(shown, background);
            assert!(
                ratio >= 4.5,
                "{label}, theme {scheme}: selected text {shown:?} on {background:?} is {ratio:.2}:1"
            );
        }
    }
}

//! The UI polish gate's findings on the main window (20 November 2026): panel controls
//! without a hover state, a function key cut to a sliver, status fields cut by their
//! strip, a zero-width time column, and a shortened name, a retry button and the panel
//! splitter without a tooltip.
use crate::support::ui;

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, ComplexText, Opacity, SimpleText},
    window::{WindowInner, WindowKind},
};
use midnight_ui::{DetailRow, FileRow, PathPartRow};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel, platform::WindowEvent};
use std::{cell::Cell, time::Duration};

/// Window rectangle: x, y, width, height.
type Rect = (f32, f32, f32, f32);

/// A folder whose listing failed, with a name too long for its row.
const LONG_FOLDER: &str =
    "a-folder-whose-long-name-makes-the-error-path-long-enough-to-wrap-in-a-notice";
const FREE_SPACE: &str = "9.5 GB free";

fn file(identity: &str, name: &str) -> FileRow {
    FileRow {
        identity: identity.into(),
        name: name.into(),
        size: "1 234".into(),
        modified_date: "2026-11-20".into(),
        modified_time: "14:03:22".into(),
        ..Default::default()
    }
}

/// Both panels: a failed folder (a retry button and an error icon), files with a date
/// and a time, the cursor's details with the free space, and a path too long for its bar.
fn furnish(test: &ui::TestUi) {
    let failed = FileRow {
        directory: true,
        tree_disclosure: 6,
        tree_error: "Permission denied".into(),
        ..file("failed", LONG_FOLDER)
    };
    let cursor = FileRow {
        cursor: true,
        ..file("alpha", "alpha.bin")
    };
    let rows = || {
        ModelRc::new(VecModel::from(vec![
            failed.clone(),
            cursor.clone(),
            file("beta", "beta.txt"),
        ]))
    };
    let details = || {
        let fields = [
            ("Name", "alpha.bin"),
            ("Bytes", "1,234"),
            ("Date", "2026-11-20"),
            ("Time", "14:03:22"),
            ("Permissions", "rw-r--r--"),
        ];
        ModelRc::new(VecModel::from(
            fields
                .into_iter()
                .enumerate()
                .map(|(index, (label, value))| DetailRow {
                    identity: format!("detail:{index}").into(),
                    label: label.into(),
                    value: value.into(),
                    ..Default::default()
                })
                .collect::<Vec<_>>(),
        ))
    };
    let mut prefix = String::new();
    let parts = [
        "C:\\",
        "Users\\",
        "Owner\\",
        "Documents\\",
        "a-rather-long-project-directory\\",
        "日本語-sources-of-the-project\\",
        "november-polish-gate-review\\",
        "screenshots-and-findings\\",
        "main-window-states",
    ]
    .iter()
    .enumerate()
    .map(|(index, label)| {
        prefix.push_str(label);
        PathPartRow {
            identity: format!("part-{index}").into(),
            label: (*label).into(),
            prefix: prefix.as_str().into(),
        }
    })
    .collect::<Vec<_>>();
    test.window.set_left_rows(rows());
    test.window.set_right_rows(rows());
    test.window.set_left_details(details());
    test.window.set_right_details(details());
    test.window.set_left_free_space(FREE_SPACE.into());
    test.window.set_right_free_space(FREE_SPACE.into());
    // The presenter's samples of the widest values size the metadata columns.
    let (size, date, time) = ("1 024 KB", "2000-12-28", "23:58:58");
    test.window.set_left_size_column_sample(size.into());
    test.window.set_right_size_column_sample(size.into());
    test.window.set_left_date_column_sample(date.into());
    test.window.set_right_date_column_sample(date.into());
    test.window.set_left_time_column_sample(time.into());
    test.window.set_right_time_column_sample(time.into());
    test.window.set_left_path(prefix.as_str().into());
    test.window
        .set_left_path_parts(ModelRc::new(VecModel::from(parts)));
}

fn root(test: &ui::TestUi) -> ItemRc {
    ItemRc::new_root(WindowInner::from_pub(test.window.window()).component())
}

fn walk(item: &ItemRc, visit: &mut dyn FnMut(&ItemRc)) {
    visit(item);
    let mut child = item.first_child();
    while let Some(next) = child {
        child = next.next_sibling();
        walk(&next, visit);
    }
}

fn label(item: &ItemRc) -> String {
    item.accessible_string_property(AccessibleStringProperty::Label)
        .map(|label| label.to_string())
        .unwrap_or_default()
}

fn flag(item: &ItemRc, property: AccessibleStringProperty) -> bool {
    item.accessible_string_property(property)
        .is_some_and(|value| value == "true")
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

/// Where the clipping ancestors of `item` let it draw: None when they hide all of it.
/// The outer Option is None when no ancestor clips.
fn clip(item: &ItemRc) -> Option<Option<Rect>> {
    let mut area: Option<Option<Rect>> = None;
    let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(ancestor) = parent {
        if ancestor.borrow().as_ref().clips_children() {
            let rect = bounds(&ancestor);
            area = Some(match area {
                None => Some(rect),
                Some(inner) => inner.and_then(|inner| intersect(inner, rect)),
            });
        }
        parent = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    area
}

/// The part of `item` that shows: None when it is hidden or clipped away.
fn shown(item: &ItemRc) -> Option<Rect> {
    let area = bounds(item);
    if !item.is_visible() || area.2 <= 0.0 || area.3 <= 0.0 {
        return None;
    }
    match clip(item) {
        None => Some(area),
        Some(clip) => clip.and_then(|clip| intersect(area, clip)),
    }
}

fn whole(item: &ItemRc, area: Rect) -> bool {
    let full = bounds(item);
    (area.2 - full.2).abs() < 0.5 && (area.3 - full.3).abs() < 0.5
}

fn center((x, y, w, h): Rect) -> slint::LogicalPosition {
    slint::LogicalPosition::new(x + w / 2.0, y + h / 2.0)
}

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

/// Whether hovering the centre of `item` changes how it (or its margin) looks, after
/// the 200 ms that Fluent's hover animations take.
fn answers_hovering(test: &mut ui::TestUi, clock: &Cell<Duration>, item: &ItemRc) -> bool {
    let area = shown(item).unwrap_or_else(|| panic!("{} is not shown", label(item)));
    pointer_out(test, clock);
    let before = region(test, area);
    test.window
        .window()
        .dispatch_event(WindowEvent::PointerMoved {
            position: center(area),
        });
    ui::advance(clock, Duration::from_millis(200));
    test.draw();
    let after = region(test, area);
    pointer_out(test, clock);
    before != after
}

/// The texts of the tooltip that hovering `item` shows (none: empty).
fn tooltip_after_hover(
    test: &mut ui::TestUi,
    clock: &Cell<Duration>,
    item: &ItemRc,
) -> Vec<SharedString> {
    let area = shown(item).unwrap_or_else(|| panic!("{} is not shown", label(item)));
    pointer_out(test, clock);
    test.window
        .window()
        .dispatch_event(WindowEvent::PointerMoved {
            position: center(area),
        });
    // Past the tooltip's delay.
    ui::advance(clock, Duration::from_millis(1200));
    test.draw();
    let texts = WindowInner::from_pub(test.window.window())
        .active_popups()
        .iter()
        .filter(|popup| matches!(popup.window_kind, WindowKind::ToolTip))
        .flat_map(|popup| ui::visible_text_values(ItemRc::new_root(popup.component.clone())))
        .filter(|text| !text.trim().is_empty())
        .collect();
    pointer_out(test, clock);
    texts
}

#[test]
fn panel_controls_that_showed_no_hover_state_answer_hovering() {
    let (mut test, clock) = ui::TestUi::with_virtual_clock();
    furnish(&test);
    test.draw();
    let capacity = format!("{FREE_SPACE}. Copy to clipboard");
    for label in [
        "Sort by Name",
        "Sort by Extension",
        "Sort by Size",
        "Sort by Modified",
        "Edit folder path",
        "Edit full folder path",
        capacity.as_str(),
        "Folder listing error",
    ] {
        let item = ui::root_item(&test.window, label, AccessibleRole::Button);
        assert!(
            answers_hovering(&mut test, &clock, &item),
            "{label} does not change when hovered"
        );
    }
}

/// Every control a user can operate in the main window (the gate's U-K30 sweep): shown,
/// enabled, not the active page or choice, and not inside another control.
fn operable(item: &ItemRc, inside: bool, found: &mut Vec<ItemRc>) {
    let role = item.accessible_role();
    let interactive = matches!(
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
    );
    if item
        .accessible_string_property(AccessibleStringProperty::Enabled)
        .is_some_and(|value| value == "false")
    {
        return;
    }
    if interactive
        && !inside
        && !matches!(
            role,
            AccessibleRole::Slider | AccessibleRole::Spinbox | AccessibleRole::TextInput
        )
        && !(flag(item, AccessibleStringProperty::Checkable)
            && flag(item, AccessibleStringProperty::Checked))
        && !flag(item, AccessibleStringProperty::ItemSelected)
        && shown(item).is_some_and(|(_, _, w, h)| w >= 2.0 && h >= 2.0)
        && !transparent(item)
    {
        found.push(item.clone());
    }
    let mut child = item.first_child();
    while let Some(next) = child {
        child = next.next_sibling();
        operable(&next, inside || interactive, found);
    }
}

#[test]
fn every_operable_main_window_control_answers_hovering() {
    let (mut test, clock) = ui::TestUi::with_virtual_clock();
    furnish(&test);
    test.draw();
    let mut controls = Vec::new();
    operable(&root(&test), false, &mut controls);
    assert!(controls.len() > 20, "only {} controls", controls.len());
    let silent: Vec<String> = controls
        .iter()
        .filter(|item| !answers_hovering(&mut test, &clock, item))
        .map(|item| {
            format!(
                "{:?} {:?} at {:?}",
                item.accessible_role(),
                label(item),
                bounds(item)
            )
        })
        .collect();
    assert!(silent.is_empty(), "no hover feedback: {silent:#?}");
}

#[test]
fn narrow_function_bar_shows_whole_keys_only() {
    let mut test = ui::TestUi::new();
    for density in [0, 1] {
        for network in [false, true] {
            test.window.set_network_available(network);
            test.window.invoke_apply_preferences(0, density);
            test.resize(320, 200);
            let case = format!("density {density}, network {network}, 320×200");
            let mut keys = Vec::new();
            walk(&root(&test), &mut |item| {
                let name = label(item);
                if item.accessible_role() == AccessibleRole::Button
                    && (name == "Jobs"
                        || name.strip_prefix('F').is_some_and(|rest| {
                            rest.starts_with(|first: char| first.is_ascii_digit())
                        }))
                {
                    keys.push((name, item.clone()));
                }
            });
            assert!(keys.len() >= 11, "{case}: {} keys", keys.len());
            let (mut shown_keys, mut scrolled) = (0, 0);
            // A key hidden with its action (unavailable network) has no width.
            for (name, item) in keys.iter().filter(|(_, item)| bounds(item).2 > 0.0) {
                match shown(item) {
                    Some(area) => {
                        assert!(
                            whole(item, area),
                            "{case}: {name} shows {area:?} of {:?}",
                            bounds(item)
                        );
                        shown_keys += 1;
                    }
                    None => {
                        assert_ne!(name, "Jobs", "{case}");
                        scrolled += 1;
                    }
                }
            }
            assert!(
                shown_keys > 1 && scrolled > 0,
                "{case}: {shown_keys} shown, {scrolled} scrolled"
            );
        }
    }
}

/// Visible texts that the window draws with no width or height (the gate's U-K18).
fn zero_size_texts(test: &ui::TestUi) -> Vec<String> {
    let mut found = Vec::new();
    walk(&root(test), &mut |item| {
        let text = item
            .downcast::<SimpleText>()
            .map(|text| text.as_pin_ref().text())
            .or_else(|| {
                item.downcast::<ComplexText>()
                    .map(|text| text.as_pin_ref().text())
            });
        let Some(text) = text else { return };
        if text.trim().is_empty() || !item.is_visible() || transparent(item) {
            return;
        }
        let (_, _, w, h) = bounds(item);
        let clipped_away = matches!(clip(item), Some(None));
        if (w <= 0.0 || h <= 0.0) && !clipped_away {
            found.push(format!("{text:?} is {w}x{h}"));
        }
    });
    found
}

fn heading_shown(test: &ui::TestUi, label: &str) -> bool {
    ui::try_root_item(&test.window, label, AccessibleRole::Button)
        .is_some_and(|item| shown(&item).is_some())
}

/// Lay the furnished window out at Dark, Touch spacing (the gate's case was 960×600)
/// and at narrower widths, which hide more, and check each one.
fn touch_widths(mut check: impl FnMut(&ui::TestUi, &str)) {
    let mut test = ui::TestUi::new();
    furnish(&test);
    test.window.invoke_apply_preferences(1, 1);
    for width in (640..=960).rev().step_by(8) {
        test.set_size(width, 600);
        test.layout();
        check(&test, &format!("Dark Touch {width}×600"));
    }
}

#[test]
fn touch_status_bar_shows_whole_fields_only() {
    let mut field_waiting = false;
    touch_widths(|test, case| {
        let mut fields = Vec::new();
        walk(&root(test), &mut |item| {
            if item.accessible_role() == AccessibleRole::Button
                && label(item).ends_with(". Copy to clipboard")
            {
                fields.push(item.clone());
            }
        });
        assert!(fields.len() >= 12, "{case}: {} status fields", fields.len());
        // The strip keeps an empty copy of the name field, which the bar shows apart.
        for item in fields.iter().filter(|item| bounds(item).2 > 0.0) {
            match shown(item) {
                Some(area) => assert!(
                    whole(item, area),
                    "{case}: {:?} shows {area:?} of {:?}",
                    label(item),
                    bounds(item)
                ),
                None => field_waiting = true,
            }
        }
    });
    assert!(field_waiting, "no width left a status field out of view");
}

#[test]
fn a_hidden_time_column_leaves_no_zero_width_text() {
    let mut time_hidden_alone = false;
    // Window width, panel width, then whether the date and the time columns show.
    let mut columns = Vec::new();
    touch_widths(|test, case| {
        let zero = zero_size_texts(test);
        assert!(zero.is_empty(), "{case}: zero-size texts {zero:?}");
        let date = heading_shown(test, "Sort by Modified");
        let time = heading_shown(test, "Sort by Modified time");
        columns.push((test.width, test.window.get_left_pane_width(), date, time));
        time_hidden_alone |= date && !time;
    });
    assert!(
        time_hidden_alone,
        "no width showed the date column without its time: {columns:?}"
    );
}

#[test]
fn shortened_name_retry_button_and_splitter_show_tooltips() {
    let (mut test, clock) = ui::TestUi::with_virtual_clock();
    furnish(&test);
    test.draw();
    let retry = format!("Retry folders in {LONG_FOLDER}");
    let mut missing = Vec::new();
    for (role, name, expected) in [
        (AccessibleRole::Text, LONG_FOLDER, LONG_FOLDER),
        (AccessibleRole::Button, retry.as_str(), "Retry folders in"),
        (
            AccessibleRole::Button,
            "Panel splitter",
            "resize the panels",
        ),
    ] {
        let item = ui::root_item(&test.window, name, role);
        let tip = tooltip_after_hover(&mut test, &clock, &item);
        if !tip.iter().any(|text| text.contains(expected)) {
            missing.push(format!("{name}: tooltip {tip:?}"));
        }
    }
    assert!(
        missing.is_empty(),
        "no tooltip with the expected text: {missing:#?}"
    );
    // A name that fits needs no tooltip of its own.
    let row = ui::root_item(&test.window, "alpha.bin", AccessibleRole::ListItem);
    let mut name = None;
    walk(&row, &mut |item| {
        if name.is_none()
            && item.accessible_role() == AccessibleRole::Text
            && label(item) == "alpha.bin"
        {
            name = Some(item.clone());
        }
    });
    let tip = tooltip_after_hover(&mut test, &clock, &name.expect("name text"));
    assert!(tip.is_empty(), "alpha.bin: tooltip {tip:?}");
}

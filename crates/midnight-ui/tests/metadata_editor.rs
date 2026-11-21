//! Exercise the actual merged editor, including deliberate preview before Apply, the
//! time rows that tick themselves when edited, and the time picker's spinners and the
//! calendar with the pointer, the wheel and the keyboard.
#[path = "support/metadata_form.rs"]
mod metadata_form;
#[path = "support/render.rs"]
mod render;
use crate::support::ui;
use i_slint_core::{
    accessibility::AccessibleStringProperty, item_tree::ItemRc, items::AccessibleRole,
    window::WindowInner,
};
use metadata_form::Editor;
use midnight_core::{MetadataCapabilities, TimeEdit};
use midnight_ui::{MetadataDraft, Theme};
use slint::{ComponentHandle, platform::Key};
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, UNIX_EPOCH},
};

#[test]
fn unix_permissions_offer_owner_group_other_and_special_bits() {
    let mut test = ui::TestUi::new();
    test.window.set_metadata_can_unix(true);
    test.window.set_metadata_token("unix-grid:1".into());
    settle(&mut test);
    for label in [
        "Owner read",
        "Owner write",
        "Owner execute",
        "Group read",
        "Group write",
        "Group execute",
        "Other read",
        "Other write",
        "Other execute",
        "SUID",
        "SGID",
        "Sticky",
    ] {
        ui::popup_item(&test.window, label, AccessibleRole::Checkbox);
    }
}

fn settle(test: &mut ui::TestUi) {
    // Slint repositions embedded popups through a deferred property-tracker timer.
    for _ in 0..2 {
        slint::platform::update_timers_and_animations();
        test.draw();
    }
}

fn choose_attribute(test: &ui::TestUi, field: &str, value: &str) {
    for _ in 0..3 {
        let item = ui::popup_item(&test.window, field, AccessibleRole::Checkbox);
        if item
            .accessible_string_property(AccessibleStringProperty::Value)
            .unwrap()
            == value
        {
            return;
        }
        ui::click_popup(&test.window, field, AccessibleRole::Checkbox);
    }
    panic!("attribute {field} cannot reach {value}");
}

fn action_is_not_vertically_clipped(test: &ui::TestUi, label: &str) {
    use i_slint_core::{item_tree::ParentItemTraversalMode, items::Flickable};
    let action = ui::popup_item(&test.window, label, AccessibleRole::Button);
    let origin = action.map_to_native_window(action.geometry().origin);
    let mut parent = action.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(item) = parent {
        if item.downcast::<Flickable>().is_some() {
            let clip = item.map_to_native_window(item.geometry().origin);
            assert!(
                origin.y >= clip.y
                    && origin.y + action.geometry().height()
                        <= clip.y + item.geometry().height() + 0.5,
                "{label} is clipped vertically: action={origin:?} {:?}, viewport={clip:?} {:?}",
                action.geometry(),
                item.geometry()
            );
        }
        parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
}

fn has_label(item: ItemRc, label: &str) -> bool {
    if item
        .accessible_string_property(AccessibleStringProperty::Label)
        .is_some_and(|value| value == label)
    {
        return true;
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if has_label(item, label) {
            return true;
        }
    }
    false
}

#[test]
fn attributes_use_one_compact_three_state_control_each() {
    let mut test = ui::TestUi::new();
    test.window.set_metadata_can_windows(true);
    test.window.set_metadata_windows_mask(39);
    test.window.set_metadata_can_compress(true);
    test.window.set_metadata_token("compact:1".into());
    settle(&mut test);
    let drafts = Rc::new(RefCell::new(Vec::<MetadataDraft>::new()));
    let captured = drafts.clone();
    test.window
        .on_preview_metadata(move |_, draft| captured.borrow_mut().push(draft));
    for label in ["Read-only", "Hidden", "System", "Archive", "Compression"] {
        let control = ui::popup_item(&test.window, label, AccessibleRole::Checkbox);
        assert!(
            control.geometry().height() <= 30.0,
            "attribute row should be compact"
        );
        assert_eq!(
            control
                .accessible_string_property(AccessibleStringProperty::Value)
                .unwrap()
                .as_str(),
            "Unchanged"
        );
    }
    for (expected, value) in [(1, "Set"), (2, "Clear"), (0, "Unchanged")] {
        ui::click_popup(&test.window, "Read-only", AccessibleRole::Checkbox);
        let control = ui::popup_item(&test.window, "Read-only", AccessibleRole::Checkbox);
        assert_eq!(
            control
                .accessible_string_property(AccessibleStringProperty::Value)
                .unwrap()
                .as_str(),
            value
        );
        ui::click_popup(&test.window, "Review changes", AccessibleRole::Button);
        assert_eq!(drafts.borrow().last().unwrap().read_only, expected);
    }
    ui::key(&test.window, Key::Escape);
}

#[test]
fn compact_attribute_keyboard_and_capabilities_preserve_leave_semantics() {
    let mut test = ui::TestUi::new();
    test.window.set_metadata_can_windows(true);
    test.window.set_metadata_windows_mask(1);
    test.window.set_metadata_token("keyboard:1".into());
    settle(&mut test);
    let hidden = ui::popup_item(&test.window, "Hidden", AccessibleRole::Checkbox);
    assert_eq!(
        hidden
            .accessible_string_property(AccessibleStringProperty::Enabled)
            .unwrap(),
        "false"
    );
    assert_eq!(
        hidden
            .accessible_string_property(AccessibleStringProperty::Value)
            .unwrap(),
        "Unavailable"
    );
    drop(hidden);
    ui::click_popup(&test.window, "Read-only", AccessibleRole::Checkbox);
    ui::key(&test.window, Key::Space);
    let value = || {
        ui::popup_item(&test.window, "Read-only", AccessibleRole::Checkbox)
            .accessible_string_property(AccessibleStringProperty::Value)
            .unwrap()
    };
    assert_eq!(value(), "Clear");
    ui::chord(&test.window, &[Key::Shift], Key::Space);
    assert_eq!(value(), "Set");
    ui::key(&test.window, Key::Delete);
    assert_eq!(value(), "Unchanged");
}

#[test]
fn metadata_edit_sections_exclude_obsolete_preview_and_require_prepared_confirmation() {
    let mut test = ui::TestUi::new();
    test.window.set_metadata_can_windows(true);
    test.window.set_metadata_can_modified(true);
    test.window.set_metadata_can_rename(true);
    test.window.set_metadata_token("review:1".into());
    settle(&mut test);
    let root = {
        let popups = WindowInner::from_pub(test.window.window()).active_popups();
        ItemRc::new_root(popups.last().unwrap().component.clone())
    };
    assert!(
        !has_label(root.clone(), "Preview"),
        "obsolete Preview tab remains"
    );
    for tab in ["Attributes", "Times", "Case"] {
        assert!(has_label(root.clone(), tab));
    }
    let previews = Rc::new(RefCell::new(Vec::new()));
    let captured = previews.clone();
    test.window
        .on_preview_metadata(move |_, draft| captured.borrow_mut().push(draft));
    let applied = Rc::new(RefCell::new(false));
    let captured = applied.clone();
    test.window
        .on_confirm_metadata(move |_, _, _| *captured.borrow_mut() = true);
    ui::click_popup(&test.window, "Review changes", AccessibleRole::Button);
    assert_eq!(previews.borrow().len(), 1);
    assert!(
        !*applied.borrow(),
        "review must not apply an unprepared draft"
    );
    test.window.set_metadata_revision("4".into());
    test.window.set_metadata_ready(true);
    test.window.set_metadata_preview_current(true);
    settle(&mut test);
    ui::click_popup(&test.window, "Apply", AccessibleRole::Button);
    assert!(*applied.borrow());
}

#[test]
fn metadata_editor_requires_review_and_keeps_sections_readable() {
    let (mut test, clock) = ui::TestUi::with_virtual_clock();
    test.window.set_metadata_can_windows(true);
    test.window.set_metadata_windows_mask(39);
    test.window.set_metadata_can_compress(true);
    test.window.set_metadata_can_created(true);
    test.window.set_metadata_can_modified(true);
    test.window.set_metadata_can_accessed(true);
    test.window.set_metadata_can_rename(true);
    let drafts = Rc::new(RefCell::new(Vec::<MetadataDraft>::new()));
    let captured = drafts.clone();
    test.window
        .on_preview_metadata(move |_, draft| captured.borrow_mut().push(draft));
    let applies = Rc::new(RefCell::new(Vec::new()));
    let captured = applies.clone();
    test.window
        .on_confirm_metadata(move |token, revision, choice| {
            captured.borrow_mut().push((token, revision, choice))
        });
    let weak = test.window.as_weak();
    test.window
        .on_cancel_metadata(move |_| weak.upgrade().unwrap().set_metadata_token("".into()));

    test.window.set_metadata_start_case(true);
    test.window.set_metadata_summary("2 selected items".into());
    test.window.set_metadata_token("1".into());
    slint::platform::update_timers_and_animations();
    test.draw();
    let popups = WindowInner::from_pub(test.window.window()).active_popups();
    let text = ui::text_values(ItemRc::new_root(popups.last().unwrap().component.clone()));
    assert!(
        !text.iter().any(|value| value.as_str() == "Current name"),
        "the redundant captured-file preview list is no longer part of the editor"
    );
    drop(popups);
    let popups = WindowInner::from_pub(test.window.window()).active_popups();
    assert!(
        !has_label(
            ItemRc::new_root(popups.last().unwrap().component.clone()),
            "Apply"
        ),
        "Apply must only be offered after an explicit prepared review"
    );
    drop(popups);
    assert!(applies.borrow().is_empty(), "unreviewed draft cannot apply");
    ui::click_popup(&test.window, "Attributes", AccessibleRole::Button);
    choose_attribute(&test, "Read-only", "Set");
    choose_attribute(&test, "Hidden", "Clear");
    choose_attribute(&test, "Read-only", "Clear");
    settle(&mut test);
    render::snapshot(&test, "metadata-before-review");
    action_is_not_vertically_clipped(&test, "Review changes");
    ui::click_popup(&test.window, "Review changes", AccessibleRole::Button);
    assert_eq!(
        drafts
            .borrow()
            .last()
            .expect("Review validates the current draft")
            .read_only,
        2
    );
    assert_eq!(drafts.borrow().last().unwrap().hidden, 2);
    choose_attribute(&test, "Read-only", "Unchanged");
    choose_attribute(&test, "Compression", "Compress");
    choose_attribute(&test, "Compression", "Uncompress");
    ui::click_popup(&test.window, "Review changes", AccessibleRole::Button);
    assert_eq!(drafts.borrow().last().unwrap().read_only, 0);
    assert_eq!(drafts.borrow().last().unwrap().hidden, 2);
    assert_eq!(drafts.borrow().last().unwrap().compressed, 2);
    ui::click_popup(&test.window, "Times", AccessibleRole::Button);
    ui::click_popup(&test.window, "Change Modified", AccessibleRole::Checkbox);
    ui::click_popup(&test.window, "Modified: Now", AccessibleRole::Button);
    ui::click_popup(&test.window, "Review changes", AccessibleRole::Button);
    // Now fills in a time to apply; nothing is left for the moment of review.
    assert_eq!(drafts.borrow().last().unwrap().modified.mode, 1);
    assert_eq!(drafts.borrow().last().unwrap().created.mode, 0);
    ui::click_popup(&test.window, "Times", AccessibleRole::Button);
    ui::click_popup(&test.window, "Change Modified", AccessibleRole::Checkbox);
    ui::click_popup(&test.window, "Review changes", AccessibleRole::Button);
    assert_eq!(drafts.borrow().last().unwrap().modified.mode, 0);
    ui::click_popup(&test.window, "Review changes", AccessibleRole::Button);
    assert!(
        drafts
            .borrow()
            .last()
            .expect("preview callback")
            .case_enabled
    );
    ui::click_popup(&test.window, "Case", AccessibleRole::Button);
    ui::click_popup(&test.window, "Conversion", AccessibleRole::Combobox);
    ui::key(&test.window, Key::Home);
    ui::key(&test.window, Key::DownArrow);
    ui::key(&test.window, Key::Return);
    ui::click_popup(&test.window, "Review changes", AccessibleRole::Button);
    assert_eq!(drafts.borrow().last().unwrap().case_mode, 1);
    test.window.set_metadata_revision("7".into());
    test.window.set_metadata_ready(true);
    test.window.set_metadata_preview_current(true);
    settle(&mut test);
    ui::click_popup(&test.window, "Apply", AccessibleRole::Button);
    assert_eq!(applies.borrow()[0].0.as_str(), "1");
    assert_eq!(applies.borrow()[0].1.as_str(), "7");
    ui::click_popup(&test.window, "Back to editing", AccessibleRole::Button);

    // Recursion belongs to the whole prepared operation, including edits on other topics.
    ui::click_popup(&test.window, "Include subfolders", AccessibleRole::Checkbox);
    for tab in ["Times", "Case", "Attributes"] {
        ui::click_popup(&test.window, tab, AccessibleRole::Button);
        ui::click_popup(&test.window, "Review changes", AccessibleRole::Button);
        assert!(drafts.borrow().last().unwrap().recursive);
    }
    ui::click_popup(&test.window, "Include subfolders", AccessibleRole::Checkbox);
    ui::click_popup(&test.window, "Review changes", AccessibleRole::Button);
    assert!(!drafts.borrow().last().unwrap().recursive);

    test.resize(1040, 720);
    settle(&mut test);
    for density in [0, 1] {
        for font in [11, 32] {
            test.window.invoke_apply_preferences(7, density);
            test.window.invoke_apply_fonts("".into(), font, font);
            // Let Fluent's colour animations settle (and keep the tab clicks below
            // single clicks).
            ui::advance(&clock, Duration::from_millis(600));
            settle(&mut test);
            for tab in ["Attributes", "Times", "Case"] {
                ui::click_popup(&test.window, tab, AccessibleRole::Button);
                settle(&mut test);
                render::snapshot(&test, &format!("metadata-{tab}-{density}-{font}"));
                if tab == "Times" {
                    let explanation = ui::popup_item(
                        &test.window,
                        "Times start from the current value, and Now fills in the current one. The UTC offset is used exactly as shown: after moving a date into or out of daylight-saving time, change the offset too. Review notes any offset that differs from local time.",
                        AccessibleRole::Text,
                    );
                    let status = ui::popup_item(
                        &test.window,
                        "Draft changed. Review it again before applying.",
                        AccessibleRole::Text,
                    );
                    let body = explanation.geometry();
                    let bottom = explanation.map_to_native_window(body.origin).y + body.height();
                    let status_top = status.map_to_native_window(status.geometry().origin).y;
                    assert!(
                        bottom <= status_top,
                        "wrapped time help overlaps status: density={density}, font={font}, help_bottom={bottom}, status_top={status_top}"
                    );
                }
                let apply = ui::popup_item(&test.window, "Review changes", AccessibleRole::Button);
                action_is_not_vertically_clipped(&test, "Review changes");
                let area = apply.geometry();
                let origin = apply.map_to_native_window(area.origin);
                assert!(area.size.width > 0.0 && area.size.height > 0.0);
                assert!(
                    origin.y >= 0.0 && origin.y + area.size.height <= test.height as f32,
                    "{tab}, density={density}, font={font}: origin={origin:?}, bounds={area:?}"
                );
                assert!(
                    origin.x >= 0.0 && origin.x + area.size.width <= test.width as f32,
                    "{tab}, density={density}, font={font}: origin={origin:?}, bounds={area:?}"
                );
            }
            ui::click_popup(&test.window, "Review changes", AccessibleRole::Button);
            assert_eq!(
                drafts.borrow().last().unwrap().case_mode,
                1,
                "resize/font changes preserve the edited draft"
            );
            test.window.set_metadata_error(
                format!(
                    "Access denied\n\nPath\n資料/{}/žluťoučký.txt\n\nOperation: inspect metadata",
                    "long folder/".repeat(30)
                )
                .into(),
            );
            settle(&mut test);
            render::snapshot(&test, &format!("metadata-error-{density}-{font}"));
            let error = ui::popup_item(
                &test.window,
                &format!(
                    "Access denied\n\nPath\n資料/{}/žluťoučký.txt\n\nOperation: inspect metadata",
                    "long folder/".repeat(30)
                ),
                AccessibleRole::Text,
            );
            let mut ancestor =
                error.parent_item(i_slint_core::item_tree::ParentItemTraversalMode::StopAtPopups);
            let viewport = loop {
                let item = ancestor.expect("error text has a scroll viewport");
                if item.downcast::<i_slint_core::items::Flickable>().is_some() {
                    break item;
                }
                ancestor = item
                    .parent_item(i_slint_core::item_tree::ParentItemTraversalMode::StopAtPopups);
            };
            assert!(
                error.geometry().width() >= viewport.geometry().width() - 24.0,
                "error text must fill the available compact viewport"
            );
            let apply = ui::popup_item(&test.window, "Review changes", AccessibleRole::Button);
            let area = apply.geometry();
            let origin = apply.map_to_native_window(area.origin);
            assert!(
                origin.y >= 0.0 && origin.y + area.size.height <= test.height as f32,
                "error density={density}, font={font}: origin={origin:?}, bounds={area:?}"
            );
            test.window.set_metadata_error("".into());
            settle(&mut test);
        }
    }
    for (density, font) in [(0, 11), (1, 32)] {
        test.window.invoke_apply_preferences(0, density);
        test.window.invoke_apply_fonts("".into(), font, font);
        test.resize(320, 200);
        settle(&mut test);
        for action in ["Review changes", "Cancel"] {
            action_is_not_vertically_clipped(&test, action);
        }
        render::snapshot(&test, &format!("metadata-minimum-{density}-{font}"));
    }
    ui::key(&test.window, Key::Escape);
    slint::platform::update_timers_and_animations();
    assert!(test.window.get_metadata_token().is_empty());
    test.window.hide().unwrap();
}

#[test]
fn timestamp_fields_are_single_line_and_refresh_has_no_duplicate_action() {
    let mut test = ui::TestUi::new();
    test.window.set_metadata_can_modified(true);
    test.window.set_metadata_can_created(true);
    test.window.set_metadata_can_accessed(true);
    test.window.set_metadata_token("times:1".into());
    settle(&mut test);
    ui::click_popup(&test.window, "Times", AccessibleRole::Button);
    for (density, font, width, height) in
        [(0, 11, 1340, 820), (1, 32, 1340, 960), (0, 13, 600, 700)]
    {
        test.window.invoke_apply_preferences(0, density);
        test.window.invoke_apply_fonts("".into(), font, font);
        test.resize(width, height);
        settle(&mut test);
        for label in ["Modified", "Created", "Accessed"] {
            let now = ui::popup_item(
                &test.window,
                &format!("{label}: Now"),
                AccessibleRole::Button,
            );
            for part in ["date", "time", "UTC offset"] {
                let field = ui::popup_item(
                    &test.window,
                    &format!("{label} {part}"),
                    AccessibleRole::TextInput,
                );
                assert!(
                    field.geometry().height() <= now.geometry().height() + 1.0,
                    "{label} {part} must not stretch into a tall editor: {:?} vs {:?}",
                    field.geometry(),
                    now.geometry()
                );
                assert!(field.geometry().width() > 20.0, "{label} {part} is usable");
            }
            for picker in ["choose date", "choose time"] {
                ui::popup_item(
                    &test.window,
                    &format!("{label}: {picker}"),
                    AccessibleRole::Button,
                );
            }
        }
        let inner = WindowInner::from_pub(test.window.window());
        for popup in inner.active_popups().iter() {
            assert!(!has_label(
                ItemRc::new_root(popup.component.clone()),
                "Refresh"
            ));
        }
        action_is_not_vertically_clipped(&test, "Review changes");
    }
    ui::key(&test.window, Key::Escape);
}

/// The arrows of the time picker's hour, minute and second columns.
const ARROWS: [&str; 6] = [
    "Increase Hour",
    "Decrease Hour",
    "Increase Minute",
    "Decrease Minute",
    "Increase Second",
    "Decrease Second",
];

/// Window rectangle: left, top, width, height.
type Rect = (f32, f32, f32, f32);

fn bounds(item: &ItemRc) -> Rect {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    (origin.x, origin.y, rect.width(), rect.height())
}

fn within(inner: Rect, outer: Rect) -> bool {
    inner.0 >= outer.0 - 0.01
        && inner.1 >= outer.1 - 0.01
        && inner.0 + inner.2 <= outer.0 + outer.2 + 0.01
        && inner.1 + inner.3 <= outer.1 + outer.3 + 0.01
}

/// The rendered pixels of `area`, scaled to device pixels.
fn region(test: &ui::TestUi, (x, y, w, h): Rect) -> Vec<slint::Rgb8Pixel> {
    let scale = test.window.window().scale_factor();
    let left = (x * scale).floor().max(0.0) as usize;
    let top = (y * scale).floor().max(0.0) as usize;
    let right = (((x + w) * scale).ceil() as usize).min(test.width);
    let bottom = (((y + h) * scale).ceil() as usize).min(test.height);
    (top..bottom)
        .flat_map(|row| (left..right).map(move |column| row * test.width + column))
        .map(|index| test.pixels[index])
        .collect()
}

fn colour(pixel: slint::Rgb8Pixel) -> slint::Color {
    slint::Color::from_rgb_u8(pixel.r, pixel.g, pixel.b)
}

fn luminance(color: slint::Color) -> f64 {
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

fn contrast(first: slint::Color, second: slint::Color) -> f64 {
    let (first, second) = (luminance(first), luminance(second));
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

/// Of the 3 by 3 pixels around a window point, the one that stands out most from `on`.
fn strongest(test: &ui::TestUi, x: f32, y: f32, on: slint::Color) -> slint::Color {
    let (x, y) = (x.floor() as usize, y.floor() as usize);
    (y.saturating_sub(1)..=y + 1)
        .flat_map(|row| (x.saturating_sub(1)..=x + 1).map(move |column| (row, column)))
        .map(|(row, column)| colour(test.pixels[row * test.width + column]))
        .max_by(|a, b| contrast(*a, on).total_cmp(&contrast(*b, on)))
        .unwrap()
}

/// The box around what is drawn in `rect` on the plain `surface`, ignoring a 3 px rim
/// (where a chosen column draws its frame): left, top, right and bottom edges.
fn ink(test: &ui::TestUi, rect: Rect, surface: slint::Color) -> Option<Rect> {
    let mut found: Option<Rect> = None;
    let (left, top) = (
        (rect.0 + 3.0).ceil() as usize,
        (rect.1 + 3.0).ceil() as usize,
    );
    let right = (rect.0 + rect.2 - 3.0).floor() as usize;
    let bottom = (rect.1 + rect.3 - 3.0).floor() as usize;
    for row in top..bottom {
        for column in left..right {
            let pixel = test.pixels[row * test.width + column];
            let distance = [
                (pixel.r, surface.red()),
                (pixel.g, surface.green()),
                (pixel.b, surface.blue()),
            ]
            .iter()
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or_default();
            if distance > 40 {
                let (x, y) = (column as f32, row as f32);
                found = Some(found.map_or((x, y, x + 1.0, y + 1.0), |(l, t, r, b)| {
                    (l.min(x), t.min(y), r.max(x + 1.0), b.max(y + 1.0))
                }));
            }
        }
    }
    found
}

/// Open the Times page of the editor: its file's Modified time is 2026-01-02 03:04:05.789
/// +01:00, and Created and Accessed start from the worker's now, 2026-11-19 10:00:00.
fn times(mut editor: Editor) -> Editor {
    editor.click("Times", AccessibleRole::Button);
    editor
}

/// Leave `label`'s row unticked with the keyboard on its box.
fn untick_with_focus(editor: &mut Editor, label: &str) {
    let name = format!("Change {label}");
    if editor.checked(&name) == "false" {
        editor.click(&name, AccessibleRole::Checkbox);
    }
    editor.click(&name, AccessibleRole::Checkbox);
    assert_eq!(editor.checked(&name), "false");
    assert_eq!(editor.focused().0, name);
}

/// Whether a text input takes input, and the brush its text is drawn with.
fn input_state(item: &ItemRc) -> (bool, slint::Brush) {
    let input = item
        .downcast::<i_slint_core::items::TextInput>()
        .expect("a text input");
    let input = input.as_pin_ref();
    (input.enabled(), input.color())
}

#[test]
fn unticked_time_rows_show_their_current_values_ready_for_input() {
    let mut editor = times(Editor::open());
    let text = editor.test.window.global::<Theme>().get_text();
    for (label, date, time) in [
        ("Modified", "2026-01-02", "03:04:05"),
        ("Created", "2026-11-19", "10:00:00"),
        ("Accessed", "2026-11-19", "10:00:00"),
    ] {
        assert_eq!(
            editor.checked(&format!("Change {label}")),
            "false",
            "{label} starts unticked"
        );
        for (part, value) in [("date", date), ("time", time), ("UTC offset", "+01:00")] {
            let name = format!("{label} {part}");
            assert_eq!(editor.value(&name), value, "{name} shows the current value");
            let (enabled, brush) = input_state(&editor.item(&name, AccessibleRole::TextInput));
            assert!(enabled, "{name} takes input");
            assert_eq!(
                brush,
                slint::Brush::from(text),
                "{name} is drawn as text, not greyed as a placeholder or a disabled field"
            );
        }
        for picker in ["choose date", "choose time"] {
            let button = editor.item(&format!("{label}: {picker}"), AccessibleRole::Button);
            assert_eq!(
                button
                    .accessible_string_property(AccessibleStringProperty::Enabled)
                    .as_deref(),
                Some("true"),
                "{label}: {picker}"
            );
        }
    }
    render::snapshot(&editor.test, "metadata-times-unticked");
    assert!(
        editor.review().1.is_empty(),
        "nothing is applied before a row is ticked"
    );
}

#[test]
fn a_pointer_click_on_a_time_field_or_picker_ticks_its_row_and_keeps_its_value() {
    let mut editor = times(Editor::open());
    for (label, part, value) in [
        ("Modified", "date", "2026-01-02"),
        ("Created", "time", "10:00:00"),
        ("Accessed", "UTC offset", "+01:00"),
    ] {
        let name = format!("{label} {part}");
        editor.click(&name, AccessibleRole::TextInput);
        assert_eq!(
            editor.checked(&format!("Change {label}")),
            "true",
            "a click into {name} ticks its row"
        );
        assert_eq!(editor.focused().0, name, "the click starts editing {name}");
        assert_eq!(
            editor.value(&name),
            value,
            "the click changes nothing in {name}"
        );
    }
    // A press on a field's padding, beside its text, edits it too.
    untick_with_focus(&mut editor, "Modified");
    let input = bounds(&editor.item("Modified time", AccessibleRole::TextInput));
    ui::click(&editor.test.window, input.0 - 3.0, input.1 + input.3 / 2.0);
    editor.test.draw();
    assert_eq!(
        editor.checked("Change Modified"),
        "true",
        "a click on the field's padding ticks its row"
    );
    assert_eq!(editor.focused().0, "Modified time");
    // A click on a picker button opens the picker and ticks the row. Escape there
    // restores the row as the picker found it, so the row stays ticked.
    for (label, picker) in [("Created", "choose date"), ("Accessed", "choose time")] {
        untick_with_focus(&mut editor, label);
        editor.click(&format!("{label}: {picker}"), AccessibleRole::Button);
        assert!(editor.picker_open(), "{label}: {picker} opens");
        assert_eq!(
            editor.checked(&format!("Change {label}")),
            "true",
            "the click on {label}: {picker} ticks its row"
        );
        editor.key(Key::Escape);
        assert!(!editor.picker_open());
        assert_eq!(editor.checked(&format!("Change {label}")), "true");
    }
    // The clicked rows apply what they show: Modified its exact starting time.
    let patch = editor.review().1;
    assert_eq!(
        patch.modified,
        TimeEdit::Set(UNIX_EPOCH + Duration::from_millis(1_767_319_445_789))
    );
    for edit in [patch.created, patch.accessed] {
        assert_eq!(
            edit,
            TimeEdit::Set(UNIX_EPOCH + Duration::from_secs(1_795_078_800))
        );
    }
}

#[test]
fn tab_moves_the_keyboard_through_an_unticked_row_without_ticking_it() {
    let mut editor = times(Editor::open());
    untick_with_focus(&mut editor, "Modified");
    // The pointer rests on the date field, without a click, while Tab moves into it.
    editor.move_to(editor.centre("Modified date", AccessibleRole::TextInput));
    for next in [
        "Modified date",
        "Modified: choose date",
        "Modified time",
        "Modified: choose time",
        "Modified UTC offset",
        "Modified: Now",
    ] {
        editor.key(Key::Tab);
        assert_eq!(editor.focused().0, next);
        assert_eq!(
            editor.checked("Change Modified"),
            "false",
            "Tab to {next} only moves the keyboard"
        );
    }
    editor.chord(&[Key::Shift], Key::Tab);
    assert_eq!(editor.focused().0, "Modified UTC offset");
    assert_eq!(editor.checked("Change Modified"), "false");
    // Typing is a change: it ticks the row.
    editor.key("Z");
    assert_eq!(editor.value("Modified UTC offset"), "Z");
    assert_eq!(editor.checked("Change Modified"), "true");
}

#[test]
fn a_changed_value_ticks_its_row_and_escape_in_the_time_picker_restores_the_row() {
    let mut editor = times(Editor::open());
    untick_with_focus(&mut editor, "Modified");
    editor.key(Key::Tab);
    assert_eq!(editor.focused().0, "Modified date");
    // Up changes the part under the caret, the day at the end of the text.
    editor.key(Key::UpArrow);
    assert_eq!(editor.value("Modified date"), "2026-01-03");
    assert_eq!(
        editor.checked("Change Modified"),
        "true",
        "a step ticks the row"
    );

    // Opening the time picker from the keyboard ticks nothing; its first step does.
    untick_with_focus(&mut editor, "Modified");
    for _ in 0..3 {
        editor.key(Key::Tab);
    }
    assert_eq!(editor.focused().0, "Modified time");
    editor.chord(&[Key::Alt], Key::DownArrow);
    assert_eq!(editor.focused().0, "Modified hour");
    assert_eq!(editor.checked("Change Modified"), "false");
    editor.click("Increase Minute", AccessibleRole::Button);
    assert_eq!(
        editor.value("Modified time"),
        "03:05:05",
        "the field follows the picker at once"
    );
    assert_eq!(editor.checked("Change Modified"), "true");
    // Escape puts the time and the box back as the picker found them.
    editor.key(Key::Escape);
    assert!(!editor.picker_open());
    assert_eq!(editor.value("Modified time"), "03:04:05");
    assert_eq!(editor.checked("Change Modified"), "false");
    assert_eq!(editor.focused().0, "Modified time");
    // Enter keeps the picked time.
    editor.chord(&[Key::Alt], Key::DownArrow);
    editor.key(Key::UpArrow);
    editor.key(Key::Return);
    assert!(!editor.picker_open());
    assert_eq!(editor.value("Modified time"), "04:04:05");
    assert_eq!(editor.checked("Change Modified"), "true");

    // A date chosen in the calendar ticks the row as well.
    untick_with_focus(&mut editor, "Modified");
    editor.key(Key::Tab);
    editor.chord(&[Key::Alt], Key::DownArrow);
    assert_eq!(editor.checked("Change Modified"), "false");
    editor.key(Key::RightArrow);
    editor.key(Key::Return);
    assert_eq!(editor.value("Modified date"), "2026-01-04");
    assert_eq!(editor.checked("Change Modified"), "true");
}

#[test]
fn unticking_keeps_the_typed_value_and_only_ticked_rows_are_applied() {
    let mut editor = times(Editor::open());
    editor.type_into("Modified date", "2025-12-24");
    editor.click("Change Modified", AccessibleRole::Checkbox);
    assert_eq!(editor.checked("Change Modified"), "false");
    assert_eq!(
        editor.value("Modified date"),
        "2025-12-24",
        "unticking keeps the typed date"
    );
    // The worker's next answer brings a newer starting value: it replaces the time
    // still shown, never the typed date.
    editor.defaults.modified = Some(metadata_form::calendar(2026, 2, 3, 4, 5, 6));
    let (revision, patch) = editor.review();
    assert_eq!(
        patch.modified,
        TimeEdit::Keep,
        "an unticked row is not applied"
    );
    editor.preview(revision, patch);
    editor.click("Back to editing", AccessibleRole::Button);
    assert_eq!(editor.value("Modified date"), "2025-12-24");
    assert_eq!(
        editor.value("Modified time"),
        "04:05:06",
        "a value still shown follows the newer starting value"
    );
    // Ticking the row again applies what it shows.
    editor.click("Change Modified", AccessibleRole::Checkbox);
    assert_eq!(editor.value("Modified date"), "2025-12-24");
    let patch = editor.review().1;
    // 2025-12-24 04:05:06 +01:00
    assert_eq!(
        patch.modified,
        TimeEdit::Set(UNIX_EPOCH + Duration::from_secs(1_766_545_506))
    );
    assert_eq!(patch.created, TimeEdit::Keep);
    assert_eq!(patch.accessed, TimeEdit::Keep);
}

#[test]
fn a_time_the_file_system_cannot_set_stays_unavailable() {
    let mut editor = times(Editor::open_capable(MetadataCapabilities {
        modified: true,
        accessed: true,
        ..Default::default()
    }));
    let tick = editor.item("Change Created", AccessibleRole::Checkbox);
    assert_eq!(
        tick.accessible_string_property(AccessibleStringProperty::Enabled)
            .as_deref(),
        Some("false")
    );
    editor.click("Created time", AccessibleRole::TextInput);
    assert_eq!(
        editor.checked("Change Created"),
        "false",
        "a click does not tick a time that cannot be set"
    );
    assert_ne!(editor.focused().0, "Created time");
    assert!(!input_state(&editor.item("Created time", AccessibleRole::TextInput)).0);
    editor.click("Created: choose time", AccessibleRole::Button);
    assert!(!editor.picker_open(), "its pickers stay closed");
    assert_eq!(editor.review().1.created, TimeEdit::Keep);
}

#[test]
fn time_picker_arrows_step_on_press_and_repeat_while_held_on_them() {
    let mut editor = times(Editor::open_with_clock());
    editor.click("Modified: choose time", AccessibleRole::Button);
    let arrow = editor.centre("Increase Minute", AccessibleRole::Button);
    editor.press("Increase Minute", AccessibleRole::Button);
    assert_eq!(
        editor.value("Modified time"),
        "03:05:05",
        "the press steps at once"
    );
    editor.advance(Duration::from_millis(499));
    assert_eq!(editor.value("Modified time"), "03:05:05");
    editor.advance(Duration::from_millis(2));
    assert_eq!(
        editor.value("Modified time"),
        "03:06:05",
        "held half a second, it repeats"
    );
    for minute in 7..=9 {
        editor.advance(Duration::from_millis(50));
        assert_eq!(editor.value("Modified time"), format!("03:{minute:02}:05"));
    }
    // Off the arrow the repeat pauses; back on it, it goes on.
    editor.move_to(slint::LogicalPosition::new(arrow.x, arrow.y + 200.0));
    editor.advance(Duration::from_millis(300));
    assert_eq!(editor.value("Modified time"), "03:09:05");
    editor.move_to(arrow);
    editor.advance(Duration::from_millis(50));
    assert_eq!(editor.value("Modified time"), "03:10:05");
    editor.release(arrow);
    editor.advance(Duration::from_millis(600));
    assert_eq!(
        editor.value("Modified time"),
        "03:10:05",
        "the release ends it"
    );
    assert!(editor.picker_open(), "the arrows keep the picker open");
}

#[test]
fn the_wheel_over_a_time_picker_column_chooses_it_and_steps_it_a_notch_at_a_time() {
    let mut editor = times(Editor::open());
    editor.click("Modified: choose time", AccessibleRole::Button);
    assert_eq!(editor.focused().0, "Modified hour");
    editor.wheel("Increase Second", AccessibleRole::Button, 60.0);
    assert_eq!(
        editor.value("Modified time"),
        "03:04:06",
        "a notch up increases"
    );
    assert_eq!(
        editor.focused().0,
        "Modified second",
        "and chooses the column"
    );
    editor.wheel("Decrease Second", AccessibleRole::Button, -120.0);
    assert_eq!(
        editor.value("Modified time"),
        "03:04:04",
        "two notches down"
    );
    // A touchpad's or a fine wheel's small steps add up to a notch.
    editor.wheel("Increase Minute", AccessibleRole::Button, 30.0);
    assert_eq!(editor.value("Modified time"), "03:04:04");
    editor.wheel("Increase Minute", AccessibleRole::Button, 30.0);
    assert_eq!(editor.value("Modified time"), "03:05:04");
    assert_eq!(editor.focused().0, "Modified minute");
    // A press on a column's value or name chooses that column for the keyboard.
    let up = bounds(&editor.item("Increase Hour", AccessibleRole::Button));
    let down = bounds(&editor.item("Decrease Hour", AccessibleRole::Button));
    let value = (up.0 + up.2 / 2.0, (up.1 + up.3 + down.1) / 2.0);
    ui::click(&editor.test.window, value.0, value.1);
    editor.test.draw();
    assert_eq!(editor.focused().0, "Modified hour");
    assert_eq!(
        editor.value("Modified time"),
        "03:05:04",
        "and changes nothing"
    );
    let name = (value.0, up.1 - up.3 / 2.0);
    ui::click(&editor.test.window, name.0 + up.2, name.1);
    editor.test.draw();
    assert_eq!(editor.focused().0, "Modified minute");
    assert!(editor.picker_open());
}

#[test]
fn time_picker_keys_choose_columns_type_values_and_accept_or_cancel() {
    let mut editor = times(Editor::open());
    untick_with_focus(&mut editor, "Modified");
    for _ in 0..3 {
        editor.key(Key::Tab);
    }
    editor.chord(&[Key::Alt], Key::DownArrow);
    assert_eq!(editor.focused(), ("Modified hour".into(), "03".into()));
    // Tab and Shift+Tab go round the columns and stay in the picker.
    editor.key(Key::Tab);
    assert_eq!(editor.focused().0, "Modified minute");
    editor.chord(&[Key::Shift], Key::Tab);
    editor.chord(&[Key::Shift], Key::Tab);
    assert_eq!(editor.focused().0, "Modified second");
    editor.key(Key::Tab);
    assert_eq!(editor.focused().0, "Modified hour");
    // Home and End give the column its first and last value.
    editor.key(Key::End);
    assert_eq!(editor.value("Modified time"), "23:04:05");
    editor.key(Key::Home);
    assert_eq!(editor.value("Modified time"), "00:04:05");
    // Digits type the column and move on once it is complete.
    for key in ["1", "7"] {
        editor.key(key);
    }
    assert_eq!(editor.value("Modified time"), "17:04:05");
    assert_eq!(editor.focused().0, "Modified minute");
    for key in ["4", "5", "9"] {
        editor.key(key);
    }
    assert_eq!(editor.value("Modified time"), "17:45:09");
    assert_eq!(editor.focused(), ("Modified second".into(), "09".into()));
    assert_eq!(editor.checked("Change Modified"), "true");
    // Page Down steps ten back and wraps like the arrows.
    editor.key(Key::PageDown);
    assert_eq!(editor.value("Modified time"), "17:45:59");
    // Escape restores the row as the picker found it: the time and the unticked box.
    editor.key(Key::Escape);
    assert!(!editor.picker_open());
    assert_eq!(editor.value("Modified time"), "03:04:05");
    assert_eq!(editor.checked("Change Modified"), "false");
    assert_eq!(editor.focused().0, "Modified time");
    // 23 → 00 and 59 → 00 wrap within their column: nothing carries into the next.
    editor.chord(&[Key::Alt], Key::DownArrow);
    for key in [
        Key::End,
        Key::UpArrow,
        Key::RightArrow,
        Key::End,
        Key::UpArrow,
    ] {
        editor.key(key);
    }
    editor.key(Key::Return);
    assert!(!editor.picker_open());
    assert_eq!(editor.value("Modified time"), "00:00:05");
    assert_eq!(
        editor.value("Modified date"),
        "2026-01-02",
        "the date stays"
    );
    assert_eq!(editor.checked("Change Modified"), "true");
}

#[test]
fn only_the_hovered_time_picker_arrow_lights_up_and_it_shows_in_every_theme() {
    let mut editor = times(Editor::open());
    for scheme in 0..26 {
        editor.test.window.invoke_apply_preferences(scheme, 0);
        editor.click("Modified: choose time", AccessibleRole::Button);
        let theme = editor.test.window.global::<Theme>();
        let (surface, hover, active) = (
            theme.get_surface(),
            theme.get_hover(),
            theme.get_active_frame(),
        );
        // The pointer rests on the picker's padding, away from every arrow.
        let frame = bounds(&editor.item("Modified time picker", AccessibleRole::Groupbox));
        editor.move_to(slint::LogicalPosition::new(frame.0 + 4.0, frame.1 + 4.0));
        let rects = ARROWS.map(|label| bounds(&editor.item(label, AccessibleRole::Button)));
        // The keyboard's column, Hour, wears a 2 px active frame beside its value.
        let value_row = rects[0].1 + rects[0].3 * 1.5;
        for x in [rects[0].0, rects[0].0 + 1.0] {
            let frame = colour(editor.test.pixel(x, value_row));
            assert_eq!(
                frame, active,
                "theme {scheme}: the chosen column's frame at {x}"
            );
        }
        let before = rects.map(|rect| region(&editor.test, rect));
        editor.move_to(editor.centre("Increase Minute", AccessibleRole::Button));
        for ((label, rect), before) in ARROWS.iter().zip(rects).zip(before) {
            let after = region(&editor.test, rect);
            if *label != "Increase Minute" {
                assert!(
                    after == before,
                    "theme {scheme}: {label} answers another arrow's hover"
                );
                continue;
            }
            assert!(
                after != before,
                "theme {scheme}: the hovered arrow lights up"
            );
            // Its frame stands out from the surface, its triangle from the hover colour.
            let edge = colour(editor.test.pixel(rect.0, rect.1 + rect.3 / 2.0));
            assert!(
                contrast(edge, surface) >= 3.0,
                "theme {scheme}: hover frame {edge:?} on {surface:?}"
            );
            let middle = strongest(
                &editor.test,
                rect.0 + rect.2 / 2.0,
                rect.1 + rect.3 / 2.0,
                hover,
            );
            assert!(
                contrast(middle, hover) >= 4.5,
                "theme {scheme}: the triangle {middle:?} on {hover:?}"
            );
        }
        editor.key(Key::Escape);
        assert!(!editor.picker_open());
    }
}

#[test]
fn time_picker_arrows_and_calendar_chevrons_are_drawn_centred_and_alike() {
    let mut editor = times(Editor::open());
    let surface = editor.test.window.global::<Theme>().get_surface();
    editor.click("Modified: choose time", AccessibleRole::Button);
    let frame = bounds(&editor.item("Modified time picker", AccessibleRole::Groupbox));
    editor.move_to(slint::LogicalPosition::new(frame.0 + 4.0, frame.1 + 4.0));
    render::snapshot(&editor.test, "metadata-time-picker");
    let mut sizes = Vec::new();
    for label in ARROWS.iter().chain(&["Previous month", "Next month"]) {
        if *label == "Previous month" {
            editor.key(Key::Escape);
            editor.click("Modified: choose date", AccessibleRole::Button);
            let frame = bounds(&editor.item("Modified calendar", AccessibleRole::Groupbox));
            editor.move_to(slint::LogicalPosition::new(
                frame.0 + frame.2 - 3.0,
                frame.1 + frame.3 - 3.0,
            ));
            render::snapshot(&editor.test, "metadata-calendar");
            sizes.clear();
        }
        let rect = bounds(&editor.item(label, AccessibleRole::Button));
        let drawn =
            ink(&editor.test, rect, surface).unwrap_or_else(|| panic!("{label} draws nothing"));
        let (x, y) = ((drawn.0 + drawn.2) / 2.0, (drawn.1 + drawn.3) / 2.0);
        assert!(
            (x - (rect.0 + rect.2 / 2.0)).abs() <= 1.0
                && (y - (rect.1 + rect.3 / 2.0)).abs() <= 1.0,
            "{label} is drawn off centre: {drawn:?} in {rect:?}"
        );
        sizes.push((label, drawn.2 - drawn.0, drawn.3 - drawn.1));
        let (first, width, height) = sizes[0];
        let (_, this_width, this_height) = sizes[sizes.len() - 1];
        assert!(
            (this_width - width).abs() <= 1.0 && (this_height - height).abs() <= 1.0,
            "{label} ({this_width}x{this_height}) differs from {first} ({width}x{height})"
        );
    }
}

/// Resize the window to `width` by `height` logical pixels at `scale` device pixels each.
fn resize_scaled(test: &mut ui::TestUi, width: f32, height: f32, scale: f32) {
    use slint::platform::WindowEvent;
    test.window
        .window()
        .dispatch_event(WindowEvent::ScaleFactorChanged {
            scale_factor: scale,
        });
    let (physical_width, physical_height) = (
        (width * scale).round() as usize,
        (height * scale).round() as usize,
    );
    test.width = physical_width;
    test.height = physical_height;
    test.native.set_size(slint::PhysicalSize::new(
        physical_width as u32,
        physical_height as u32,
    ));
    test.window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(width, height),
    });
    test.pixels.resize(
        physical_width * physical_height,
        slint::Rgb8Pixel::default(),
    );
    test.draw();
}

/// Whether `item` scrolls into view: it lies in a Flickable whose content is taller
/// than the Flickable.
fn scrolls_into_view(item: &ItemRc) -> bool {
    use i_slint_core::item_tree::ParentItemTraversalMode;
    let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(current) = parent {
        if let Some(flickable) = current.downcast::<i_slint_core::items::Flickable>() {
            return flickable.as_pin_ref().content_height().get()
                > current.geometry().height() + 0.5;
        }
        parent = current.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    false
}

#[test]
fn both_pickers_fit_the_smallest_window_at_every_density_font_and_scale() {
    let mut editor = times(Editor::open());
    for (density, font, scale) in [
        (0, 11, 1.0),
        (1, 15, 1.0),
        (0, 32, 1.0),
        (1, 32, 1.0),
        (1, 32, 1.5),
        (0, 13, 2.0),
        (1, 32, 2.0),
    ] {
        let case = format!("density {density}, font {font}, scale {scale}");
        editor.test.window.invoke_apply_preferences(0, density);
        editor.test.window.invoke_apply_fonts("".into(), font, font);
        resize_scaled(&mut editor.test, 320.0, 200.0, scale);
        let window = (0.0, 0.0, 320.0, 200.0);
        // A screen reader's action opens the picker wherever the page has scrolled.
        editor.act("Modified: choose time", AccessibleRole::Button);
        let picker = bounds(&editor.item("Modified time picker", AccessibleRole::Groupbox));
        assert!(
            within(picker, window),
            "{case}: the time picker {picker:?} leaves the window"
        );
        for edge in [picker.0, picker.1] {
            assert!(
                (edge * scale - (edge * scale).round()).abs() < 0.01,
                "{case}: the time picker {picker:?} is off the device pixel grid"
            );
        }
        for label in ARROWS {
            let arrow = bounds(&editor.item(label, AccessibleRole::Button));
            assert!(
                within(arrow, picker),
                "{case}: {label} {arrow:?} is cut off by {picker:?}"
            );
        }
        // The column names show while they fit, not with 32 px fonts in 320 by 200.
        let named = editor.texts().iter().any(|text| text == "Minute");
        assert_eq!(named, font < 32, "{case}: column names shown: {named}");
        render::snapshot(
            &editor.test,
            &format!("metadata-time-picker-{density}-{font}-{scale}"),
        );
        editor.key(Key::Escape);
        editor.act("Modified: choose date", AccessibleRole::Button);
        let calendar = bounds(&editor.item("Modified calendar", AccessibleRole::Groupbox));
        assert!(
            within(calendar, window),
            "{case}: the calendar {calendar:?} leaves the window"
        );
        for edge in [calendar.0, calendar.1, calendar.2, calendar.3] {
            assert!(
                (edge * scale - (edge * scale).round()).abs() < 0.01,
                "{case}: the calendar {calendar:?} is off the device pixel grid"
            );
        }
        for label in [
            "Previous month",
            "Next month",
            "2026-01-02",
            "2026-01-31",
            "Today",
        ] {
            let item = editor.item(label, AccessibleRole::Button);
            let rect = bounds(&item);
            assert!(
                within(rect, calendar) || scrolls_into_view(&item),
                "{case}: {label} {rect:?} is cut off by the calendar {calendar:?}"
            );
            assert!(
                rect.0 >= calendar.0 && rect.0 + rect.2 <= calendar.0 + calendar.2 + 0.01,
                "{case}: {label} {rect:?} is wider than the calendar {calendar:?}"
            );
        }
        let cursor = bounds(&editor.item("2026-01-02", AccessibleRole::Button));
        assert!(
            within(cursor, calendar),
            "{case}: the keyboard's day {cursor:?} is out of view"
        );
        render::snapshot(
            &editor.test,
            &format!("metadata-calendar-{density}-{font}-{scale}"),
        );
        editor.key(Key::Escape);
    }
}

#[test]
fn the_calendar_turns_months_with_the_wheel_and_knows_home_end_and_ctrl_page_keys() {
    let mut editor = times(Editor::open());
    editor.click("Modified: choose date", AccessibleRole::Button);
    let cursor = |editor: &Editor| {
        editor.state(
            "Modified calendar date",
            AccessibleRole::Spinbox,
            AccessibleStringProperty::Value,
        )
    };
    assert_eq!(cursor(&editor), "2026-01-02");
    editor.wheel("2026-01-15", AccessibleRole::Button, -60.0);
    assert_eq!(
        cursor(&editor),
        "2026-02-02",
        "a notch down shows the next month"
    );
    assert!(editor.texts().iter().any(|text| text == "February 2026"));
    editor.wheel("2026-02-15", AccessibleRole::Button, 120.0);
    assert_eq!(
        cursor(&editor),
        "2025-12-02",
        "two notches up go two months back"
    );
    editor.wheel("2025-12-15", AccessibleRole::Button, 30.0);
    assert_eq!(cursor(&editor), "2025-12-02");
    editor.wheel("2025-12-15", AccessibleRole::Button, 30.0);
    assert_eq!(cursor(&editor), "2025-11-02");
    editor.key(Key::End);
    assert_eq!(cursor(&editor), "2025-11-30", "End: the month's last day");
    editor.key(Key::Home);
    assert_eq!(cursor(&editor), "2025-11-01", "Home: its first day");
    editor.chord(&[Key::Control], Key::PageDown);
    assert_eq!(cursor(&editor), "2026-11-01", "Ctrl+Page Down: a year on");
    editor.chord(&[Key::Control], Key::PageUp);
    assert_eq!(cursor(&editor), "2025-11-01");
    assert!(editor.picker_open());
    editor.key(Key::Escape);
    assert!(!editor.picker_open());
    assert_eq!(
        editor.value("Modified date"),
        "2026-01-02",
        "Escape chooses nothing"
    );
}

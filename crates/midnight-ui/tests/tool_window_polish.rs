//! The UI polish gate's batch 2 for Compare directories, Checksums and Jobs (20 November
//! 2026): Enter starts the shown page's primary action in Compare directories and in
//! Checksums, while controls with an Enter of their own keep it and typing still works
//! (U-D15); the Checksums page buttons (U-D17) and the focused Jobs list (U-D18) show
//! the keyboard focus with 3:1; and the Compare tolerance field selects with the
//! theme's colours (the U-D07 follow-up).
#[path = "support/window_input.rs"]
mod input;
#[path = "support/window_polish.rs"]
mod polish;
use crate::support::{modeless, ui};

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, TextInput},
    window::WindowInner,
};
use midnight_ui::{
    ChecksumDraft, ChecksumWindow, ComparisonChoices, ComparisonView, JobsView, JobsWindow,
    OperationRow, Theme,
};
use slint::{
    Color, ComponentHandle, Model, ModelRc, SharedString, VecModel,
    platform::{Key, WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

/// Themes whose focus colours differ most: Blue, Dark, Light and both high contrasts.
const FOCUS_THEMES: [i32; 5] = [0, 1, 2, 21, 22];

fn platform() -> Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>> {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    windows
}

fn counter() -> (Rc<Cell<usize>>, Rc<Cell<usize>>) {
    let count = Rc::new(Cell::new(0));
    (count.clone(), count)
}

// ---------------------------------------------------------------------------------
// U-D15: Enter in Compare directories.

type Submission = (bool, bool, bool, String, bool, bool, bool);

fn settle(test: &mut ui::TestUi) {
    for _ in 0..2 {
        slint::platform::update_timers_and_animations();
        test.draw();
    }
}

/// Open Compare directories comparing sizes and times; returns its submissions.
fn compare_directories(test: &mut ui::TestUi) -> Rc<RefCell<Vec<Submission>>> {
    let submissions = Rc::new(RefCell::new(Vec::new()));
    let sink = submissions.clone();
    test.window.on_submit_comparison(move |choices| {
        sink.borrow_mut().push((
            choices.by_size,
            choices.by_modified,
            choices.case_sensitive,
            choices.tolerance_seconds.to_string(),
            choices.unmatched_directories,
            choices.by_content,
            choices.recursive,
        ));
    });
    let weak = test.window.as_weak();
    test.window.on_cancel_comparison(move || {
        let window = weak.upgrade().unwrap();
        let mut model = window.get_comparison_view();
        model.open = false;
        window.set_comparison_view(model);
    });
    test.window.set_comparison_view(ComparisonView {
        open: true,
        left_path: "C:\\文档\\left".into(),
        right_path: "C:\\文档\\right".into(),
        choices: ComparisonChoices {
            by_size: true,
            by_modified: true,
            tolerance_seconds: "2".into(),
            ..Default::default()
        },
        ..Default::default()
    });
    settle(test);
    submissions
}

fn focused_label(window: &slint::Window) -> String {
    polish::focused_control(window)
        .and_then(|item| item.accessible_string_property(AccessibleStringProperty::Label))
        .map(|label| label.to_string())
        .unwrap_or_default()
}

fn checked(test: &ui::TestUi, label: &str) -> bool {
    ui::popup_item(&test.window, label, AccessibleRole::Checkbox)
        .accessible_string_property(AccessibleStringProperty::Checked)
        .is_some_and(|value| value == "true")
}

fn set_busy(test: &mut ui::TestUi, busy: bool) {
    let mut model = test.window.get_comparison_view();
    model.busy = busy;
    test.window.set_comparison_view(model);
    settle(test);
}

fn held_enter(window: &slint::Window) {
    let text: SharedString = Key::Return.into();
    window.dispatch_event(WindowEvent::KeyPressRepeated { text: text.clone() });
    window.dispatch_event(WindowEvent::KeyReleased { text });
}

#[test]
fn enter_compares_from_the_options_and_the_tolerance_field_of_compare_directories() {
    let mut test = ui::TestUi::new();
    let submissions = compare_directories(&mut test);
    assert_eq!(
        focused_label(test.window.window()),
        "Compare file size",
        "Compare directories opens in its first option"
    );
    // Enter compares with the options as they are; it does not toggle the focused one.
    ui::key(&test.window, Key::Return);
    settle(&mut test);
    let expected = (true, true, false, "2".to_owned(), false, false, false);
    assert_eq!(submissions.borrow().as_slice(), [expected]);
    assert!(
        checked(&test, "Compare file size"),
        "Enter leaves the option alone"
    );
    // A held Enter answers nothing more.
    held_enter(test.window.window());
    settle(&mut test);
    assert_eq!(submissions.borrow().len(), 1, "a held Enter compares once");
    // Space still toggles the focused option, and compares nothing.
    ui::key(&test.window, Key::Space);
    settle(&mut test);
    assert!(!checked(&test, "Compare file size"), "Space toggles");
    assert_eq!(submissions.borrow().len(), 1);
    // The tolerance field takes typed text, and Enter there compares with it.
    ui::click_popup(
        &test.window,
        "Time tolerance in seconds",
        AccessibleRole::TextInput,
    );
    ui::chord(&test.window, &[Key::Control], "a");
    ui::key(&test.window, "12");
    settle(&mut test);
    ui::key(&test.window, Key::Return);
    settle(&mut test);
    assert_eq!(
        submissions.borrow().last(),
        Some(&(false, true, false, "12".to_owned(), false, false, false)),
        "Enter in the field compares with the typed tolerance"
    );
    // While a comparison runs, Enter starts no other one.
    set_busy(&mut test, true);
    ui::key(&test.window, Key::Return);
    settle(&mut test);
    assert_eq!(submissions.borrow().len(), 2, "no Enter while busy");
    set_busy(&mut test, false);
    // Enter on Close closes the dialog without comparing.
    let mut reached = false;
    for _ in 0..24 {
        ui::key(&test.window, Key::Tab);
        settle(&mut test);
        if focused_label(test.window.window()) == "Close" {
            reached = true;
            break;
        }
    }
    assert!(reached, "Tab reaches Close");
    ui::key(&test.window, Key::Return);
    settle(&mut test);
    assert!(
        !test.window.get_comparison_view().open,
        "Enter on Close closes"
    );
    assert_eq!(submissions.borrow().len(), 2, "Close compares nothing");
}

// ---------------------------------------------------------------------------------
// The U-D07 follow-up: the Compare tolerance field selects with the theme's colours.

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

fn color_contrast(first: Color, second: Color) -> f64 {
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

/// The first text input at or below `item`.
fn text_input(item: &ItemRc) -> Option<ItemRc> {
    if item.downcast::<TextInput>().is_some() {
        return Some(item.clone());
    }
    let mut child = item.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        if let Some(found) = text_input(&current) {
            return Some(found);
        }
    }
    None
}

#[test]
fn the_compare_tolerance_field_selects_with_theme_colours_in_every_theme() {
    let mut test = ui::TestUi::new();
    compare_directories(&mut test);
    for scheme in midnight_core::PreferenceTheme::ALL.map(|theme| theme.index()) {
        test.window.invoke_apply_preferences(scheme, 0);
        settle(&mut test);
        let field = ui::popup_item(
            &test.window,
            "Time tolerance in seconds",
            AccessibleRole::TextInput,
        );
        let input = text_input(&field).expect("the field's text input");
        let input = input.downcast::<TextInput>().unwrap();
        let input = input.as_pin_ref();
        let theme = test.window.global::<Theme>();
        let (foreground, background) = (
            input.selection_foreground_color(),
            input.selection_background_color(),
        );
        assert_eq!(
            (foreground, background),
            (theme.get_selection_text(), theme.get_selection_background()),
            "theme {scheme}: the selection uses the theme's colours"
        );
        // As the polish gate measures it: the selection foreground at the text's
        // opacity over the selection background.
        let alpha = f64::from(input.color().color().alpha()) / 255.0;
        let shown = blend(foreground, alpha, background);
        let ratio = color_contrast(shown, background);
        assert!(
            ratio >= 4.5,
            "theme {scheme}: selected text {shown:?} on {background:?} is {ratio:.2}:1"
        );
    }
}

// ---------------------------------------------------------------------------------
// U-D15: Enter in Checksums.

fn checksum_window(
    windows: &Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>,
    scheme: i32,
) -> (ChecksumWindow, Rc<MinimalSoftwareWindow>) {
    let window = ChecksumWindow::new().unwrap();
    window.invoke_apply_preferences(scheme, 0, "".into(), 12, 15);
    window.set_draft(ChecksumDraft {
        sha256: true,
        ..Default::default()
    });
    window.show().unwrap();
    let native = windows.borrow().last().unwrap().clone();
    window.invoke_begin();
    (window, native)
}

fn has_popup(native: &MinimalSoftwareWindow) -> bool {
    !WindowInner::from_pub(native.window())
        .active_popups()
        .is_empty()
}

#[test]
fn enter_in_checksums_starts_the_shown_pages_primary_action() {
    let windows = platform();
    let (window, native) = checksum_window(&windows, 0);
    let (calculated, sink) = counter();
    window.on_calculate(move |_| sink.set(sink.get() + 1));
    let (stopped, sink) = counter();
    window.on_stop(move || sink.set(sink.get() + 1));
    let verified = Rc::new(RefCell::new(Vec::new()));
    let sink = verified.clone();
    window.on_verify_manifest(move |path| sink.borrow_mut().push(path.to_string()));
    modeless::draw(&native, 860, 620, "checksums-enter");
    let results = modeless::accessible_item(&native, AccessibleRole::List, "Checksum results")
        .expect("the results list");
    let focus = polish::focus_item(native.window()).expect("a focused item");
    let mut holds = Some(results);
    while let Some(item) = holds.clone()
        && item != focus
    {
        holds = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    assert!(holds.is_some(), "Checksums opens in its results");
    // A freshly opened window: Enter calculates the chosen checksums.
    input::key(&native, Key::Return);
    assert_eq!(calculated.get(), 1, "Enter calculates");
    held_enter(native.window());
    assert_eq!(calculated.get(), 1, "a held Enter calculates once");
    // A running calculation is neither stopped nor started again by Enter.
    window.set_running(true);
    slint::platform::update_timers_and_animations();
    modeless::draw(&native, 860, 620, "checksums-enter-running");
    input::key(&native, Key::Return);
    assert_eq!((calculated.get(), stopped.get()), (1, 0));
    window.set_running(false);
    // Without a chosen checksum there is nothing to calculate, as for the button.
    window.set_draft(ChecksumDraft::default());
    slint::platform::update_timers_and_animations();
    modeless::draw(&native, 860, 620, "checksums-enter-nothing-chosen");
    input::key(&native, Key::Return);
    assert_eq!(calculated.get(), 1, "nothing chosen, nothing calculated");
    window.set_draft(ChecksumDraft {
        sha256: true,
        ..Default::default()
    });
    // A control with an Enter of its own keeps it: the algorithm menu opens.
    slint::platform::update_timers_and_animations();
    modeless::draw(&native, 860, 620, "checksums-enter-menu");
    modeless::click(&native, "Checksum algorithms");
    assert!(has_popup(&native), "a click opens the menu");
    input::key(&native, Key::Escape);
    assert!(!has_popup(&native), "Escape closes the menu");
    assert!(window.window().is_visible(), "and only the menu");
    input::key(&native, Key::Return);
    assert!(
        has_popup(&native),
        "Enter on the menu button opens the menu"
    );
    assert_eq!(calculated.get(), 1, "Enter on the menu calculates nothing");
    input::key(&native, Key::Escape);
    // The Verify page: Enter verifies the named checksum file.
    window.set_manifest_path("checksums.sha256".into());
    window.set_page(1);
    slint::platform::update_timers_and_animations();
    modeless::draw(&native, 860, 620, "checksums-enter-verify");
    input::key(&native, Key::Return);
    assert_eq!(verified.borrow().as_slice(), ["checksums.sha256"]);
    assert_eq!(calculated.get(), 1);
    // Typing in its path field still works, and Enter there verifies as before.
    let field = modeless::accessible_item(
        &native,
        AccessibleRole::TextInput,
        "Checksum file to verify",
    )
    .expect("the checksum file field");
    input::click(&native, &field);
    input::chord(&native, &[Key::Control], "a");
    input::key(&native, "other.md5");
    input::key(&native, Key::Return);
    assert_eq!(
        verified.borrow().last().map(String::as_str),
        Some("other.md5")
    );
    assert_eq!(window.get_manifest_path(), "other.md5");
    window.hide().unwrap();
}

// ---------------------------------------------------------------------------------
// U-D17 and U-D18: the focus shows.

/// Tab through the window from its first focus, rendering each step, and return how
/// each of `wanted` (a control, or the focus scope of a list) showed when Tab moved the
/// focus onto it: the best contrast of its changed pixels (grown by 4 px) against the
/// frame before, or None when none changed.
fn focus_shown_on(
    native: &MinimalSoftwareWindow,
    size: (u32, u32),
    wanted: &[ItemRc],
    name: &str,
) -> Vec<Option<f64>> {
    let (width, height) = size;
    let mut shown: Vec<Option<f64>> = vec![None; wanted.len()];
    let mut reached = vec![false; wanted.len()];
    let mut before = modeless::render(native, width, height, name);
    for _ in 0..40 {
        input::key(native, Key::Tab);
        slint::platform::update_timers_and_animations();
        let after = modeless::render(native, width, height, name);
        // A control, or a list's own focus scope, which has no accessible role.
        let focus = polish::focus_item(native.window());
        let control = polish::focused_control(native.window());
        if let Some(index) = wanted
            .iter()
            .position(|item| Some(item) == focus.as_ref() || Some(item) == control.as_ref())
        {
            reached[index] = true;
            let area = polish::bounds(&wanted[index]);
            if let Some(contrast) =
                polish::focus_contrast(&before, &after, width as usize, height as usize, area)
            {
                shown[index] = Some(shown[index].map_or(contrast, |best| best.max(contrast)));
            }
        }
        before = after;
        if reached.iter().all(|reached| *reached) {
            break;
        }
    }
    assert!(
        reached.iter().all(|reached| *reached),
        "{name}: Tab reaches every control checked"
    );
    shown
}

/// How a focus indicator fails the gate (U-K24, U-K25); None when it passes.
fn focus_failure(what: &str, contrast: Option<f64>) -> Option<String> {
    match contrast {
        None => Some(format!("{what} shows no focus")),
        Some(contrast) if contrast < 3.0 => {
            Some(format!("{what}'s focus reaches only {contrast:.2}:1"))
        }
        Some(_) => None,
    }
}

#[test]
fn the_checksum_page_buttons_and_results_show_the_keyboard_focus() {
    let windows = platform();
    let mut failures = Vec::new();
    for scheme in FOCUS_THEMES {
        let (window, native) = checksum_window(&windows, scheme);
        modeless::layout(&native, 860, 620);
        // The window opens in its results list, a Tab stop as well.
        let results = polish::focus_item(native.window()).expect("the results have the focus");
        // The page buttons, before the footer's Calculate.
        let stops = [
            modeless::accessible_items(&native, AccessibleRole::Button, "Calculate")[0].clone(),
            modeless::accessible_item(&native, AccessibleRole::Button, "Verify")
                .expect("the Verify page button"),
            results,
        ];
        let shown = focus_shown_on(
            &native,
            (860, 620),
            &stops,
            &format!("checksum-pages-{scheme}"),
        );
        for (stop, contrast) in ["Calculate", "Verify", "The results list"]
            .into_iter()
            .zip(shown)
        {
            failures.extend(focus_failure(&format!("theme {scheme}: {stop}"), contrast));
        }
        window.hide().unwrap();
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

fn finished(identity: &str, title: &str) -> OperationRow {
    OperationRow {
        identity: identity.into(),
        title: title.into(),
        state: "Completed".into(),
        summary: "Copied 3 files".into(),
        completed: true,
        ..Default::default()
    }
}

#[test]
fn the_focused_jobs_list_shows_its_focus() {
    let windows = platform();
    let mut failures = Vec::new();
    for scheme in FOCUS_THEMES {
        let window = JobsWindow::new().unwrap();
        let view = window.global::<JobsView>();
        view.on_find(|rows, identity| {
            rows.iter()
                .position(|row| row.identity == identity)
                .map_or(if rows.row_count() == 0 { -1 } else { 0 }, |index| {
                    index as i32
                })
        });
        view.on_has_finished(|rows, without_errors| {
            rows.iter()
                .any(|row| !row.active && (!without_errors || !row.failed))
        });
        window.set_rows(ModelRc::new(VecModel::from(vec![
            finished("copy-job:1", "Copy"),
            finished("7", "Permanently delete"),
        ])));
        window.invoke_apply_preferences(scheme, 0, "".into(), 12, 15);
        window.show().unwrap();
        let native = windows.borrow().last().unwrap().clone();
        modeless::layout(&native, 720, 500);
        // Jobs opens with the keyboard in its list.
        let list = polish::focus_item(native.window()).expect("the list has the focus");
        assert!(
            polish::focused_control(native.window())
                .is_none_or(|control| control.accessible_role() != AccessibleRole::Button),
            "theme {scheme}: Jobs opens in its list"
        );
        // Tab away and back again: the list must look focused when the focus returns.
        let size = (720, 500);
        let mut before = modeless::render(&native, size.0, size.1, "jobs-list-focus");
        let mut contrast = None;
        let mut returned = false;
        for _ in 0..20 {
            input::key(&native, Key::Tab);
            slint::platform::update_timers_and_animations();
            let after = modeless::render(&native, size.0, size.1, "jobs-list-focus");
            if polish::focus_item(native.window()).is_some_and(|item| item == list) {
                contrast = polish::focus_contrast(
                    &before,
                    &after,
                    size.0 as usize,
                    size.1 as usize,
                    polish::bounds(&list),
                );
                returned = true;
                break;
            }
            before = after;
        }
        assert!(returned, "theme {scheme}: Tab comes back to the list");
        failures.extend(focus_failure(
            &format!("theme {scheme}: the job list"),
            contrast,
        ));
        window.hide().unwrap();
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

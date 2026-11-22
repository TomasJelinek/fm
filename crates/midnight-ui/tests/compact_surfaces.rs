//! The surfaces the UI polish gate found overlapping or cut at the minimum window size,
//! 320×200, with touch density (U-D27 in docs/developer/acceptance/ui-polish.md): the
//! left panel's Sort menu, Select by mask, Checksums with no results and Attributes ›
//! Times with its calendar and time picker. Each is judged as the gate judges a surface
//! (`support/minimum_layout.rs`: no control outside the window, nothing cut by a
//! container that does not scroll or beyond what a scroll view reaches, no texts or
//! controls over each other, no label shown narrower than a character), in Dark Touch
//! and Light Touch with the default touch font and the large fonts the minimum-size
//! tests use, and in Light Desktop.
#[path = "support/minimum_layout.rs"]
mod layout;
use crate::support::{modeless, ui};

use i_slint_core::{
    item_tree::{ItemRc, ItemTreeRc, ParentItemTraversalMode},
    items::{AccessibleRole, Flickable},
    layout::Orientation,
    lengths::LogicalLength,
    window::WindowInner,
};
use midnight_ui::{ChecksumDraft, ChecksumWindow, PanelChoices};
use slint::{
    ComponentHandle,
    platform::{Key, WindowAdapter, software_renderer::MinimalSoftwareWindow},
};
use std::rc::Rc;

/// (scheme, density, font): Dark and Light at touch density with the default touch
/// font (15 px) and the 24 and 32 px fonts of the minimum-size tests, and Light at
/// desktop density with a 32 px font.
const CASES: [(i32, i32, i32); 7] = [
    (1, 1, 15),
    (1, 1, 24),
    (1, 1, 32),
    (2, 1, 15),
    (2, 1, 24),
    (2, 1, 32),
    (2, 0, 32),
];

fn settle(test: &mut ui::TestUi) {
    // Slint places embedded popups through a deferred property-tracker timer.
    for _ in 0..2 {
        slint::platform::update_timers_and_animations();
        test.draw();
    }
}

fn apply(test: &mut ui::TestUi, (scheme, density, font): (i32, i32, i32)) {
    test.window.invoke_apply_fonts("".into(), font, font);
    test.window.invoke_apply_preferences(scheme, density);
    test.resize(320, 200);
    settle(test);
}

/// What is wrong, case by case: a test checks every case before it fails, so that one
/// run measures them all.
#[derive(Default)]
struct Failures(Vec<String>);

impl Failures {
    /// The layout checks' findings on what the window shows now.
    fn layout(&mut self, window: &slint::Window, case: &str) {
        for finding in layout::findings(window) {
            self.0.push(format!("{case}: {finding}"));
        }
    }

    /// The item is shown whole: no ancestor clips any of it.
    fn whole(&mut self, item: &ItemRc, what: &str) {
        let (shown, bounds) = (layout::shown(item), layout::bounds(item));
        if shown != Some(bounds) {
            self.0.push(format!(
                "{what} is not shown whole: {shown:?} of {bounds:?}"
            ));
        }
    }

    fn assert_none(self, surface: &str) {
        assert!(
            self.0.is_empty(),
            "{surface}: {} findings:\n{}",
            self.0.len(),
            self.0.join("\n")
        );
    }
}

/// The scroll offset that brings `start..start + size` of the content into the view
/// `view..view + extent`, from `offset`.
fn into_view(offset: f32, view: f32, extent: f32, start: f32, size: f32) -> f32 {
    if start < view {
        offset + (view - start)
    } else if start + size > view + extent {
        offset - (start + size - view - extent)
    } else {
        offset
    }
}

/// Scroll the nearest scroll view around `item`, both ways, so that all of it is in view.
fn reveal(item: &ItemRc) {
    let mut current = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(ancestor) = current {
        if let Some(flickable) = ancestor.downcast::<Flickable>() {
            let (left, top, width, height) = layout::bounds(&ancestor);
            let (x, y, w, h) = layout::bounds(item);
            let flickable = flickable.as_pin_ref();
            let moved_x = into_view(flickable.content_x().get(), left, width, x, w);
            let moved_y = into_view(flickable.content_y().get(), top, height, y, h);
            flickable
                .get_ref()
                .content_x
                .set(LogicalLength::new(moved_x));
            flickable
                .get_ref()
                .content_y
                .set(LogicalLength::new(moved_y));
            return;
        }
        current = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
}

#[test]
fn the_panel_sort_menu_fits_the_minimum_window() {
    let mut test = ui::TestUi::new();
    let mut failures = Failures::default();
    for case in CASES {
        apply(&mut test, case);
        ui::click_root(&test.window, "Left panel sort", AccessibleRole::Combobox);
        settle(&mut test);
        assert!(ui::has_popup(&test.window), "{case:?}: the Sort menu opens");
        failures.layout(test.window.window(), &format!("{case:?}"));
        // With the default touch font every choice shows whole, without scrolling.
        if case.2 == 15 {
            for choice in ["Name", "Size", "Modified", "Extension", "Reverse order"] {
                let row = ui::popup_item(&test.window, choice, AccessibleRole::Button);
                failures.whole(&row, &format!("{case:?}: {choice}"));
            }
        }
        ui::key(&test.window, Key::Escape);
        settle(&mut test);
        assert!(!ui::has_popup(&test.window), "{case:?}: Escape closes it");
    }
    test.window.hide().unwrap();
    failures.assert_none("The Sort menu");
}

#[test]
fn select_by_mask_keeps_its_footer_in_the_minimum_window() {
    let mut test = ui::TestUi::new();
    let window = test.window.clone_strong();
    let choices = window.global::<PanelChoices>();
    let mut failures = Failures::default();
    for (opening, case) in CASES.into_iter().enumerate() {
        apply(&mut test, case);
        for error in ["", "The mask \"[\" is not valid: a bracket is not closed."] {
            choices.set_mask_select(opening % 2 == 0);
            choices.set_mask_pattern("*".into());
            choices.set_mask_folders(false);
            choices.set_mask_error(error.into());
            choices.set_mask_token(format!("mask:{opening}:{}", error.len()).into());
            settle(&mut test);
            let what = format!("{case:?}, error {:?}", !error.is_empty());
            failures.layout(test.window.window(), &what);
            for footer in ["OK", "Cancel"] {
                let button = ui::popup_item(&test.window, footer, AccessibleRole::Button);
                failures.whole(&button, &format!("{what}: {footer}"));
            }
            // The mask field has the keyboard: it shows, or Slint sends it no key at all.
            let field = ui::popup_item(&test.window, "Selection mask", AccessibleRole::TextInput);
            failures.whole(&field, &format!("{what}: the mask field"));
            choices.set_mask_token("".into());
            settle(&mut test);
        }
    }
    test.window.hide().unwrap();
    failures.assert_none("Select by mask");
}

/// The window's own minimum size, from its root layout, as the gate sizes tool windows.
fn minimum_size(window: &slint::Window) -> (u32, u32) {
    let component = WindowInner::from_pub(window).component();
    let component = ItemTreeRc::borrow_pin(&component);
    let width = component.as_ref().layout_info(Orientation::Horizontal).min;
    let height = component.as_ref().layout_info(Orientation::Vertical).min;
    (width.max(1.0).ceil() as u32, height.max(1.0).ceil() as u32)
}

fn draw(native: &MinimalSoftwareWindow, width: u32, height: u32) {
    for _ in 0..2 {
        slint::platform::update_timers_and_animations();
        modeless::layout(native, width, height);
    }
}

#[test]
fn checksums_without_results_keep_their_note_clear_of_the_headings() {
    let windows = Rc::new(std::cell::RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let window = ChecksumWindow::new().unwrap();
    window.set_draft(ChecksumDraft {
        sha256: true,
        ..Default::default()
    });
    window.show().unwrap();
    let native = windows.borrow().last().unwrap().clone();
    window.invoke_begin();
    let mut failures = Failures::default();
    for (scheme, density, font) in CASES {
        window.invoke_apply_preferences(scheme, density, "".into(), font, font);
        draw(&native, 320, 200);
        let (width, height) = minimum_size(native.window());
        let size = (width.max(320), height.max(200));
        draw(&native, size.0, size.1);
        let case = format!("{:?} at {size:?}", (scheme, density, font));
        failures.layout(native.window(), &case);
    }
    window.hide().unwrap();
    failures.assert_none("Checksums");
}

#[test]
fn attributes_times_rows_and_pickers_fit_the_minimum_window() {
    let mut test = ui::TestUi::new();
    test.window.set_metadata_can_modified(true);
    test.window.set_metadata_can_created(true);
    test.window.set_metadata_can_accessed(true);
    test.window.set_metadata_token("times:1".into());
    settle(&mut test);
    ui::click_popup(&test.window, "Times", AccessibleRole::Button);
    let mut failures = Failures::default();
    for case in CASES {
        apply(&mut test, case);
        for label in ["Modified", "Created", "Accessed"] {
            let now = ui::popup_item(
                &test.window,
                &format!("{label}: Now"),
                AccessibleRole::Button,
            );
            reveal(&now);
            settle(&mut test);
            failures.layout(test.window.window(), &format!("{label} {case:?}"));
        }
        // The pickers open over the Modified row, as the gate shows them.
        for (picker, opened) in [
            ("Modified: choose date", "Modified calendar"),
            ("Modified: choose time", "Modified time picker"),
        ] {
            let button = ui::popup_item(&test.window, picker, AccessibleRole::Button);
            reveal(&button);
            settle(&mut test);
            ui::click_popup(&test.window, picker, AccessibleRole::Button);
            settle(&mut test);
            assert!(
                ui::try_popup_item(&test.window, opened, AccessibleRole::Groupbox).is_some(),
                "{case:?}: {picker} opens the {opened}"
            );
            failures.layout(test.window.window(), &format!("{picker} {case:?}"));
            ui::key(&test.window, Key::Escape);
            settle(&mut test);
        }
    }
    ui::key(&test.window, Key::Escape);
    test.window.hide().unwrap();
    failures.assert_none("Attributes › Times");
}

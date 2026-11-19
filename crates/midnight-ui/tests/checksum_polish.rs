use crate::support::modeless;
use i_slint_core::items::AccessibleRole;
use midnight_ui::{ChecksumDraft, ChecksumWindow};
use modeless::{Windows, accessible_item, click, draw};
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc};

#[test]
fn verification_glyphs_are_green_and_red_in_light_and_dark_schemes() {
    use midnight_ui::ChecksumDisplayRow;
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let window = ChecksumWindow::new().unwrap();
    window.set_verifying(true);
    window.set_rows(
        Rc::new(slint::VecModel::from(vec![
            ChecksumDisplayRow {
                name: "matching.txt".into(),
                status: "Verified".into(),
                verified: true,
                ..Default::default()
            },
            ChecksumDisplayRow {
                name: "changed.txt".into(),
                status: "Mismatch".into(),
                error: true,
                ..Default::default()
            },
        ]))
        .into(),
    );
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    for theme in [1, 2] {
        window.invoke_apply_preferences(theme, 0, "".into(), 13, 15);
        let pixels = modeless::render(&native, 860, 620, &format!("verification-colors-{theme}"));
        for (label, passed) in [
            ("Passed: matching.txt", true),
            ("Failed: changed.txt", false),
        ] {
            let icon = accessible_item(&native, AccessibleRole::Image, label).unwrap();
            let rect = icon.geometry();
            let position = icon.map_to_native_window(rect.origin);
            let mut colored = 0;
            for y in position.y.ceil() as usize..(position.y + rect.size.height).floor() as usize {
                for x in position.x.ceil() as usize..(position.x + rect.size.width).floor() as usize
                {
                    let pixel = pixels[y * 860 + x];
                    let [r, g, b] = [pixel.r, pixel.g, pixel.b].map(i32::from);
                    if if passed {
                        g > r + 20 && g > b + 20
                    } else {
                        r > g + 20 && r > b + 20
                    } {
                        colored += 1;
                    }
                }
            }
            assert!(
                colored >= 8,
                "{label} must visibly render in its result color, theme={theme}"
            );
        }
    }
    window.hide().unwrap();
}

#[test]
fn results_receive_the_window_instead_of_always_visible_configuration() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let window = ChecksumWindow::new().unwrap();
    window.invoke_apply_preferences(1, 0, "".into(), 13, 15);
    window.set_draft(ChecksumDraft {
        sha256: true,
        ..Default::default()
    });
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    draw(&native, 860, 620, "results-space");
    let list = accessible_item(&native, AccessibleRole::List, "Checksum results").unwrap();
    let rect = list.geometry();
    let position = list.map_to_native_window(rect.origin);
    assert!(position.y < 110.0, "Results begin too low: {position:?}");
    assert!(rect.size.height >= 440.0, "Results are squeezed: {rect:?}");
    window.hide().unwrap();
}

/// The footer button; the "Calculate" page tab has the same label and comes first.
fn click_calculate(native: &slint::platform::software_renderer::MinimalSoftwareWindow) {
    let buttons = modeless::accessible_items(native, AccessibleRole::Button, "Calculate");
    modeless::click_item(native, buttons.last().unwrap(), "Calculate");
}

fn window(windows: &Rc<RefCell<Vec<Rc<slint::platform::software_renderer::MinimalSoftwareWindow>>>>) -> ChecksumWindow {
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let window = ChecksumWindow::new().unwrap();
    window.invoke_apply_preferences(1, 0, "".into(), 13, 15);
    window
}

#[test]
fn there_are_only_calculate_and_verify_pages() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    let window = window(&windows);
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    draw(&native, 860, 620, "checksum-pages");
    for gone in ["Details", "Options", "Results", "Save manifest"] {
        assert!(
            accessible_item(&native, AccessibleRole::Button, gone).is_none(),
            "{gone} is no longer a page"
        );
    }
    window.hide().unwrap();
}

#[test]
fn save_button_opens_the_picker_with_the_shown_algorithm_and_waits_for_busy_work() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    let window = window(&windows);
    window.set_has_output(true);
    window.set_output_available(Rc::new(slint::VecModel::from(vec![true; 5])).into());
    window.set_output_algorithm(4);
    let choices = Rc::new(RefCell::new(Vec::new()));
    window.on_browse_manifest({
        let choices = choices.clone();
        move |verify, algorithm| choices.borrow_mut().push((verify, algorithm))
    });
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    draw(&native, 860, 620, "save-button");
    click(&native, "Save manifest…");
    assert_eq!(&*choices.borrow(), &[(false, 4)]);
    assert_eq!(window.get_page(), 0, "there is no save page to switch to");
    window.set_picker_busy(true);
    draw(&native, 860, 620, "save-picker-busy");
    click(&native, "Save manifest…");
    assert_eq!(choices.borrow().len(), 1);
    window.set_picker_busy(false);
    window.set_saving(true);
    draw(&native, 860, 620, "save-write-busy");
    click(&native, "Stop save");
    window.hide().unwrap();
}

#[test]
fn an_existing_file_or_partial_results_are_asked_about_in_the_window() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    let window = window(&windows);
    window.set_has_output(true);
    window.set_output_available(Rc::new(slint::VecModel::from(vec![true; 5])).into());
    let answers = Rc::new(RefCell::new(Vec::new()));
    window.on_confirm_save({
        let answers = answers.clone();
        move || answers.borrow_mut().push("confirm")
    });
    window.on_cancel_save_prompt({
        let answers = answers.clone();
        move || answers.borrow_mut().push("cancel")
    });
    window.set_save_prompt("checksums.sha256 already exists. Replace it?".into());
    window.set_save_prompt_action("Replace".into());
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    draw(&native, 860, 620, "save-replace-prompt");
    assert!(accessible_item(&native, AccessibleRole::Text, "checksums.sha256 already exists. Replace it?").is_some());
    click(&native, "Replace");
    click(&native, "Cancel");
    assert_eq!(&*answers.borrow(), &["confirm", "cancel"]);
    window.hide().unwrap();
}

#[test]
fn calculate_sends_the_chosen_checksums_and_needs_at_least_one() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    let window = window(&windows);
    let sent = Rc::new(RefCell::new(Vec::new()));
    window.on_calculate({
        let sent = sent.clone();
        move |draft| sent.borrow_mut().push((draft.sha256, draft.md5, draft.recursive))
    });
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    draw(&native, 860, 620, "calculate-without-algorithm");
    click_calculate(&native);
    assert!(sent.borrow().is_empty(), "no algorithm is chosen yet");
    window.set_draft(ChecksumDraft {
        sha256: true,
        md5: true,
        recursive: true,
        ..Default::default()
    });
    draw(&native, 860, 620, "calculate-with-algorithms");
    click_calculate(&native);
    assert_eq!(&*sent.borrow(), &[(true, true, true)]);
    window.hide().unwrap();
}

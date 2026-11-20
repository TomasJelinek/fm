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

fn window(
    windows: &Rc<RefCell<Vec<Rc<slint::platform::software_renderer::MinimalSoftwareWindow>>>>,
) -> ChecksumWindow {
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
    assert!(
        accessible_item(
            &native,
            AccessibleRole::Text,
            "checksums.sha256 already exists. Replace it?"
        )
        .is_some()
    );
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
        move |draft| {
            sent.borrow_mut()
                .push((draft.sha256, draft.md5, draft.recursive))
        }
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

fn key(
    native: &slint::platform::software_renderer::MinimalSoftwareWindow,
    text: impl Into<slint::SharedString>,
) {
    use slint::platform::{WindowAdapter, WindowEvent};
    let text = text.into();
    native
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    native
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

#[test]
fn the_algorithm_menu_works_from_the_keyboard() {
    use slint::platform::Key;
    let windows = Rc::new(RefCell::new(Vec::new()));
    let window = window(&windows);
    window.set_draft(ChecksumDraft {
        sha256: true,
        ..Default::default()
    });
    window.set_output_available(
        Rc::new(slint::VecModel::from(vec![true, true, false, false, false])).into(),
    );
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    draw(&native, 860, 620, "algorithm-menu-keyboard");
    // A click focuses the menu as well as opening it; Escape closes it again.
    click(&native, "Checksum algorithms");
    key(&native, Key::Escape);
    assert!(window.window().is_visible(), "Escape closed only the menu");
    // Keyboard alone from here: Down opens the menu on the shown algorithm.
    key(&native, Key::DownArrow);
    key(&native, Key::DownArrow);
    key(&native, Key::DownArrow);
    key(&native, " "); // MD5: included in the next calculation.
    key(&native, Key::End);
    key(&native, Key::Return); // Include subfolders.
    key(&native, Key::Home);
    key(&native, Key::DownArrow);
    key(&native, Key::Return); // Show SHA-512 and close.
    let draft = window.get_draft();
    assert!(draft.sha256 && draft.md5 && draft.recursive, "{draft:?}");
    assert_eq!(window.get_output_algorithm(), 1);
    window.hide().unwrap();
}

/// Counts the window's close requests: Escape asks the owner to close the session.
fn close_requests(window: &ChecksumWindow) -> Rc<std::cell::Cell<usize>> {
    let closed = Rc::new(std::cell::Cell::new(0));
    window.on_close_session({
        let closed = closed.clone();
        move || closed.set(closed.get() + 1)
    });
    closed
}

#[test]
fn escape_closes_a_window_opened_on_the_verify_page() {
    use slint::platform::Key;
    let windows = Rc::new(RefCell::new(Vec::new()));
    let window = window(&windows);
    let closed = close_requests(&window);
    // As for Ctrl+F8 with a checksum file selected: the Verify page from the start.
    window.set_manifest_path("checksums.sha256".into());
    window.set_page(1);
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    window.invoke_begin();
    draw(&native, 860, 620, "verify-page-escape");
    key(&native, Key::Escape);
    assert_eq!(
        closed.get(),
        1,
        "Escape must close a window opened on Verify"
    );
    window.hide().unwrap();
}

#[test]
fn escape_closes_the_window_after_the_save_prompt_is_answered() {
    use slint::platform::Key;
    let windows = Rc::new(RefCell::new(Vec::new()));
    let window = window(&windows);
    let closed = close_requests(&window);
    window.set_has_output(true);
    window.set_output_available(Rc::new(slint::VecModel::from(vec![true; 5])).into());
    // As the presenter does, an answer removes the prompt with its buttons.
    window.on_confirm_save({
        let weak = window.as_weak();
        move || weak.unwrap().set_save_prompt("".into())
    });
    window.on_cancel_save_prompt({
        let weak = window.as_weak();
        move || weak.unwrap().set_save_prompt("".into())
    });
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    window.invoke_begin();
    for (answered, answer) in ["Replace", "Cancel"].into_iter().enumerate() {
        window.set_save_prompt("checksums.sha256 already exists. Replace it?".into());
        window.set_save_prompt_action("Replace".into());
        draw(&native, 860, 620, &format!("save-prompt-{answer}"));
        // A click focuses the button before its row goes away.
        click(&native, answer);
        draw(&native, 860, 620, &format!("save-prompt-{answer}-answered"));
        assert!(accessible_item(&native, AccessibleRole::Button, answer).is_none());
        key(&native, Key::Escape);
        assert_eq!(closed.get(), answered + 1, "Escape after {answer}");
    }
    window.hide().unwrap();
}

#[test]
fn escape_closes_the_window_after_details_back_and_dismiss_remove_their_buttons() {
    use slint::platform::Key;
    let windows = Rc::new(RefCell::new(Vec::new()));
    let window = window(&windows);
    let closed = close_requests(&window);
    window.set_attention(true);
    window.set_issue("Checksum manifest mixes different algorithms.".into());
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    window.invoke_begin();
    for (used, control) in ["Details", "Back", "Dismiss checksum warning"]
        .into_iter()
        .enumerate()
    {
        draw(&native, 860, 620, &format!("checksum-attention-{used}"));
        click(&native, control);
        draw(
            &native,
            860,
            620,
            &format!("checksum-attention-{used}-used"),
        );
        assert!(accessible_item(&native, AccessibleRole::Button, control).is_none());
        key(&native, Key::Escape);
        assert_eq!(closed.get(), used + 1, "Escape after {control}");
    }
    window.hide().unwrap();
}
